//! Notebook operations: zk adapter parameters and result parsing, backlinks
//! positions, availability, local fallbacks, and the fuzzy filter.

use std::path::Path;

use ramble::app::picker_filter;
use ramble::doc;
use ramble::lsp::notebook::{self as lsp_notebook, zk};
use ramble::lsp::{Encoding, Kind};
use ramble::notebook::{self, Item, Op, Sources};
use serde_json::json;

#[test]
fn zk_adapter_params_pass_the_root_as_args0() {
    let root = Path::new("/nb");
    assert_eq!(
        zk::notes(root),
        json!({"command": "zk.list", "arguments": ["/nb",
            {"select": ["title", "absPath"], "sort": ["modified"]}]})
    );
    assert_eq!(
        zk::search(root, "-foo bar*"),
        json!({"command": "zk.list", "arguments": ["/nb",
            {"select": ["title", "absPath"], "match": ["-foo bar*"]}]})
    );
    assert_eq!(
        zk::tags(root),
        json!({"command": "zk.tag.list", "arguments": ["/nb"]})
    );
    assert_eq!(
        zk::notes_by_tag(root, "project"),
        json!({"command": "zk.list", "arguments": ["/nb",
            {"select": ["title", "absPath"], "tags": ["project"]}]})
    );
}

#[test]
fn zk_results_become_items() {
    let notes = zk::note_items(
        &json!([{"title": "Note A", "absPath": "/nb/a.md"},
                {"title": "", "absPath": "/nb/sub/x.md"}, {"title": "no path"}]),
        Path::new("/nb"),
    );
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0].label, "Note A");
    assert_eq!(notes[0].detail, "a.md");
    assert_eq!(notes[0].path.as_deref(), Some(Path::new("/nb/a.md")));
    assert_eq!(notes[1].label, "x", "empty title falls back to the stem");
    let tags =
        zk::tag_items(&json!([{"id": 1, "kind": "tag", "name": "project", "note_count": 3}]));
    assert_eq!(tags[0].label, "project");
    assert_eq!(tags[0].detail, "3");
}

#[test]
fn backlinks_positions_per_server() {
    let d = doc::parse("Intro 😀\n\n## Topic\n".into());
    assert_eq!(
        lsp_notebook::backlinks_position(Kind::Zk, &d, Encoding::Utf32),
        Some(lsp_types::Position::new(0, 0))
    );
    assert_eq!(
        lsp_notebook::backlinks_position(Kind::Marksman, &d, Encoding::Utf16),
        Some(lsp_types::Position::new(2, 0))
    );
    let none = doc::parse("no heading\n".into());
    assert_eq!(
        lsp_notebook::backlinks_position(Kind::Marksman, &none, Encoding::Utf16),
        None
    );
    let p = lsp_notebook::references_params(Path::new("/x/a.md"), lsp_types::Position::new(2, 0));
    assert_eq!(p["context"]["includeDeclaration"], json!(false));
    assert_eq!(p["position"], json!({"line": 2, "character": 0}));
}

#[test]
fn locations_label_with_first_heading_and_line() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join("b.md"), "intro\n\n# Bee\n").unwrap();
    std::fs::write(root.join("plain.md"), "no heading\n").unwrap();
    let uri = |f: &str| {
        ramble::lsp::canonical_uri(&root.join(f))
            .as_str()
            .to_string()
    };
    let items = lsp_notebook::location_items(
        &json!([{"uri": uri("b.md"), "range": {"start": {"line": 3, "character": 0},
                                               "end": {"line": 3, "character": 0}}},
                {"uri": uri("plain.md"), "range": {"start": {"line": 0, "character": 0},
                                                   "end": {"line": 0, "character": 0}}}]),
        dir.path(),
    );
    assert_eq!(items[0].label, "Bee");
    assert_eq!(items[0].detail, "b.md:4");
    assert_eq!(items[0].line, Some(4));
    assert_eq!(items[1].label, "plain");
}

