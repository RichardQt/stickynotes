//! App settings persisted to `%LOCALAPPDATA%\stickynotes\config.json`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const DEFAULT_REMOTE_DIR: &str = "/StickyNotes/";

/// File name of the note database inside the data folder.
pub const DB_FILE_NAME: &str = "stickynotes.db";

/// How long after the last edit the debounced WebDAV upload starts.
pub const DEFAULT_SYNC_DELAY_MS: u64 = 2500;
pub const MIN_SYNC_DELAY_MS: u64 = 500;
pub const MAX_SYNC_DELAY_MS: u64 = 600_000;

/// How often the background loop silently checks the server for other
/// devices' changes. Returning to the window still checks right away.
pub const PULL_INTERVAL_VISIBLE: Duration = Duration::from_secs(300);
pub const PULL_INTERVAL_TRAY: Duration = Duration::from_secs(600);

fn default_true() -> bool {
    true
}

fn default_sync_delay_ms() -> u64 {
    DEFAULT_SYNC_DELAY_MS
}

/// Read a setting, falling back to its default when the stored value has the
/// wrong shape. One bad setting must not fail the whole file: `load` would
/// then start from defaults and forget the WebDAV account.
fn lenient<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned + Default,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

fn lenient_sync_delay_ms<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_u64()
        .filter(|ms| *ms > 0)
        .map(clamp_sync_delay_ms)
        .unwrap_or(DEFAULT_SYNC_DELAY_MS))
}

pub fn clamp_sync_delay_ms(ms: u64) -> u64 {
    ms.clamp(MIN_SYNC_DELAY_MS, MAX_SYNC_DELAY_MS)
}

/// Why a typed sync delay was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncDelayError {
    Empty,
    NotANumber,
    TooPrecise,
    OutOfRange,
}

/// Parse the sync delay as the user types it: seconds, at most one decimal
/// place, between 0.5 and 600. Returns milliseconds.
pub fn parse_sync_delay_secs(text: &str) -> Result<u64, SyncDelayError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(SyncDelayError::Empty);
    }
    let (whole, frac) = text.split_once('.').unwrap_or((text, ""));
    let digits = |part: &str| part.chars().all(|c| c.is_ascii_digit());
    if !digits(whole) || !digits(frac) || (whole.is_empty() && frac.is_empty()) {
        return Err(SyncDelayError::NotANumber);
    }
    if frac.len() > 1 {
        return Err(SyncDelayError::TooPrecise);
    }
    // More digits than any value in range has: out of range, not an overflow.
    if whole.trim_start_matches('0').len() > 6 {
        return Err(SyncDelayError::OutOfRange);
    }
    let whole_ms = whole.parse::<u64>().unwrap_or(0) * 1000;
    let frac_ms = frac.parse::<u64>().unwrap_or(0) * 100;
    let ms = whole_ms + frac_ms;
    if !(MIN_SYNC_DELAY_MS..=MAX_SYNC_DELAY_MS).contains(&ms) {
        return Err(SyncDelayError::OutOfRange);
    }
    Ok(ms)
}

/// A delay in the form it is typed and shown: `2.5`, `10`, `0.5`.
pub fn format_sync_delay_secs(ms: u64) -> String {
    let tenths = (ms + 50) / 100;
    if tenths % 10 == 0 {
        (tenths / 10).to_string()
    } else {
        format!("{}.{}", tenths / 10, tenths % 10)
    }
}

/// What closing the main window does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseBehavior {
    /// Quit the app.
    #[default]
    Quit,
    /// Hide the window; the app keeps running in the notification area.
    Tray,
}

/// How the note list is ordered inside each pinned group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteSort {
    /// Most recently edited first (the historical order).
    #[default]
    RecentUpdated,
    /// Most recently created first.
    RecentCreated,
}

/// Where the main window was last seen, in logical pixels. Written whenever
/// the window is moved or resized, so the app reopens at the same place.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowPlacement {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
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
    /// Optional proxy for sync traffic, e.g. `http://127.0.0.1:7890` or
    /// `socks5://127.0.0.1:7891`. Empty connects directly: the sync client
    /// reads proxy environment variables only, never the Windows system
    /// proxy setting.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub proxy: String,
}

