//! Scripted fake language server for `tests/lsp.rs`.
//!
//! Usage: `fake-lsp <script.json> [<log.jsonl>]`
//!
//! The script is a JSON array of steps, run in order:
//!
//! - `{"expect": "<method>", "reply": <result>, "as": "<name>"}`: read
//!   messages until one has `method`. Messages read meanwhile get the default
//!   handling below. If `reply` is present the request is answered with it;
//!   `as` remembers its id for a later `respond`.
//! - `{"respond": "<name>", "result": <result>}`: answer a remembered request.
//! - `{"send": <message>}`: write a raw message (notification, server request).
//! - `{"expect_response": true}`: read messages until a response arrives.
//! - `{"sleep_ms": <n>}`
//! - `{"stderr": "<text>"}`: write a line to stderr.
//! - `{"exit": <code>}`: exit immediately (a crash, from the client's view).
//! - `{"close_stdin": true}`: close stdin but keep running (client writes fail).
//!
//! After the script: requests get a `null` result, `exit` ends the process,
//! and end of input ends it too. Every message read is appended to the log
//! as one JSON line.

use std::fs::OpenOptions;
use std::io::{self, BufReader, Write};
use std::path::PathBuf;
use std::{collections::HashMap, thread, time::Duration};

use ramble::lsp::{read_message, write_message};
use serde_json::{Value, json};

struct Server {
    input: BufReader<io::Stdin>,
    log: Option<PathBuf>,
    held: HashMap<String, Value>,
}

impl Server {
    fn read(&mut self) -> Value {
        let msg = match read_message(&mut self.input) {
            Ok(Some(m)) => m,
            Ok(None) => std::process::exit(0),
            Err(e) => {
                eprintln!("fake-lsp: read error: {e}");
                std::process::exit(3);
            }
        };
        if let Some(log) = &self.log {
            let mut f = OpenOptions::new()
                .create(true)
                .append(true)
                .open(log)
                .expect("open log");
            writeln!(f, "{msg}").expect("write log");
        }
        msg
    }

    fn send(&self, msg: &Value) {
        if write_message(&mut io::stdout().lock(), msg).is_err() {
            std::process::exit(4);
        }
    }

    fn reply(&self, id: &Value, result: &Value) {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }

    /// Default handling for a message the script is not waiting for.
    fn default(&self, msg: &Value) {
        match (msg.get("method").and_then(Value::as_str), msg.get("id")) {
            (Some("exit"), _) => std::process::exit(0),
            (Some(_), Some(id)) => self.reply(id, &Value::Null),
            _ => {}
        }
    }
}

#[cfg(unix)]
fn close_stdin() {
    use std::os::fd::FromRawFd;
    // SAFETY: fd 0 is ours; the `BufReader<Stdin>` is never read again.
    drop(unsafe { std::fs::File::from_raw_fd(0) });
}

#[cfg(not(unix))]
fn close_stdin() {
    panic!("close_stdin is unix-only");
}

fn main() {
    let mut args = std::env::args().skip(1);
    let script_path = args.next().expect("usage: fake-lsp <script.json> [<log>]");
    let script: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(&script_path).expect("read script"))
            .expect("parse script");
    let mut s = Server {
        input: BufReader::new(io::stdin()),
        log: args.next().map(PathBuf::from),
        held: HashMap::new(),
    };

    for step in &script {
        if let Some(method) = step.get("expect").and_then(Value::as_str) {
            let msg = loop {
                let msg = s.read();
                if msg.get("method").and_then(Value::as_str) == Some(method) {
                    break msg;
                }
                s.default(&msg);
            };
            let id = msg.get("id").cloned().unwrap_or(Value::Null);
            if let Some(name) = step.get("as").and_then(Value::as_str) {
                s.held.insert(name.to_owned(), id.clone());
            }
            if let Some(result) = step.get("reply") {
                s.reply(&id, result);
            }
        } else if let Some(name) = step.get("respond").and_then(Value::as_str) {
            let id = s.held.get(name).cloned().expect("respond to unknown name");
            s.reply(&id, step.get("result").unwrap_or(&Value::Null));
        } else if let Some(msg) = step.get("send") {
            s.send(msg);
        } else if step.get("expect_response").is_some() {
            loop {
                let msg = s.read();
                if msg.get("method").is_none() {
                    break;
                }
                s.default(&msg);
            }
        } else if let Some(ms) = step.get("sleep_ms").and_then(Value::as_u64) {
            thread::sleep(Duration::from_millis(ms));
        } else if let Some(text) = step.get("stderr").and_then(Value::as_str) {
            eprintln!("{text}");
        } else if step.get("close_stdin").is_some() {
            close_stdin();
        } else if let Some(code) = step.get("exit").and_then(Value::as_i64) {
            std::process::exit(code as i32);
        } else {
            panic!("bad script step: {step}");
        }
    }
    loop {
        let msg = s.read();
        s.default(&msg);
    }
}
