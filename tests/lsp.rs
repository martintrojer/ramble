//! LSP client: position conversion, URIs, server selection, and the client
//! driven against the scripted fake server (`tests/support/fake_lsp.rs`).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use lsp_types::Position;
use proptest::prelude::*;
use ramble::lsp::{
    Client, Encoding, Kind, LspEvent, ServerSpec, byte_to_position, canonical_uri, from_byte,
    position_to_byte, select_server, to_byte, uri_to_path,
};
use serde_json::{Value, json};

use Encoding::{Utf8, Utf16, Utf32};

// ---------------------------------------------------------------- conversion

#[test]
fn to_byte_and_from_byte_tables() {
    // (line, character, encoding, byte)
    let cases: &[(&str, u32, Encoding, usize)] = &[
        ("abc", 2, Utf16, 2),
        // é: 2 bytes, 1 utf-16 unit, 1 code point
        ("é!", 1, Utf8, 0), // inside é rounds down
        ("é!", 2, Utf8, 2),
        ("é!", 1, Utf16, 2),
        ("é!", 1, Utf32, 2),
        // 😀: 4 bytes, 2 utf-16 units, 1 code point
        ("😀x", 1, Utf16, 0), // between surrogates rounds down
        ("😀x", 2, Utf16, 4),
        ("😀x", 1, Utf32, 4),
        ("😀x", 3, Utf8, 0),
        ("😀x", 4, Utf8, 4),
        ("😀x", 3, Utf16, 5),
        // 日本: 3 bytes, 1 unit, 1 code point each
        ("日本", 1, Utf16, 3),
        ("日本", 1, Utf32, 3),
        ("日本", 6, Utf8, 6),
        // mixed
        ("aé😀日b", 5, Utf16, 10),
        ("aé😀日b", 4, Utf32, 10),
        ("aé😀日b", 10, Utf8, 10),
        // end of line and past end clamp
        ("aé", 2, Utf32, 3),
        ("aé", 99, Utf16, 3),
        ("aé", 99, Utf8, 3),
        ("", 5, Utf32, 0),
    ];
    for &(line, ch, enc, byte) in cases {
        assert_eq!(
            to_byte(line, ch, enc),
            byte,
            "to_byte({line:?}, {ch}, {enc:?})"
        );
    }

    // (line, byte, encoding, character)
    let back: &[(&str, usize, Encoding, u32)] = &[
        ("é!", 2, Utf8, 2),
        ("é!", 2, Utf16, 1),
        ("é!", 1, Utf16, 0), // inside é rounds down
        ("😀x", 4, Utf16, 2),
        ("😀x", 4, Utf32, 1),
        ("😀x", 2, Utf16, 0),
        ("😀x", 5, Utf16, 3),
        ("日本", 3, Utf16, 1),
        ("日本", 6, Utf32, 2),
        ("aé😀日b", 10, Utf16, 5),
        ("aé😀日b", 10, Utf32, 4),
        ("aé😀日b", 11, Utf16, 6),
        ("aé", 99, Utf16, 2), // past end clamps
        ("aé", 99, Utf8, 3),
    ];
    for &(line, byte, enc, ch) in back {
        assert_eq!(
            from_byte(line, byte, enc),
            ch,
            "from_byte({line:?}, {byte}, {enc:?})"
        );
    }
}

#[test]
fn position_to_byte_lines() {
    let src = "a😀\r\n日b\n\nlast";
    let p = |l, c| Position::new(l, c);
    assert_eq!(position_to_byte(src, p(0, 3), Utf16), 5);
    // the trailing '\r' is part of line 0
    assert_eq!(position_to_byte(src, p(0, 4), Utf16), 6);
    assert_eq!(position_to_byte(src, p(0, 99), Utf16), 6);
    assert_eq!(position_to_byte(src, p(1, 1), Utf32), 10);
    assert_eq!(position_to_byte(src, p(2, 0), Utf8), 12);
    assert_eq!(position_to_byte(src, p(3, 2), Utf8), 15);
    // line past EOF
    assert_eq!(position_to_byte(src, p(9, 0), Utf8), src.len());
    assert_eq!(byte_to_position(src, 10, Utf32), p(1, 1));
    assert_eq!(byte_to_position(src, src.len(), Utf16), p(3, 4));
    assert_eq!(byte_to_position(src, 3, Utf16), p(0, 1)); // inside 😀
}

