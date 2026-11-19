//! App settings persisted to `%LOCALAPPDATA%\stickynotes\config.json`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_REMOTE_DIR: &str = "/StickyNotes/";

fn default_true() -> bool {
    true
}

fn default_dir() -> String {
    DEFAULT_REMOTE_DIR.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebDavConfig {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub username: String,
    /// Stored as-is in the user's profile; set an app-specific password.
    #[serde(default)]
    pub password: String,
    /// Collection on the server; created with MKCOL when missing.
    #[serde(default = "default_dir")]
    pub dir: String,
}

impl Default for WebDavConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            username: String::new(),
            password: String::new(),
            dir: default_dir(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    /// WebDAV sync: debounced upload after edits + periodic background pull.
    #[serde(default = "default_true")]
    pub webdav_auto: bool,
    /// Opt-in: follow edits made in the native Windows Sticky Notes app and
    /// its cloud. The cloud exchange itself is performed by that app whenever
    /// it runs; plum only stores plain text (no headings, fences or tables),
    /// so this path never carries the Markdown formatting kept for WebDAV.
    #[serde(default)]
    pub microsoft: bool,
    #[serde(default)]
    pub webdav: WebDavConfig,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            webdav_auto: true,
            microsoft: false,
            webdav: WebDavConfig { dir: default_dir(), ..Default::default() },
        }
    }
}

pub fn config_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join("stickynotes")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

impl SyncConfig {
    pub fn load() -> Self {
        let Ok(text) = std::fs::read_to_string(config_path()) else {
            return Self::default();
        };
        let mut cfg: Self = serde_json::from_str(&text).unwrap_or_default();
        if cfg.webdav.dir.trim().is_empty() {
            cfg.webdav.dir = default_dir();
        }
        cfg
    }

    pub fn save(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(config_dir())?;
        let text = serde_json::to_string_pretty(self)
            .map_err(std::io::Error::other)?;
        std::fs::write(config_path(), text)
    }

    /// Enough settings to talk to a WebDAV server.
    pub fn webdav_configured(&self) -> bool {
        let w = &self.webdav;
        !w.url.trim().is_empty() && !w.username.trim().is_empty() && !w.password.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_enable_webdav_and_disable_microsoft() {
        let cfg = SyncConfig::default();
        assert!(cfg.webdav_auto);
        assert!(!cfg.microsoft);
        assert_eq!(cfg.webdav.dir, "/StickyNotes/");
        assert!(!cfg.webdav_configured());
    }

    #[test]
    fn old_config_files_get_defaults() {
        let cfg: SyncConfig = serde_json::from_str(r#"{"microsoft":true}"#).unwrap();
        assert!(cfg.webdav_auto);
        assert_eq!(cfg.webdav.dir, "/StickyNotes/");
    }
}
