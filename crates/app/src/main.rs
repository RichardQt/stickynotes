mod state;
mod ui;

use gpui_kit::*;

fn main() {
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

            gpui_kit::open_window(
                WindowOptions {
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
                |window, cx| cx.new(|cx| state::AppState::new(window, cx)),
            )
            .expect("failed to open window");
        });
}