fn text() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop_oneof![
            Just('a'),
            Just('é'),
            Just('😀'),
            Just('日'),
            Just('\n'),
            Just('\r'),
            Just(' '),
        ],
        0..40,
    )
    .prop_map(|v| v.into_iter().collect())
}

proptest! {
    #[test]
    fn position_round_trip(src in text(), pick in any::<prop::sample::Index>()) {
        let mut bytes: Vec<usize> = src.char_indices().map(|(i, _)| i).collect();
        bytes.push(src.len());
        let byte = bytes[pick.index(bytes.len())];
        for enc in [Utf8, Utf16, Utf32] {
            let pos = byte_to_position(&src, byte, enc);
            prop_assert_eq!(position_to_byte(&src, pos, enc), byte, "{:?}", enc);
        }
    }
}

// ------------------------------------------------------------ uri, selection

fn spec(name: &str, kind: Kind, markers: &[&str], command: Vec<String>) -> ServerSpec {
    ServerSpec {
        name: name.into(),
        kind,
        command,
        root_markers: markers.iter().map(|m| m.to_string()).collect(),
        encoding_override: None,
    }
}

fn default_specs() -> Vec<ServerSpec> {
    vec![
        spec("zk", Kind::Zk, &[".zk"], vec!["zk".into(), "lsp".into()]),
        spec(
            "marksman",
            Kind::Marksman,
            &[".git", ".marksman.toml"],
            vec![],
        ),
    ]
}

#[test]
fn select_server_picks_first_matching_spec() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    let nb = repo.join("notes");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir_all(nb.join(".zk")).unwrap();
    std::fs::create_dir_all(nb.join("sub")).unwrap();
    std::fs::write(nb.join("sub/n.md"), "").unwrap();
    std::fs::write(repo.join("README.md"), "").unwrap();
    let specs = default_specs();

    let (s, root) = select_server(&specs, &nb.join("sub/n.md")).unwrap();
    assert_eq!(s.name, "zk");
    assert_eq!(root, nb.canonicalize().unwrap());

    let (s, root) = select_server(&specs, &repo.join("README.md")).unwrap();
    assert_eq!(s.name, "marksman");
    assert_eq!(root, repo.canonicalize().unwrap());

    let lone = tmp.path().join("lone.md");
    std::fs::write(&lone, "").unwrap();
    assert!(select_server(&specs, &lone).is_none());
}

#[cfg(unix)]
#[test]
fn canonical_uri_resolves_symlinks() {
    let tmp = tempfile::tempdir().unwrap();
    let real = tmp.path().join("real");
    std::fs::create_dir(&real).unwrap();
    std::fs::write(real.join("n é.md"), "").unwrap();
    let link = tmp.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let a = canonical_uri(&real.join("n é.md"));
    let b = canonical_uri(&link.join("n é.md"));
    assert_eq!(a, b);
    assert!(a.as_str().starts_with("file:///"), "{}", a.as_str());
    assert_eq!(
        uri_to_path(&b).unwrap(),
        real.join("n é.md").canonicalize().unwrap()
    );
    // a not-yet-existing file below a symlinked dir canonicalizes too
    assert_eq!(
        canonical_uri(&link.join("new.md")),
        canonical_uri(&real.join("new.md"))
    );
    let http: lsp_types::Uri = "https://example.com/x".parse().unwrap();
    assert_eq!(uri_to_path(&http), None);
}

// -------------------------------------------------------------- fake server

const T: Duration = Duration::from_secs(10);

struct Fake {
    _dir: tempfile::TempDir,
    root: PathBuf,
    log: PathBuf,
    spec: ServerSpec,
}

