//! What surrounds the notes: the notification-area icon and its menu, the
//! system-wide quick-note key, starting with Windows, what closing the main
//! window does, and quitting.

use super::AppState;
use crate::win::{self, TrayEvent};
use futures::StreamExt as _;
use gpui_kit::*;
use std::collections::HashMap;
use std::time::Duration;
use stickynotes_core::config::{self, CloseBehavior, WindowPlacement};
use stickynotes_core::shortcuts;

impl AppState {
    /// Hook the app state up to the window and the system: keyboard
    /// shortcuts, the close button and the notification-area icon.
    pub(super) fn start_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        cx.intercept_keystrokes(move |event, window, cx| {
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| this.on_keystroke(event, window, cx));
            }
        })
        .detach();

        // Coming back to the window is the moment to look for other devices'
        // changes — silently, and no more than once per half minute.
        cx.observe_window_activation(window, move |this, window, cx| {
            if window.is_window_active() {
                this.pull_on_return(window, cx);
            }
        })
        .detach();

        cx.observe_window_bounds(window, |this, window, cx| {
            this.remember_window_bounds(window, cx);
        })
        .detach();

        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            this.update(cx, |this, cx| this.on_close_requested(window, cx))
                .unwrap_or(true)
        });

        if let Some((tray, events)) = win::Tray::start() {
            if self.config.close_behavior == CloseBehavior::Tray {
                tray.show_icon();
            }
            self.tray = Some(tray);
            self.apply_global_hotkey();
            let this = cx.weak_entity();
            window
                .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                    let mut events = events;
                    while let Some(event) = events.next().await {
                        let handled = this.update_in(cx, |this, window, cx| {
                            this.on_tray_event(event, window, cx)
                        });
                        if handled.is_err() {
                            break;
                        }
                    }
                })
                .detach();
        }

        // Started at sign-in, the window stays out of the way — as long as
        // there is an icon to bring it back and notes for it to show.
        let in_tray = self.tray_available()
            && self.config.close_behavior == CloseBehavior::Tray
            && self.startup_error.is_none();
        if self.awaiting_first_show && !in_tray {
            self.reveal_main(window, cx);
        }

        // Quitting while the window is merely hidden: nothing else writes
        // the last edits down or takes the icon away.
        cx.on_app_quit(|this, cx| {
            this.flush_pending_save(cx);
            this.quick_write(cx);
            // A move or resize in the last moment may still be waiting for
            // its debounce.
            let _ = this.config.save();
            if let Some(tray) = &this.tray {
                tray.shutdown();
            }
            async {}
        })
        .detach();
    }

    /// Remember where the window is. Dragging emits a stream of bounds
    /// changes, so the config file is written once, shortly after the window
    /// comes to rest; the in-memory value updates on every change.
    fn remember_window_bounds(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let bounds = window.window_bounds().get_bounds();
        let placement = WindowPlacement {
            x: bounds.origin.x.as_f32(),
            y: bounds.origin.y.as_f32(),
            width: bounds.size.width.as_f32(),
            height: bounds.size.height.as_f32(),
        };
        if self.config.window_bounds == Some(placement) {
            return;
        }
        self.config.window_bounds = Some(placement);
        self.window_bounds_epoch = self.window_bounds_epoch.wrapping_add(1);
        let epoch = self.window_bounds_epoch;
        let this = cx.weak_entity();
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                cx.background_executor()
                    .timer(Duration::from_millis(700))
                    .await;
                let _ = this.update_in(cx, |this, _window, _cx| {
                    if this.window_bounds_epoch == epoch {
                        // Nothing to show on failure: the next start simply
                        // opens at the previous place.
                        let _ = this.config.save();
                    }
                });
            })
            .detach();
    }

    /// Whether closing the main window can leave the app running: there is
    /// an icon to come back from and a window that can be hidden.
    pub fn tray_available(&self) -> bool {
        self.tray.is_some() && self.main_hwnd.is_some()
    }

    fn on_tray_event(&mut self, event: TrayEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            TrayEvent::QuickNote => self.open_quick_note(window, cx),
            TrayEvent::ShowMain => self.show_main(window, cx),
            TrayEvent::OpenMain => self.open_main_panel(window, cx),
            TrayEvent::OpenSettings => {
                self.show_main(window, cx);
                // Not over another dialog, and not a second time: opening
                // the settings starts them afresh.
                let free = self.startup_error.is_none()
                    && !self.show_settings
                    && !self.quick_overlay_open;
                if free {
                    self.open_settings(window, cx);
                }
            }
            TrayEvent::Quit => self.quit_app(window, cx),
        }
    }

    /// The system-wide "open main panel" key: a quick note being written is
    /// saved and continued in the main panel; otherwise the panel is simply
    /// brought to the front.
    pub(super) fn open_main_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quick.is_some() {
            self.quick_open_main(window, cx);
        } else {
            self.show_main(window, cx);
        }
    }

    /// Hand the system-wide keys to Windows, which then report them from any
    /// program. While a shortcut is being recorded there are none: every key
    /// press has to reach the settings page instead.
    pub(super) fn apply_global_hotkey(&mut self) {
        let mut problems = HashMap::new();
        if let Some(tray) = &self.tray {
            let recording = self.recording_shortcut.is_some();
            for (key, action) in [
                (win::GlobalKey::QuickNote, shortcuts::QUICK_NOTE),
                (win::GlobalKey::MainPanel, shortcuts::OPEN_MAIN),
            ] {
                let combo = if recording {
                    None
                } else {
                    self.keymap.combo(action)
                };
                if let Err(problem) = tray.set_hotkey(key, combo) {
                    problems.insert(action, problem);
                }
            }
        }
        self.hotkey_problems = problems;
    }

    /// The main window was asked to close (title-bar button, Alt+F4, the
    /// taskbar). Returns whether it may: with "stay in the notification
    /// area" it is hidden instead and the app keeps running.
    fn on_close_requested(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        // Whatever happens next, nothing typed so far may be lost.
        if self.show_settings {
            let _ = self.persist_settings(cx);
        }
        self.flush_pending_save(cx);
        self.quick_flush(window, cx);

        let stay = self.config.close_behavior == CloseBehavior::Tray
            // The recovery screen has nothing worth keeping in the background.
            && self.startup_error.is_none();
        match (stay, &self.tray, self.main_hwnd) {
            (true, Some(tray), Some(hwnd)) => {
                win::hide(hwnd);
                if !self.webdav_failed {
                    if let Some(last) = self.webdav_last_pass {
                        self.webdav_next_try =
                            self.webdav_next_try.max(last + config::PULL_INTERVAL_TRAY);
                    }
                }
                if !self.config.tray_hint_shown {
                    // Once: say where the app went and how to get it back.
                    tray.show_hint();
                    self.config.tray_hint_shown = true;
                    let _ = self.config.save();
                }
                false
            }
            _ => true,
        }
    }

    /// Bring the main window back from the notification area (or from
    /// behind other windows).
    pub fn show_main(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reveal_main(window, cx);
        if !self.webdav_failed {
            if let Some(last) = self.webdav_last_pass {
                self.webdav_next_try = self
                    .webdav_next_try
                    .min(last + config::PULL_INTERVAL_VISIBLE);
            }
        }
        // Freshen the notes the moment the window is back, not a whole pull
        // interval later.
        self.pull_on_return(window, cx);
    }

    /// Put the main window on screen, in front.
    fn reveal_main(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.awaiting_first_show) {
            // A window created hidden gets its size and position from the
            // toolkit only when that first activates it.
            window.activate_window();
        }
        match self.main_hwnd {
            // Showing a window makes the system call back into it at once,
            // which must not happen in the middle of an update: do it from a
            // task of its own.
            Some(hwnd) => window
                .spawn(cx, async move |_: &mut AsyncWindowContext| win::show_and_activate(hwnd))
                .detach(),
            None => window.activate_window(),
        }
    }

    /// The "开机自启动" switch of the settings.
    pub fn set_autostart(&mut self, on: bool, cx: &mut Context<Self>) {
        self.autostart_error = win::set_autostart(on).err().map(|e| {
            format!("无法{}开机自启动：{e}", if on { "开启" } else { "关闭" })
        });
        // Whatever was asked for, the switch shows what the system holds.
        self.autostart = win::autostart_enabled();
        cx.notify();
    }

    /// "退出应用" in the settings: the only way out while closing the
    /// window merely hides it.
    pub fn quit_app(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.show_settings {
            let _ = self.persist_settings(cx);
        }
        self.flush_pending_save(cx);
        self.quick_flush(window, cx);
        cx.quit();
    }

    pub fn set_close_behavior(&mut self, behavior: CloseBehavior, cx: &mut Context<Self>) {
        self.settings_menu = None;
        if self.config.close_behavior != behavior {
            self.config.close_behavior = behavior;
            let _ = self.config.save();
            // The icon is only there while closing leads to it.
            if let Some(tray) = &self.tray {
                match behavior {
                    CloseBehavior::Tray => tray.show_icon(),
                    CloseBehavior::Quit => tray.hide_icon(),
                }
            }
        }
        cx.notify();
    }
}
