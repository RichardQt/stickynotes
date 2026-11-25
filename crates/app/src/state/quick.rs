//! The quick note: a small always-on-top window, opened from the
//! notification-area icon, for writing something down without the main
//! panel. This is its session logic; the view is `crate::quick_note`.
//!
//! Nothing is ever asked: whatever is typed is saved by itself, as an
//! ordinary note. A session that stays empty leaves no note behind.

use super::{AppState, QuickHost, QuickSession, SyncStatus};
use crate::quick_note::{self, QuickAction, QuickNote, QuickStatus};
use crate::win;
use gpui_kit::*;
use std::time::Duration;
use stickynotes_core::store::{self, StoreError};
use stickynotes_core::ticks;

/// What writing the quick note down did to the database.
pub(super) enum Written {
    Unchanged,
    Changed,
    Failed,
}

impl AppState {
    /// Left click on the notification-area icon: show the quick note, ready
    /// to type.
    pub fn open_quick_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Nothing can be saved without a database: show what is wrong.
        if self.startup_error.is_some() {
            self.show_main(window, cx);
            return;
        }
        // The window is kept between uses. Should it be gone after all,
        // start over.
        let live = cx.windows();
        if self
            .quick
            .as_ref()
            .and_then(|host| host.window)
            .is_some_and(|(handle, _)| !live.contains(&handle))
        {
            self.quick = None;
            self.quick_session = QuickSession::default();
        }

