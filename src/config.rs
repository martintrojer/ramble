//! Config loading over built-in defaults: servers, leader key, launchers,
//! render and sidebar options.
//!
//! Interface fixed by the orchestrator: t04_cli uses [`Config::default`],
//! [`Config::load`] and [`write_default`]; t09_config implements loading
//! and the full option set and may add fields, but must keep these names.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub render: RenderConfig,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderConfig {
    /// Upper bound on the reflow width.
    pub max_width: u16,
    pub theme: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            render: RenderConfig {
                max_width: 100,
                theme: "catppuccin-mocha".into(),
            },
        }
    }
}

impl Config {
    /// `~/.config/ramble/config.toml` (respecting `$XDG_CONFIG_HOME`).
    pub fn default_path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("ramble").join("config.toml"))
    }

    /// Load `path` over the defaults. A missing file yields the defaults.
    /// A parse error is returned with its line number in the message.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        todo!("t09_config")
    }
}

/// Write the commented default config to `path`. Refuses to overwrite an
/// existing file.
pub fn write_default(path: &Path) -> anyhow::Result<()> {
    let _ = path;
    todo!("t09_config")
}
