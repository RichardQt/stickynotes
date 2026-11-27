#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod paste;
mod quick_note;
mod settings_ui;
mod state;
mod ui;
mod win;

use gpui_kit::*;
use stickynotes_core::config::{AppConfig, CloseBehavior, WindowPlacement};

/// The built-in start bounds, used on first run and whenever the saved
/// placement would be off-screen (for example after a monitor is unplugged).
fn default_window_bounds() -> WindowBounds {
    WindowBounds::Windowed(Bounds {
        origin: Point { x: px(120.), y: px(80.) },
        size: size(px(1100.), px(720.)),
    })
}

/// Reopen where the user left the window, as long as enough of it would still
/// be visible on one of the current displays. The minimum window size is
/// enforced here too, so a placement stored by an older version cannot open
/// a window too small for the three columns.
fn restored_window_bounds(saved: Option<WindowPlacement>, cx: &App) -> WindowBounds {
    let Some(saved) = saved else {
        return default_window_bounds();
    };
    let bounds = Bounds {
        origin: Point { x: px(saved.x), y: px(saved.y) },
        size: size(
            px(saved.width.clamp(1000., 4800.)),
            px(saved.height.clamp(640., 3200.)),
        ),
    };
    let visible = cx.displays().iter().any(|display| {
        let d = display.bounds();
        let w = d.right().as_f32().min(bounds.right().as_f32())
            - d.left().as_f32().max(bounds.left().as_f32());
        let h = d.bottom().as_f32().min(bounds.bottom().as_f32())
            - d.top().as_f32().max(bounds.top().as_f32());
        w >= 160. && h >= 96.
    });
    if visible {
        WindowBounds::Windowed(bounds)
    } else {
        default_window_bounds()
    }
}

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
            let config = AppConfig::load();
            let hidden = win::launched_at_login()
                && config.close_behavior == CloseBehavior::Tray;
            let window_bounds = restored_window_bounds(config.window_bounds, cx);

            let (main, _) = gpui_kit::open_window(
                WindowOptions {
                    show: !hidden,
                    focus: !hidden,
                    window_bounds: Some(window_bounds),
                    // Below this the three columns leave too little for the
                    // note paper: the editor toolbar starts hiding tools and
                    // table cells wrap one character per line. The window
                    // itself refuses to shrink past it.
                    window_min_size: Some(size(px(1000.), px(640.))),
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