        let Some(host) = &self.quick else {
            self.create_quick_note(window, cx);
            return;
        };
        let view = host.view.clone();
        match host.window {
            Some((handle, hwnd)) => Self::present_quick_window(handle, hwnd, view, window, cx),
            None => {
                self.show_quick_overlay(window, cx);
                view.update(cx, |view, cx| {
                    view.note_opened(cx);
                    view.focus_editor(window, cx);
                });
            }
        }
    }

    /// Put the quick-note window on screen, in front, with the caret in its
    /// text. `window` is the main window.
    fn present_quick_window(
        handle: AnyWindowHandle,
        hwnd: isize,
        view: Entity<QuickNote>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                let was_hidden = !win::is_visible(hwnd);
                // A quick note already on screen stays where it was put.
                if was_hidden {
                    win::place_bottom_right(hwnd);
                }
                win::show_and_activate(hwnd);
                let _ = handle.update(cx, |_, window, cx| {
                    view.update(cx, |view, cx| {
                        if was_hidden {
                            view.note_opened(cx);
                        }
                        view.focus_editor(window, cx);
                    });
                });
            })
            .detach();
    }

    /// Show the quick note as a card in the main window. It is the only
    /// dialog then: the settings would otherwise sit behind it and still get
    /// its key presses.
    fn show_quick_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.link_popover_open {
            self.link_popover_open = false;
            self.link_popover = None;
        }
        if self.show_settings {
            self.close_settings(window, cx);
        }
        self.quick_overlay_open = true;
        self.show_main(window, cx);
        cx.notify();
    }

    /// First use: open the quick-note window, or — when it cannot have a
    /// window of its own — show it as a card in the main window.
    fn create_quick_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let app = cx.weak_entity();
        let main = self.window_handle;
        // `STICKYNOTES_QUICK_EMBED` forces the fallback, to try it out.
        let own_window =
            self.main_hwnd.is_some() && std::env::var_os("STICKYNOTES_QUICK_EMBED").is_none();
        let opened = if own_window {
            let options = quick_note::window_options(cx);
            let app = app.clone();
            gpui_kit::open_window(options, cx, move |window, cx| {
                let view = cx.new(|cx| QuickNote::new(app, main, false, window, cx));
                view.update(cx, |view, cx| view.focus_editor(window, cx));
                view
            })
            .ok()
        } else {
            None
        };

        if let Some((handle, view)) = opened {
            let hwnd = handle
                .update(cx, |_, window, _| win::hwnd_of(window))
                .ok()
                .flatten();
            if let Some(hwnd) = hwnd {
                win::round_corners(hwnd);
                self.quick = Some(QuickHost { view: view.clone(), window: Some((handle, hwnd)) });
                // A new window is shown, but not necessarily in front of the
                // application the user was just working in.
                Self::present_quick_window(handle, hwnd, view, window, cx);
                return;
            }
            // A window that cannot be hidden and shown again is no use.
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }

        let view = cx.new(|cx| QuickNote::new(app, main, true, window, cx));
        self.quick = Some(QuickHost { view: view.clone(), window: None });
        self.show_quick_overlay(window, cx);
        view.update(cx, |view, cx| view.focus_editor(window, cx));
    }

    /// Run a quick-note action once the current event is done. The actions
    /// need the main window, and that may be the one handling the event.
    pub(crate) fn quick_request(&self, action: QuickAction, cx: &mut Context<Self>) {
        quick_note::request(cx.weak_entity(), self.window_handle, action, cx);
    }

    /// `window` is the main window.
    pub(crate) fn quick_action(
        &mut self,
        action: QuickAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            QuickAction::Edited => self.quick_schedule_save(window, cx),
            QuickAction::Flush => {
                self.quick_flush(window, cx);
            }
            QuickAction::Close => self.quick_finish(window, cx),
            QuickAction::OpenMain => self.quick_open_main(window, cx),
        }
    }

    /// Save 400ms after the last key press, like the main editor does.
    fn quick_schedule_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.quick_session.epoch = self.quick_session.epoch.wrapping_add(1);
        let epoch = self.quick_session.epoch;
        let this = cx.weak_entity();
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                cx.background_executor()
                    .timer(Duration::from_millis(400))
                    .await;
                let _ = this.update_in(cx, |this, window, cx| {
                    if this.quick_session.epoch == epoch {
                        this.quick_flush(window, cx);
                    }
                });
            })
            .detach();
    }

    /// Save now instead of after the pause, and schedule the upload.
    /// `false` when the text could not be stored: the quick note then has to
    /// stay open, since its window holds the only copy.
    pub(super) fn quick_flush(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        // This save replaces the one waiting for the pause.
        self.quick_session.epoch = self.quick_session.epoch.wrapping_add(1);
        match self.quick_write(cx) {
            Written::Unchanged => true,
            Written::Changed => {
                self.schedule_webdav_sync(window, cx);
                true
            }
            Written::Failed => false,
        }
    }

    /// Bring the database in line with the quick note's text: the note
    /// exists exactly while there is something in it. Needs no window, so it
    /// also runs while the app quits.
    pub(super) fn quick_write(&mut self, cx: &mut Context<Self>) -> Written {
        let Some(view) = self.quick.as_ref().map(|host| host.view.clone()) else {
            return Written::Unchanged;
        };
        let text = view.read(cx).text(cx);
        let blank = text.trim().is_empty();
        self.quick_session.retry = false;

        let result = match (self.quick_session.note_id.clone(), blank) {
            (None, true) => Ok(false),
            (None, false) => self.quick_create(&text),
            // Everything was deleted again: no empty note is kept.
            (Some(id), true) => self.quick_remove(&id),
            (Some(_), false) if text == self.quick_session.saved => Ok(false),
            (Some(id), false) => match self.store.save_markdown(&id, &text) {
                Ok(_) => Ok(true),
                // The note was deleted meanwhile (on another device) while
                // its text is still being written: keep the text.
                Err(StoreError::NoteNotFound(_)) => {
                    self.search_index_remove(&id);
                    self.quick_create(&text)
                }
                Err(e) => Err(e),
            },
        };

        match result {
            Ok(changed) => {
                if !changed {
                    self.quick_session.saved = if blank { String::new() } else { text };
                    return Written::Unchanged;
                }
                // The note the quick note just wrote is searchable straight
                // away, without waiting for a full index rebuild.
                if let Some(id) = self.quick_session.note_id.clone() {
                    self.search_index_put(&id, &text);
                }
                self.quick_session.saved = if blank { String::new() } else { text };
                // The main panel lists the note right away.
                if let Ok(notes) = self.store.list_notes() {
                    self.notes = notes;
                }
                let status = match self.quick_session.note_id {
                    Some(_) => {
                        QuickStatus::Saved(ticks::format_friendly_ticks(ticks::now_ticks()))
                    }
                    None => QuickStatus::Idle,
                };
                view.update(cx, |view, cx| view.set_status(status, cx));
                cx.notify();
                Written::Changed
            }
            Err(e) => {
                // A locked database is retried by the background loop.
                self.quick_session.retry = e.is_busy();
                let message = if e.is_busy() {
                    "数据库被占用，正在自动重试保存…".to_string()
                } else {
                    format!("保存失败：{e}")
                };
                view.update(cx, |view, cx| view.set_status(QuickStatus::Error(message), cx));
                Written::Failed
            }
        }
    }

    fn quick_create(&mut self, text: &str) -> store::Result<bool> {
        let id = self.store.create_note(text, "White")?;
        self.quick_session.note_id = Some(id);
        Ok(true)
    }

    fn quick_remove(&mut self, id: &str) -> store::Result<bool> {
        self.store.soft_delete(id)?;
        self.search_index_remove(id);
        // The note may already be on the server: remove it there as well.
        if self.config.webdav_configured() {
            self.webdav_deletions
                .record(&self.config.webdav, id, ticks::now_ticks());
            let _ = self.webdav_deletions.save();
        }
        self.quick_session.note_id = None;
        self.reload_tags_cache();
        Ok(true)
    }

    /// The background loop's turn to retry a write that hit a locked
    /// database.
    pub(super) fn retry_quick_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quick_session.retry {
            self.quick_flush(window, cx);
        }
    }

    /// The note the quick note is writing, if it has created one.
    pub(super) fn quick_note_id(&self) -> Option<&str> {
        self.quick_session.note_id.as_deref()
    }

    /// Put the quick note away and end its session. Its text has to be saved
    /// by now — or is meant to be dropped, as when its note was deleted in
    /// the main panel.
    pub(super) fn quick_close(&mut self, cx: &mut Context<Self>) {
        self.quick_session = QuickSession {
            // Also invalidates a save still waiting for the pause.
            epoch: self.quick_session.epoch.wrapping_add(1),
            ..QuickSession::default()
        };
        self.quick_overlay_open = false;
        if let Some(host) = &self.quick {
            if let Some((_, hwnd)) = host.window {
                win::hide(hwnd);
            }
            // The next time it opens, it is a blank note again.
            host.view.update(cx, |view, cx| view.reset(cx));
        }
        cx.notify();
    }

    /// 完成 / Enter / Esc / the close button.
    fn quick_finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.quick_flush(window, cx) {
            // The reason is on show in the quick note, which stays open.
            return;
        }
        let wrote = self.quick_session.note_id.is_some();
        self.quick_close(cx);
        // An empty main panel shows the note that was just written.
        if wrote && self.selected_id.is_none() {
            self.select_note_by_index(0, window, cx);
        }
    }

    /// 打开主面板: continue with this note in the main panel.
    pub(super) fn quick_open_main(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.quick_flush(window, cx) {
            return;
        }
        let note = self.quick_session.note_id.clone();
        self.quick_close(cx);
        if self.show_settings {
            self.close_settings(window, cx);
        }
        self.show_main(window, cx);
        if let Some(id) = note {
            // The note must not be hidden by a tag filter or a search.
            if !self.filtered_notes().iter().any(|n| n.id == id) {
                if self.tag_filter.is_some() {
                    self.clear_tag_filter(window, cx);
                }
                if !self.search_query.is_empty() {
                    self.clear_search(window, cx);
                }
            }
            self.select_note(&id, window, cx);
            if let Some(editor) = &self.editor {
                let handle = editor.read(cx).focus_handle(cx);
                window.focus(&handle, cx);
            }
        }
        cx.notify();
    }

    /// The main panel is about to open the note the quick note is writing.
    /// Two editors on one note would overwrite each other's text, so the
    /// quick note is saved and put away first. `false` when it could not be
    /// saved; the note is then left to it.
    pub(super) fn quick_release_note(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.quick_flush(window, cx) {
            self.status = SyncStatus::Error("快速记事还没有保存成功，请稍后再试".into());
            cx.notify();
            return false;
        }
        self.quick_close(cx);
        true
    }
}