impl Default for WebDavConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            username: String::new(),
            password: String::new(),
            dir: default_dir(),
            proxy: String::new(),
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
    /// Delay between the last edit and the automatic upload, in milliseconds.
    #[serde(default = "default_sync_delay_ms", deserialize_with = "lenient_sync_delay_ms")]
    pub sync_delay_ms: u64,
    #[serde(default, deserialize_with = "lenient")]
    pub close_behavior: CloseBehavior,
    /// Order of the note list, within the pinned notes.
    #[serde(default, deserialize_with = "lenient")]
    pub note_sort: NoteSort,
    /// Size and position of the main window, so it reopens where the user
    /// left it. Missing (or unreadable) means the built-in start bounds.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient"
    )]
    pub window_bounds: Option<WindowPlacement>,
    /// Keyboard shortcuts the user changed: action id → key combination, an
    /// empty string for "no shortcut". Actions not listed keep their default.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty", deserialize_with = "lenient")]
    pub shortcuts: BTreeMap<String, String>,
    /// The "still running in the notification area" hint has been shown once.
    #[serde(default, skip_serializing_if = "std::ops::Not::not", deserialize_with = "lenient")]
    pub tray_hint_shown: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            webdav_auto: true,
            webdav: WebDavConfig::default(),
            data_dir: None,
            legacy_migrated: false,
            sync_delay_ms: DEFAULT_SYNC_DELAY_MS,
            close_behavior: CloseBehavior::default(),
            note_sort: NoteSort::default(),
            window_bounds: None,
            shortcuts: BTreeMap::new(),
            tray_hint_shown: false,
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

    /// Delay between the last edit and the automatic upload.
    pub fn sync_delay(&self) -> Duration {
        Duration::from_millis(clamp_sync_delay_ms(self.sync_delay_ms))
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
    fn a_proxy_is_optional_and_round_trips() {
        let cfg: AppConfig = serde_json::from_str("{}").unwrap();
        assert!(cfg.webdav.proxy.is_empty());
        assert!(!serde_json::to_string(&cfg).unwrap().contains("proxy"));

        let cfg: AppConfig = serde_json::from_str(
            r#"{"webdav":{"proxy":"socks5://127.0.0.1:7891"}}"#,
        )
        .unwrap();
        assert_eq!(cfg.webdav.proxy, "socks5://127.0.0.1:7891");
        let text = serde_json::to_string(&cfg).unwrap();
        assert!(text.contains(r#""proxy":"socks5://127.0.0.1:7891""#), "{text}");
    }

    #[test]
    fn old_config_files_get_defaults() {
        // Written by versions that still had the Microsoft-sync switch.
        let cfg: AppConfig = serde_json::from_str(r#"{"microsoft":true}"#).unwrap();
        assert!(cfg.webdav_auto);
        assert_eq!(cfg.note_sort, NoteSort::RecentUpdated);
        assert_eq!(cfg.webdav.dir, "/StickyNotes/");
        assert!(cfg.data_dir.is_none());
        assert!(!cfg.legacy_migrated);
    }

    #[test]
    fn the_window_placement_round_trips_and_bad_values_are_ignored() {
        let cfg: AppConfig = serde_json::from_str(
            r#"{"window_bounds":{"x":-40.0,"y":72.0,"width":1180.0,"height":760.0}}"#,
        )
        .unwrap();
        let placed = WindowPlacement { x: -40., y: 72., width: 1180., height: 760. };
        assert_eq!(cfg.window_bounds, Some(placed));
        let text = serde_json::to_string(&cfg).unwrap();
        let back: AppConfig = serde_json::from_str(&text).unwrap();
        assert_eq!(back.window_bounds, Some(placed));

        // A malformed placement costs only the placement, not the whole file.
        let cfg: AppConfig = serde_json::from_str(
            r#"{"webdav":{"url":"https://dav.example/","username":"u","password":"p"},
                "window_bounds":{"x":"left"}}"#,
        )
        .unwrap();
        assert!(cfg.webdav_configured());
        assert_eq!(cfg.window_bounds, None);
        assert!(!serde_json::to_string(&AppConfig::default())
            .unwrap()
            .contains("window_bounds"));
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

    #[test]
    fn new_settings_default_to_quitting_and_a_2_5_second_delay() {
        let cfg: AppConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.close_behavior, CloseBehavior::Quit);
        assert_eq!(cfg.sync_delay(), Duration::from_millis(2500));
        assert!(cfg.shortcuts.is_empty());
        assert!(!cfg.tray_hint_shown);
        let text = serde_json::to_string(&cfg).unwrap();
        assert!(!text.contains("shortcuts") && !text.contains("tray_hint_shown"));
    }

    #[test]
    fn new_settings_round_trip() {
        let mut cfg = AppConfig::default();
        cfg.close_behavior = CloseBehavior::Tray;
        cfg.sync_delay_ms = 10_000;
        cfg.shortcuts.insert("search".into(), "Ctrl+Alt+N".into());
        cfg.shortcuts.insert("md_bold".into(), String::new());
        cfg.tray_hint_shown = true;
        let text = serde_json::to_string(&cfg).unwrap();
        assert!(text.contains(r#""close_behavior":"tray""#), "{text}");
        let back: AppConfig = serde_json::from_str(&text).unwrap();
        assert_eq!(back.close_behavior, CloseBehavior::Tray);
        assert_eq!(back.sync_delay_ms, 10_000);
        assert_eq!(back.shortcuts, cfg.shortcuts);
        assert!(back.tray_hint_shown);
    }

    #[test]
    fn a_malformed_new_setting_does_not_cost_the_webdav_account() {
        let cfg: AppConfig = serde_json::from_str(
            r#"{"webdav":{"url":"https://dav.example/","username":"u","password":"p"},
                "close_behavior":"minimise","sync_delay_ms":"soon",
                "shortcuts":["ctrl-n"],"tray_hint_shown":"yes"}"#,
        )
        .unwrap();
        assert!(cfg.webdav_configured());
        assert_eq!(cfg.close_behavior, CloseBehavior::Quit);
        assert_eq!(cfg.sync_delay_ms, DEFAULT_SYNC_DELAY_MS);
        assert!(cfg.shortcuts.is_empty());
        assert!(!cfg.tray_hint_shown);
    }

    #[test]
    fn a_stored_delay_is_kept_inside_the_allowed_range() {
        let parse = |json: &str| serde_json::from_str::<AppConfig>(json).unwrap().sync_delay_ms;
        assert_eq!(parse(r#"{"sync_delay_ms":0}"#), DEFAULT_SYNC_DELAY_MS);
        assert_eq!(parse(r#"{"sync_delay_ms":-5}"#), DEFAULT_SYNC_DELAY_MS);
        assert_eq!(parse(r#"{"sync_delay_ms":20}"#), MIN_SYNC_DELAY_MS);
        assert_eq!(parse(r#"{"sync_delay_ms":99999999}"#), MAX_SYNC_DELAY_MS);
        assert_eq!(parse(r#"{"sync_delay_ms":4000}"#), 4000);
        // A value changed behind the app's back is still clamped when used.
        let cfg = AppConfig { sync_delay_ms: 1, ..AppConfig::default() };
        assert_eq!(cfg.sync_delay(), Duration::from_millis(MIN_SYNC_DELAY_MS));
    }

    #[test]
    fn legacy_pull_interval_field_is_ignored_without_losing_settings() {
        let cfg: AppConfig = serde_json::from_str(
            r#"{"webdav":{"url":"https://dav.example/","username":"u","password":"p"},
                "pull_interval_secs":180,"sync_delay_ms":5000}"#,
        )
        .unwrap();
        assert!(cfg.webdav_configured());
        assert_eq!(cfg.sync_delay_ms, 5000);
        let serialized = serde_json::to_string(&cfg).unwrap();
        assert!(!serialized.contains("pull_interval_secs"), "{serialized}");
    }

    #[test]
    fn typed_delays_are_seconds_with_at_most_one_decimal() {
        assert_eq!(parse_sync_delay_secs("2.5"), Ok(2500));
        assert_eq!(parse_sync_delay_secs(" 10 "), Ok(10_000));
        assert_eq!(parse_sync_delay_secs("0.5"), Ok(500));
        assert_eq!(parse_sync_delay_secs(".5"), Ok(500));
        assert_eq!(parse_sync_delay_secs("3."), Ok(3000));
        assert_eq!(parse_sync_delay_secs("600"), Ok(600_000));
        assert_eq!(parse_sync_delay_secs("0600.0"), Ok(600_000));
        assert_eq!(parse_sync_delay_secs(""), Err(SyncDelayError::Empty));
        assert_eq!(parse_sync_delay_secs("  "), Err(SyncDelayError::Empty));
        for bad in ["abc", "2,5", "-3", "+3", "1e2", "2.5s", ".", "1.2.3", "２"] {
            assert_eq!(parse_sync_delay_secs(bad), Err(SyncDelayError::NotANumber), "{bad}");
        }
        assert_eq!(parse_sync_delay_secs("2.55"), Err(SyncDelayError::TooPrecise));
        for out in ["0", "0.4", "600.1", "601", "99999999999999999999999"] {
            assert_eq!(parse_sync_delay_secs(out), Err(SyncDelayError::OutOfRange), "{out}");
        }
    }

    #[test]
    fn delays_are_shown_without_a_trailing_zero() {
        assert_eq!(format_sync_delay_secs(2500), "2.5");
        assert_eq!(format_sync_delay_secs(10_000), "10");
        assert_eq!(format_sync_delay_secs(500), "0.5");
        assert_eq!(format_sync_delay_secs(600_000), "600");
        // Every value the form accepts comes back as it was typed.
        for text in ["0.5", "1", "2.5", "59.9", "600"] {
            assert_eq!(format_sync_delay_secs(parse_sync_delay_secs(text).unwrap()), text);
        }
    }
}
