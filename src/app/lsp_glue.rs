//! Connects [`crate::lsp::Client`] to the app (spec § Data flow): one client
//! per (server, server root), started lazily off the UI thread; `didOpen` +
//! `documentLink` on every open and reload; link targets, broken links
//! (diagnostics), `gd` via documentLink / definition, and `K` hover.
//!
//! Every request carries a tag `(page_id << 32) | version`; a reply for
//! another page or an older version is dropped.

use std::collections::{BTreeSet, HashMap};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::App;
use crate::config::{PositionEncoding, ServerConfig, ServerKind};
use crate::doc::LinkKind;
use crate::lsp::{
    Client, Encoding, Kind, LspEvent, ServerSpec, byte_to_position, canonical_uri,
    position_to_byte, select_server, uri_to_path,
};
use crate::nav::{self, Target};

/// A request pending this long shows the spinner in the status line.
pub const SPINNER_AFTER: Duration = Duration::from_secs(2);
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Maps a config server entry to a client spec. `name` is filled in per
/// server root by [`instance_spec`].
pub fn server_spec(c: &ServerConfig) -> ServerSpec {
    ServerSpec {
        name: kind_label(c.kind).into(),
        kind: match c.kind {
            ServerKind::Zk => Kind::Zk,
            ServerKind::Marksman => Kind::Marksman,
            ServerKind::Generic => Kind::Generic,
        },
        command: c.command.clone(),
        root_markers: c.root_markers.clone(),
        encoding_override: c.position_encoding.map(|e| match e {
            PositionEncoding::Utf8 => Encoding::Utf8,
            PositionEncoding::Utf16 => Encoding::Utf16,
            PositionEncoding::Utf32 => Encoding::Utf32,
        }),
    }
}

fn kind_label(kind: ServerKind) -> &'static str {
    match kind {
        ServerKind::Zk => "zk",
        ServerKind::Marksman => "marksman",
        ServerKind::Generic => "generic",
    }
}

fn spec_kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Zk => "zk",
        Kind::Marksman => "marksman",
        Kind::Generic => "generic",
    }
}

/// The spec for one (server, root) instance: name `"<kind>@<root>"`.
fn instance_spec(spec: &ServerSpec, root: &Path) -> ServerSpec {
    ServerSpec {
        name: format!("{}@{}", spec_kind_label(spec.kind), root.display()),
        ..spec.clone()
    }
}

/// The request tag for `page_id` and document `version`.
pub fn tag(page_id: u64, version: i32) -> u64 {
    (page_id << 32) | u64::from(version as u32)
}

enum Instance {
    Starting,
    Running(Box<Client>),
    /// Crashed or failed to start: no-LSP for this root, never respawned.
    Dead,
}

/// What background threads send.
enum Incoming {
    Event(LspEvent),
    Started(String, Result<Box<Client>, String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReqKind {
    DocumentLink,
    /// `gd` on link `i`.
    Definition(usize),
    Hover,
}

struct Request {
    id: u64,
    kind: ReqKind,
    sent_at: Instant,
}

/// LSP state owned by [`App`].
pub(super) struct LspState {
    specs: Vec<ServerSpec>,
    instances: HashMap<String, Instance>,
    tx: Sender<Incoming>,
    rx: Receiver<Incoming>,
    /// Last version sent per canonical path; never reset this session.
    versions: HashMap<PathBuf, i32>,
    /// Bumped on every page shown (open, follow, back/forward, reload).
    page_id: u64,
    // Per page, reset by `lsp_page_changed`.
    /// Instance name and kind for the current page.
    instance: Option<(String, Kind)>,
    version: Option<i32>,
    /// documentLink target per local link index.
    targets: HashMap<usize, PathBuf>,
    /// Byte ranges of the current page's diagnostics.
    diagnostics: Vec<Range<usize>>,
    broken: BTreeSet<usize>,
    requests: Vec<Request>,
    hover: Option<String>,
}

impl LspState {
    pub(super) fn new(servers: &[ServerConfig]) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            specs: servers.iter().map(server_spec).collect(),
            instances: HashMap::new(),
            tx,
            rx,
            versions: HashMap::new(),
            page_id: 0,
            instance: None,
            version: None,
            targets: HashMap::new(),
            diagnostics: Vec::new(),
            broken: BTreeSet::new(),
            requests: Vec::new(),
            hover: None,
        }
    }