#[test]
fn availability_rules() {
    let none = Sources {
        page: true,
        ..Sources::default()
    };
    assert_eq!(notebook::available(Op::Notes, none), Ok(()));
    assert_eq!(notebook::available(Op::Links, none), Ok(()));
    assert_eq!(
        notebook::available(Op::Search, none),
        Err("Search needs mdroots or a zk server")
    );
    assert_eq!(
        notebook::available(Op::Backlinks, none),
        Err("Backlinks need mdroots or a language server")
    );
    let zk = Sources {
        server: Some(Kind::Zk),
        references: true,
        ..none
    };
    assert!(
        Op::ALL
            .iter()
            .all(|&op| notebook::available(op, zk).is_ok())
    );
    let marksman = Sources {
        server: Some(Kind::Marksman),
        references: true,
        ..none
    };
    assert!(
        notebook::available(Op::Backlinks, marksman).is_err(),
        "no heading"
    );
    assert!(notebook::available(Op::Tags, marksman).is_err());
    let with_heading = Sources {
        heading: true,
        ..marksman
    };
    assert_eq!(notebook::available(Op::Backlinks, with_heading), Ok(()));
    let no_refs = Sources {
        references: false,
        ..with_heading
    };
    assert!(notebook::available(Op::Backlinks, no_refs).is_err());
    // mdroots serves the page: everything, backlinks without a heading.
    let mdroots = Sources {
        mdroots: true,
        ..none
    };
    assert!(
        Op::ALL
            .iter()
            .all(|&op| notebook::available(op, mdroots).is_ok())
    );
}

#[test]
fn walk_lists_markdown_respecting_ignores_and_depth() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    std::fs::write(r.join("a.md"), "").unwrap();
    std::fs::write(r.join("x.txt"), "").unwrap();
    std::fs::write(r.join(".ignore"), "skipped/\n").unwrap();
    std::fs::create_dir_all(r.join("skipped")).unwrap();
    std::fs::write(r.join("skipped/s.md"), "").unwrap();
    std::fs::create_dir_all(r.join(".hidden")).unwrap();
    std::fs::write(r.join(".hidden/h.md"), "").unwrap();
    let deep = r.join("1/2/3/4/5/6/7/8");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("too-deep.md"), "").unwrap();
    std::fs::write(r.join("1/2/3/4/5/6/7/ok.md"), "").unwrap();
    let details: Vec<String> = notebook::walk_notes(r)
        .into_iter()
        .map(|i| i.detail)
        .collect();
    assert_eq!(details, ["1/2/3/4/5/6/7/ok.md", "a.md"]);
}

#[test]
fn links_items_from_the_local_parse() {
    let d = doc::parse("[one](a.md) and [[wiki]] and <https://x.y>\n".into());
    let items = notebook::link_items(&d);
    let pairs: Vec<(&str, &str)> = items
        .iter()
        .map(|i| (i.label.as_str(), i.detail.as_str()))
        .collect();
    assert_eq!(
        pairs,
        [
            ("one", "a.md"),
            ("wiki", "wiki"),
            ("https://x.y", "https://x.y")
        ]
    );
    assert_eq!(items[1].link, Some(1));
}

#[test]
fn fuzzy_filter_ranks_and_keeps_order_for_empty_queries() {
    let item = |l: &str, d: &str| Item {
        label: l.into(),
        detail: d.into(),
        ..Item::default()
    };
    let items = [
        item("Meeting notes", "m.md"),
        item("Project plan", "p.md"),
        item("Plan B", "b.md"),
    ];
    assert_eq!(picker_filter(&items, ""), [0, 1, 2]);
    let hits = picker_filter(&items, "plan");
    assert_eq!(hits.len(), 2);
    assert!(hits.contains(&1) && hits.contains(&2));
    assert_eq!(picker_filter(&items, "prjpln"), [1], "fuzzy");
    assert!(picker_filter(&items, "zzz").is_empty());
}

#[test]
fn front_matter_is_not_a_heading_for_labels() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join("t.md"), "---\ntags: [project]\n---\n# Tagged\n").unwrap();
    let uri = ramble::lsp::canonical_uri(&root.join("t.md"))
        .as_str()
        .to_string();
    let items = lsp_notebook::location_items(
        &json!([{"uri": uri, "range": {"start": {"line": 0, "character": 0},
                                       "end": {"line": 0, "character": 0}}}]),
        &root,
    );
    assert_eq!(items[0].label, "Tagged");
}

/// Front matter is metadata: the marksman backlinks position is the note's
/// real first heading, not a line inside the YAML block.
#[test]
fn marksman_backlinks_position_skips_front_matter() {
    let src = std::fs::read_to_string("tests/fixtures/zk/tagged.md").unwrap();
    assert!(src.starts_with("---\n"), "fixture must have front matter");
    let doc = ramble::doc::parse(src.clone());
    let pos = lsp_notebook::backlinks_position(
        ramble::lsp::Kind::Marksman,
        &doc,
        ramble::lsp::Encoding::Utf16,
    )
    .unwrap();
    let line = src.lines().nth(pos.line as usize).unwrap();
    assert!(line.starts_with("# "), "position lands on {line:?}");
}
