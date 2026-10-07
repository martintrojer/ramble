//! End to end against the real `marksman server` (skipped when `marksman`
//! is not on PATH). The zk fixture notebook also holds a `.marksman.toml`.
//! The spec is built directly: `select_server` would pick zk for `.zk`.
//! marksman runs with HOME and XDG_* set inside a tempdir.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use ramble::lsp::{Client, Kind, LspEvent, ServerSpec, canonical_uri, uri_to_path};
use ramble::notebook;
use serde_json::{Value, json};

fn marksman_on_path() -> bool {
    Command::new("marksman")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).unwrap();
        }
    }
}

fn marksman_spec(home: &Path) -> ServerSpec {
    let mut command = vec!["env".to_string()];
    for var in ["HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"] {
        let dir = home.join(var.to_lowercase());
        std::fs::create_dir_all(&dir).unwrap();
        command.push(format!("{var}={}", dir.display()));
    }
    command.extend(["marksman".into(), "server".into()]);
    ServerSpec {
        name: "marksman".into(),
        kind: Kind::Marksman,
        command,
        root_markers: vec![".marksman.toml".into()],
        encoding_override: None,
    }
}

fn request(client: &Client, rx: &Receiver<LspEvent>, method: &str, params: Value) -> Value {
    let id = client.request(method, params, 7).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(LspEvent::Response {
                id: rid, result, ..
            }) if rid == id => return result.unwrap_or_else(|e| panic!("{method}: {e}")),
            Ok(LspEvent::Exited { error, .. }) => panic!("marksman exited: {error:?}"),
            Ok(_) => {}
            Err(_) => panic!("timed out waiting for {method}"),
        }
    }
}

fn first_path(v: &Value) -> Option<PathBuf> {
    let first = v.as_array().and_then(|xs| xs.first()).unwrap_or(v);
    let uri = first
        .get("uri")
        .or_else(|| first.get("targetUri"))?
        .as_str()?;
    uri_to_path(&uri.parse().ok()?)
}

#[test]
fn marksman_end_to_end() {
    if !marksman_on_path() {
        eprintln!("skipping marksman end-to-end test: `marksman` not on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let nb = tmp.path().join("notes");
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zk"),
        &nb,
    );
    let root = nb.canonicalize().unwrap();
    let (tx, rx) = mpsc::channel();
    let client = Client::start(&marksman_spec(&tmp.path().join("home")), &root, tx)
        .expect("marksman starts");
    let open = |name: &str| {
        let path = root.join(name);
        let text = std::fs::read_to_string(&path).unwrap();
        client.did_open(&path, &text, 1).unwrap();
        (path, text)
    };
    let (a, a_src) = open("a.md");
    let (b, b_src) = open("b.md");
    let enc = client.encoding();

    // definition and hover on `[Note B](b)` in a.md.
    let at = ramble::lsp::byte_to_position(&a_src, a_src.find("Note B](b)").unwrap(), enc);
    let pos = json!({"textDocument": {"uri": canonical_uri(&a)}, "position": at});
    let def = request(&client, &rx, "textDocument/definition", pos.clone());
    assert_eq!(first_path(&def), Some(b.clone()), "{def}");
    let hover = request(&client, &rx, "textDocument/hover", pos);
    assert!(!hover.is_null(), "hover on a link answers");

    // references from b.md's first heading: a.md links to it.
    let doc = ramble::doc::parse(b_src);
    let at = notebook::backlinks_position(Kind::Marksman, &doc, enc).expect("b.md has a heading");
    let refs = request(
        &client,
        &rx,
        "textDocument/references",
        notebook::references_params(&b, at),
    );
    let items = notebook::location_items(&refs, &root);
    assert!(
        items.iter().any(|i| i.path.as_deref() == Some(a.as_path())),
        "{refs}"
    );
    client.shutdown();
}