impl Fake {
    fn new(kind: Kind, script: Value) -> Fake {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let script_path = dir.path().join("script.json");
        std::fs::write(&script_path, script.to_string()).unwrap();
        let log = dir.path().join("log.jsonl");
        let command = vec![
            env!("CARGO_BIN_EXE_fake-lsp").to_string(),
            script_path.display().to_string(),
            log.display().to_string(),
        ];
        let spec = spec("fake", kind, &[], command);
        Fake {
            _dir: dir,
            root,
            log,
            spec,
        }
    }

    fn start(&self) -> (Client, Receiver<LspEvent>) {
        let (tx, rx) = mpsc::channel();
        (Client::start(&self.spec, &self.root, tx).unwrap(), rx)
    }

    fn messages(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn methods(&self) -> Vec<String> {
        self.messages()
            .iter()
            .filter_map(|m| m.get("method").and_then(Value::as_str).map(String::from))
            .collect()
    }
}

fn init(caps: Value) -> Vec<Value> {
    vec![
        json!({"expect": "initialize", "reply": {"capabilities": caps}}),
        json!({"expect": "initialized"}),
    ]
}

fn script(steps: Vec<Vec<Value>>) -> Value {
    Value::Array(steps.into_iter().flatten().collect())
}

fn next(rx: &Receiver<LspEvent>) -> LspEvent {
    rx.recv_timeout(T).expect("event within timeout")
}

fn response(ev: LspEvent) -> (u64, u64, Result<Value, String>) {
    match ev {
        LspEvent::Response {
            server,
            id,
            tag,
            result,
        } => {
            assert_eq!(server, "fake");
            (id, tag, result)
        }
        other => panic!("expected Response, got {other:?}"),
    }
}

#[test]
fn initialize_offers_encodings_and_root() {
    let fake = Fake::new(Kind::Generic, script(vec![init(json!({}))]));
    let (client, _rx) = fake.start();
    client.shutdown();
    let msgs = fake.messages();
    let init = &msgs[0];
    assert_eq!(init["method"], "initialize");
    assert_eq!(
        init["params"]["capabilities"]["general"]["positionEncodings"],
        json!(["utf-8", "utf-16"])
    );
    let root_uri = canonical_uri(&fake.root);
    assert_eq!(init["params"]["rootUri"], root_uri.as_str());
    assert_eq!(
        init["params"]["workspaceFolders"][0]["uri"],
        root_uri.as_str()
    );
    assert_eq!(
        fake.methods(),
        ["initialize", "initialized", "shutdown", "exit"]
    );
}

#[test]
fn large_messages_both_directions() {
    let big = "😀é日".repeat(10_000); // ~90 KiB
    let fake = Fake::new(
        Kind::Generic,
        script(vec![
            init(json!({})),
            vec![json!({"expect": "test/big", "reply": {"text": big}})],
        ]),
    );
    let (client, rx) = fake.start();
    client.did_open(&fake.root.join("a.md"), &big, 1).unwrap();
    let id = client.request("test/big", json!({}), 7).unwrap();
    let (rid, tag, result) = response(next(&rx));
    assert_eq!((rid, tag), (id, 7));
    assert_eq!(result.unwrap()["text"], big);
    client.shutdown();
    let open = fake
        .messages()
        .into_iter()
        .find(|m| m["method"] == "textDocument/didOpen")
        .unwrap();
    let doc = &open["params"]["textDocument"];
    assert_eq!(doc["text"], big);
    assert_eq!(doc["languageId"], "markdown");
    assert_eq!(doc["version"], 1);
    assert_eq!(doc["uri"], canonical_uri(&fake.root.join("a.md")).as_str());
}

#[test]
fn out_of_order_replies_keep_their_tags() {
    let fake = Fake::new(
        Kind::Generic,
        script(vec![
            init(json!({})),
            vec![
                json!({"expect": "test/a", "as": "a"}),
                json!({"expect": "test/b", "as": "b"}),
                json!({"respond": "b", "result": "B"}),
                json!({"respond": "a", "result": "A"}),
            ],
        ]),
    );
    let (client, rx) = fake.start();
    let a = client.request("test/a", json!({}), 100).unwrap();
    let b = client.request("test/b", json!({}), 200).unwrap();
    assert_ne!(a, b);
    assert_eq!(response(next(&rx)), (b, 200, Ok(json!("B"))));
    assert_eq!(response(next(&rx)), (a, 100, Ok(json!("A"))));
}

#[test]
fn error_reply_is_err() {
    let fake = Fake::new(
        Kind::Generic,
        script(vec![
            init(json!({})),
            vec![
                json!({"expect": "test/a", "as": "a"}),
                json!({"send": {"jsonrpc": "2.0", "id": 2, "error": {"code": -32603, "message": "nope"}}}),
            ],
        ]),
    );
    let (client, rx) = fake.start();
    // initialize used id 1, so this is id 2
    let id = client.request("test/a", json!({}), 5).unwrap();
    assert_eq!(id, 2);
    assert_eq!(response(next(&rx)), (2, 5, Err("nope".into())));
}

#[test]
fn diagnostics_before_response() {
    let fake = Fake::new(
        Kind::Generic,
        script(vec![
            init(json!({})),
            vec![
                json!({"expect": "textDocument/documentLink", "as": "l"}),
                json!({"send": {"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
                    "params": {"uri": "file:///x/a.md", "version": 3, "diagnostics": [
                        {"range": {"start": {"line": 1, "character": 2},
                                   "end": {"line": 1, "character": 5}},
                         "severity": 1, "message": "not found"}]}}}),
                json!({"respond": "l", "result": []}),
            ],
        ]),
    );
    let (client, rx) = fake.start();
    let id = client
        .request("textDocument/documentLink", json!({}), 9)
        .unwrap();
    match next(&rx) {
        LspEvent::Diagnostics {
            server,
            uri,
            version,
            diagnostics,
        } => {
            assert_eq!(server, "fake");
            assert_eq!(uri.as_str(), "file:///x/a.md");
            assert_eq!(version, Some(3));
            assert_eq!(diagnostics.len(), 1);
            assert_eq!(diagnostics[0].message, "not found");
            assert_eq!(diagnostics[0].range.start, Position::new(1, 2));
        }
        other => panic!("expected Diagnostics, got {other:?}"),
    }
    assert_eq!(response(next(&rx)), (id, 9, Ok(json!([]))));
}

#[test]
fn position_encoding_negotiation() {
    let enc = |kind, caps, over| {
        let mut fake = Fake::new(kind, script(vec![init(caps)]));
        fake.spec.encoding_override = over;
        let (client, _rx) = fake.start();
        let e = client.encoding();
        client.shutdown();
        e
    };
    assert_eq!(
        enc(Kind::Generic, json!({"positionEncoding": "utf-8"}), None),
        Utf8
    );
    assert_eq!(enc(Kind::Generic, json!({}), None), Utf16);
    assert_eq!(enc(Kind::Marksman, json!({}), None), Utf16);
    assert_eq!(enc(Kind::Generic, json!({}), Some(Utf32)), Utf32);
    assert_eq!(
        enc(
            Kind::Generic,
            json!({"positionEncoding": "utf-8"}),
            Some(Utf16)
        ),
        Utf16
    );
}

#[test]
fn capabilities_are_recorded() {
    let fake = Fake::new(
        Kind::Generic,
        script(vec![init(
            json!({"hoverProvider": true, "documentLinkProvider": {}}),
        )]),
    );
    let (client, _rx) = fake.start();
    assert_eq!(
        client.capabilities().hover_provider,
        Some(lsp_types::HoverProviderCapability::Simple(true))
    );
    assert!(client.capabilities().document_link_provider.is_some());
    assert!(client.capabilities().definition_provider.is_none());
}

#[test]
fn zk_kind_runs_zk_index_after_initialized() {
    let fake = Fake::new(
        Kind::Zk,
        script(vec![
            init(json!({})),
            vec![json!({"expect": "workspace/executeCommand", "reply": {"sourceCount": 2}})],
        ]),
    );
    let (client, _rx) = fake.start();
    assert_eq!(client.encoding(), Utf32);
    assert_eq!(client.index_result(), Some(&json!({"sourceCount": 2})));
    client.shutdown();
    let msgs = fake.messages();
    let methods: Vec<_> = msgs.iter().map(|m| m["method"].as_str().unwrap()).collect();
    assert_eq!(
        methods,
        [
            "initialize",
            "initialized",
            "workspace/executeCommand",
            "shutdown",
            "exit"
        ]
    );
    let root = fake.root.canonicalize().unwrap();
    assert_eq!(
        msgs[2]["params"],
        json!({"command": "zk.index", "arguments": [root.to_str().unwrap()]})
    );
}

#[test]
fn other_kinds_do_not_run_zk_index() {
    for kind in [Kind::Marksman, Kind::Generic] {
        let fake = Fake::new(kind, script(vec![init(json!({}))]));
        let (client, _rx) = fake.start();
        assert_eq!(client.index_result(), None);
        client.shutdown();
        assert_eq!(
            fake.methods(),
            ["initialize", "initialized", "shutdown", "exit"]
        );
    }
}

#[test]
fn server_working_directory_is_root() {
    // A relative log path lands in the server's cwd.
    let mut fake = Fake::new(Kind::Generic, script(vec![init(json!({}))]));
    fake.spec.command[2] = "cwd-log.jsonl".into();
    let (client, _rx) = fake.start();
    client.shutdown();
    assert!(fake.root.join("cwd-log.jsonl").exists());
}

#[test]
fn server_requests_are_answered() {
    let fake = Fake::new(
        Kind::Generic,
        script(vec![
            init(json!({})),
            vec![
                json!({"expect": "test/done", "as": "done"}),
                json!({"send": {"jsonrpc": "2.0", "id": "c1", "method": "workspace/configuration",
                    "params": {"items": [{"section": "a"}, {"section": "b"}]}}}),
                json!({"expect_response": true}),
                json!({"send": {"jsonrpc": "2.0", "id": 77, "method": "window/workDoneProgress/create",
                    "params": {"token": "t"}}}),
                json!({"expect_response": true}),
                json!({"send": {"jsonrpc": "2.0", "id": 78, "method": "client/registerCapability",
                    "params": {"registrations": []}}}),
                json!({"expect_response": true}),
                json!({"send": {"jsonrpc": "2.0", "id": 79, "method": "workspace/applyEdit",
                    "params": {}}}),
                json!({"expect_response": true}),
                json!({"respond": "done", "result": true}),
            ],
        ]),
    );
    let (client, rx) = fake.start();
    // The fake answers test/done only after it has read all four replies.
    client.request("test/done", json!({}), 0).unwrap();
    assert_eq!(response(next(&rx)).2, Ok(json!(true)));
    client.shutdown();
    let replies: Vec<Value> = fake
        .messages()
        .into_iter()
        .filter(|m| m.get("method").is_none())
        .collect();
    assert_eq!(replies.len(), 4, "{replies:?}");
    assert_eq!(replies[0]["id"], "c1");
    assert_eq!(replies[0]["result"], json!([null, null]));
    assert_eq!(replies[1]["id"], 77);
    assert_eq!(replies[1]["result"], Value::Null);
    assert!(replies[1].get("error").is_none());
    assert_eq!(replies[2]["id"], 78);
    assert_eq!(replies[2]["result"], Value::Null);
    assert_eq!(replies[3]["id"], 79);
    assert_eq!(replies[3]["error"]["code"], -32601);
}

#[test]
fn crash_reports_exited_with_stderr_tail() {
    let fake = Fake::new(
        Kind::Generic,
        script(vec![
            init(json!({})),
            vec![
                json!({"expect": "test/a"}),
                json!({"stderr": "first line"}),
                json!({"stderr": "panic: boom"}),
                json!({"exit": 7}),
            ],
        ]),
    );
    let (client, rx) = fake.start();
    client.request("test/a", json!({}), 1).unwrap();
    match next(&rx) {
        LspEvent::Exited { server, error } => {
            assert_eq!(server, "fake");
            let error = error.expect("crash is an error");
            assert!(error.contains("panic: boom"), "{error}");
            assert!(error.contains("first line"), "{error}");
            assert!(error.contains('7'), "{error}");
        }
        other => panic!("expected Exited, got {other:?}"),
    }
    // Writing to a dead server fails instead of hanging or panicking.
    std::thread::sleep(Duration::from_millis(50));
    let _ = client.notify("test/late", json!({}));
}

#[test]
fn stderr_tail_keeps_last_50_lines() {
    let mut steps = init(json!({}));
    steps.push(json!({"expect": "test/a"}));
    for i in 0..60 {
        steps.push(json!({"stderr": format!("line {i}")}));
    }
    steps.push(json!({"exit": 1}));
    let fake = Fake::new(Kind::Generic, Value::Array(steps));
    let (client, rx) = fake.start();
    client.request("test/a", json!({}), 1).unwrap();
    let LspEvent::Exited {
        error: Some(error), ..
    } = next(&rx)
    else {
        panic!("expected Exited with error")
    };
    assert!(error.contains("line 59"), "{error}");
    assert!(error.contains("line 10\n"), "{error}");
    assert!(!error.contains("line 9\n"), "{error}");
}

#[test]
fn crash_during_initialize_fails_start() {
    let fake = Fake::new(
        Kind::Generic,
        json!([{"expect": "initialize"}, {"stderr": "bad config"}, {"exit": 2}]),
    );
    let (tx, _rx) = mpsc::channel();
    let err = Client::start(&fake.spec, &fake.root, tx)
        .err()
        .expect("start fails");
    assert!(format!("{err:#}").contains("bad config"), "{err:#}");
}

#[test]
fn missing_binary_fails_start() {
    let s = spec(
        "nope",
        Kind::Generic,
        &[],
        vec!["/nonexistent/ramble-lsp".into()],
    );
    let (tx, _rx) = mpsc::channel();
    assert!(Client::start(&s, Path::new("/"), tx).is_err());
}

#[test]
fn shutdown_is_not_an_error_and_kills_stuck_server() {
    // The server never answers shutdown and ignores exit until stdin closes;
    // shutdown still returns and reports a clean exit.
    let fake = Fake::new(
        Kind::Generic,
        script(vec![
            init(json!({})),
            vec![json!({"expect": "shutdown"}), json!({"sleep_ms": 60_000})],
        ]),
    );
    let (client, rx) = fake.start();
    let t = std::time::Instant::now();
    client.shutdown();
    assert!(t.elapsed() < Duration::from_secs(8), "{:?}", t.elapsed());
    match next(&rx) {
        LspEvent::Exited { error, .. } => assert_eq!(error, None),
        other => panic!("expected Exited, got {other:?}"),
    }
}

// ------------------------------------------------------------ write deadlocks

/// Runs `f` on its own thread and waits at most `limit` for it. On timeout
/// the test fails and the thread (and whatever it owns) is leaked instead
/// of hanging the test run.
fn within<T: Send + 'static>(
    limit: Duration,
    what: &str,
    f: impl FnOnce() -> T + Send + 'static,
) -> (T, Duration) {
    let (tx, rx) = mpsc::channel();
    let t = std::time::Instant::now();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(limit) {
        Ok(v) => (v, t.elapsed()),
        Err(_) => panic!("{what} did not return within {limit:?}"),
    }
}

