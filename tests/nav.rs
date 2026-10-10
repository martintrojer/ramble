//! nav: link destination resolution and history.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ramble::doc::LinkKind;
use ramble::nav::{self, Entry, History, PageRef, Target};

fn file(path: &str, anchor: Option<&str>) -> Target {
    Target::File {
        path: PathBuf::from(path),
        anchor: anchor.map(str::to_string),
    }
}

#[test]
fn resolve_table() {
    let dir = Path::new("/notes/sub");
    let md = LinkKind::Markdown;
    let wiki = LinkKind::Wiki;
    let cases: &[(&str, &LinkKind, Target)] = &[
        ("b.md", &md, file("/notes/sub/b.md", None)),
        ("./b.md", &md, file("/notes/sub/./b.md", None)),
        ("../up.md", &md, file("/notes/sub/../up.md", None)),
        ("/abs/x.md", &md, file("/abs/x.md", None)),
        ("#section-two", &md, Target::Anchor("section-two".into())),
        ("b.md#intro", &md, file("/notes/sub/b.md", Some("intro"))),
        ("b.md#", &md, file("/notes/sub/b.md", None)),
        ("my%20note.md", &md, file("/notes/sub/my note.md", None)),
        ("caf%C3%A9.md", &md, file("/notes/sub/café.md", None)),
        ("100%.md", &md, file("/notes/sub/100%.md", None)),
        (
            "https://example.com/a#b",
            &md,
            Target::External("https://example.com/a#b".into()),
        ),
        ("http://x.org", &md, Target::External("http://x.org".into())),
        (
            "mailto:me@x.org",
            &md,
            Target::External("mailto:me@x.org".into()),
        ),
        (
            "obsidian+x://open",
            &md,
            Target::External("obsidian+x://open".into()),
        ),
        ("C:/dos.md", &md, file("/notes/sub/C:/dos.md", None)),
        ("file:///abs/f.md#h", &md, file("/abs/f.md", Some("h"))),
        ("file://localhost/abs/f.md", &md, file("/abs/f.md", None)),
        ("other", &wiki, file("/notes/sub/other.md", None)),
        (
            "other#head",
            &wiki,
            file("/notes/sub/other.md", Some("head")),
        ),
        ("pic.png", &wiki, file("/notes/sub/pic.png", None)),
        ("other", &md, file("/notes/sub/other", None)),
    ];
    for (dest, kind, want) in cases {
        assert_eq!(&nav::resolve(dest, kind, dir), want, "dest {dest:?}");
    }
}

#[test]
fn extensionless_markdown_link_gets_md_only_when_that_file_exists() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    std::fs::write(d.join("a.md"), "").unwrap();
    std::fs::write(d.join("both"), "").unwrap();
    std::fs::write(d.join("both.md"), "").unwrap();
    std::fs::create_dir(d.join("sub")).unwrap();
    std::fs::write(d.join("sub.md"), "").unwrap();
    let md = LinkKind::Markdown;
    let at = |name: &str, anchor: Option<&str>| Target::File {
        path: d.join(name),
        anchor: anchor.map(str::to_string),
    };
    assert_eq!(nav::resolve("a", &md, d), at("a.md", None));
    assert_eq!(nav::resolve("a#h", &md, d), at("a.md", Some("h")));
    assert_eq!(
        nav::resolve("both", &md, d),
        at("both", None),
        "bare file wins"
    );
    assert_eq!(
        nav::resolve("sub", &md, d),
        at("sub", None),
        "bare dir wins"
    );
    assert_eq!(nav::resolve("none", &md, d), at("none", None));
}

#[test]
fn is_markdown_extensions() {
    for p in ["a.md", "a.MD", "a.markdown", "a.mdown", "a.mkd"] {
        assert!(nav::is_markdown(Path::new(p)), "{p}");
    }
    for p in ["a.txt", "a", "a.md.bak"] {
        assert!(!nav::is_markdown(Path::new(p)), "{p}");
    }
}

fn e(name: &str, row: usize) -> Entry {
    Entry {
        page: PageRef::File(PathBuf::from(name)),
        cursor_row: row,
        cursor_col: 0,
        scroll: 0,
        anchor: None,
        sidebar: None,
        raw: false,
    }
}

#[test]
fn history_back_forward_depth() {
    let mut h = History::new();
    assert_eq!(h.depth(), 0);
    assert_eq!(h.back(e("a", 0)), None);
    assert_eq!(h.forward(e("a", 0)), None);
    assert_eq!(h.depth(), 0, "failed moves change nothing");

    h.push(e("a", 1));
    h.push(e("b", 2));
    assert_eq!(h.depth(), 2);
    // current is c
    assert_eq!(h.back(e("c", 3)), Some(e("b", 2)));
    assert_eq!(h.depth(), 1);
    assert_eq!(h.back(e("b", 4)), Some(e("a", 1)));
    assert_eq!(h.depth(), 0);
    assert_eq!(h.back(e("a", 1)), None);
    assert_eq!(h.forward(e("a", 5)), Some(e("b", 4)));
    assert_eq!(h.forward(e("b", 4)), Some(e("c", 3)));
    assert_eq!(h.forward(e("c", 3)), None);
    assert_eq!(h.depth(), 2);
    assert_eq!(h.back(e("c", 3)), Some(e("b", 4)));
    assert_eq!(
        h.back(e("b", 4)),
        Some(e("a", 5)),
        "back remembers updated a"
    );
}

#[test]
fn push_clears_forward() {
    let mut h = History::new();
    h.push(e("a", 0));
    assert_eq!(h.back(e("b", 0)), Some(e("a", 0)));
    assert_eq!(h.forward_depth(), 1);
    h.push(e("a", 0));
    assert_eq!(h.forward_depth(), 0);
    assert_eq!(h.forward(e("d", 0)), None);
    assert_eq!(h.depth(), 1);
}

#[test]
fn stdin_entries_keep_text() {
    let text = Arc::new(String::from("# Hi\n"));
    let mut h = History::new();
    h.push(Entry {
        page: PageRef::Stdin(text.clone()),
        cursor_row: 0,
        cursor_col: 0,
        scroll: 0,
        anchor: None,
        sidebar: None,
        raw: false,
    });
    let back = h.back(e("b", 0)).unwrap();
    assert_eq!(back.page, PageRef::Stdin(text));
}
