use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use markdown_editor::{BlockEditorEvent, BlockEditorState};
use stickynotes_core::{
    config::SyncConfig,
    markup,
    store::model::NoteSummary,
    sync::{self, SyncCoordinator},
    tags::{Tag, TagStore},
    webdav,
};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

/// Visual spec for plum themes with refined paper aesthetics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeSpec {
    pub name: &'static str,
    pub label: &'static str,
    pub accent: u32,
    pub paper_bg: u32,
    pub card_active_bg: u32,
    pub border: u32,
    pub is_dark: bool,
}

pub const THEMES: &[ThemeSpec] = &[
    ThemeSpec {
        name: "Yellow",
        label: "暖阳黄",
        accent: 0xE8B955,
        paper_bg: 0xFFFDF4,
        card_active_bg: 0xFBF3D0,
        border: 0xF0E4B8,
        is_dark: false,
    },
    ThemeSpec {
        name: "Green",
        label: "薄荷绿",
        accent: 0x6FB594,
        paper_bg: 0xF3FBF5,
        card_active_bg: 0xDFF2E4,
        border: 0xC4E6CC,
        is_dark: false,
    },
    ThemeSpec {
        name: "Pink",
        label: "蔷薇粉",
        accent: 0xD994A9,
        paper_bg: 0xFDF5F8,
        card_active_bg: 0xF7E1E9,
        border: 0xEFD0DB,
        is_dark: false,
    },
    ThemeSpec {
        name: "Purple",
        label: "薰衣紫",
        accent: 0xA693D6,
        paper_bg: 0xF9F6FD,
        card_active_bg: 0xEBE2F6,
        border: 0xDCCFF0,
        is_dark: false,
    },
    ThemeSpec {
        name: "Blue",
        label: "天空蓝",
        accent: 0x7FABC8,
        paper_bg: 0xF2F8FC,
        card_active_bg: 0xDFECF4,
        border: 0xC6DDEA,
        is_dark: false,
    },
    ThemeSpec {
        name: "Gray",
        label: "晨雾灰",
        accent: 0x9AA5B1,
        paper_bg: 0xF9FAFB,
        card_active_bg: 0xEEF1F4,
        border: 0xE0E5EA,
        is_dark: false,
    },
    ThemeSpec {
        name: "Charcoal",
        label: "暗夜黑",
        accent: 0x8A8A93,
        paper_bg: 0x212126,
        card_active_bg: 0x2C2C33,
        border: 0x3F3F46,
        is_dark: true,
    },
    ThemeSpec {
        name: "White",
        label: "纯白",
        accent: 0xB4BCC8,
        paper_bg: 0xFFFFFF,
        card_active_bg: 0xF3F4F6,
        border: 0xE8EAEF,
        is_dark: false,
    },
];

pub fn get_theme(name: Option<&str>) -> &'static ThemeSpec {
    let target = name.unwrap_or("Yellow");
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

/// Result of the settings dialog's "test connection" probe.
#[derive(Debug, Clone, PartialEq)]
pub enum WebDavTest {
    Idle,
    Testing,
    Ok(String),
    Err(String),
}

/// Settings dialog form fields, created on first open.
pub struct SettingsInputs {
    pub url: Entity<InputState>,
    pub user: Entity<InputState>,
    pub pass: Entity<InputState>,
    pub dir: Entity<InputState>,
}

/// Cheap change fingerprint for the plum database: (mtime nanos, length) of
/// the main file and of the WAL, which is where WAL-mode writes land first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DbSignature {
    main: (u64, u64),
    wal: (u64, u64),
}

fn file_sig(path: &Path) -> (u64, u64) {
    match std::fs::metadata(path) {
        Ok(m) => {
            let mtime = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            (mtime, m.len())
        }
        Err(_) => (0, 0),
    }
}

