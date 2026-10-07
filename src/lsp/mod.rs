//! Hand-written stdio JSON-RPC client for markdown language servers (zk, marksman).
//!
//! One process per (server, server root), started with its working directory
//! set to the server root. One reader thread per server routes responses (by
//! request id, carrying the caller's opaque `tag`), `publishDiagnostics`
//! notifications, and server-to-client requests. Everything the app needs
//! arrives as an [`LspEvent`] on the channel passed to [`Client::start`].

mod framing;
mod position;
mod uri;

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use lsp_types::{Diagnostic, ServerCapabilities, Uri};
use serde_json::{Value, json};

pub use framing::{read_message, write_message};
pub use position::{Encoding, byte_to_position, from_byte, position_to_byte, to_byte};
pub use uri::{canonical_uri, uri_to_path};

const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(10);
const INDEX_TIMEOUT: Duration = Duration::from_secs(30);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const STDERR_LINES: usize = 50;

/// Which built-in behaviour a server gets (startup steps, default encoding).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Zk,
    Marksman,
    Generic,
}

/// How to run one language server. Independent of the config module, which
/// maps its own server entries into this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSpec {
    pub name: String,
    pub kind: Kind,
    pub command: Vec<String>,
    pub root_markers: Vec<String>,
    pub encoding_override: Option<Encoding>,
}

/// What the reader thread reports.
#[derive(Debug, Clone)]
pub enum LspEvent {
    /// The reply to [`Client::request`]; `result` is the error message on failure.
    Response {
        server: String,
        id: u64,
        tag: u64,
        result: Result<Value, String>,
    },
    Diagnostics {
        server: String,
        uri: Uri,
        version: Option<i32>,
        diagnostics: Vec<Diagnostic>,
    },
    /// The server process ended. `error` is `None` after [`Client::shutdown`]
    /// or drop, else a description with the last lines of its stderr.
    Exited {
        server: String,
        error: Option<String>,
    },
}

/// The first spec (in order) with a root marker in an ancestor directory of
/// `file`; the server root is the nearest such directory.
pub fn select_server<'a>(
    servers: &'a [ServerSpec],
    file: &Path,
) -> Option<(&'a ServerSpec, PathBuf)> {
    let file = uri::canonical_path(file);
    let start = if file.is_dir() {
        file.as_path()
    } else {
        file.parent()?
    };
    servers.iter().find_map(|spec| {
        start
            .ancestors()
            .find(|dir| spec.root_markers.iter().any(|m| dir.join(m).exists()))
            .map(|dir| (spec, dir.to_path_buf()))
    })
}

type Reply = Result<Value, String>;

enum Pending {
    /// Sent by [`Client::request`]: reply goes to the event channel.
    Tagged(u64),
    /// Sent internally: reply goes to a waiting caller.
    Sync(Sender<Reply>),
}

struct Shared {
    name: String,
    stdin: Mutex<Option<ChildStdin>>,
    pending: Mutex<HashMap<u64, Pending>>,
    child: Mutex<Child>,
    stderr_tail: Mutex<VecDeque<String>>,
    /// Set when we stop the server, so its exit is not an error.
    stopping: AtomicBool,
}

impl Shared {
    fn send(&self, msg: &Value) -> anyhow::Result<()> {
        let mut stdin = self.stdin.lock().unwrap();
        let w = stdin
            .as_mut()
            .ok_or_else(|| anyhow!("{}: server stdin closed", self.name))?;
        write_message(w, msg).with_context(|| format!("{}: write to server", self.name))
    }
}

/// A running language server.
pub struct Client {
    shared: Arc<Shared>,
    next_id: AtomicU64,
    encoding: Encoding,
    capabilities: ServerCapabilities,
    index_result: Option<Value>,
}

impl Client {
    /// Spawns the server (cwd = `root`), runs `initialize`/`initialized`, and
    /// for [`Kind::Zk`] runs `zk.index` on `root`, waiting for each reply.
    pub fn start(
        spec: &ServerSpec,
        root: &Path,
        events: Sender<LspEvent>,
    ) -> anyhow::Result<Client> {
        let (program, args) = spec
            .command
            .split_first()
            .ok_or_else(|| anyhow!("{}: empty server command", spec.name))?;
        let mut child = Command::new(program)
            .args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("{}: cannot start {program:?}", spec.name))?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");

        let shared = Arc::new(Shared {
            name: spec.name.clone(),
            stdin: Mutex::new(stdin),
            pending: Mutex::new(HashMap::new()),
            child: Mutex::new(child),
            stderr_tail: Mutex::new(VecDeque::new()),
            stopping: AtomicBool::new(false),
        });