#[test]
fn server_request_during_large_write_does_not_deadlock() {
    // The server answers test/go, then (not reading meanwhile) asks for
    // configuration and floods ~400 KB of output while we write 1 MiB. Our
    // reply must not block the reader, or neither side drains its pipe.
    let flood = "x".repeat(20_000);
    let mut steps = init(json!({}));
    steps.push(json!({"expect": "test/go", "reply": null}));
    steps.push(json!({"sleep_ms": 300}));
    steps.push(
        json!({"send": {"jsonrpc": "2.0", "id": "cfg", "method": "workspace/configuration",
        "params": {"items": [{"section": "s"}]}}}),
    );
    for _ in 0..20 {
        steps.push(
            json!({"send": {"jsonrpc": "2.0", "method": "window/logMessage",
            "params": {"type": 4, "message": flood}}}),
        );
    }
    steps.push(json!({"expect": "textDocument/didOpen"}));
    steps.push(json!({"expect": "test/done", "reply": true}));
    let fake = Fake::new(Kind::Generic, Value::Array(steps));
    let (client, rx) = fake.start();
    client.request("test/go", json!({}), 1).unwrap();
    assert_eq!(response(next(&rx)).1, 1);

    let path = fake.root.join("big.md");
    let big = "a".repeat(1 << 20);
    let (client, took) = within(Duration::from_secs(1), "did_open", move || {
        client.did_open(&path, &big, 1).unwrap();
        client
    });
    eprintln!("did_open took {took:?}");
    client.request("test/done", json!({}), 2).unwrap();
    assert_eq!(response(next(&rx)), (3, 2, Ok(json!(true))));
    let (_, took) = within(Duration::from_secs(3), "shutdown", move || {
        client.shutdown()
    });
    eprintln!("shutdown took {took:?}");
    let msgs = fake.messages();
    let reply = msgs.iter().find(|m| m["id"] == "cfg").expect("cfg reply");
    assert_eq!(reply["result"], json!([null]));
    let open = msgs
        .iter()
        .find(|m| m["method"] == "textDocument/didOpen")
        .unwrap();
    assert_eq!(
        open["params"]["textDocument"]["text"]
            .as_str()
            .unwrap()
            .len(),
        1 << 20
    );
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(unix)]
#[test]
fn server_that_stops_reading_blocks_neither_writes_nor_drop() {
    let fake = Fake::new(
        Kind::Generic,
        script(vec![init(json!({})), vec![json!({"sleep_ms": 60_000})]]),
    );
    let (client, _rx) = fake.start();
    let pid = client.pid();
    assert!(alive(pid));

    let path = fake.root.join("big.md");
    let big = "a".repeat(1 << 20);
    let (client, took) = within(Duration::from_secs(1), "did_open", move || {
        client.did_open(&path, &big, 1).unwrap();
        client.did_open(&path, &big, 2).unwrap();
        client
    });
    eprintln!("did_open x2 took {took:?}");
    // Let the writer fill the pipe and block on it.
    std::thread::sleep(Duration::from_millis(200));
    let (_, took) = within(Duration::from_secs(3), "drop", move || drop(client));
    eprintln!("drop took {took:?}");
    assert!(!alive(pid), "fake-lsp {pid} still running after drop");
}

#[cfg(unix)]
#[test]
fn write_error_reports_exited_and_kills_server() {
    let fake = Fake::new(
        Kind::Generic,
        script(vec![
            init(json!({})),
            vec![json!({"close_stdin": true}), json!({"sleep_ms": 60_000})],
        ]),
    );
    let (client, rx) = fake.start();
    let pid = client.pid();
    std::thread::sleep(Duration::from_millis(100));
    // Queued; the writer hits EPIPE.
    let _ = client.did_open(&fake.root.join("a.md"), &"a".repeat(1 << 16), 1);
    match next(&rx) {
        LspEvent::Exited { error, .. } => {
            let error = error.expect("write failure is an error");
            assert!(error.contains("write to server failed"), "{error}");
        }
        other => panic!("expected Exited, got {other:?}"),
    }
    assert!(!alive(pid), "fake-lsp {pid} still running");
    assert!(client.notify("test/late", json!({})).is_err());
}
