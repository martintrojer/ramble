//! End to end against the real `zk lsp` (skipped when `zk` is not on PATH).
//!
//! The fixture notebook (`tests/fixtures/zk`, no index database) is copied
//! into a tempdir nested in a fresh git repo. zk runs with HOME and XDG_* set
//! to that tempdir so it never touches the real home directory.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use ramble::lsp::{
    Client, Encoding, Kind, LspEvent, ServerSpec, canonical_uri, position_to_byte, select_server,
    uri_to_path,
};
use serde_json::{Value, json};

const FIXTURE_NOTES: u64 = 4;

fn zk_on_path() -> bool {
    Command::new("zk")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zk")
}

/// Every file below `dir`, relative to it, sorted.
fn file_list(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path.strip_prefix(dir).unwrap().to_path_buf());
            }
        }
    }
    out.sort();
    out
}

/// Copies the fixture tree, skipping a stray index database so zk always
/// starts from an unindexed notebook.
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() == "notebook.db" {
            continue;
        }
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).unwrap();
        }
    }
}

/// Runs `zk lsp` through `env` so the child gets an isolated HOME/XDG_*.
fn zk_spec(home: &Path) -> ServerSpec {
    let mut command = vec!["env".to_string()];
    for var in ["HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"] {
        let dir = home.join(var.to_lowercase());
        std::fs::create_dir_all(&dir).unwrap();
        command.push(format!("{var}={}", dir.display()));
    }
    command.extend(["zk".into(), "lsp".into()]);
    ServerSpec {
        name: "zk".into(),
        kind: Kind::Zk,
        command,
        root_markers: vec![".zk".into()],
        encoding_override: None,
    }
}

fn wait_for<T>(
    rx: &Receiver<LspEvent>,
    what: &str,
    mut pick: impl FnMut(LspEvent) -> Option<T>,
) -> T {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(LspEvent::Exited { error, .. }) => panic!("zk exited waiting for {what}: {error:?}"),
            Ok(ev) => {
                if let Some(t) = pick(ev) {
                    return t;
                }
            }
            Err(_) => panic!("timed out waiting for {what}"),
        }
    }
}

fn request(client: &Client, rx: &Receiver<LspEvent>, method: &str, params: Value) -> Value {
    let tag = 42;
    let id = client.request(method, params, tag).unwrap();
    wait_for(rx, method, |ev| match ev {
        LspEvent::Response {
            id: rid,
            tag: t,
            result,
            ..
        } if rid == id => {
            assert_eq!(t, tag);
            Some(result.unwrap_or_else(|e| panic!("{method}: {e}")))
        }
        _ => None,
    })
}

fn document_links(client: &Client, rx: &Receiver<LspEvent>, path: &Path) -> Vec<Value> {
    let text = std::fs::read_to_string(path).unwrap();
    client.did_open(path, &text, 1).unwrap();
    let links = request(
        client,
        rx,
        "textDocument/documentLink",
        json!({"textDocument": {"uri": canonical_uri(path)}}),
    );
    links.as_array().cloned().unwrap_or_default()
}

fn target_path(link: &Value) -> Option<PathBuf> {
    let uri: lsp_types::Uri = link["target"].as_str()?.parse().ok()?;
    uri_to_path(&uri)
}

#[test]
fn zk_end_to_end() {
    if !zk_on_path() {
        eprintln!("skipping zk end-to-end test: `zk` not on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let git = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .env("HOME", tmp.path())
        .env("XDG_CONFIG_HOME", tmp.path())
        .status();
    if !git.is_ok_and(|s| s.success()) {
        std::fs::create_dir(repo.join(".git")).unwrap();
    }
    // zk writes its index into the notebook, so it only ever runs on a copy.
    let fixture = fixture_dir();
    let fixture_files = file_list(&fixture);
    let nb = repo.join("notes");
    copy_dir(&fixture, &nb);
    let nb_real = nb.canonicalize().unwrap();
    assert!(!nb_real.starts_with(fixture.canonicalize().unwrap()));
    assert!(!nb.join(".zk/notebook.db").exists());

    let zk = zk_spec(&tmp.path().join("home"));
    let marksman = ServerSpec {
        name: "marksman".into(),
        kind: Kind::Marksman,
        command: vec!["marksman".into(), "server".into()],
        root_markers: vec![".git".into()],
        encoding_override: None,
    };
    let specs = [zk, marksman];
    let a = nb.join("a.md");
    let (spec, root) = select_server(&specs, &a).expect("a server for a.md");
    assert_eq!(spec.name, "zk");
    assert_eq!(root, nb.canonicalize().unwrap());

    let (tx, rx) = mpsc::channel();
    let client = Client::start(spec, &root, tx).expect("zk lsp starts");

    // zk.index ran on start and saw every fixture note.
    let stats = client.index_result().expect("zk.index result");
    let count = stats["sourceCount"].as_u64().expect("sourceCount");
    assert!(count >= FIXTURE_NOTES, "{stats}");
    assert!(nb.join(".zk/notebook.db").exists());

    // zk 0.15.6 does not advertise positionEncoding: kind default is code points.
    assert_eq!(client.encoding(), Encoding::Utf32);

    // documentLink resolves a.md -> b.md.
    let links = document_links(&client, &rx, &a);
    let b = nb.join("b.md").canonicalize().unwrap();
    assert!(
        links
            .iter()
            .any(|l| target_path(l).as_deref() == Some(b.as_path())),
        "{links:?}"
    );

    // A link after an emoji: its range converts to the right byte offset.
    let emoji = nb.join("emoji.md");
    let src = std::fs::read_to_string(&emoji).unwrap();
    let links = document_links(&client, &rx, &emoji);
    let link = links
        .iter()
        .find(|l| target_path(l).as_deref() == Some(b.as_path()))
        .unwrap_or_else(|| panic!("no link to b.md in emoji.md: {links:?}"));
    let start: lsp_types::Position =
        serde_json::from_value(link["range"]["start"].clone()).unwrap();
    let byte = position_to_byte(&src, start, client.encoding());
    assert_eq!(byte, src.find("[Note B](b)").unwrap(), "{link}");

    // A broken link produces a diagnostic.
    let broken = nb.join("broken.md");
    let broken_uri = canonical_uri(&broken);
    let text = std::fs::read_to_string(&broken).unwrap();
    client.did_open(&broken, &text, 1).unwrap();
    let diagnostics = wait_for(&rx, "diagnostics for broken.md", |ev| match ev {
        LspEvent::Diagnostics {
            uri, diagnostics, ..
        } if uri_to_path(&uri) == uri_to_path(&broken_uri) && !diagnostics.is_empty() => {
            Some(diagnostics)
        }
        _ => None,
    });
    let src = text;
    let d = &diagnostics[0];
    let at = position_to_byte(&src, d.range.start, client.encoding());
    assert_eq!(at, src.find("[nowhere]").unwrap(), "{diagnostics:?}");

    client.shutdown();
    assert!(nb.join(".zk/notebook.db").exists(), "zk indexed the copy");
    assert_eq!(
        file_list(&fixture),
        fixture_files,
        "zk created files inside tests/fixtures"
    );
}
