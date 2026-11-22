use crate::quick_note::QuickNote;
use crate::win;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use markdown_editor::{BlockEditorEvent, BlockEditorState};
use stickynotes_core::{
    config::{self, AppConfig, SyncDelayError},
    shortcuts,
    store::{
        self, backup, import, legacy,
        model::{NoteSummary, RemoteUpdate, Tag},
        NoteStore,
    },
    ticks, webdav,
};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant, UNIX_EPOCH};

mod keys;
mod quick;
mod shell;

pub(crate) use keys::md_command;

/// Where the source code lives; shown on the settings dialog's About page.
pub const REPOSITORY_URL: &str = "https://github.com/RichardQt/stickynotes";

/// Visual spec for note papers with refined paper aesthetics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeSpec {
    pub name: &'static str,
    pub label: &'static str,
    pub accent: u32,
    pub paper_bg: u32,
    pub card_active_bg: u32,
    pub border: u32,
}

pub const THEMES: &[ThemeSpec] = &[
    ThemeSpec {
        name: "White",
        label: "纯白",
        accent: 0xFFFFFF,
        paper_bg: 0xFFFFFF,
        card_active_bg: 0xEEF4FC,
        border: 0xBDD2EC,
    },
    ThemeSpec {
        name: "Yellow",
        label: "暖阳黄",
        accent: 0xEAB548,
        paper_bg: 0xFFFCF0,
        card_active_bg: 0xFDF3CE,
        border: 0xEEDB9B,
    },
    ThemeSpec {
        name: "Green",
        label: "薄荷绿",
        accent: 0x5DB589,
        paper_bg: 0xF4FCF7,
        card_active_bg: 0xE0F4E7,
        border: 0xBEE5CC,
    },
    ThemeSpec {
        name: "Blue",
        label: "天空蓝",
        accent: 0x67A2D6,
        paper_bg: 0xF4F9FE,
        card_active_bg: 0xE1EFFB,
        border: 0xBED8F0,
    },
    ThemeSpec {
        name: "Pink",
        label: "蔷薇粉",
        accent: 0xDF8AA5,
        paper_bg: 0xFEF6F9,
        card_active_bg: 0xF9E3EB,
        border: 0xF0CAD8,
    },
    ThemeSpec {
        name: "Purple",
        label: "薰衣紫",
        accent: 0x9E88D8,
        paper_bg: 0xF9F6FE,
        card_active_bg: 0xEDE4F9,
        border: 0xD9CBF2,
    },
    ThemeSpec {
        name: "Gray",
        label: "云雾灰",
        accent: 0x909CAA,
        paper_bg: 0xF7F9FB,
        card_active_bg: 0xEAEFF5,
        border: 0xCFD8E4,
    },
];

pub fn get_theme(name: &str) -> &'static ThemeSpec {
    // Windows Sticky Notes also offers a dark "Charcoal" paper. This app draws
    // only light papers, so a note imported with it is shown on the neutral
    // gray one. Its stored theme stays as it is until a colour is picked here.
    let target = if name.eq_ignore_ascii_case("Charcoal") {
        "Gray"
    } else {
        name
    };
    THEMES
        .iter()
        .find(|t| t.name.eq_ignore_ascii_case(target))
        .unwrap_or(&THEMES[0])
}

#[derive(Debug, Clone, PartialEq)]
pub enum SyncStatus {
    Idle,
    Saved(String),
    Error(String),
    Busy(String),
}

/// What a WebDAV pass is for, which decides whether it shows itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPass {
    /// The periodic or "back from another app" freshness check: no
    /// "正在同步…" and no status line unless it changed something or failed.
    Quiet,
    /// Started by an edit or a settings change: shows its state.
    Visible,
    /// The "立即同步" button: shows its state and ignores the version cache.
    Forced,
}

impl SyncPass {
    fn quiet(self) -> bool {
        self == Self::Quiet
    }

    fn force(self) -> bool {
        self == Self::Forced
    }
}

/// Which drop-down list of the settings dialog is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsMenu {
    CloseBehavior,
    PullInterval,
}

/// Result of the settings dialog's "test connection" probe.
#[derive(Debug, Clone, PartialEq)]
pub enum WebDavTest {
    Idle,
    Testing,
    Ok(String),
    Err(String),
}

/// Outcome of the last action in the settings dialog's storage section.
#[derive(Debug, Clone, PartialEq)]
pub enum StorageNote {
    Info(String),
    Error(String),
}

/// Settings dialog form fields, created on first open.
pub struct SettingsInputs {
    pub url: Entity<InputState>,
    pub user: Entity<InputState>,
    pub pass: Entity<InputState>,
    pub dir: Entity<InputState>,
    /// Seconds between the last edit and the automatic upload.
    pub delay: Entity<InputState>,
}

/// The pages of the settings dialog, in the order of its left column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPage {
    General,
    Shortcuts,
    About,
}

impl SettingsPage {
    pub const fn index(self) -> usize {
        match self {
            Self::General => 0,
            Self::Shortcuts => 1,
            Self::About => 2,
        }
    }
}

/// The quick-note view and, when it has a window of its own, that window.
pub(crate) struct QuickHost {
    pub(crate) view: Entity<QuickNote>,
    /// The window and its native handle. `None` when no separate window
    /// could be opened: the view is then shown as a card in the main window.
    pub(crate) window: Option<(AnyWindowHandle, isize)>,
}

/// One quick note being written: it starts with the first character typed
/// and ends when the quick-note window is put away.
#[derive(Default)]
pub(crate) struct QuickSession {
    /// The note this session created. `None` until something was typed:
    /// opening the window and closing it again leaves no empty note behind.
    note_id: Option<String>,
    /// Text last written to that note.
    saved: String,
    /// Bumped on every edit so only the last debounced save runs.
    epoch: u64,
    /// The last write hit a locked database; the background loop retries it.
    retry: bool,
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

/// Do two paths name the same existing file? Spelling differences (case,
/// `..`, links) are resolved; a path that does not exist only matches itself.
fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Open the database the config points at.
///
/// In the default folder a missing file is simply a first start, so it is
/// created. A folder the user picked always held a database when it was
/// chosen: if the file is gone now (the drive is not connected, the folder
/// was moved), starting an empty database there would look like every note
/// had vanished — that is reported instead.
fn open_configured_database(config: &AppConfig) -> store::Result<NoteStore> {
    let path = config.database_path();
    if config.uses_default_data_dir() {
        NoteStore::open(&path)
    } else {
        NoteStore::open_existing(&path)
    }
}

/// Opt-in diagnostics: set `STICKYNOTES_DEBUG=1` to append save/sync decisions
/// (note-id prefix, text lengths, flags — never note content) to
/// `%LOCALAPPDATA%\stickynotes\debug.log`.
fn debug_log(msg: &str) {
    use std::io::Write as _;
    if std::env::var_os("STICKYNOTES_DEBUG").is_none() {
        return;
    }
    let dir = config::config_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("debug.log");
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > 1_000_000 {
            let _ = std::fs::remove_file(&path);
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let ms = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let _ = writeln!(f, "{ms} {msg}");
    }
}

/// One markdown-toolbar hover hint, anchored in window coordinates.
#[derive(Clone, Copy)]
pub struct MdTip {
    /// Which button owns this hint, so leaving another button cannot clear it.
    pub id: &'static str,
    pub label: &'static str,
    /// The shortcut action behind the button; the hint shows whatever keys
    /// are bound to it at the moment.
    pub action: &'static str,
    /// Button's left edge and bottom edge in window coordinates.
    pub x: Pixels,
    pub y: Pixels,
    /// Whether the hint transitioned from an adjacent button while warm, so
    /// its x-position springs smoothly instead of snapping from a cold origin.
    pub glide: bool,
}

pub struct AppState {
    /// The app's own database: notes, tags and pins.
    pub store: NoteStore,
    /// Why the configured database could not be opened. While set, the window
    /// shows only a recovery screen and `store` is an empty in-memory
    /// stand-in that is never listed, edited or synced.
    pub startup_error: Option<String>,
    pub notes: Vec<NoteSummary>,
    pub selected_id: Option<String>,
    pub editor: Option<Entity<BlockEditorState>>,
    pub scroll: ScrollHandle,
    pub tag_filter: Option<i64>,
    pub search_query: String,
    pub tags_list: Vec<Tag>,
    pub status: SyncStatus,

    // In-memory caches to eliminate all SQLite queries during UI render
    pub note_tags_map: HashMap<String, Vec<Tag>>,
    pub tag_counts: HashMap<i64, usize>,
    pub pinned: HashSet<String>,

    // Typing debounce & stats cache
    pub save_epoch: u64,
    pub current_stats: (usize, usize),

    // Autosave bookkeeping
    /// Last editor content successfully written to the database (tells
    /// whether replacing the editor would discard unsaved edits).
    pub last_saved_md: Option<String>,
    /// A save failed because the database was momentarily locked: the note id
    /// and Markdown to retry, captured at failure time.
    pub pending_save: Option<(String, String)>,
    /// When the background loop next checks whether a backup is due.
    next_backup_check: Instant,

    // Settings: storage
    pub config: AppConfig,
    pub show_settings: bool,
    pub settings: Option<SettingsInputs>,
    /// Result of the last storage action (move, import, …), shown in settings.
    pub storage_note: Option<StorageNote>,
    /// A folder picked in settings that already holds a database. Switching
    /// to it replaces what is on screen, so it waits for a confirmation.
    pub pending_adopt: Option<PathBuf>,
    /// The system folder picker is open.
    pub picking_folder: bool,
    /// Bumped whenever `store` starts pointing at different notes, so a sync
    /// pass that was computed for the previous ones is discarded.
    store_generation: u64,