fn plum_signature(plum_path: &Path) -> DbSignature {
    DbSignature {
        main: file_sig(plum_path),
        wal: file_sig(&plum_path.with_extension("sqlite-wal")),
    }
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

/// Opt-in diagnostics: set `STICKYNOTES_DEBUG=1` to append save/sync decisions
/// (note-id prefix, text lengths, flags — never note content) to
/// `%LOCALAPPDATA%\stickynotes\debug.log`.
fn debug_log(msg: &str) {
    use std::io::Write as _;
    if std::env::var_os("STICKYNOTES_DEBUG").is_none() {
        return;
    }
    let Ok(local) = std::env::var("LOCALAPPDATA") else {
        return;
    };
    let dir = Path::new(&local).join("stickynotes");
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
    /// Keyboard shortcut, empty when the editor binds none.
    pub keys: &'static str,
    /// Button's left edge and bottom edge in window coordinates.
    pub x: Pixels,
    pub y: Pixels,
}

pub struct AppState {
    pub sync: Arc<SyncCoordinator>,
    pub tags: Arc<TagStore>,
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

    // Auto-sync bookkeeping
    /// Last editor content successfully written to plum (used to tell whether
    /// replacing the editor from disk would discard unsaved edits).
    pub last_saved_md: Option<String>,
    /// Exact plum text of our last successful write for the open note. Lets the
    /// poll skip the row when the only change on disk is our own save, instead
    /// of round-tripping it through Markdown and rewriting the editor.
    pub last_saved_plum: Option<String>,
    /// A save failed because the DB was momentarily locked: the note id and
    /// Markdown to retry, captured at failure time.
    pub pending_save: Option<(String, String)>,
    /// Fingerprint (mtime, size) of plum.sqlite + its WAL, to detect external writes.
    db_sig: Option<DbSignature>,
    /// Is the native Sticky Notes app open? It owns the cloud sync and keeps
    /// its own copy of every note, which it can write back over ours.
    pub native_app_running: bool,
    poll_ticks: u32,

    // Settings & WebDAV sync
    pub config: SyncConfig,
    pub show_settings: bool,
    pub settings: Option<SettingsInputs>,
    pub webdav_test: WebDavTest,
    pub webdav_syncing: bool,
    pub webdav_message: Option<String>,
    /// Bumped on every edit so only the last debounced WebDAV push runs.
    webdav_epoch: u64,

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
    /// Button the pointer rests on right now; the hint appears after a short
    /// delay only if it is still this button.
    pub md_tip_hover: Option<&'static str>,
    /// Latest painted bounds of each toolbar button, keyed by element id.
    /// Written from prepaint (no notify) and read when a hover starts.
    pub md_tip_bounds: Rc<RefCell<HashMap<&'static str, Bounds<Pixels>>>>,
}

impl AppState {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (sync, tags, err) = match (SyncCoordinator::new(), TagStore::open()) {
            (Ok(s), Ok(t)) => (Some(Arc::new(s)), Some(Arc::new(t)), None),
            (Err(e), _) => (None, None, Some(format!("初始化失败: {e}"))),
            (Ok(_), Err(e)) => (None, None, Some(format!("标签库打开失败: {e}"))),
        };
        let (sync, tags) = match (sync, tags) {
            (Some(s), Some(t)) => (s, t),
            _ => unreachable!(),
        };
        let _ = err;

        let notes = sync.reload().unwrap_or_default();

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

        let config = SyncConfig::load();
        let window_handle = window.window_handle();
        let mut app = Self {
            sync,
            tags,
            notes,
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
            last_saved_plum: None,
            pending_save: None,
            db_sig: None,
            native_app_running: false,
            poll_ticks: 0,
            config,
            show_settings: false,
            settings: None,
            webdav_test: WebDavTest::Idle,
            webdav_syncing: false,
            webdav_message: None,
            webdav_epoch: 0,
            show_sidebar_add_tag: false,
            new_tag_input: Some(new_tag_input),
            search_input: Some(search_input),
            title_input: None,
            window_handle,
            md_tip: None,
            md_tip_hover: None,
            md_tip_bounds: Rc::new(RefCell::new(HashMap::new())),
        };

        app.reload_tags_cache();
        app.prune_tag_orphans();
        if !app.notes.is_empty() {
            app.select_note_by_index(0, window, cx);
        }
        app.start_auto_sync(window, cx);
        // Pull WebDAV changes once at startup; silent when the server is not
        // configured yet.
        if app.config.webdav_auto && app.config.webdav_configured() {
            app.start_webdav_sync(window, cx);
        }
        app
    }

    /// Refresh in-memory tag cache with a single batch query.
    pub fn reload_tags_cache(&mut self) {
        let tags = self.tags.list_tags().unwrap_or_default();
        let tag_map: HashMap<i64, Tag> = tags.iter().map(|t| (t.id, t.clone())).collect();

        let pairs = self.tags.all_note_tags().unwrap_or_default();
        let mut note_tags: HashMap<String, Vec<Tag>> = HashMap::new();
        let mut counts: HashMap<i64, usize> = HashMap::new();

        for (guid, tid) in pairs {
            if let Some(tag) = tag_map.get(&tid) {
                note_tags.entry(guid).or_default().push(tag.clone());
                *counts.entry(tid).or_default() += 1;
            }
        }

        self.tags_list = tags;
        self.note_tags_map = note_tags;
        self.tag_counts = counts;
        self.pinned = self
            .tags
            .pinned_guids()
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
        let theme_name = self.current_note().and_then(|n| n.theme.as_deref());
        get_theme(theme_name)
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

    pub fn ensure_selection_after_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let filtered = self.filtered_notes();
        if filtered.is_empty() {
            self.flush_pending_save(cx);
            self.selected_id = None;
            self.editor = None;
            self.title_input = None;
            self.last_saved_md = None;
            self.last_saved_plum = None;
            self.current_stats = (0, 0);
        } else if !filtered.iter().any(|n| Some(&n.id) == self.selected_id.as_ref()) {
            self.select_note_by_index(0, window, cx);
        }
    }

    pub fn select_note(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_id.as_deref() == Some(id) && self.editor.is_some() {
            return;
        }
        // The debounced autosave may still be pending for the note we are
        // leaving. Flush it before its editor is replaced — otherwise the
        // timer fires against the *new* editor and the edit is silently lost.
        self.flush_pending_save(cx);

        let Some(plum_text) = self.sync.plum.get_note_text(id).ok().flatten() else {
            return;
        };
        // Prefer our full-fidelity copy: plum cannot hold headings, fences,
        // tables and so on, so converting back would strip the formatting.
        let stored = self.sync.note_markdown(id, &plum_text);
        let md = stored.clone().unwrap_or_else(|| markup::plum_to_markdown(&plum_text));
        debug_log(&format!(
            "select id={} len={} shadow={}",
            short_id(id),
            md.len(),
            stored.is_some()
        ));
        // The first paragraph is the note title; the rest is the body.
        let (title, body) = Self::split_title_body(&md);
        let scroll = self.scroll.clone();

        let theme_spec = self
            .notes
            .iter()
            .find(|n| n.id == id)
            .and_then(|n| n.theme.as_deref())
            .map(|t| get_theme(Some(t)))
            .unwrap_or(&THEMES[0]);
        let is_dark = theme_spec.is_dark;

        let editor = cx.new(|cx| {
            let mut state = BlockEditorState::new(window, cx).with_source(&body);
            state.set_scroll_handle(scroll);
            if is_dark {
                state.set_editor_style(
                    markdown_editor::BlockEditorStyle::default()
                        .with_foreground(rgb(0xF3F4F6).into())
                        .with_muted_foreground(rgb(0x9CA3AF).into())
                        .with_caret(rgb(0xF3F4F6).into()),
                    cx,
                );
            } else {
                state.set_editor_style(
                    markdown_editor::BlockEditorStyle::default()
                        .with_foreground(rgb(0x1F2937).into())
                        .with_muted_foreground(rgb(0x6B7280).into())
                        .with_caret(rgb(0x2563EB).into()),
                    cx,
                );
            }
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
        self.pending_save = None;
        // Store the composed document (title + body): the auto-sync check
        // compares it against `compose_md` later. `last_saved_plum` is the
        // exact row text we loaded, so a poll can recognise it as unchanged.
        self.last_saved_md = Some(self.compose_md(cx));
        self.last_saved_plum = Some(plum_text);
        self.update_stats_from_editor(cx);
        cx.notify();
    }

    pub fn select_note_by_index(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        let notes = self.filtered_notes();
        if let Some(note) = notes.get(idx) {
            let id = note.id.clone();
            self.select_note(&id, window, cx);
        }
    }

    /// Split canonical markdown into (plain-text title, body markdown). The
    /// first paragraph is the note title, matching Windows Sticky Notes.
    fn split_title_body(md: &str) -> (String, String) {
        match md.split_once('\n') {
            Some((first, rest)) => (
                stickynotes_core::store::strip_markdown_syntax(first),
                rest.to_string(),
            ),
            None => (
                stickynotes_core::store::strip_markdown_syntax(md),
                String::new(),
            ),
        }
    }

    /// Compose the title field and body editor back into one markdown
    /// document — the form written to plum and compared by auto-sync.
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
            self.save_current(cx);
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
                    this.save_current(cx);
                    this.schedule_webdav_sync(window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn save_current(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_id.clone() else {
            return;
        };
        if self.editor.is_none() {
            return;
        }
        let md = self.compose_md(cx);
        debug_log(&format!("save id={} len={}", short_id(&id), md.len()));
        match self.sync.save_note(&id, &md) {
            Ok((plum_text, updated_at)) => {
                // Did the row actually keep our text, and does the Markdown
                // survive a plum round trip? (No note content is logged.)
                let stick = self.sync.plum.get_note_text(&id).ok().flatten().as_deref()
                    == Some(plum_text.as_str());
                let rt = markup::plum_to_markdown(&plum_text);
                debug_log(&format!(
                    "save ok stick={stick} rt_eq={} md={} rt={}",
                    rt == md,
                    md.len(),
                    rt.len()
                ));
                self.status = SyncStatus::Saved("已自动保存".into());
                self.last_saved_md = Some(md.clone());
                self.last_saved_plum = Some(plum_text);
                self.pending_save = None;
                // Our own write must not look like an external change.
                self.refresh_db_sig();
                if let Some(note) = self.notes.iter_mut().find(|n| n.id == id) {
                    note.updated_at = Some(updated_at);
                }
                debug_log("save ok");
            }
            Err(e) => {
                let msg = format!("{e}");
                // The Windows app may hold the write lock for a moment; the
                // background poll retries the exact text that failed instead
                // of losing the edit.
                let retryable = msg.contains("locked") || msg.contains("busy");
                self.pending_save = retryable.then(|| (id.clone(), md.clone()));
                self.status = if retryable {
                    SyncStatus::Busy("数据库被占用，正在自动重试保存…".into())
                } else {
                    SyncStatus::Error(msg.clone())
                };
                debug_log(&format!("save err retryable={retryable} msg={msg}"));
            }
        }

        // Direct in-memory summary update (no full disk reload)
        let plum_text = markup::markdown_to_plum(&md);
        let (title, snippet) = stickynotes_core::store::extract_title_and_snippet(&plum_text);
        if let Some(note) = self.notes.iter_mut().find(|n| n.id == id) {
            note.title = title;
            note.snippet = snippet;
        }
        self.update_stats_from_editor(cx);
        cx.notify();
    }

    pub fn new_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.sync.create_note("", "Yellow") {
            Ok(new_id) => {
                if let Ok(notes) = self.sync.reload() {
                    self.notes = notes;
                }
                self.refresh_db_sig();
                self.prune_tag_orphans();
                self.select_note(&new_id, window, cx);
                self.status = SyncStatus::Saved("已新建便签".into());
            }
            Err(e) => {
                self.status = SyncStatus::Error(format!("{e}"));
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
        // Deleting the open note drops its pending debounce; writing it after
        // the soft delete would bump UpdatedAt on a tombstone. Deleting another
        // note flushes the open one so no edit is lost.
        if self.selected_id.as_deref() == Some(id) {
            self.save_epoch = self.save_epoch.wrapping_add(1);
            self.pending_save = None;
            self.selected_id = None;
            self.editor = None;
            self.title_input = None;
            self.last_saved_md = None;
            self.last_saved_plum = None;
        }
        match self.sync.delete_note(id) {
            Ok(()) => {
                self.status = SyncStatus::Saved("便签已删除".into());
                self.reload(window, cx);
            }
            Err(e) => {
                self.status = SyncStatus::Error(format!("{e}"));
                cx.notify();
            }
        }
    }

    pub fn set_theme(&mut self, theme: &str, cx: &mut Context<Self>) {
        let Some(id) = self.selected_id.clone() else { return };
        match self.sync.plum.set_theme(&id, theme) {
            Ok(()) => {
                self.status = SyncStatus::Saved(format!("颜色已设为 {theme}"));
                self.refresh_db_sig();
                if let Some(note) = self.notes.iter_mut().find(|n| n.id == id) {
                    note.theme = Some(theme.to_string());
                }
                let is_dark = get_theme(Some(theme)).is_dark;
                if let Some(editor) = &self.editor {
                    editor.update(cx, |state, cx| {
                        if is_dark {
                            state.set_editor_style(
                                markdown_editor::BlockEditorStyle::default()
                                    .with_foreground(rgb(0xF3F4F6).into())
                                    .with_muted_foreground(rgb(0x9CA3AF).into())
                                    .with_caret(rgb(0xF3F4F6).into()),
                                cx,
                            );
                        } else {
                            state.set_editor_style(
                                markdown_editor::BlockEditorStyle::default()
                                    .with_foreground(rgb(0x1F2937).into())
                                    .with_muted_foreground(rgb(0x6B7280).into())
                                    .with_caret(rgb(0x2563EB).into()),
                                cx,
                            );
                        }
                    });
                }
            }
            Err(e) => self.status = SyncStatus::Error(format!("{e}")),
        }
        cx.notify();
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

    /// Push local changes through the native app's Microsoft sync. Kept for
    /// the Microsoft toggle path; the top-bar button is gone.
    #[allow(dead_code)]
    pub fn cloud_sync(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.save_current(cx);
        if sync::is_sticky_notes_running() {
            self.status =
                SyncStatus::Busy("Windows 便签已打开，正在上传本地修改…".into());
            cx.notify();
            return;
        }
        self.status = SyncStatus::Busy("已打开 Windows 便签，正在上传本地修改…".into());
        let _ = sync::launch_sticky_notes();
        cx.notify();
    }

    pub fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_pending_save(cx);
        match self.sync.reload() {
            Ok(notes) => {
                self.notes = notes;
                self.status = SyncStatus::Saved("已刷新".into());
                self.refresh_db_sig();
            }
            Err(e) => self.status = SyncStatus::Error(format!("{e}")),
        }
        self.reload_tags_cache();
        self.prune_tag_orphans();
        let keep = self.selected_id.clone();
        self.selected_id = None;
        self.editor = None;
        self.last_saved_plum = None;
        if !self.notes.is_empty() {
            let idx = keep
                .and_then(|id| self.notes.iter().position(|n| n.id == id))
                .unwrap_or(0);
            self.select_note_by_index(idx, window, cx);
        }
        cx.notify();
    }

    fn prune_tag_orphans(&mut self) {
        let guids: Vec<String> = self.notes.iter().map(|n| n.id.clone()).collect();
        let _ = self.tags.prune_orphans(&guids);
        self.reload_tags_cache();
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

    pub fn add_tag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.new_tag_input.clone() else { return };
        let name = input.read(cx).value().to_string();
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        match self.tags.create_tag(&name) {
            Ok(tid) => {
                if let Some(id) = &self.selected_id {
                    let _ = self.tags.tag_note(id, tid);
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
        let _ = self.tags.delete_tag(id);
        if self.tag_filter == Some(id) {
            self.tag_filter = None;
        }
        self.reload_tags_cache();
        self.status = SyncStatus::Saved("标签已删除".into());
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
            self.tags.untag_note(note_id, tag_id)
        } else {
            self.tags.tag_note(note_id, tag_id)
        };
        if let Err(e) = result {
            self.status = SyncStatus::Error(format!("{e}"));
        } else {
            self.reload_tags_cache();
        }
        cx.notify();
    }

    // --- Pin ---

    pub fn toggle_pin(&mut self, note_id: &str, cx: &mut Context<Self>) {
        let pin = !self.pinned.contains(note_id);
        match self.tags.set_pinned(note_id, pin) {
            Ok(()) => {
                if pin {
                    self.pinned.insert(note_id.to_string());
                } else {
                    self.pinned.remove(note_id);
                }
                self.status =
                    SyncStatus::Saved(if pin { "已置顶" } else { "已取消置顶" }.into());
            }
            Err(e) => self.status = SyncStatus::Error(format!("{e}")),
        }
        cx.notify();
    }

    // --- Automatic sync ---

    fn refresh_db_sig(&mut self) {
        self.db_sig = Some(plum_signature(self.sync.plum.path()));
    }

    /// Background loop: every 2s, notice writes made by the Windows Sticky
    /// Notes app and load them without a manual refresh; also retries a save
    /// that failed because the database was momentarily locked.
    fn start_auto_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_db_sig();
        let this = cx.weak_entity();
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(2000))
                        .await;
                    if this
                        .update_in(cx, |this, window, cx| {
                            this.poll_external_changes(window, cx);
                            // Background WebDAV pull, roughly once a minute.
                            if this.poll_ticks % 30 == 0
                                && this.config.webdav_auto
                                && !this.webdav_syncing
                            {
                                this.start_webdav_sync(window, cx);
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
    }

    // --- Settings dialog & WebDAV sync ---

    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_settings_inputs(window, cx);
        self.show_settings = true;
        self.webdav_test = WebDavTest::Idle;
        if let Some(s) = &self.settings {
            let handle = s.url.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        }
        cx.notify();
    }

    fn ensure_settings_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.is_some() {
            return;
        }
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
        self.settings = Some(SettingsInputs { url, user, pass, dir });
    }

    /// Copy the form back into the config and persist it.
    fn persist_settings(&mut self, cx: &App) {
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
        if let Err(e) = self.config.save() {
            self.status = SyncStatus::Error(format!("保存设置失败：{e}"));
        }
    }

    pub fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.persist_settings(cx);
        self.show_settings = false;
        // A fresh configuration is worth an immediate (debounced) sync.
        self.schedule_webdav_sync(window, cx);
        cx.notify();
    }

    pub fn set_webdav_auto(&mut self, on: bool, cx: &mut Context<Self>) {
        self.config.webdav_auto = on;
        let _ = self.config.save();
        cx.notify();
    }

    pub fn set_microsoft_sync(&mut self, on: bool, cx: &mut Context<Self>) {
        self.config.microsoft = on;
        let _ = self.config.save();
        if !on {
            // No longer following the native app; forget its state.
            self.native_app_running = false;
        }
        cx.notify();
    }

    pub fn test_webdav(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.persist_settings(cx);
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

    /// One full WebDAV sync in the background. Silent when not configured.
    pub fn start_webdav_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.webdav_syncing {
            return;
        }
        if self.show_settings {
            self.persist_settings(cx);
        }
        if !self.config.webdav_configured() {
            if self.show_settings {
                self.webdav_message = Some("请先填写服务器地址、用户名和密码".into());
                cx.notify();
            }
            return;
        }

        // Make sure the note being edited is on disk before we snapshot.
        self.flush_pending_save(cx);

        self.webdav_syncing = true;
        self.webdav_message = Some("正在同步…".into());
        let cfg = self.config.webdav.clone();
        let notes = self.local_sync_notes(cx);
        let this = cx.weak_entity();
        cx.notify();

        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                let result = cx
                    .background_executor()
                    .spawn(async move { webdav::sync(&cfg, &notes) })
                    .await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.finish_webdav_sync(result, window, cx);
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
        let this = cx.weak_entity();
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                cx.background_executor()
                    .timer(Duration::from_millis(2500))
                    .await;
                let _ = this.update_in(cx, |this, window, cx| {
                    if this.webdav_epoch == epoch {
                        this.start_webdav_sync(window, cx);
                    }
                });
            })
            .detach();
    }

    fn local_sync_notes(&self, _cx: &App) -> Vec<webdav::SyncNote> {
        self.notes
            .iter()
            .filter_map(|note| {
                let plum = self.sync.plum.get_note_text(&note.id).ok().flatten()?;
                let md = self
                    .sync
                    .note_markdown(&note.id, &plum)
                    .unwrap_or_else(|| markup::plum_to_markdown(&plum));
                let (title, _) = Self::split_title_body(&md);
                Some(webdav::SyncNote {
                    id: note.id.clone(),
                    title,
                    color: note.theme.clone().unwrap_or_else(|| "Yellow".into()),
                    tags: self
                        .tags_for_note(&note.id)
                        .iter()
                        .map(|t| t.name.clone())
                        .collect(),
                    pinned: self.is_pinned(&note.id),
                    updated_at: note.updated_at.unwrap_or(0),
                    markdown: md,
                })
            })
            .collect()
    }

    fn finish_webdav_sync(
        &mut self,
        result: Result<webdav::SyncReport, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.webdav_syncing = false;
        match result {
            Ok(report) => {
                if !report.remote_notes.is_empty() {
                    // Save local edits first; the remote note then wins only
                    // where its `updated_at` is newer (decided by the sync).
                    self.flush_pending_save(cx);
                    for note in &report.remote_notes {
                        if let Err(e) = self.sync.apply_remote_note(note) {
                            self.status = SyncStatus::Error(format!("写入远端便签失败：{e}"));
                            continue;
                        }
                        // Replace the local tag set and pin state with the remote.
                        if let Ok(existing) = self.tags.tags_of_note(&note.id) {
                            for tag_id in existing {
                                let _ = self.tags.untag_note(&note.id, tag_id);
                            }
                        }
                        for name in &note.tags {
                            if let Ok(tag_id) = self.tags.create_tag(name) {
                                let _ = self.tags.tag_note(&note.id, tag_id);
                            }
                        }
                        let _ = self.tags.set_pinned(&note.id, note.pinned);
                    }
                    self.reload_tags_cache();
                    self.prune_tag_orphans();
                    self.reload(window, cx);
                }
                let summary = report.summary();
                self.webdav_message = Some(summary.clone());
                self.status = SyncStatus::Saved(summary);
            }
            Err(e) => {
                self.webdav_message = Some(format!("同步失败：{e}"));
                self.status = SyncStatus::Error(format!("WebDAV 同步失败：{e}"));
            }
        }
        cx.notify();
    }

    fn poll_external_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The native app's process state is cheap to check but not free
        // (a tasklist spawn): refresh it every ~10s, not every poll.
        self.poll_ticks = self.poll_ticks.wrapping_add(1);
        if self.poll_ticks % 5 == 1 {
            self.native_app_running = sync::is_sticky_notes_running();
        }

        // Retry the exact text that failed, even if the selection has moved on
        // since — never the current editor's content, which belongs to another note.
        if let Some((id, md)) = self.pending_save.clone() {
            match self.sync.save_note(&id, &md) {
                Ok((plum_text, updated_at)) => {
                    self.pending_save = None;
                    if let Some(note) = self.notes.iter_mut().find(|n| n.id == id) {
                        note.updated_at = Some(updated_at);
                    }
                    if self.selected_id.as_deref() == Some(id.as_str()) {
                        self.last_saved_md = Some(md);
                        self.last_saved_plum = Some(plum_text);
                    }
                    self.refresh_db_sig();
                    self.status = SyncStatus::Saved("已自动保存".into());
                    debug_log("retry save ok");
                }
                Err(_) => debug_log("retry save err"),
            }
        }

        // Microsoft-account sync is opt-in: by default the companion app does
        // not follow edits made in the native app or its cloud.
        if !self.config.microsoft {
            return;
        }

        let sig = plum_signature(self.sync.plum.path());
        if self.db_sig == Some(sig) {
            return;
        }
        self.db_sig = Some(sig);
        debug_log(&format!("poll sig changed native={}", self.native_app_running));

        let Ok(notes) = self.sync.reload() else {
            return;
        };

        let selected = self.selected_id.clone();
        let old_updated = selected
            .as_deref()
            .and_then(|id| self.notes.iter().find(|n| n.id == id))
            .and_then(|n| n.updated_at);
        let new_updated = selected
            .as_deref()
            .and_then(|id| notes.iter().find(|n| n.id == id))
            .and_then(|n| n.updated_at);

        self.notes = notes;

        // The selected note was deleted in the Windows app.
        if selected
            .as_deref()
            .is_some_and(|id| !self.notes.iter().any(|n| n.id == id))
        {
            self.selected_id = None;
            self.editor = None;
            self.title_input = None;
            self.last_saved_md = None;
            self.last_saved_plum = None;
            self.ensure_selection_after_filter(window, cx);
            self.status = SyncStatus::Saved("检测到外部删除，已自动更新列表".into());
            cx.notify();
            return;
        }

        if old_updated == new_updated {
            // Only note metadata changed (e.g. a WAL checkpoint); the open
            // note is unaffected.
            cx.notify();
            return;
        }

        // Refresh the open note, but only when the editor has no unsaved
        // local edits — otherwise the debounced save owns the content.
        if let (Some(id), Some(editor)) = (self.selected_id.clone(), self.editor.clone()) {
            let local = self.compose_md(cx);
            if self.last_saved_md.as_deref() == Some(local.as_str()) {
                if let Ok(Some(plum_text)) = self.sync.plum.get_note_text(&id) {
                    // If the row is byte-for-byte what we last wrote, this is
                    // our own save seen back through the WAL: the Markdown after
                    // a plum round trip is merely canonicalised, and replacing
                    // the editor with it would look like the edit reverting.
                    // Only a genuinely different row is an external write.
                    let ours = self.last_saved_plum.as_deref() == Some(plum_text.as_str());
                    let md = self
                        .sync
                        .note_markdown(&id, &plum_text)
                        .unwrap_or_else(|| markup::plum_to_markdown(&plum_text));
                    if !ours && md != local {
                        let (title, body) = Self::split_title_body(&md);
                        let ok = editor.update(cx, |state, cx| state.set_source(&body, cx).is_ok());
                        if ok {
                            if let Some(input) = &self.title_input {
                                input.update(cx, |s, cx| s.set_value(title.clone(), window, cx));
                            }
                            // `set_value` emits Change; cancel the autosave it
                            // scheduled — this content came from disk, not the user.
                            self.save_epoch = self.save_epoch.wrapping_add(1);
                            self.last_saved_md = Some(self.compose_md(cx));
                            self.last_saved_plum = Some(plum_text);
                            self.update_stats_from_editor(cx);
                        }
                    }
                    debug_log(&format!("poll note ours={ours} md_eq={}", md == local));
                }
            }
        }
        self.status = SyncStatus::Saved("已自动同步 Windows 便签的修改".into());
        cx.notify();
    }
}
