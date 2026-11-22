//! What surrounds the notes: the notification-area icon and its menu, the
//! system-wide quick-note key, starting with Windows, what closing the main
//! window does, and quitting.

use super::AppState;
use crate::win::{self, TrayEvent};
use futures::StreamExt as _;
use gpui_kit::*;
use stickynotes_core::config::CloseBehavior;
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
            if let Some(tray) = &this.tray {
                tray.shutdown();
            }
            async {}
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

    /// Hand the quick-note key to Windows, which then reports it from any
    /// program. While a shortcut is being recorded there is no such key:
    /// every key press has to reach the settings page instead.
    pub(super) fn apply_global_hotkey(&mut self) {
        let Some(tray) = &self.tray else {
            return;
        };
        if self.recording_shortcut.is_some() {
            let _ = tray.set_hotkey(None);
        } else {
            let combo = self.keymap.combo(shortcuts::QUICK_NOTE);
            self.hotkey_problem = tray.set_hotkey(combo).err();
        }
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
