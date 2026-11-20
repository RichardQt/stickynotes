//! What surrounds the notes: the notification-area icon, what closing the
//! main window does, and quitting.

use super::AppState;
use crate::win::{self, TrayEvent};
use futures::StreamExt as _;
use gpui_kit::*;
use stickynotes_core::config::CloseBehavior;

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
        match self.main_hwnd {
            // Showing a window makes the system call back into it at once,
            // which must not happen in the middle of an update: do it from a
            // task of its own.
            Some(hwnd) => window
                .spawn(cx, async move |_: &mut AsyncWindowContext| win::show_and_activate(hwnd))
                .detach(),
            None => window.activate_window(),
        }
        // Freshen the notes the moment the window is back, not a whole pull
        // interval later.
        self.pull_on_return(window, cx);
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