        let (stderr_done_tx, stderr_done) = mpsc::channel();
        {
            let shared = Arc::clone(&shared);
            thread::spawn(move || drain_stderr(stderr, &shared, stderr_done_tx));
        }
        {
            let shared = Arc::clone(&shared);
            thread::spawn(move || read_loop(BufReader::new(stdout), &shared, &events, stderr_done));
        }

        let mut client = Client {
            shared,
            next_id: AtomicU64::new(1),
            encoding: Encoding::Utf16,
            capabilities: ServerCapabilities::default(),
            index_result: None,
        };

        let root_uri = canonical_uri(root);
        let root_name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let init = client
            .request_sync(
                "initialize",
                json!({
                    "processId": std::process::id(),
                    "clientInfo": {"name": "ramble", "version": env!("CARGO_PKG_VERSION")},
                    "rootUri": root_uri,
                    "workspaceFolders": [{"uri": root_uri, "name": root_name}],
                    "capabilities": {
                        "general": {"positionEncodings": ["utf-8", "utf-16"]},
                        "textDocument": {
                            "publishDiagnostics": {"versionSupport": true},
                            "synchronization": {"didSave": false},
                        },
                        "workspace": {"configuration": true, "workspaceFolders": true},
                    },
                }),
                INITIALIZE_TIMEOUT,
            )
            .with_context(|| format!("{}: initialize", spec.name))?;
        client.capabilities = init
            .get("capabilities")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .with_context(|| format!("{}: parse server capabilities", spec.name))?
            .unwrap_or_default();
        let negotiated = client
            .capabilities
            .position_encoding
            .as_ref()
            .and_then(|k| Encoding::from_lsp(k.as_str()));
        let kind_default = match spec.kind {
            Kind::Zk => Encoding::Utf32,
            Kind::Marksman | Kind::Generic => Encoding::Utf16,
        };
        client.encoding = spec
            .encoding_override
            .or(negotiated)
            .unwrap_or(kind_default);

        client.notify("initialized", json!({}))?;

        if spec.kind == Kind::Zk {
            let root_arg = uri::canonical_path(root).to_string_lossy().into_owned();
            let stats = client
                .request_sync(
                    "workspace/executeCommand",
                    json!({"command": "zk.index", "arguments": [root_arg]}),
                    INDEX_TIMEOUT,
                )
                .with_context(|| format!("{}: zk.index", spec.name))?;
            client.index_result = Some(stats);
        }
        Ok(client)
    }

    /// The position encoding used for this server.
    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// The capabilities from the server's `initialize` reply.
    pub fn capabilities(&self) -> &ServerCapabilities {
        &self.capabilities
    }

    /// The reply to `zk.index` sent on start (zk only), e.g. `{"sourceCount": 3, ..}`.
    pub fn index_result(&self) -> Option<&Value> {
        self.index_result.as_ref()
    }

    /// Sends a request without waiting; the reply arrives as
    /// [`LspEvent::Response`] carrying `tag`. Returns the request id.
    pub fn request(&self, method: &str, params: Value, tag: u64) -> anyhow::Result<u64> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.shared
            .pending
            .lock()
            .unwrap()
            .insert(id, Pending::Tagged(tag));
        let sent = self.shared.send(&json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": params,
        }));
        if sent.is_err() {
            self.shared.pending.lock().unwrap().remove(&id);
        }
        sent.map(|()| id)
    }

    /// Sends a notification.
    pub fn notify(&self, method: &str, params: Value) -> anyhow::Result<()> {
        self.shared.send(&json!({
            "jsonrpc": "2.0", "method": method, "params": params,
        }))
    }

    /// `textDocument/didOpen` with `languageId = "markdown"`. To reload, call
    /// again with a higher `version`.
    pub fn did_open(&self, path: &Path, text: &str, version: i32) -> anyhow::Result<()> {
        self.notify(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": canonical_uri(path),
                "languageId": "markdown",
                "version": version,
                "text": text,
            }}),
        )
    }

    /// `shutdown` then `exit`; kills the process if it has not exited after 2s.
    pub fn shutdown(self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        let _ = self.request_sync("shutdown", Value::Null, SHUTDOWN_TIMEOUT);
        let _ = self.notify("exit", Value::Null);
        // Closing stdin also tells a server that ignores `exit` to stop.
        self.shared.stdin.lock().unwrap().take();
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.shared.child.lock().unwrap().try_wait() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        // Drop kills it.
    }

    fn request_sync(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> anyhow::Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.shared
            .pending
            .lock()
            .unwrap()
            .insert(id, Pending::Sync(tx));
        self.shared.send(&json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": params,
        }))?;
        match rx.recv_timeout(timeout) {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => bail!("{e}"),
            Err(RecvTimeoutError::Timeout) => {
                self.shared.pending.lock().unwrap().remove(&id);
                bail!("no reply within {timeout:?}")
            }
            Err(RecvTimeoutError::Disconnected) => {
                let tail = stderr_tail(&self.shared);
                bail!("server exited{}", tail_suffix(&tail))
            }
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        self.shared.stdin.lock().unwrap().take();
        let mut child = self.shared.child.lock().unwrap();
        if let Ok(None) = child.try_wait() {
            let _ = child.kill();
        }
        let _ = child.wait();
    }
}

