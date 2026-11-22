#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod quick_note;
mod settings_ui;
mod state;
mod ui;
mod win;

use gpui_kit::*;
use stickynotes_core::config::{AppConfig, CloseBehavior};

fn main() {
    // Desktop panics otherwise vanish with the window; keep a one-line trace
    // in `%LOCALAPPDATA%\stickynotes\panic.log` (never note content).
    std::panic::set_hook(Box::new(|info| {
        let mut text = String::from("PANIC ");
        if let Some(loc) = info.location() {
            text.push_str(&format!("{}:{} ", loc.file(), loc.line()));
        }
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string payload>".into()
        };
        text.push_str(&msg);
        eprintln!("{text}");
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let dir = std::path::Path::new(&local).join("stickynotes");
            let _ = std::fs::create_dir_all(&dir);
            use std::io::Write as _;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("panic.log"))
            {
                let _ = writeln!(f, "{text}");
            }
        }
    }));

    // One instance per profile. A second start only brings the running one
    // to the front: two would write the same database and sync it twice, and
    // with "stay in the notification area" the first is easy to forget.
    if !win::claim_single_instance() {
        return;
    }

    // Register the bundled Lucide icon set. Without an AssetSource every
    // `IconName` (window controls, search, pin, tag icons…) resolves to a
    // missing path and renders as nothing. `AllAssets` embeds the complete
    // catalog — the default `Assets` bundle only carries the ~100 icons the
    // component library itself draws and lacks e.g. `pin.svg`.
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(|cx| {
            gpui_kit::init(cx);
            markdown_editor::init(cx);
            // The app does not end with its last visible window: the main
            // window may only be hidden in the notification area, and the
            // quick note is a window of its own that stays around.
            cx.set_quit_mode(QuitMode::Explicit);

            // Started by Windows at sign-in while closing the window means
            // "stay in the notification area": come up there, without a
            // window in the way.
            let hidden = win::launched_at_login()
                && AppConfig::load().close_behavior == CloseBehavior::Tray;

            let (main, _) = gpui_kit::open_window(
                WindowOptions {
                    show: !hidden,
                    focus: !hidden,
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Point { x: px(120.), y: px(80.) },
                        size: size(px(1100.), px(720.)),
                    })),
                    titlebar: Some(TitlebarOptions {
                        title: Some("便签 Sticky Notes".into()),
                        // Immersive light titlebar: the app draws its own bar and
                        // the OS hides the native dark caption.
                        appears_transparent: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                cx,
                move |window, cx| cx.new(|cx| state::AppState::new(hidden, window, cx)),
            )
            .expect("failed to open window");

            // A main window that really closed is the end of the app, even
            // while the (hidden) quick-note window still exists.
            let main_id = main.window_id();
            cx.on_window_closed(move |cx, id| {
                if id == main_id {
                    cx.quit();
                }
            })
            .detach();
        });
}