    // Settings: WebDAV sync
    pub webdav_test: WebDavTest,
    pub webdav_syncing: bool,
    pub webdav_message: Option<String>,
    /// Bumped on every edit so only the last debounced WebDAV push runs.
    webdav_epoch: u64,
    /// Remote file versions from the last pass. Lets a pull skip unchanged
    /// files (one PROPFIND, no downloads) and survives restarts.
    webdav_cache: webdav::SyncCache,
    /// Notes deleted locally whose remote `.md` must be removed on the next
    /// pass. Without it a delete would resurrect from the server.
    webdav_deletions: webdav::DeleteLedger,
    /// The remote collection has been created/confirmed during this run.
    webdav_dir_ready: bool,
    /// Consecutive failed background passes, for exponential backoff.
    webdav_failures: u32,
    /// No background pass starts before this instant.
    webdav_next_try: Instant,
    /// A sync was requested while one was running; run once more afterwards
    /// instead of dropping the request.
    webdav_pending: bool,
    /// The pass in flight is a background pull (see [`SyncPass`]).
    pub webdav_quiet: bool,
    /// A visible or forced pass was asked for while a quiet one ran: run it
    /// right after, without the usual upload delay.
    webdav_pending_force: bool,
    /// When the last pass started. Returning to the window pulls at most
    /// once per half minute, however much the user alt-tabs.
    webdav_last_pass: Option<Instant>,
    /// The last pass failed; the sidebar dot stays red until one succeeds.
    pub webdav_failed: bool,

    // UI state toggles
    pub show_sidebar_add_tag: bool,

    // Inputs
    pub new_tag_input: Option<Entity<InputState>>,
    pub search_input: Option<Entity<InputState>>,
    /// Title field shown above the body editor; the note's first paragraph.
    pub title_input: Option<Entity<InputState>>,

    /// Handle to the app window, so menu callbacks (which only get `&mut App`)
    /// can still run actions that need a `&mut Window`.
    pub window_handle: AnyWindowHandle,

    /// Hover hint currently shown under the markdown toolbar, if any.
    pub md_tip: Option<MdTip>,
    /// Last displayed hint kept across exit fade-out so `Presence` can render
    /// its label and coordinates until the exit transition settles.
    pub md_tip_last: Option<MdTip>,
    /// When the last toolbar hint closed, enabling warm instant glide across
    /// neighboring format buttons.
    pub md_tip_closed_at: Option<Instant>,
    /// Button the pointer rests on right now; the hint appears after a short
    /// delay only if it is still this button.
    pub md_tip_hover: Option<&'static str>,
    /// Latest painted bounds of each toolbar button, keyed by element id.
    /// Written from prepaint (no notify) and read when a hover starts.
    pub md_tip_bounds: Rc<RefCell<HashMap<&'static str, Bounds<Pixels>>>>,

    // Settings dialog
    pub settings_page: SettingsPage,
    /// `+1.0` when navigating down the settings rail, `-1.0` when navigating up.
    pub settings_page_dir: f32,
    /// Monotonically bumped only when the user switches tabs inside Settings,
    /// so opening Settings directly onto an existing tab never double-animates
    /// the inner page alongside the modal's own entrance.
    pub settings_page_seq: usize,
    /// Scroll position of the page shown, shared by the three pages.
    pub settings_scroll: ScrollHandle,
    /// Why the typed sync delay was refused. The stored delay stays as it
    /// was while this is set.
    pub delay_error: Option<SyncDelayError>,
    /// Bumped whenever the delay field changes, so that only the last value
    /// typed can turn into an error message.
    delay_epoch: u64,
    /// The drop-down list of the settings dialog that is open, if any.
    pub settings_menu: Option<SettingsMenu>,
    /// The "clear all notes" confirmation is open, since this instant.
    pub confirm_clear: Option<Instant>,
    /// How many notes the confirmation is about. Kept apart from the list,
    /// so the question still reads right while it fades out over an
    /// already empty list.
    pub confirm_clear_count: usize,
    /// Outcome of the last "clear all notes".
    pub danger_note: Option<StorageNote>,
    /// Holds the focus while a dialog is open, so key presses reach neither
    /// the note behind it nor a field of a page that is not shown.
    pub modal_focus: FocusHandle,
    /// The repository address was copied a moment ago (About page).
    pub about_copied: bool,
    about_copied_epoch: u64,

