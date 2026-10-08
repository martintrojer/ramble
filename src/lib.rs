//! ramble: a read-only TUI markdown reader backed by markdown language servers.
//!
//! Module layout follows `docs/specs/2026-10-07-ramble.md` § Architecture.

pub mod app;
pub mod cli;
pub mod config;
pub mod doc;
pub mod frontmatter;
pub mod lsp;
pub mod nav;
pub mod notebook;
pub mod render;
pub mod review;
pub mod ui;