    fn client(&self) -> Option<&Client> {
        let (name, _) = self.instance.as_ref()?;
        match self.instances.get(name)? {
            Instance::Running(c) => Some(c),
            _ => None,
        }
    }
}

fn start_in_background(spec: ServerSpec, root: PathBuf, tx: Sender<Incoming>) {
    std::thread::spawn(move || {
        let (ev_tx, ev_rx) = mpsc::channel::<LspEvent>();
        let fwd = tx.clone();
        std::thread::spawn(move || {
            for ev in ev_rx {
                if fwd.send(Incoming::Event(ev)).is_err() {
                    return;
                }
            }
        });
        let result = Client::start(&spec, &root, ev_tx)
            .map(Box::new)
            .map_err(|e| format!("{e:#}"));
        let _ = tx.send(Incoming::Started(spec.name, result));
    });
}

/// Plain text of a hover `contents` value (MarkupContent, MarkedString, or
/// an array of MarkedStrings).
fn hover_text(contents: &Value) -> String {
    match contents {
        Value::String(s) => s.clone(),
        Value::Array(xs) => xs
            .iter()
            .map(hover_text)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        Value::Object(o) => o
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    }
}

/// The first file of a definition reply (Location, Location[], LocationLink[]).
fn definition_path(v: &Value) -> Option<PathBuf> {
    let first = match v {
        Value::Array(xs) => xs.first()?,
        other => other,
    };
    let uri = first
        .get("uri")
        .or_else(|| first.get("targetUri"))?
        .as_str()?;
    uri_to_path(&uri.parse().ok()?)
}

impl App {
    /// Called by `set_page` for every page shown: a new page id, fresh
    /// per-page LSP state, then `didOpen` + `documentLink` (starting the
    /// server first if needed). Stdin pages get no LSP.
    pub(super) fn lsp_page_changed(&mut self) {
        let l = &mut self.lsp;
        l.page_id += 1;
        l.instance = None;
        l.version = None;
        l.targets.clear();
        l.diagnostics.clear();
        l.broken.clear();
        l.requests.clear();
        l.hover = None;
        let Some(path) = self.page.as_ref().and_then(|p| p.path.clone()) else {
            return;
        };
        let Some((spec, root)) = select_server(&l.specs, &path) else {
            return;
        };
        let spec = instance_spec(spec, &root);
        l.instance = Some((spec.name.clone(), spec.kind));
        if !l.instances.contains_key(&spec.name) {
            l.instances.insert(spec.name.clone(), Instance::Starting);
            start_in_background(spec, root, l.tx.clone());
            return;
        }
        self.lsp_open_current();
    }

    /// Send `didOpen` (next version) and `documentLink` for the current page,
    /// if its server is running.
    fn lsp_open_current(&mut self) {
        let Some(page) = &self.page else { return };
        let Some(path) = &page.path else { return };
        let l = &mut self.lsp;
        let Some(client) = l.client() else { return };
        let key = crate::lsp::uri_to_path(&canonical_uri(path)).unwrap_or_else(|| path.clone());
        let version = l.versions.get(&key).map_or(1, |v| v + 1);
        if let Err(e) = client.did_open(path, &page.doc.source, version) {
            self.status = format!("LSP: {e:#}");
            return;
        }
        let tag = tag(l.page_id, version);
        let sent = client.request(
            "textDocument/documentLink",
            json!({"textDocument": {"uri": canonical_uri(path)}}),
            tag,
        );
        l.versions.insert(key, version);
        l.version = Some(version);
        if let Ok(id) = sent {
            l.requests.push(Request {
                id,
                kind: ReqKind::DocumentLink,
                sent_at: self.now,
            });
        }
    }