fn drain_stderr(stderr: impl std::io::Read, shared: &Shared, done: Sender<()>) {
    for line in BufReader::new(stderr).lines() {
        let Ok(line) = line else { break };
        let mut tail = shared.stderr_tail.lock().unwrap();
        if tail.len() == STDERR_LINES {
            tail.pop_front();
        }
        tail.push_back(line);
    }
    let _ = done.send(());
}

fn stderr_tail(shared: &Shared) -> String {
    let tail = shared.stderr_tail.lock().unwrap();
    tail.iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("\n")
}

fn tail_suffix(tail: &str) -> String {
    if tail.is_empty() {
        String::new()
    } else {
        format!("; stderr:\n{tail}")
    }
}

fn read_loop(
    mut stdout: impl BufRead,
    shared: &Shared,
    events: &Sender<LspEvent>,
    stderr_done: Receiver<()>,
) {
    let read_error = loop {
        match read_message(&mut stdout) {
            Ok(Some(msg)) => handle_message(msg, shared, events),
            Ok(None) => break None,
            Err(e) => break Some(e.to_string()),
        }
    };
    let stopping = shared.stopping.load(Ordering::SeqCst);
    if !stopping {
        // Collect the stderr tail before waiters look at it.
        let _ = stderr_done.recv_timeout(Duration::from_millis(500));
    }
    // Drop waiters so blocked `request_sync` calls see the exit.
    let orphans: Vec<_> = shared.pending.lock().unwrap().drain().collect();
    drop(orphans);

    let error = if stopping {
        None
    } else {
        let status = wait_status(shared, Duration::from_millis(500));
        let mut msg = match (read_error, status) {
            (Some(e), _) => format!("protocol error: {e}"),
            (None, Some(s)) => format!("server exited ({s})"),
            (None, None) => "server closed its output".to_string(),
        };
        msg.push_str(&tail_suffix(&stderr_tail(shared)));
        Some(msg)
    };
    let _ = events.send(LspEvent::Exited {
        server: shared.name.clone(),
        error,
    });
}

fn wait_status(shared: &Shared, timeout: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(Some(s)) = shared.child.lock().unwrap().try_wait() {
            return Some(s);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn handle_message(msg: Value, shared: &Shared, events: &Sender<LspEvent>) {
    let method = msg.get("method").and_then(Value::as_str);
    let id = msg.get("id").filter(|id| !id.is_null());
    match (method, id) {
        (Some(method), Some(id)) => answer_server_request(method, id.clone(), &msg, shared),
        (Some("textDocument/publishDiagnostics"), None) => {
            let Some(params) = msg.get("params").cloned() else {
                return;
            };
            if let Ok(p) = serde_json::from_value::<lsp_types::PublishDiagnosticsParams>(params) {
                let _ = events.send(LspEvent::Diagnostics {
                    server: shared.name.clone(),
                    uri: p.uri,
                    version: p.version,
                    diagnostics: p.diagnostics,
                });
            }
        }
        (Some(_), None) => {} // Other notifications ($/progress, window/logMessage, ...).
        (None, Some(id)) => {
            let Some(id) = id.as_u64() else { return };
            let result = match msg.get("error") {
                Some(err) => Err(err
                    .get("message")
                    .and_then(Value::as_str)
                    .map_or_else(|| err.to_string(), str::to_owned)),
                None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
            };
            let pending = shared.pending.lock().unwrap().remove(&id);
            match pending {
                Some(Pending::Tagged(tag)) => {
                    let _ = events.send(LspEvent::Response {
                        server: shared.name.clone(),
                        id,
                        tag,
                        result,
                    });
                }
                Some(Pending::Sync(tx)) => {
                    let _ = tx.send(result);
                }
                None => {}
            }
        }
        (None, None) => {}
    }
}

fn answer_server_request(method: &str, id: Value, msg: &Value, shared: &Shared) {
    let reply = match method {
        "workspace/configuration" => {
            let n = msg
                .pointer("/params/items")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            json!({"jsonrpc": "2.0", "id": id, "result": vec![Value::Null; n]})
        }
        "window/workDoneProgress/create" | "client/registerCapability" => {
            json!({"jsonrpc": "2.0", "id": id, "result": null})
        }
        _ => json!({
            "jsonrpc": "2.0", "id": id,
            "error": {"code": -32601, "message": format!("method not found: {method}")},
        }),
    };
    let _ = shared.send(&reply);
}
