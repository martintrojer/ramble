//! Optional backends next to the built-in [mdroots](https://github.com/martintrojer/mdroots)
//! one (`app::mdroots_glue`): a third-party markdown language server, turned
//! on per root with `[[lsp.server]]`.

pub mod lsp;