    /// Apply pending LSP events, blocking up to `timeout` for the first.
    /// Returns how many were applied. `run` calls it every loop iteration.
    pub fn pump_lsp(&mut self, timeout: Duration) -> usize {
        let first = match self.lsp.rx.recv_timeout(timeout) {
            Ok(m) => m,
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return 0,
        };
        self.incoming(first);
        let mut n = 1;
        while let Ok(m) = self.lsp.rx.try_recv() {
            self.incoming(m);
            n += 1;
        }
        n
    }

    fn incoming(&mut self, m: Incoming) {
        match m {
            Incoming::Event(ev) => self.lsp_event(ev),
            Incoming::Started(name, Ok(client)) => {
                self.lsp
                    .instances
                    .insert(name.clone(), Instance::Running(client));
                if self.lsp.instance.as_ref().is_some_and(|(n, _)| *n == name) {
                    self.lsp_open_current();
                }
            }
            Incoming::Started(name, Err(e)) => {
                self.lsp.instances.insert(name, Instance::Dead);
                self.status = format!("LSP failed to start: {}", first_line(&e));
            }
        }
    }

    /// One event from a language server.
    pub fn lsp_event(&mut self, ev: LspEvent) {
        match ev {
            LspEvent::Response {
                server,
                id,
                tag: t,
                result,
            } => self.lsp_response(&server, id, t, result),
            LspEvent::Diagnostics {
                server,
                uri,
                version,
                diagnostics,
            } => {
                if !self.is_current_instance(&server) {
                    return;
                }
                let Some(page) = &self.page else { return };
                let Some(path) = &page.path else { return };
                if uri_to_path(&uri) != uri_to_path(&canonical_uri(path)) {
                    return;
                }
                if version.is_some() && version != self.lsp.version {
                    return;
                }
                let Some(enc) = self.lsp.client().map(Client::encoding) else {
                    return;
                };
                let src = &page.doc.source;
                self.lsp.diagnostics = diagnostics
                    .iter()
                    .map(|d| {
                        position_to_byte(src, d.range.start, enc)
                            ..position_to_byte(src, d.range.end, enc)
                    })
                    .collect();
                self.lsp.broken = page
                    .doc
                    .links
                    .iter()
                    .enumerate()
                    .filter(|(_, link)| link.kind != LinkKind::CodePath)
                    .filter(|(_, link)| {
                        self.lsp
                            .diagnostics
                            .iter()
                            .any(|r| overlaps(r, &link.range))
                    })
                    .map(|(i, _)| i)
                    .collect();
            }
            LspEvent::Exited { server, error } => {
                self.lsp.instances.insert(server.clone(), Instance::Dead);
                if self.is_current_instance(&server) {
                    self.lsp.requests.clear();
                }
                let kind = server.split('@').next().unwrap_or(&server);
                self.status = match error {
                    Some(e) => format!("{kind}: {}; LSP off", first_line(&e)),
                    None => format!("{kind}: stopped; LSP off"),
                };
            }
        }
    }

    fn is_current_instance(&self, server: &str) -> bool {
        self.lsp.instance.as_ref().is_some_and(|(n, _)| n == server)
    }