    // Keyboard shortcuts
    /// The shortcuts in effect: the defaults plus the changes in `config`.
    pub keymap: shortcuts::Keymap,
    /// The action whose shortcut is being recorded on the settings page.
    pub recording_shortcut: Option<&'static str>,
    /// Why the last recorded combination was refused, and for which action.
    pub shortcut_error: Option<(&'static str, String)>,
    /// Why a system-wide key is not in effect although one is set — usually
    /// another program holds the same keys. Keyed by action id.
    pub hotkey_problems: HashMap<&'static str, &'static str>,

    // Notification area and quick note
    /// `None` when there is no notification-area icon to come back from:
    /// closing the main window then always quits.
    tray: Option<win::Tray>,
    /// Native handle of the main window, for hiding and showing it.
    main_hwnd: Option<isize>,
    /// The main window was created hidden (a start at sign-in, straight into
    /// the notification area) and has not been on screen yet.
    awaiting_first_show: bool,
    /// Windows starts the app at sign-in. Mirrors the system's own entry.
    pub autostart: bool,
    /// Why that entry could not be changed.
    pub autostart_error: Option<String>,
    pub(crate) quick: Option<QuickHost>,
    /// The quick note is shown as a card in the main window — the fallback
    /// when it has no window of its own.
    pub quick_overlay_open: bool,
    quick_session: QuickSession,
}

impl AppState {
    /// `hidden`: the window was created without being shown.
    pub fn new(hidden: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut config = AppConfig::load();
        // A shortcut stored for an action that no longer exists (新建便签 had
        // one) is forgotten: it would count as a change nobody can see.
        config.shortcuts.retain(|action, _| shortcuts::action_def(action).is_some());
        let (store, startup_error) = match open_configured_database(&config) {
            Ok(store) => (store, None),
            Err(e) => {
                debug_log(&format!("open database failed: {e}"));
                let stand_in =
                    NoteStore::open_in_memory().expect("in-memory SQLite is always available");
                (stand_in, Some(e.to_string()))
            }
        };

        let new_tag_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("新标签名称…")
        });
        cx.subscribe_in(&new_tag_input, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.add_tag(window, cx);
            }
        })
        .detach();

        let search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("搜索便签内容或标题…")
        });
        cx.subscribe_in(&search_input, window, |this, input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change) {
                let text = input.read(cx).value().to_string();
                this.search_query = text;
                this.ensure_selection_after_filter(window, cx);
                cx.notify();
            }
        })
        .detach();

        let window_handle = window.window_handle();
        let keymap = shortcuts::Keymap::resolve(&config.shortcuts);
        let mut app = Self {
            store,
            startup_error,
            notes: Vec::new(),
            selected_id: None,
            editor: None,
            scroll: ScrollHandle::new(),
            tag_filter: None,
            search_query: String::new(),
            tags_list: Vec::new(),
            status: SyncStatus::Idle,
            note_tags_map: HashMap::new(),
            tag_counts: HashMap::new(),
            pinned: HashSet::new(),
            save_epoch: 0,
            current_stats: (0, 0),
            last_saved_md: None,
            pending_save: None,
            next_backup_check: Instant::now(),
            config,
            show_settings: false,
            settings: None,
            storage_note: None,
            pending_adopt: None,
            picking_folder: false,
            store_generation: 0,
            webdav_test: WebDavTest::Idle,
            webdav_syncing: false,
            webdav_message: None,
            webdav_epoch: 0,
            webdav_cache: webdav::SyncCache::load(),
            webdav_deletions: webdav::DeleteLedger::load(),
            webdav_dir_ready: false,
            webdav_failures: 0,
            webdav_next_try: Instant::now(),
            webdav_pending: false,
            webdav_quiet: false,
            webdav_pending_force: false,
            webdav_last_pass: None,
            webdav_failed: false,
            show_sidebar_add_tag: false,
            new_tag_input: Some(new_tag_input),
            search_input: Some(search_input),
            title_input: None,
            window_handle,
            md_tip: None,
            md_tip_last: None,
            md_tip_closed_at: None,
            md_tip_hover: None,
            md_tip_bounds: Rc::new(RefCell::new(HashMap::new())),
            settings_page: SettingsPage::General,
            settings_page_dir: 1.0,
            settings_page_seq: 0,
            settings_scroll: ScrollHandle::new(),
            delay_error: None,
            delay_epoch: 0,
            settings_menu: None,
            confirm_clear: None,
            confirm_clear_count: 0,
            danger_note: None,
            modal_focus: cx.focus_handle(),
            about_copied: false,
            about_copied_epoch: 0,
            keymap,
            recording_shortcut: None,
            shortcut_error: None,
            hotkey_problems: HashMap::new(),
            tray: None,
            main_hwnd: win::hwnd_of(window),
            awaiting_first_show: hidden,
            autostart: win::autostart_enabled(),
            autostart_error: None,
            quick: None,
            quick_overlay_open: false,
            quick_session: QuickSession::default(),
        };

        app.start_shell(window, cx);
        if app.startup_error.is_none() {
            app.start_session(window, cx);
        }
        app.start_background_loop(window, cx);
        app
    }

    /// Everything that follows opening a database: carry over the previous
    /// layout on first use, show the notes and start syncing.
    fn start_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.migrate_legacy_data();
        self.show_store_contents(window, cx);
        // Pull WebDAV changes right away; silent when the server is not
        // configured yet.
        if self.config.webdav_auto && self.config.webdav_configured() {
            self.start_webdav_sync(window, cx, SyncPass::Quiet);
        }
    }

    /// First start with the app's own database: move in the notes of Windows
    /// Sticky Notes plus the Markdown, tags and pins earlier versions kept
    /// beside them. Nothing is deleted or changed at the old places.
    fn migrate_legacy_data(&mut self) {
        // Once per machine: a database created later (for instance after
        // choosing the default folder again) must start empty rather than
        // bring back notes that were deleted here since the migration.
        if self.config.legacy_migrated {
            return;
        }
        match legacy::migrate_once(&self.store, &legacy::LegacySources::detect()) {
            Ok(report) => {
                if let Some(report) = report {
                    debug_log(&format!(
                        "migrated notes={} tags={} links={} pins={} windows_found={} tags_unreadable={}",
                        report.notes.added,
                        report.tags.tags,
                        report.tags.links,
                        report.tags.pins,
                        report.windows_notes_found,
                        report.tags_unreadable
                    ));
                    let moved = format!(
                        "已从 Windows 便签迁移 {} 条便签到独立数据库",
                        report.notes.added
                    );
                    if report.tags_unreadable {
                        self.storage_note = Some(StorageNote::Error(format!(
                            "{moved}；旧的标签文件无法读取，标签和置顶没有迁移"
                        )));
                    } else if report.notes.added > 0 {
                        self.storage_note = Some(StorageNote::Info(moved));
                    }
                }
                self.config.legacy_migrated = true;
                if let Err(e) = self.config.save() {
                    debug_log(&format!("saving the migration flag failed: {e}"));
                }
            }
            Err(e) => {
                debug_log(&format!("migration failed: {e}"));
                let msg = format!("迁移 Windows 便签失败，下次启动会重试：{e}");
                self.status = SyncStatus::Error(msg.clone());
                self.storage_note = Some(StorageNote::Error(msg));
            }
        }
    }

    /// Load the list, tags and pins from `store` and open the first note.
    fn show_store_contents(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_selection();
        self.pending_save = None;
        match self.store.list_notes() {
            Ok(notes) => self.notes = notes,
            Err(e) => {
                self.notes.clear();
                self.status = SyncStatus::Error(format!("读取便签失败：{e}"));
            }
        }
        self.reload_tags_cache();
        self.select_note_by_index(0, window, cx);
        cx.notify();
    }

    /// Refresh in-memory tag cache with a single batch query.
    pub fn reload_tags_cache(&mut self) {
        let tags = self.store.list_tags().unwrap_or_default();
        let tag_map: HashMap<i64, Tag> = tags.iter().map(|t| (t.id, t.clone())).collect();

        let pairs = self.store.note_tags().unwrap_or_default();
        let mut note_tags: HashMap<String, Vec<Tag>> = HashMap::new();
        let mut counts: HashMap<i64, usize> = HashMap::new();

        for (note_id, tid) in pairs {
            if let Some(tag) = tag_map.get(&tid) {
                note_tags.entry(note_id).or_default().push(tag.clone());
                *counts.entry(tid).or_default() += 1;
            }
        }

        self.tags_list = tags;
        self.note_tags_map = note_tags;
        self.tag_counts = counts;
        self.pinned = self
            .store
            .pinned_ids()
            .unwrap_or_default()
            .into_iter()
            .collect();
    }

    pub fn is_pinned(&self, id: &str) -> bool {
        self.pinned.contains(id)
    }

    pub fn tags_for_note(&self, id: &str) -> &[Tag] {
        self.note_tags_map.get(id).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn current_note_stats(&self, _cx: &App) -> (usize, usize) {
        self.current_stats
    }

    pub fn current_note(&self) -> Option<&NoteSummary> {
        let id = self.selected_id.as_deref()?;
        self.notes.iter().find(|n| n.id == id)
    }

    pub fn current_theme(&self) -> &'static ThemeSpec {
        self.current_note()
            .map(|n| get_theme(&n.theme))
            .unwrap_or(&THEMES[0])
    }

    pub fn filtered_notes(&self) -> Vec<NoteSummary> {
        let mut notes: Vec<NoteSummary> = match self.tag_filter {
            None => self.notes.clone(),
            Some(tag_id) => self
                .notes
                .iter()
                .filter(|n| {
                    self.note_tags_map
                        .get(&n.id)
                        .map(|tags| tags.iter().any(|t| t.id == tag_id))
                        .unwrap_or(false)
                })
                .cloned()
                .collect(),
        };

        let q = self.search_query.trim().to_lowercase();
        if !q.is_empty() {
            notes.retain(|n| {
                n.title.to_lowercase().contains(&q) || n.snippet.to_lowercase().contains(&q)
            });
        }

        // Pinned notes float to the top; the stable sort keeps the
        // newest-first order within each group.
        notes.sort_by_key(|n| !self.pinned.contains(&n.id));
        notes
    }

    /// Close the open note without saving it. Callers that may hold unsaved
    /// edits flush them first.
    fn clear_selection(&mut self) {
        self.selected_id = None;
        self.editor = None;
        self.title_input = None;
        self.last_saved_md = None;
        self.current_stats = (0, 0);
    }

    pub fn ensure_selection_after_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let filtered = self.filtered_notes();
        if filtered.is_empty() {
            self.flush_pending_save(cx);
            self.clear_selection();
        } else if !filtered.iter().any(|n| Some(&n.id) == self.selected_id.as_ref()) {
            self.select_note_by_index(0, window, cx);
        }
    }

    pub fn select_note(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_id.as_deref() == Some(id) && self.editor.is_some() {
            return;
        }
        // The quick note hands its note over before the main panel opens it.
        if self.quick_note_id() == Some(id) && !self.quick_release_note(window, cx) {
            return;
        }
        // The debounced autosave may still be pending for the note we are
        // leaving. Flush it before its editor is replaced — otherwise the
        // timer fires against the *new* editor and the edit is silently lost.
        self.flush_pending_save(cx);

        let Some(md) = self.store.note_markdown(id).ok().flatten() else {
            return;
        };
        debug_log(&format!("select id={} len={}", short_id(id), md.len()));
        // The first paragraph is the note title; the rest is the body.
        let (title, body) = Self::split_title_body(&md);
        let scroll = self.scroll.clone();

        let editor = cx.new(|cx| {
            let mut state = BlockEditorState::new(window, cx).with_source(&body);
            state.set_scroll_handle(scroll);
            state.set_editor_style(
                markdown_editor::BlockEditorStyle::default()
                    .with_foreground(rgb(0x232B36).into())
                    .with_muted_foreground(rgb(0x6E7785).into())
                    .with_caret(rgb(0x3B6EA8).into()),
                cx,
            );
            state
        });
        cx.subscribe_in(&editor, window, |this, _, event: &BlockEditorEvent, window, cx| {
            match event {
                BlockEditorEvent::Changed { .. } => this.schedule_save(window, cx),
                // Keep the formatting bar's active states in sync with the caret.
                BlockEditorEvent::SelectionChanged => cx.notify(),
                _ => {}
            }
        })
        .detach();

        // Title field: a borderless single-line input above the body editor.
        let title_input = cx.new(|cx| InputState::new(window, cx).placeholder("标题"));
        title_input.update(cx, |s, cx| s.set_value(title.clone(), window, cx));
        cx.subscribe_in(&title_input, window, |this, _, event: &InputEvent, window, cx| {
            match event {
                InputEvent::Change => this.schedule_save(window, cx),
                // Enter in the title jumps into the body.
                InputEvent::PressEnter { .. } => {
                    if let Some(editor) = &this.editor {
                        let handle = editor.read(cx).focus_handle(cx);
                        window.focus(&handle, cx);
                    }
                }
                _ => {}
            }
        })
        .detach();

        self.editor = Some(editor);
        self.title_input = Some(title_input);
        self.selected_id = Some(id.to_string());
        // Store the composed document (title + body): it is what the editor
        // would write back, so comparing against it later tells whether the
        // user changed anything.
        self.last_saved_md = Some(self.compose_md(cx));
        self.update_stats_from_editor(cx);
        cx.notify();
    }

    pub fn select_note_by_index(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        // Picking a note automatically never takes the one the quick note is
        // still writing: that would end its session behind the user's back.
        let quick = self.quick_note_id().map(str::to_string);
        let notes = self.filtered_notes();
        let pick = notes
            .iter()
            .filter(|note| Some(&note.id) != quick.as_ref())
            .nth(idx);
        if let Some(note) = pick {
            let id = note.id.clone();
            self.select_note(&id, window, cx);
        }
    }

    /// Split canonical markdown into (plain-text title, body markdown). The
    /// first paragraph is the note title, matching Windows Sticky Notes.
    fn split_title_body(md: &str) -> (String, String) {
        match md.split_once('\n') {
            Some((first, rest)) => (store::strip_markdown_syntax(first), rest.to_string()),
            None => (store::strip_markdown_syntax(md), String::new()),
        }
    }

    /// Compose the title field and body editor back into one markdown
    /// document — the form written to the database.
    pub fn compose_md(&self, cx: &App) -> String {
        let body = self
            .editor
            .as_ref()
            .map(|e| e.read(cx).source())
            .unwrap_or_default();
        let title = self
            .title_input
            .as_ref()
            .map(|i| i.read(cx).value().to_string())
            .unwrap_or_default();
        let title = title.trim();
        if title.is_empty() {
            body
        } else if body.trim().is_empty() {
            title.to_string()
        } else {
            format!("{title}\n{body}")
        }
    }

    pub fn update_stats_from_editor(&mut self, cx: &App) {
        if let Some(editor) = &self.editor {
            let body = editor.read(cx).source();
            let title = self
                .title_input
                .as_ref()
                .map(|i| i.read(cx).value().to_string())
                .unwrap_or_default();
            let src = if title.trim().is_empty() {
                body
            } else {
                format!("{title}\n{body}")
            };
            let chars = src.chars().filter(|c| !c.is_whitespace()).count();
            let lines = if src.is_empty() { 0 } else { src.lines().count() };
            self.current_stats = (chars, lines);
        } else {
            self.current_stats = (0, 0);
        }
    }

    /// Persist the open note's last edit *before* its editor is replaced.
    ///
    /// Saves are debounced by 400ms. A note or tag switch inside that window
    /// would otherwise swap the editor first, and the timer would then write
    /// the new note's text (or nothing), silently losing the edit — which then
    /// looks like the old note "reverting" on the next visit.
    fn flush_pending_save(&mut self, cx: &mut Context<Self>) {
        if self.editor.is_none() {
            return;
        }
        let dirty = self.last_saved_md.as_deref() != Some(self.compose_md(cx).as_str());
        debug_log(&format!("flush dirty={dirty}"));
        if dirty {
            let _ = self.save_current(cx);
        }
        // Invalidate any debounce timer captured against the old editor.
        self.save_epoch = self.save_epoch.wrapping_add(1);
    }

    /// Debounced autosave: 400ms after the last edit, cancelling previous pending saves via epoch.
    fn schedule_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_epoch = self.save_epoch.wrapping_add(1);
        let epoch = self.save_epoch;
        debug_log(&format!("schedule epoch={epoch}"));
        let this = cx.weak_entity();

        // Note: Heavy string/AST serialization is decoupled from keystrokes
        // and performed during the debounced save.

        window.spawn(cx, async move |cx: &mut AsyncWindowContext| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(400))
                .await;
            this.update_in(cx, |this, window, cx| {
                if this.save_epoch == epoch {
                    // Only a save that actually changed the stored text is
                    // worth an upload: a no-op write would bump `updated_at`,
                    // and that alone starts a WebDAV pass a few seconds later.
                    if this.save_current(cx) {
                        this.schedule_webdav_sync(window, cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Write the open note to the database. Returns whether anything was
    /// written: an unchanged text is not touched, so its `updated_at` (the
    /// timestamp sync compares) does not move and no upload follows.
    pub fn save_current(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(id) = self.selected_id.clone() else {
            return false;
        };
        if self.editor.is_none() {
            return false;
        }
        let md = self.compose_md(cx);
        if self.last_saved_md.as_deref() == Some(md.as_str()) {
            debug_log(&format!("save skipped (unchanged) id={}", short_id(&id)));
            return false;
        }
        debug_log(&format!("save id={} len={}", short_id(&id), md.len()));
        match self.store.save_markdown(&id, &md) {
            Ok(updated_at) => {
                self.status = SyncStatus::Saved("已自动保存".into());
                self.last_saved_md = Some(md.clone());
                self.pending_save = None;
                if let Some(note) = self.notes.iter_mut().find(|n| n.id == id) {
                    note.updated_at = updated_at;
                }
                debug_log("save ok");
            }
            Err(e) => {
                // Another program may hold the database lock for a moment;
                // the background loop retries the exact text that failed
                // instead of losing the edit.
                let retryable = e.is_busy();
                self.pending_save = retryable.then(|| (id.clone(), md.clone()));
                self.status = if retryable {
                    SyncStatus::Busy("数据库被占用，正在自动重试保存…".into())
                } else {
                    SyncStatus::Error(format!("保存失败：{e}"))
                };
                debug_log(&format!("save err retryable={retryable} msg={e}"));
                return false;
            }
        }

        // Direct in-memory summary update (no full disk reload)
        let (title, snippet) = store::title_and_snippet(&md);
        if let Some(note) = self.notes.iter_mut().find(|n| n.id == id) {
            note.title = title;
            note.snippet = snippet;
        }
        self.update_stats_from_editor(cx);
        cx.notify();
        true
    }

    pub fn new_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.store.create_note("", "White") {
            Ok(new_id) => {
                if let Ok(notes) = self.store.list_notes() {
                    self.notes = notes;
                }
                self.select_note(&new_id, window, cx);
                self.status = SyncStatus::Saved("已新建便签".into());
            }
            Err(e) => {
                self.status = SyncStatus::Error(format!("新建便签失败：{e}"));
            }
        }
        cx.notify();
    }

    /// Kept for the editor overflow menu, which is temporarily hidden.
    #[allow(dead_code)]
    pub fn delete_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected_id.clone() else { return };
        self.delete_note_by_id(&id, window, cx);
    }

    /// Delete an arbitrary note — used by the card context menu, where the
    /// target is not necessarily the selected note.
    pub fn delete_note_by_id(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        // Deleting the open note drops its pending debounce: there is nothing
        // left to save it into. Deleting another note flushes the open one
        // (in `reload`) so no edit is lost.
        if self.selected_id.as_deref() == Some(id) {
            self.save_epoch = self.save_epoch.wrapping_add(1);
            self.pending_save = None;
            self.clear_selection();
        }
        // The same goes for the quick note, should it be writing this note:
        // left open, its next key press would bring the note back.
        if self.quick_note_id() == Some(id) {
            self.quick_close(cx);
        }
        match self.store.soft_delete(id) {
            Ok(()) => {
                // Remember the deletion so the remote `.md` goes away too;
                // otherwise the next WebDAV pass would happily re-import the
                // file as a "remote-only" note and resurrect what the user
                // just deleted.
                if self.config.webdav_configured() {
                    self.webdav_deletions
                        .record(&self.config.webdav, id, stickynotes_core::ticks::now_ticks());
                    // Persist the intent now: if the app closes before the
                    // debounced pass runs, the remote `.md` would otherwise
                    // come back as a remote-only note on the next launch.
                    let _ = self.webdav_deletions.save();
                }
                self.status = SyncStatus::Saved("便签已删除".into());
                self.reload(window, cx);
                self.schedule_webdav_sync(window, cx);
            }
            Err(e) => {
                self.status = SyncStatus::Error(format!("删除失败：{e}"));
                cx.notify();
            }
        }
    }

    pub fn set_theme(&mut self, theme: &str, cx: &mut Context<Self>) {
        let Some(id) = self.selected_id.clone() else { return };
        match self.store.set_theme(&id, theme) {
            Ok(changed) => {
                if changed {
                    self.status = SyncStatus::Saved(format!("颜色已设为 {theme}"));
                    if let Some(note) = self.notes.iter_mut().find(|n| n.id == id) {
                        note.theme = theme.to_string();
                    }
                    self.metadata_changed(cx);
                }
            }
            Err(e) => self.status = SyncStatus::Error(format!("{e}")),
        }
        cx.notify();
    }

    /// A note's colour, tags or pin changed on its own. Unlike a text edit it
    /// has its own upload: the debounced save only runs when the text did,
    /// so without this the change would wait for the next text edit — and be
    /// overwritten by an older copy from another device in the meantime.
    ///
    /// A pass is only scheduled after the usual upload delay. The window is
    /// reached through its handle because context-menu callbacks only get
    /// the app, not a window.
    fn metadata_changed(&mut self, cx: &mut Context<Self>) {
        let handle = self.window_handle;
        let this = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |this, cx| this.schedule_webdav_sync(window, cx));
            });
        });
    }

    pub fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_query.clear();
        if let Some(input) = &self.search_input {
            input.update(cx, |s, cx| s.set_value("", window, cx));
        }
        self.ensure_selection_after_filter(window, cx);
        cx.notify();
    }

    pub fn clear_tag_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tag_filter = None;
        self.ensure_selection_after_filter(window, cx);
        cx.notify();
    }

    /// Re-read the list, tags and pins from the database, keeping the open
    /// note open when it is still there.
    pub fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_pending_save(cx);
        match self.store.list_notes() {
            Ok(notes) => {
                self.notes = notes;
                self.status = SyncStatus::Saved("已刷新".into());
            }
            Err(e) => self.status = SyncStatus::Error(format!("读取便签失败：{e}")),
        }
        self.reload_tags_cache();
        let keep = self.selected_id.take();
        self.clear_selection();
        let visible = self.filtered_notes();
        match keep.filter(|id| visible.iter().any(|n| &n.id == id)) {
            Some(id) => self.select_note(&id, window, cx),
            None => self.select_note_by_index(0, window, cx),
        }
        cx.notify();
    }

    // --- Tag management ---

    pub fn toggle_sidebar_add_tag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_sidebar_add_tag = !self.show_sidebar_add_tag;
        if self.show_sidebar_add_tag {
            if let Some(input) = &self.new_tag_input {
                input.update(cx, |s, cx| {
                    s.set_value("", window, cx);
                });
                let handle = input.read(cx).focus_handle(cx);
                window.focus(&handle, cx);
            }
        }
        cx.notify();
    }

    /// The "new tag" shortcut: show the sidebar's field and put the caret in
    /// it. Unlike the button it never hides a field that is already there.
    pub fn begin_new_tag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.show_sidebar_add_tag {
            if let Some(input) = &self.new_tag_input {
                let handle = input.read(cx).focus_handle(cx);
                window.focus(&handle, cx);
            }
        } else {
            self.toggle_sidebar_add_tag(window, cx);
        }
    }

    pub fn add_tag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.new_tag_input.clone() else { return };
        let name = input.read(cx).value().to_string();
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        match self.store.create_tag(&name) {
            Ok(tid) => {
                if let Some(id) = &self.selected_id {
                    if self.store.tag_note(id, tid).unwrap_or(false) {
                        self.metadata_changed(cx);
                    }
                }
                input.update(cx, |s, cx| s.set_value("", window, cx));
                self.status = SyncStatus::Saved(format!("标签 #{name} 已添加"));
                self.show_sidebar_add_tag = false;
            }
            Err(e) => {
                self.status = SyncStatus::Error(format!("{e}"));
            }
        }
        self.reload_tags_cache();
        cx.notify();
    }

    pub fn delete_tag(&mut self, id: i64, cx: &mut Context<Self>) {
        match self.store.delete_tag(id) {
            Ok(affected) => {
                if self.tag_filter == Some(id) {
                    self.tag_filter = None;
                }
                self.status = SyncStatus::Saved("标签已删除".into());
                // Every note that had the tag changed with it.
                if affected > 0 {
                    self.metadata_changed(cx);
                }
            }
            Err(e) => self.status = SyncStatus::Error(format!("{e}")),
        }
        self.reload_tags_cache();
        cx.notify();
    }

    /// Toggle a tag on an arbitrary note (the context-menu target, which is
    /// not necessarily the selected note).
    pub fn toggle_tag_for(&mut self, note_id: &str, tag_id: i64, cx: &mut Context<Self>) {
        let has = self
            .note_tags_map
            .get(note_id)
            .map(|tags| tags.iter().any(|t| t.id == tag_id))
            .unwrap_or(false);

        let result = if has {
            self.store.untag_note(note_id, tag_id)
        } else {
            self.store.tag_note(note_id, tag_id)
        };
        match result {
            Err(e) => self.status = SyncStatus::Error(format!("{e}")),
            Ok(changed) => {
                self.reload_tags_cache();
                if changed {
                    self.metadata_changed(cx);
                }
            }
        }
        cx.notify();
    }

    // --- Pin ---

    pub fn toggle_pin(&mut self, note_id: &str, cx: &mut Context<Self>) {
        let pin = !self.pinned.contains(note_id);
        match self.store.set_pinned(note_id, pin) {
            Ok(changed) => {
                if pin {
                    self.pinned.insert(note_id.to_string());
                } else {
                    self.pinned.remove(note_id);
                }
                self.status =
                    SyncStatus::Saved(if pin { "已置顶" } else { "已取消置顶" }.into());
                if changed {
                    self.metadata_changed(cx);
                }
            }
            Err(e) => self.status = SyncStatus::Error(format!("{e}")),
        }
        cx.notify();
    }

    // --- Background loop ---

    /// Every 2s: retry a save that hit a locked database, take a backup when
    /// one is due and start the periodic WebDAV pull.
    fn start_background_loop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(2000))
                        .await;
                    if this
                        .update_in(cx, |this, window, cx| this.background_tick(window, cx))
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
    }

    fn background_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.startup_error.is_some() {
            return;
        }
        self.retry_pending_save(cx);
        self.retry_quick_save(window, cx);

        if Instant::now() >= self.next_backup_check {
            self.next_backup_check = Instant::now() + Duration::from_secs(30 * 60);
            match backup::backup_if_due(&self.store, &backup::default_dir()) {
                Ok(written) => debug_log(&format!("backup written={}", written.is_some())),
                // Backups are a safety net; the notes themselves are saved.
                Err(e) => debug_log(&format!("backup failed: {e}")),
            }
        }

        // Background WebDAV pull. With the version cache a quiet pass is a
        // single PROPFIND every few minutes; when the server is failing, the
        // interval backs off so it is not hammered. Local edits still upload
        // via the debounce (`sync_delay`) instead of waiting for this.
        if self.config.webdav_auto
            && self.config.webdav_configured()
            && !self.webdav_syncing
            && Instant::now() >= self.webdav_next_try
        {
            self.start_webdav_sync(window, cx, SyncPass::Quiet);
        }
    }

    /// Back in the main window: check the server right away, at most once
    /// every 30 seconds, so alt-tabbing does not become a sync storm.
    pub fn pull_on_return(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.startup_error.is_some()
            || !self.config.webdav_auto
            || !self.config.webdav_configured()
            || self.webdav_syncing
        {
            return;
        }
        // While the server is failing, only the background backoff decides
        // when to try again (the red dot says why): returning to the window
        // must not add attempts on top of it.
        if self.webdav_failed && Instant::now() < self.webdav_next_try {
            return;
        }
        if self
            .webdav_last_pass
            .is_some_and(|at| at.elapsed() < Duration::from_secs(30))
        {
            return;
        }
        self.start_webdav_sync(window, cx, SyncPass::Quiet);
    }

    /// Retry the exact text that failed, even if the selection has moved on
    /// since — never the current editor's content, which belongs to another note.
    fn retry_pending_save(&mut self, cx: &mut Context<Self>) {
        let Some((id, md)) = self.pending_save.clone() else {
            return;
        };
        match self.store.save_markdown(&id, &md) {
            Ok(updated_at) => {
                self.pending_save = None;
                if let Some(note) = self.notes.iter_mut().find(|n| n.id == id) {
                    note.updated_at = updated_at;
                }
                if self.selected_id.as_deref() == Some(id.as_str()) {
                    self.last_saved_md = Some(md);
                }
                self.status = SyncStatus::Saved("已自动保存".into());
                debug_log("retry save ok");
                cx.notify();
            }
            Err(e) if e.is_busy() => debug_log("retry save busy"),
            Err(e) => {
                // Not a lock any more (the note is gone, the disk is full, …):
                // retrying the same write cannot succeed.
                self.pending_save = None;
                self.status = SyncStatus::Error(format!("保存失败：{e}"));
                debug_log(&format!("retry save err msg={e}"));
                cx.notify();
            }
        }
    }

    // --- Settings dialog ---

    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_settings_inputs(window, cx);
        self.show_settings = true;
        self.webdav_test = WebDavTest::Idle;
        self.pending_adopt = None;
        self.settings_menu = None;
        self.set_recording(None);
        self.shortcut_error = None;
        self.confirm_clear = None;
        self.danger_note = None;
        self.about_copied = false;
        // It may have been switched off in Task Manager since the last look.
        self.autostart = win::autostart_enabled();
        self.autostart_error = None;
        // The delay field shows the delay in effect, not a refused leftover
        // from the last visit.
        self.delay_error = None;
        self.delay_epoch = self.delay_epoch.wrapping_add(1);
        self.settings_scroll.set_offset(point(px(0.), px(0.)));
        if let Some(s) = &self.settings {
            let shown = config::format_sync_delay_secs(self.config.sync_delay_ms);
            s.delay.update(cx, |s, cx| s.set_value(shown, window, cx));
        }
        // The dialog itself takes the focus — not one of its fields, where a
        // stray key press would edit a setting, and not the note behind it.
        window.focus(&self.modal_focus, cx);
        cx.notify();
    }

    /// Switch the settings dialog to another page of its left column.
    pub fn set_settings_page(
        &mut self,
        page: SettingsPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_page != page {
            self.settings_page_dir = if page.index() >= self.settings_page.index() {
                1.0
            } else {
                -1.0
            };
            self.settings_page_seq = self.settings_page_seq.wrapping_add(1);
            // Every page starts at its top.
            self.settings_scroll.set_offset(point(px(0.), px(0.)));
        }
        self.settings_page = page;
        self.settings_menu = None;
        self.set_recording(None);
        self.shortcut_error = None;
        // A field of the page that was left must not keep the caret.
        window.focus(&self.modal_focus, cx);
        cx.notify();
    }

    fn ensure_settings_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.is_some() {
            return;
        }
        let shown_delay = config::format_sync_delay_secs(self.config.sync_delay_ms);
        let delay = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("2.5");
            s.set_value(shown_delay, window, cx);
            s
        });
        cx.subscribe_in(&delay, window, |this, input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change) {
                let text = input.read(cx).value().to_string();
                this.on_sync_delay_typed(&text, window, cx);
            }
        })
        .detach();
        let w = self.config.webdav.clone();
        let url = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("https://example.com/dav/");
            s.set_value(w.url.clone(), window, cx);
            s
        });
        let user = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("请输入 WebDAV 用户名");
            s.set_value(w.username.clone(), window, cx);
            s
        });
        let pass = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("请输入密码或应用专用密码");
            s.set_value(w.password.clone(), window, cx);
            s.set_masked(true, window, cx);
            s
        });
        let dir = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("/StickyNotes/");
            let dir = if w.dir.trim().is_empty() { "/StickyNotes/".to_string() } else { w.dir.clone() };
            s.set_value(dir, window, cx);
            s
        });
        self.settings = Some(SettingsInputs { url, user, pass, dir, delay });
    }

    /// The sync-delay field changed. A valid delay takes effect and is saved
    /// at once; anything else leaves the previous delay in place.
    fn on_sync_delay_typed(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.delay_epoch = self.delay_epoch.wrapping_add(1);
        match config::parse_sync_delay_secs(text) {
            Ok(ms) => {
                self.delay_error = None;
                if self.config.sync_delay_ms != ms {
                    self.config.sync_delay_ms = ms;
                    if let Err(e) = self.config.save() {
                        self.status = SyncStatus::Error(format!("保存设置失败：{e}"));
                    }
                }
                cx.notify();
            }
            Err(error) => {
                // A number on its way ("0." before "0.5") is not worth a red
                // line: say what is wrong only once the typing pauses.
                let epoch = self.delay_epoch;
                let this = cx.weak_entity();
                window
                    .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                        cx.background_executor()
                            .timer(Duration::from_millis(600))
                            .await;
                        let _ = this.update(cx, |this, cx| {
                            if this.delay_epoch == epoch {
                                this.delay_error = Some(error);
                                cx.notify();
                            }
                        });
                    })
                    .detach();
            }
        }
    }

    /// Copy the form back into the config and persist it. Returns whether the
    /// WebDAV target (server, credentials, collection) actually changed — the
    /// only case that is worth a sync of its own.
    fn persist_settings(&mut self, cx: &App) -> bool {
        let before = self.config.webdav.clone();
        if let Some(s) = &self.settings {
            self.config.webdav.url = s.url.read(cx).value().trim().to_string();
            self.config.webdav.username = s.user.read(cx).value().trim().to_string();
            self.config.webdav.password = s.pass.read(cx).value().to_string();
            let dir = s.dir.read(cx).value().trim().to_string();
            self.config.webdav.dir = if dir.is_empty() {
                "/StickyNotes/".into()
            } else {
                dir
            };
        }
        let target_changed = before.url != self.config.webdav.url
            || before.username != self.config.webdav.username
            || before.password != self.config.webdav.password
            || before.dir != self.config.webdav.dir;
        if target_changed {
            // The version cache and the "directory exists" assumption belong
            // to the old server/credentials.
            self.webdav_cache = webdav::SyncCache::default();
            self.webdav_deletions = webdav::DeleteLedger::default();
            self.webdav_dir_ready = false;
            self.webdav_failures = 0;
            self.webdav_failed = false;
            self.webdav_next_try = Instant::now();
        }
        if let Err(e) = self.config.save() {
            self.status = SyncStatus::Error(format!("保存设置失败：{e}"));
        }
        target_changed
    }

    pub fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.show_settings {
            return;
        }
        let target_changed = self.persist_settings(cx);
        self.show_settings = false;
        self.pending_adopt = None;
        self.settings_menu = None;
        self.set_recording(None);
        self.shortcut_error = None;
        self.confirm_clear = None;
        // Return focus from the unmounting modal fields back to the open note.
        if let Some(editor) = &self.editor {
            let handle = editor.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        } else {
            window.blur(cx);
        }
        // Only a changed server/collection is worth a pass: merely opening and
        // closing the dialog must not start a sync of its own.
        if target_changed {
            self.schedule_webdav_sync(window, cx);
        }
        cx.notify();
    }

    pub fn set_webdav_auto(&mut self, on: bool, cx: &mut Context<Self>) {
        self.config.webdav_auto = on;
        let _ = self.config.save();
        if on {
            // Try soon instead of waiting out a backoff from while it was off.
            self.webdav_failures = 0;
            self.webdav_failed = false;
            self.webdav_next_try = Instant::now();
        }
        cx.notify();
    }

    // --- Settings: clear all notes ---

    /// The red "清除全部便签" button: ask first.
    pub fn request_clear_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.notes.is_empty() {
            return;
        }
        self.settings_menu = None;
        self.set_recording(None);
        self.confirm_clear = Some(Instant::now());
        self.confirm_clear_count = self.notes.len();
        window.focus(&self.modal_focus, cx);
        cx.notify();
    }

    pub fn cancel_clear_all(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.confirm_clear.take().is_some() {
            cx.notify();
        }
    }

    /// The red button of the confirmation.
    pub fn confirm_clear_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(asked) = self.confirm_clear else {
            return;
        };
        // The second click of a double click on "清除全部便签" must not
        // count as the answer to a question that has only just appeared.
        if asked.elapsed() < Duration::from_millis(450) {
            return;
        }
        self.confirm_clear = None;
        self.clear_all_notes(window, cx);
    }

    /// Delete every note. The tags stay; a snapshot of the database is taken
    /// first, and with WebDAV configured the notes go from the server (and so
    /// from the other devices) too.
    fn clear_all_notes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
        // However this ends, the outcome is written under the red button at
        // the very end of the page: keep that in view.
        self.settings_scroll.scroll_to_bottom();
        // Whatever is still being typed goes into the database first: the
        // snapshot below is the way back and has to hold every note.
        self.flush_pending_save(cx);
        self.retry_pending_save(cx);
        if !self.quick_flush(window, cx) || self.pending_save.is_some() {
            self.danger_note = Some(StorageNote::Error(
                "当前便签还没有保存成功（数据库被占用），请稍后再试".into(),
            ));
            return;
        }
        let snapshot = match backup::snapshot(&self.store, &backup::default_dir()) {
            Ok(path) => path,
            Err(e) => {
                self.danger_note = Some(StorageNote::Error(format!(
                    "备份失败，没有清除任何便签：{e}"
                )));
                return;
            }
        };
        let ids = match self.store.soft_delete_all() {
            Ok(ids) => ids,
            Err(e) => {
                self.danger_note = Some(StorageNote::Error(format!(
                    "清除失败，便签没有变化：{e}"
                )));
                return;
            }
        };
        // As with a single delete: without the ledger the next sync pass
        // would find the files on the server and bring every note back.
        if self.config.webdav_configured() {
            let now = ticks::now_ticks();
            for id in &ids {
                self.webdav_deletions.record(&self.config.webdav, id, now);
            }
            let _ = self.webdav_deletions.save();
        }

        // No note is open any more, here or in the quick note; a save still
        // waiting for its pause must not write into a deleted note.
        self.save_epoch = self.save_epoch.wrapping_add(1);
        self.pending_save = None;
        self.clear_selection();
        self.quick_close(cx);
        self.reload(window, cx);

        let file = snapshot
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.danger_note = Some(StorageNote::Info(format!(
            "已清除 {} 条便签。清除前的备份：{file}",
            ids.len()
        )));
        self.status = SyncStatus::Saved(format!("已清除 {} 条便签", ids.len()));
        debug_log(&format!("cleared notes={}", ids.len()));
        self.schedule_webdav_sync(window, cx);
    }

    // --- Settings: about ---

    pub fn open_repository(&mut self, cx: &mut Context<Self>) {
        cx.open_url(REPOSITORY_URL);
    }

    pub fn copy_repository_url(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(REPOSITORY_URL.to_string()));
        self.about_copied = true;
        self.about_copied_epoch = self.about_copied_epoch.wrapping_add(1);
        let epoch = self.about_copied_epoch;
        let this = cx.weak_entity();
        // "已复制" turns back into "复制" after a moment.
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                cx.background_executor()
                    .timer(Duration::from_millis(1600))
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if this.about_copied_epoch == epoch {
                        this.about_copied = false;
                        cx.notify();
                    }
                });
            })
            .detach();
        cx.notify();
    }

    // --- Settings: where the database lives ---

    /// The database file in use, for display.
    pub fn database_path(&self) -> &Path {
        self.store.path()
    }

    /// Show the database file in Explorer.
    pub fn reveal_database(&mut self, cx: &mut Context<Self>) {
        cx.reveal_path(self.store.path());
    }

    /// Let the user pick the folder the database should live in.
    pub fn choose_data_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.picking_folder {
            return;
        }
        self.picking_folder = true;
        self.pending_adopt = None;
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择此文件夹".into()),
        });
        let this = cx.weak_entity();
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                let picked = picked.await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.picking_folder = false;
                    match picked {
                        Ok(Ok(Some(paths))) => {
                            if let Some(dir) = paths.into_iter().next() {
                                this.request_data_folder(dir, window, cx);
                            }
                        }
                        // Cancelled.
                        Ok(Ok(None)) | Err(_) => {}
                        Ok(Err(e)) => {
                            this.storage_note =
                                Some(StorageNote::Error(format!("无法打开文件夹选择框：{e}")));
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        cx.notify();
    }

    /// Move the database back to the default app-data folder.
    pub fn restore_default_data_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.request_data_folder(config::default_data_dir(), window, cx);
    }

    /// `dir` was chosen as the data folder: move the database there, or — if
    /// one already exists there — ask before switching to it.
    fn request_data_folder(&mut self, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let target = config::database_path_in(&dir);
        if same_file(&target, self.store.path()) {
            self.pending_adopt = None;
            self.storage_note = Some(StorageNote::Info("数据库已经在这个位置".into()));
        } else if target.exists() {
            // Never overwrite a database: offer to use it instead.
            self.storage_note = None;
            self.pending_adopt = Some(dir);
        } else {
            self.pending_adopt = None;
            self.move_database(&dir, window, cx);
        }
        cx.notify();
    }

    /// Unsaved text of the open note must be in the database before the
    /// database is copied or left behind.
    fn saved_before_switch(&mut self, cx: &mut Context<Self>) -> bool {
        self.flush_pending_save(cx);
        self.retry_pending_save(cx);
        // A quick note being written is unsaved text just the same.
        let quick_saved = !matches!(self.quick_write(cx), quick::Written::Failed);
        if self.pending_save.is_some() || !quick_saved {
            self.storage_note = Some(StorageNote::Error(
                "当前便签还没有保存成功（数据库被占用），请稍后再试".into(),
            ));
            return false;
        }
        true
    }

    /// Point the config at `dir`, undoing the change when it cannot be saved:
    /// otherwise the next start would open the database in the old place.
    fn remember_data_folder(&mut self, dir: &Path) -> std::io::Result<()> {
        let previous = self.config.data_dir.clone();
        self.config.set_data_dir(dir);
        self.config.save().inspect_err(|_| self.config.data_dir = previous)
    }

    /// Move the database file into `dir`: copy, verify, switch over, and only
    /// then delete the old file.
    fn move_database(&mut self, dir: &Path, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.saved_before_switch(cx) {
            return;
        }
        let target = config::database_path_in(dir);
        let moved = match self.store.copy_and_open(&target) {
            Ok(store) => store,
            Err(e) => {
                self.storage_note = Some(StorageNote::Error(format!(
                    "移动失败，数据库仍在原位置：{e}"
                )));
                return;
            }
        };
        if let Err(e) = self.remember_data_folder(dir) {
            drop(moved);
            let _ = store::remove_database_files(&target);
            self.storage_note = Some(StorageNote::Error(format!(
                "保存设置失败，数据库仍在原位置：{e}"
            )));
            return;
        }

        // Same notes at a new address: the list, the open editor and the
        // WebDAV bookkeeping all stay valid.
        let old = std::mem::replace(&mut self.store, moved);
        let old_path = old.path().to_path_buf();
        drop(old); // closes the old file so it can be deleted
        self.storage_note = Some(match store::remove_database_files(&old_path) {
            Ok(()) => StorageNote::Info("数据库已移动到新位置".into()),
            Err(e) => StorageNote::Error(format!(
                "数据库已移动到新位置，但旧文件没能删除（{e}）：{}",
                old_path.display()
            )),
        });
        debug_log("database moved");
    }

    /// The user confirmed switching to the database that already exists in
    /// the picked folder. The current database stays where it is.
    pub fn confirm_adopt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dir) = self.pending_adopt.take() else {
            return;
        };
        cx.notify();
        if !self.saved_before_switch(cx) {
            return;
        }
        let target = config::database_path_in(&dir);
        let adopted = match NoteStore::open_existing(&target) {
            Ok(store) => store,
            Err(e) => {
                self.storage_note =
                    Some(StorageNote::Error(format!("无法使用该数据库：{e}")));
                return;
            }
        };
        if let Err(e) = self.remember_data_folder(&dir) {
            self.storage_note = Some(StorageNote::Error(format!(
                "保存设置失败，仍在使用原数据库：{e}"
            )));
            return;
        }

        // The quick note was writing into the database that is being left
        // (where its text is saved by now); its session ends with it.
        self.quick_close(cx);
        let old_path = self.store.path().to_path_buf();
        self.store = adopted;
        // Different notes from here on: what a running sync pass computed and
        // what the version cache remembers were about the previous database.
        self.store_generation = self.store_generation.wrapping_add(1);
        self.webdav_cache = webdav::SyncCache::default();
        self.storage_note = Some(StorageNote::Info(format!(
            "已改用所选文件夹中的数据库；原数据库仍保留在 {}",
            old_path.display()
        )));
        debug_log("database adopted");
        self.start_session(window, cx);
    }

    pub fn cancel_adopt(&mut self, cx: &mut Context<Self>) {
        self.pending_adopt = None;
        cx.notify();
    }

    /// Merge the notes of Windows Sticky Notes into the database (read-only
    /// on their side): new notes are added, notes edited there since the last
    /// import are updated, nothing is deleted.
    pub fn import_from_windows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
        let sources = legacy::LegacySources::detect();
        let Some(plum) = sources.plum.clone() else {
            self.storage_note = Some(StorageNote::Error(
                "没有找到 Windows 便签的数据（未安装或从未打开过“便签”应用）".into(),
            ));
            return;
        };
        if !self.saved_before_switch(cx) {
            return;
        }
        match import::import_from_windows(&self.store, &plum, &sources.shadow_markdown()) {
            Ok(report) => {
                debug_log(&format!(
                    "import added={} updated={} unchanged={}",
                    report.added, report.updated, report.unchanged
                ));
                self.storage_note =
                    Some(StorageNote::Info(format!("导入完成：{}", report.summary())));
                if report.changed() {
                    self.reload(window, cx);
                    self.schedule_webdav_sync(window, cx);
                }
            }
            Err(e) => {
                self.storage_note = Some(StorageNote::Error(format!("导入失败：{e}")));
            }
        }
    }

    // --- Recovery screen (the database could not be opened) ---

    /// Try the configured location again, e.g. after reconnecting a drive.
    pub fn retry_open_database(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match open_configured_database(&self.config) {
            Ok(store) => {
                self.store = store;
                self.startup_error = None;
                self.start_session(window, cx);
            }
            Err(e) => self.startup_error = Some(e.to_string()),
        }
        cx.notify();
    }

    /// Give up on the custom folder and use the default one. The database in
    /// the custom folder is not touched.
    pub fn use_default_database(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(e) = self.remember_data_folder(&config::default_data_dir()) {
            self.startup_error = Some(format!("保存设置失败：{e}"));
            cx.notify();
            return;
        }
        self.retry_open_database(window, cx);
    }

    /// Open the folder that holds the automatic database snapshots.
    pub fn reveal_backups(&mut self, cx: &mut Context<Self>) {
        let dir = backup::default_dir();
        let _ = std::fs::create_dir_all(&dir);
        cx.open_with_system(&dir);
    }

    // --- WebDAV sync ---

    pub fn test_webdav(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        debug_log("ui: 测试连接 clicked");
        let _ = self.persist_settings(cx);
        let cfg = self.config.webdav.clone();
        if cfg.url.trim().is_empty() || cfg.username.trim().is_empty() || cfg.password.is_empty()
        {
            self.webdav_test = WebDavTest::Err("请先填写服务器地址、用户名和密码".into());
            cx.notify();
            return;
        }
        self.webdav_test = WebDavTest::Testing;
        cx.notify();

        let this = cx.weak_entity();
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                let result = cx
                    .background_executor()
                    .spawn(async move { webdav::test_connection(&cfg) })
                    .await;
                let _ = this.update_in(cx, |this, _window, cx| {
                    this.webdav_test = match result {
                        Ok(msg) => WebDavTest::Ok(msg),
                        Err(e) => WebDavTest::Err(e),
                    };
                    cx.notify();
                });
            })
            .detach();
    }

    /// One WebDAV sync in the background. Silent when not configured.
    ///
    /// [`SyncPass::Forced`] ignores the version cache and re-creates the
    /// collection; the manual “立即同步” button uses it.
    pub fn start_webdav_sync(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        pass: SyncPass,
    ) {
        debug_log(&format!("ui: sync requested pass={:?}", pass));
        // A pass the user caused replaces the "test connection" line in the
        // settings row. That line renders with priority over sync messages,
        // so without this an old 连接成功 would hide every later sync result
        // for the rest of the visit.
        if !pass.quiet() && self.webdav_test != WebDavTest::Idle {
            self.webdav_test = WebDavTest::Idle;
            cx.notify();
        }
        if self.webdav_syncing {
            // Coalesce: run one more pass when the current one finishes.
            self.webdav_pending = true;
            if !pass.quiet() {
                // A visible pass was asked for meanwhile. If a quiet one is
                // running, show it now; the pass it queues up runs as asked.
                if self.webdav_quiet {
                    self.webdav_quiet = false;
                    self.webdav_message = Some("正在同步…".into());
                }
                self.webdav_pending_force |= pass.force();
                cx.notify();
            }
            return;
        }
        if self.show_settings {
            let _ = self.persist_settings(cx);
        }
        if !self.config.webdav_configured() {
            if self.show_settings && !pass.quiet() {
                self.webdav_message = Some("请先填写服务器地址、用户名和密码".into());
                cx.notify();
            }
            return;
        }

        // The cache only applies to the server/directory it was built for.
        if !self.webdav_cache.matches(&self.config.webdav) {
            self.webdav_cache = webdav::SyncCache::default();
            self.webdav_dir_ready = false;
        }
        if !self.webdav_deletions.matches(&self.config.webdav) {
            self.webdav_deletions = webdav::DeleteLedger::default();
        }

        // Make sure the note being edited is on disk before we snapshot.
        self.flush_pending_save(cx);

        // An incomplete snapshot would make missing notes look deleted or
        // remote-only, so a failed read skips this pass instead.
        let notes = match self.local_sync_notes() {
            Ok(notes) => notes,
            Err(e) => {
                self.webdav_next_try = Instant::now() + Duration::from_secs(30);
                self.webdav_message = Some(format!("同步失败：{e}"));
                self.webdav_failed = true;
                if !pass.quiet() {
                    self.status = SyncStatus::Error(format!("WebDAV 同步失败：{e}"));
                }
                cx.notify();
                return;
            }
        };

        self.webdav_syncing = true;
        self.webdav_quiet = pass.quiet();
        self.webdav_pending = false;
        self.webdav_pending_force = false;
        self.webdav_last_pass = Some(Instant::now());
        if !pass.quiet() {
            self.webdav_message = Some("正在同步…".into());
        }
        let cfg = self.config.webdav.clone();
        let force = pass.force();
        let cache = if force {
            webdav::SyncCache::default()
        } else {
            self.webdav_cache.clone()
        };
        let ensure_dir = force || !self.webdav_dir_ready;
        let ledger = self.webdav_deletions.clone();
        // Ids already in the ledger when this pass starts. Anything recorded
        // while it runs (a delete during an in-flight sync) is not in the
        // snapshot and must survive the pass's pruned ledger.
        let deletions_before: std::collections::BTreeSet<String> =
            ledger.deleted.keys().cloned().collect();
        let generation = self.store_generation;
        let this = cx.weak_entity();
        cx.notify();
        debug_log(&format!(
            "webdav pass start quiet={} force={} ensure={} notes={} pending={}",
            pass.quiet(),
            force,
            ensure_dir,
            notes.len(),
            self.webdav_pending
        ));

        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let mut ledger = ledger;
                        webdav::sync(&cfg, &notes, &cache, ensure_dir, &mut ledger)
                    })
                    .await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.finish_webdav_sync(result, generation, deletions_before, window, cx);
                });
            })
            .detach();
    }

    /// Debounce WebDAV uploads so a burst of typing becomes one sync.
    fn schedule_webdav_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.config.webdav_auto || !self.config.webdav_configured() {
            return;
        }
        self.webdav_epoch = self.webdav_epoch.wrapping_add(1);
        let epoch = self.webdav_epoch;
        // How long to wait is the user's choice (settings › 通用).
        let delay = self.config.sync_delay();
        let this = cx.weak_entity();
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                cx.background_executor().timer(delay).await;
                let _ = this.update_in(cx, |this, window, cx| {
                    if this.webdav_epoch == epoch {
                        this.start_webdav_sync(window, cx, SyncPass::Visible);
                    }
                });
            })
            .detach();
    }

    /// Every live note as WebDAV sees it.
    fn local_sync_notes(&self) -> store::Result<Vec<webdav::SyncNote>> {
        let notes = self.store.live_notes()?;
        Ok(notes
            .into_iter()
            .map(|note| {
                let (title, _) = Self::split_title_body(&note.markdown);
                webdav::SyncNote {
                    title,
                    color: note.theme,
                    tags: self
                        .tags_for_note(&note.id)
                        .iter()
                        .map(|t| t.name.clone())
                        .collect(),
                    pinned: self.is_pinned(&note.id),
                    updated_at: note.updated_at,
                    meta_updated_at: note.meta_updated_at,
                    markdown: note.markdown,
                    id: note.id,
                }
            })
            .collect())
    }

    fn finish_webdav_sync(
        &mut self,
        result: Result<webdav::SyncOutcome, String>,
        generation: u64,
        deletions_before: std::collections::BTreeSet<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.webdav_syncing = false;
        // Read once: a manual request during a quiet pass turns it visible.
        let quiet = std::mem::take(&mut self.webdav_quiet);
        // A pass snapshots the ledger when it starts, so a note deleted while
        // it was in flight is not in the pruned ledger it hands back.
        // Re-apply those intents or the next pass would re-import the remote
        // `.md` and resurrect the note.
        let deletions_during = self.webdav_deletions.added_since(&deletions_before);
        if generation != self.store_generation {
            // The database was switched while this pass ran: its decisions
            // were made for other notes, so they are dropped. What it removed
            // from the server is still a fact, though.
            if let Ok(outcome) = result {
                let mut ledger = outcome.ledger;
                // Settings changed while the pass ran: its ledger belongs to
                // the previous target, so keep the one already in memory.
                if ledger.matches(&self.config.webdav) {
                    for (id, at) in &deletions_during {
                        ledger.deleted.insert(id.clone(), *at);
                    }
                    self.webdav_deletions = ledger;
                    let _ = self.webdav_deletions.save();
                }
            }
            self.webdav_pending = false;
            self.webdav_pending_force = false;
            self.webdav_message = None;
            self.schedule_webdav_sync(window, cx);
            cx.notify();
            return;
        }
        match result {
            Ok(outcome) => {
                let previous_cache = std::mem::replace(&mut self.webdav_cache, outcome.cache);
                let _ = self.webdav_cache.save();
                let mut ledger = outcome.ledger;
                // Settings changed while the pass ran: its ledger belongs to
                // the previous target, so keep the one already in memory.
                if ledger.matches(&self.config.webdav) {
                    for (id, at) in &deletions_during {
                        ledger.deleted.insert(id.clone(), *at);
                    }
                    self.webdav_deletions = ledger;
                    let _ = self.webdav_deletions.save();
                }
                self.webdav_dir_ready = true;
                self.webdav_failures = 0;
                self.webdav_next_try = Instant::now() + self.pull_interval_now();

                let report = outcome.report;
                debug_log(&format!(
                    "webdav pass uploaded={} downloaded={} deleted_remote={} deleted_local={} unchanged={}",
                    report.uploaded,
                    report.downloaded,
                    report.deleted_remote,
                    report.deleted_local.len(),
                    report.unchanged
                ));
                // The pass compared a snapshot taken when it started. Text
                // typed since then must be on disk before its decisions are
                // applied, so that newer local edits are recognised as such.
                if !report.deleted_local.is_empty() || !report.remote_notes.is_empty() {
                    self.flush_pending_save(cx);
                    self.quick_write(cx);
                }
                let mut changed = false;
                let mut write_error = None;

                for id in &report.deleted_local {
                    // The file was removed on another device and this device
                    // had no newer edit when the pass compared them: follow
                    // the deletion — unless the note was edited meanwhile, in
                    // which case it stays and the next pass uploads it.
                    let last_synced = previous_cache
                        .files
                        .get(&format!("{id}.md"))
                        .map(|file| file.updated_at);
                    // The note's version: a colour, tag or pin change counts
                    // as an edit in its own right.
                    let current = self.store.note_version(id).ok().flatten();
                    if matches!((current, last_synced), (Some(now), Some(then)) if now > then) {
                        debug_log(&format!("webdav tombstone skipped (edited) id={}", short_id(id)));
                        continue;
                    }
                    match self.store.soft_delete(id) {
                        Ok(()) => {
                            debug_log(&format!("webdav tombstone applied id={}", short_id(id)));
                            changed = true;
                            if self.selected_id.as_deref() == Some(id.as_str()) {
                                self.save_epoch = self.save_epoch.wrapping_add(1);
                                self.clear_selection();
                            }
                        }
                        Err(e) => write_error = Some(format!("删除便签失败：{e}")),
                    }
                }

                for note in &report.remote_notes {
                    // Deleted here while the pass was comparing: the merged
                    // ledger removes this remote file on the next pass, so do
                    // not let the download bring the note back now.
                    if deletions_during.contains_key(&note.id) {
                        debug_log(&format!(
                            "webdav remote skipped (deleted during pass) id={}",
                            short_id(&note.id)
                        ));
                        continue;
                    }
                    // A note that became newer here while the pass ran keeps
                    // its text (`apply_remote` returns false).
                    let applied = self.store.apply_remote(&RemoteUpdate {
                        id: &note.id,
                        markdown: &note.markdown,
                        theme: &note.color,
                        updated_at: note.updated_at,
                        meta_updated_at: note.meta_updated_at,
                        tags: &note.tags,
                        pinned: note.pinned,
                    });
                    match applied {
                        Ok(applied) => {
                            debug_log(&format!(
                                "webdav remote applied={} id={}",
                                applied,
                                short_id(&note.id)
                            ));
                            changed |= applied;
                        }
                        Err(e) => write_error = Some(format!("写入远端便签失败：{e}")),
                    }
                }

                if changed {
                    // Show what arrived (or disappeared) right away.
                    self.refresh_after_remote(window, cx);
                }
                let summary = report.summary();
                let touched = report.uploaded
                    + report.downloaded
                    + report.deleted_remote
                    + report.deleted_local.len()
                    > 0;
                // A quiet pass that found nothing to do says nothing: the
                // status line belongs to what the user did, not to the poll.
                if !quiet || touched || write_error.is_some() {
                    self.webdav_message = Some(summary.clone());
                }
                if let Some(e) = write_error {
                    self.status = SyncStatus::Error(e);
                } else if !quiet || touched {
                    self.status = SyncStatus::Saved(summary);
                }
                self.webdav_failed = false;
            }
            Err(e) => {
                self.webdav_failures = self.webdav_failures.saturating_add(1);
                debug_log(&format!(
                    "webdav pass failed quiet={} failures={} next={}s msg={}",
                    quiet,
                    self.webdav_failures,
                    (30u64 << self.webdav_failures.saturating_sub(1).min(5)).min(600),
                    e
                ));
                // 30s, 60s, 120s, 240s, 480s, then 10 minutes — a down or
                // unreachable server is not retried every minute.
                let shift = self.webdav_failures.saturating_sub(1).min(5);
                let secs = (30u64 << shift).min(600);
                self.webdav_next_try = Instant::now() + Duration::from_secs(secs);
                // Say it once, and say it now: the red dot keeps saying it
                // until a pass succeeds, so the failure is never missed.
                if !quiet || self.show_settings {
                    self.webdav_message = Some(format!("同步失败：{e}"));
                    self.status = SyncStatus::Error(format!("WebDAV 同步失败：{e}"));
                } else {
                    self.webdav_message = Some(format!("同步失败：{e}"));
                }
                self.webdav_failed = true;
            }
        }
        if self.webdav_pending {
            // Work arrived while this pass was running; pick it up soon. A
            // manual request runs at once, as it was asked to.
            self.webdav_pending = false;
            if std::mem::take(&mut self.webdav_pending_force) {
                self.start_webdav_sync(window, cx, SyncPass::Forced);
            } else {
                self.schedule_webdav_sync(window, cx);
            }
        }
        cx.notify();
    }

    /// How long until the next background pull: the user's interval, and at
    /// least ten minutes while the main window stays hidden in the tray.
    fn pull_interval_now(&self) -> Duration {
        let interval = self.config.pull_interval();
        let visible = self.main_hwnd.map(win::is_visible).unwrap_or(true);
        if visible {
            interval
        } else {
            interval.max(Duration::from_secs(600))
        }
    }

    /// Bring the list, tags and pins in line with the database after a pass
    /// wrote to it. The open note keeps its editor — caret, focus and undo
    /// history — unless its own text changed, in which case the editor is
    /// rebuilt and the focus put back where it was.
    fn refresh_after_remote(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.store.list_notes() {
            Ok(notes) => self.notes = notes,
            Err(e) => self.status = SyncStatus::Error(format!("读取便签失败：{e}")),
        }
        self.reload_tags_cache();

        let Some(id) = self.selected_id.clone() else {
            self.select_note_by_index(0, window, cx);
            return;
        };
        if !self.filtered_notes().iter().any(|n| n.id == id) {
            // The note vanished — deleted on another device, or now hidden by
            // the tag or search filter.
            self.clear_selection();
            self.select_note_by_index(0, window, cx);
            return;
        }

        // Only this note's stored text can force a rebuild: the pass was
        // applied after a flush, so the editor holds exactly what was saved
        // unless the note itself was replaced by a remote version.
        let stored = self.store.note_markdown(&id).ok().flatten();
        if stored.as_deref() == Some(self.compose_md(cx).as_str()) {
            cx.notify();
            return;
        }
        let editor_focused = self
            .editor
            .as_ref()
            .is_some_and(|e| e.read(cx).focus_handle(cx).is_focused(window));
        let title_focused = self
            .title_input
            .as_ref()
            .is_some_and(|i| i.read(cx).focus_handle(cx).is_focused(window));
        // Drop any debounce captured against the editor being replaced.
        self.save_epoch = self.save_epoch.wrapping_add(1);
        self.clear_selection();
        self.select_note(&id, window, cx);
        let handle = if editor_focused {
            self.editor.as_ref().map(|e| e.read(cx).focus_handle(cx))
        } else if title_focused {
            self.title_input.as_ref().map(|i| i.read(cx).focus_handle(cx))
        } else {
            None
        };
        if let Some(handle) = handle {
            window.focus(&handle, cx);
        }
        cx.notify();
    }

    /// Change how often the background loop checks for other devices'
    /// changes. The new rhythm starts right away.
    pub fn set_pull_interval(&mut self, secs: u64, cx: &mut Context<Self>) {
        self.settings_menu = None;
        let secs = config::clamp_pull_interval_secs(secs);
        if self.config.pull_interval_secs != secs {
            self.config.pull_interval_secs = secs;
            let _ = self.config.save();
            self.webdav_next_try = Instant::now();
        }
        cx.notify();
    }
}
