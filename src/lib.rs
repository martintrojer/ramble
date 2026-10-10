//! ramble: a read-only TUI markdown reader backed by the built-in mdroots
//! index, with optional markdown language servers.
//!
//! Module layout follows `docs/design.md` § Architecture.

pub mod app;
pub mod backend;
pub mod cli;
pub mod config;
pub mod doc;
pub mod nav;
pub mod notebook;
pub mod render;
pub mod ui;

// The optional LSP backend, also at the crate root so `ramble::lsp::…` paths
// (tests, the fake-lsp binary) keep working.
pub use backend::lsp;