    fn lsp_response(&mut self, server: &str, id: u64, t: u64, result: Result<Value, String>) {
        if t & super::picker::PICKER_TAG_BASE != 0 {
            if self.is_current_instance(server) {
                self.picker_response(t, result);
            }
            return;
        }
        let current = self
            .lsp
            .version
            .is_some_and(|v| t == tag(self.lsp.page_id, v));
        let Some(pos) = self.lsp.requests.iter().position(|r| r.id == id) else {
            return;
        };
        if !current || !self.is_current_instance(server) {
            // Another page or an older version: drop.
            self.lsp.requests.remove(pos);
            return;
        }
        let req = self.lsp.requests.remove(pos);
        match req.kind {
            ReqKind::DocumentLink => {
                if let Ok(v) = result {
                    self.apply_document_links(&v);
                }
            }
            ReqKind::Definition(i) => {
                if self.link_under_cursor() != Some(i) {
                    return; // The cursor moved: drop.
                }
                match result.ok().as_ref().and_then(definition_path) {
                    Some(path) => self.follow_lsp_path(i, path),
                    None => self.follow_local(),
                }
            }
            ReqKind::Hover => {
                let text = result
                    .ok()
                    .and_then(|v| v.get("contents").map(hover_text))
                    .unwrap_or_default();
                if text.trim().is_empty() {
                    self.status = "No hover information".into();
                } else {
                    self.lsp.hover = Some(text.trim().to_string());
                }
            }
        }
    }

    /// Match each documentLink range to the local link it overlaps.
    fn apply_document_links(&mut self, v: &Value) {
        let Some(enc) = self.lsp.client().map(Client::encoding) else {
            return;
        };
        let Some(page) = &self.page else { return };
        let src = &page.doc.source;
        for dl in v.as_array().into_iter().flatten() {
            let Ok(range) = serde_json::from_value::<lsp_types::Range>(dl["range"].clone()) else {
                continue;
            };
            let Some(path) = dl["target"]
                .as_str()
                .and_then(|s| s.parse().ok())
                .and_then(|u| uri_to_path(&u))
            else {
                continue;
            };
            let r = position_to_byte(src, range.start, enc)..position_to_byte(src, range.end, enc);
            let matched = page
                .doc
                .links
                .iter()
                .position(|l| l.kind != LinkKind::CodePath && overlaps(&r, &l.range));
            if let Some(i) = matched {
                self.lsp.targets.insert(i, path);
            }
        }
    }

    /// Open `path` for link `i`, taking the `#anchor` from the local parse.
    fn follow_lsp_path(&mut self, i: usize, path: PathBuf) {
        let Some(link) = self.page.as_ref().and_then(|p| p.doc.links.get(i)).cloned() else {
            return;
        };
        let anchor = match nav::resolve(&link.dest, &link.kind, &self.link_dir()) {
            Target::File { anchor, .. } => anchor,
            _ => None,
        };
        let as_written = link.dest.split('#').next().unwrap_or("").to_string();
        self.open_link_file(path, anchor, &as_written);
    }

    /// `gd` through the language server. True when handled (followed, or a
    /// definition request is on its way); false to use the local parse.
    pub(super) fn lsp_follow(&mut self) -> bool {
        let Some(i) = self.link_under_cursor() else {
            return false;
        };
        let Some(link) = self.page.as_ref().and_then(|p| p.doc.links.get(i)).cloned() else {
            return false;
        };
        // Code paths are resolved locally (no language server knows them).
        if link.kind == LinkKind::CodePath {
            return false;
        }
        // Anchors and URLs always come from the local parse.
        if !matches!(
            nav::resolve(&link.dest, &link.kind, &self.link_dir()),
            Target::File { .. }
        ) {
            return false;
        }
        if let Some(path) = self.lsp.targets.get(&i).cloned() {
            self.follow_lsp_path(i, path);
            return true;
        }
        let has_definition = self.lsp.client().is_some_and(|c| {
            c.capabilities()
                .definition_provider
                .as_ref()
                .is_some_and(|p| !matches!(p, lsp_types::OneOf::Left(false)))
        });
        has_definition
            && self.lsp_link_request("textDocument/definition", i, ReqKind::Definition(i))
    }

