//! App settings persisted to `%LOCALAPPDATA%\stickynotes\config.json`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DEFAULT_REMOTE_DIR: &str = "/StickyNotes/";

/// File name of the note database inside the data folder.
pub const DB_FILE_NAME: &str = "stickynotes.db";

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
pub struct AppConfig {
    /// WebDAV sync: debounced upload after edits + periodic background pull.
    #[serde(default = "default_true")]
    pub webdav_auto: bool,
    #[serde(default)]
    pub webdav: WebDavConfig,
    /// Folder that holds the note database (`stickynotes.db`). `None` keeps
    /// it in the default app-data folder, next to this config file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<PathBuf>,
    /// The one-time carry-over from Windows Sticky Notes (and this app's old
    /// side files) has run on this machine. A database created afterwards
    /// starts empty instead of importing that old state again — by then it
    /// may contain notes that have since been deleted here.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub legacy_migrated: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            webdav_auto: true,
            webdav: WebDavConfig::default(),
            data_dir: None,
            legacy_migrated: false,
        }
    }
}

/// `%LOCALAPPDATA%\stickynotes`: settings, sync bookkeeping, backups and —
/// unless the user picked another folder — the note database.
pub fn config_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join("stickynotes")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

/// Where the note database lives when the user has not chosen a folder.
pub fn default_data_dir() -> PathBuf {
    config_dir()
}

/// The database file inside a data folder.
pub fn database_path_in(dir: &Path) -> PathBuf {
    dir.join(DB_FILE_NAME)
}

impl AppConfig {
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

    /// Written through a temporary file: `data_dir` decides which database
    /// the app opens, so a half-written config must never replace a good one.
    pub fn save(&self) -> std::io::Result<()> {
        let dir = config_dir();
        std::fs::create_dir_all(&dir)?;
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = dir.join("config.json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, config_path())
    }

    /// Enough settings to talk to a WebDAV server.
    pub fn webdav_configured(&self) -> bool {
        let w = &self.webdav;
        !w.url.trim().is_empty() && !w.username.trim().is_empty() && !w.password.is_empty()
    }

    /// Folder the note database is kept in.
    pub fn resolved_data_dir(&self) -> PathBuf {
        match &self.data_dir {
            Some(dir) if !dir.as_os_str().is_empty() => dir.clone(),
            _ => default_data_dir(),
        }
    }

    /// Full path of the note database.
    pub fn database_path(&self) -> PathBuf {
        database_path_in(&self.resolved_data_dir())
    }

    pub fn uses_default_data_dir(&self) -> bool {
        self.resolved_data_dir() == default_data_dir()
    }

    /// Remember `dir` as the data folder; the default folder is stored as
    /// "no override" so it keeps following `%LOCALAPPDATA%`.
    pub fn set_data_dir(&mut self, dir: &Path) {
        self.data_dir = (dir != default_data_dir()).then(|| dir.to_path_buf());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_enable_webdav_and_use_the_default_folder() {
        let cfg = AppConfig::default();
        assert!(cfg.webdav_auto);
        assert_eq!(cfg.webdav.dir, "/StickyNotes/");
        assert!(!cfg.webdav_configured());
        assert!(cfg.uses_default_data_dir());
        assert_eq!(cfg.database_path(), default_data_dir().join("stickynotes.db"));
    }

    #[test]
    fn old_config_files_get_defaults() {
        // Written by versions that still had the Microsoft-sync switch.
        let cfg: AppConfig = serde_json::from_str(r#"{"microsoft":true}"#).unwrap();
        assert!(cfg.webdav_auto);
        assert_eq!(cfg.webdav.dir, "/StickyNotes/");
        assert!(cfg.data_dir.is_none());
        assert!(!cfg.legacy_migrated);
    }

    #[test]
    fn the_migration_flag_is_only_written_once_set() {
        let mut cfg = AppConfig::default();
        assert!(!serde_json::to_string(&cfg).unwrap().contains("legacy_migrated"));
        cfg.legacy_migrated = true;
        let back: AppConfig = serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert!(back.legacy_migrated);
    }

    #[test]
    fn custom_data_dir_round_trips_and_default_is_not_stored() {
        let mut cfg = AppConfig::default();
        let custom = PathBuf::from(r"D:\Notes\便签");
        cfg.set_data_dir(&custom);
        assert!(!cfg.uses_default_data_dir());
        assert_eq!(cfg.database_path(), custom.join("stickynotes.db"));

        let text = serde_json::to_string(&cfg).unwrap();
        let back: AppConfig = serde_json::from_str(&text).unwrap();
        assert_eq!(back.data_dir.as_deref(), Some(custom.as_path()));

        cfg.set_data_dir(&default_data_dir());
        assert!(cfg.data_dir.is_none());
        assert!(!serde_json::to_string(&cfg).unwrap().contains("data_dir"));
    }

    #[test]
    fn empty_data_dir_falls_back_to_the_default() {
        let cfg: AppConfig = serde_json::from_str(r#"{"data_dir":""}"#).unwrap();
        assert!(cfg.uses_default_data_dir());
    }
}