    /// `K`: hover for the link under the cursor.
    pub(super) fn hover(&mut self) {
        let Some(i) = self.link_under_cursor() else {
            self.set_status("No link under cursor");
            return;
        };
        let has_hover = self.lsp.client().is_some_and(|c| {
            c.capabilities()
                .hover_provider
                .as_ref()
                .is_some_and(|p| !matches!(p, lsp_types::HoverProviderCapability::Simple(false)))
        });
        if !has_hover || !self.lsp_link_request("textDocument/hover", i, ReqKind::Hover) {
            self.set_status("No hover (no language server)");
        }
    }

    /// Send `method` at link `i`'s text. True when sent.
    fn lsp_link_request(&mut self, method: &str, i: usize, kind: ReqKind) -> bool {
        let Some(page) = &self.page else { return false };
        let (Some(path), Some(link)) = (&page.path, page.doc.links.get(i)) else {
            return false;
        };
        let (Some(client), Some(version)) = (self.lsp.client(), self.lsp.version) else {
            return false;
        };
        let pos = byte_to_position(&page.doc.source, link.text_range.start, client.encoding());
        let sent = client.request(
            method,
            json!({"textDocument": {"uri": canonical_uri(path)}, "position": pos}),
            tag(self.lsp.page_id, version),
        );
        let Ok(id) = sent else { return false };
        // One definition / hover at a time: a new one replaces the old.
        self.lsp
            .requests
            .retain(|r| r.kind == ReqKind::DocumentLink);
        self.lsp.requests.push(Request {
            id,
            kind,
            sent_at: self.now,
        });
        true
    }

    /// The current page's running server: client, kind, and server root.
    pub(super) fn lsp_running(&self) -> Option<(&Client, Kind, PathBuf)> {
        let (name, kind) = self.lsp.instance.as_ref()?;
        let root = name.split_once('@')?.1;
        Some((self.lsp.client()?, *kind, PathBuf::from(root)))
    }

    /// Close the hover popup.
    pub(super) fn hover_close(&mut self) {
        self.lsp.hover = None;
    }

    /// Text of the open hover popup.
    pub fn hover_popup(&self) -> Option<&str> {
        self.lsp.hover.as_deref()
    }

    /// The documentLink target of link `i` on the current page.
    pub fn link_target(&self, i: usize) -> Option<&Path> {
        self.lsp.targets.get(&i).map(PathBuf::as_path)
    }

    /// Links a diagnostic covers (drawn muted).
    pub fn broken_links(&self) -> &BTreeSet<usize> {
        &self.lsp.broken
    }

    /// Id of the current page visit (bumped on every page shown).
    pub fn page_id(&self) -> u64 {
        self.lsp.page_id
    }

    /// Document version last sent in `didOpen` for the current page.
    pub fn lsp_version(&self) -> Option<i32> {
        self.lsp.version
    }

    /// True when a request has been pending longer than [`SPINNER_AFTER`].
    pub fn lsp_spinning(&self) -> bool {
        self.oldest_slow_request().is_some()
    }

    fn oldest_slow_request(&self) -> Option<Duration> {
        self.lsp
            .requests
            .iter()
            .map(|r| self.now.saturating_duration_since(r.sent_at))
            .filter(|d| *d > SPINNER_AFTER)
            .max()
    }

    /// LSP state for the status line: `zk ●` running, `zk ○` starting, `—`
    /// none; a spinner follows while a request is slow.
    pub fn lsp_label(&self) -> String {
        let Some((name, kind)) = &self.lsp.instance else {
            return "—".into();
        };
        let kind = spec_kind_label(*kind);
        let mut label = match self.lsp.instances.get(name) {
            Some(Instance::Running(_)) => format!("{kind} ●"),
            Some(Instance::Starting) => format!("{kind} ○"),
            Some(Instance::Dead) | None => return "—".into(),
        };
        if let Some(d) = self.oldest_slow_request() {
            let frame = (d.as_millis() / 100) as usize % SPINNER.len();
            label.push(' ');
            label.push(SPINNER[frame]);
        }
        label
    }
}

fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}
