//! The quick note's view: a strip to drag the window by, the editor, and a
//! bar with the save state and the two ways out.
//!
//! The view only shows and reports. What happens to the text — saving it,
//! putting the window away — is decided in `state::quick`, which the view
//! reaches through [`request`].

use crate::state::{get_theme, AppState};
use crate::ui::{app_logo, ink, note_editor, primary_pill, INK_MUTED, INK_PRIMARY, INK_SECONDARY};
use crate::win;
use gpui_kit::assets::IconName as AssetIconName;
use gpui_kit::base::animation::cubic_bezier;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::label::Label;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use markdown_editor::{
    BlockEditorEvent, BlockEditorState, BlockEditorStyle, ClipboardPolicy, SubmitPolicy,
};
use std::time::Duration;

/// Size of the quick note, as a window and as a card.
pub(crate) const QUICK_W: Pixels = px(420.);
pub(crate) const QUICK_H: Pixels = px(260.);

/// What the quick note asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickAction {
    /// The text changed: save it once the typing pauses.
    Edited,
    /// Save now (the window lost the focus).
    Flush,
    /// Save and put the quick note away.
    Close,
    /// Save, put the quick note away and continue in the main panel.
    OpenMain,
}

/// What the bar at the bottom says about the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuickStatus {
    /// Nothing to save yet.
    Idle,
    /// Saved, at this time of day.
    Saved(String),
    Error(String),
}

pub struct QuickNote {
    app: WeakEntity<AppState>,
    /// The main window: the app's actions need it.
    main: AnyWindowHandle,
    /// Shown as a card in the main window instead of in a window of its own.
    embedded: bool,
    editor: Entity<BlockEditorState>,
    scroll: ScrollHandle,
    /// Nothing but white space has been typed.
    blank: bool,
    status: QuickStatus,
    /// Bumped whenever the quick-note surface opens so its arrival animation replays.
    open_seq: u64,
}

/// Where and how the quick-note window opens: small, always on top, without
/// a system frame, in the corner next to the notification area.
pub(crate) fn window_options(cx: &App) -> WindowOptions {
    // The display the pointer is on — it has just clicked the icon there.
    let display = win::cursor_monitor()
        .and_then(|id| cx.find_display(DisplayId::new(id)))
        .or_else(|| cx.primary_display());
    let margin = px(14.);
    let (origin, display_id) = match &display {
        Some(display) => {
            let area = display.visible_bounds();
            let origin = point(
                (area.right() - QUICK_W - margin).max(area.left()),
                (area.bottom() - QUICK_H - margin).max(area.top()),
            );
            (origin, Some(display.id()))
        }
        None => (point(px(160.), px(160.)), None),
    };
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin,
            size: size(QUICK_W, QUICK_H),
        })),
        titlebar: None,
        focus: true,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: true,
        is_resizable: false,
        is_minimizable: false,
        display_id,
        ..Default::default()
    }
}

/// Hand `action` to the app once the event being handled is done: the app
/// acts through its main window, which may be the very window that is busy
/// with this event.
pub(crate) fn request(
    app: WeakEntity<AppState>,
    main: AnyWindowHandle,
    action: QuickAction,
    cx: &mut App,
) {
    cx.defer(move |cx| {
        let _ = main.update(cx, |_, window, cx| {
            let _ = app.update(cx, |app, cx| app.quick_action(action, window, cx));
        });
    });
}

impl QuickNote {
    /// `window` is the window the view is shown in: its own, or the main
    /// window when `embedded`.
    pub fn new(
        app: WeakEntity<AppState>,
        main: AnyWindowHandle,
        embedded: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let scroll = ScrollHandle::new();
        let editor = cx.new(|cx| {
            // Enter finishes the note; Shift+Enter starts a new line. Pastes
            // come to `crate::paste`, which also takes pictures.
            let mut state = BlockEditorState::new(window, cx)
                .with_submit_policy(SubmitPolicy::Enter)
                .with_clipboard_policy(ClipboardPolicy::Host);
            state.set_scroll_handle(scroll.clone());
            state.set_editor_style(
                BlockEditorStyle::default()
                    .with_foreground(rgb(0x232B36).into())
                    .with_muted_foreground(rgb(0x6E7785).into())
                    .with_caret(rgb(0x3B6EA8).into()),
                cx,
            );
            state
        });
        cx.subscribe(&editor, |this, editor, event: &BlockEditorEvent, cx| match event {
            BlockEditorEvent::Changed { .. } => {
                this.blank = editor.read(cx).source().trim().is_empty();
                this.request(QuickAction::Edited, cx);
                cx.notify();
            }
            BlockEditorEvent::LinkClicked { url } => crate::state::open_note_link(url, cx),
            // Enter. While an input method is composing, the editor keeps
            // the key for the candidate and never gets here.
            BlockEditorEvent::SubmitRequested => this.request(QuickAction::Close, cx),
            BlockEditorEvent::PasteRequested { request, item } => {
                let view = cx.weak_entity();
                crate::paste::resolve(
                    &editor,
                    request,
                    item,
                    move |outcome, cx| {
                        // A picture that made it in is saved like typing;
                        // only one that did not needs saying.
                        if let crate::paste::Outcome::Failed(reason) = outcome {
                            let _ = view.update(cx, |this, cx| {
                                this.set_status(QuickStatus::Error(reason), cx)
                            });
                        }
                    },
                    cx,
                );
            }
            _ => {}
        })
        .detach();
        // Text being composed is not in the document yet, but the hint behind
        // it has to go the moment it appears.
        cx.observe(&editor, |_, _, cx| cx.notify()).detach();

        if !embedded {
            // Alt+F4 means "done": the window is put away, never destroyed —
            // it is the one the next click on the icon shows again.
            let (app, main) = (app.clone(), main);
            window.on_window_should_close(cx, move |_, cx| {
                request(app.clone(), main, QuickAction::Close, cx);
                false
            });
            // Switching to another window does not close the quick note (it
            // may be needed again to copy something over), but what has been
            // typed is saved right then.
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.request(QuickAction::Flush, cx);
                }
            })
            .detach();
        }

        Self {
            app,
            main,
            embedded,
            editor,
            scroll,
            blank: true,
            status: QuickStatus::Idle,
            open_seq: 1,
        }
    }

    fn request(&self, action: QuickAction, cx: &mut App) {
        request(self.app.clone(), self.main, action, cx);
    }

    /// The Markdown typed so far.
    pub fn text(&self, cx: &App) -> String {
        self.editor.read(cx).source()
    }

    pub fn editor(&self) -> &Entity<BlockEditorState> {
        &self.editor
    }

    /// Back to a blank note, for the next time the quick note is opened.
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            let _ = editor.set_source("", cx);
        });
        self.scroll.set_offset(point(px(0.), px(0.)));
        self.blank = true;
        self.status = QuickStatus::Idle;
        cx.notify();
    }

    /// Trigger a fresh entrance pass when the quick note reappears from tray.
    pub fn note_opened(&mut self, cx: &mut Context<Self>) {
        self.open_seq = self.open_seq.wrapping_add(1);
        cx.notify();
    }

    /// Put the caret into the text. `window` is the one showing the view.
    pub fn focus_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.editor.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }

    pub fn set_status(&mut self, status: QuickStatus, cx: &mut Context<Self>) {
        if self.status != status {
            self.status = status;
            cx.notify();
        }
    }
}

/// Crisp secondary capsule button for the pure-white quick-note surface.
fn paper_pill(id: &'static str, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(28.))
        .px(px(12.))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(9999.))
        .bg(rgb(0xF3F5F8))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(0xEAECEF)))
        .active(|s| s.bg(rgb(0xDFE3EA)))
        .child(
            Label::new(label)
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(INK_PRIMARY)),
        )
}

impl Render for QuickNote {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Quick notes start on crisp pure-white paper to match default notes.
        let paper = get_theme("White").paper_bg;
        let reduce_motion = cx.reduce_motion();
        let composing = self.editor.read(cx).is_composing();
        let show_hint = self.blank && !composing;

        let header = h_flex()
            .id("quick-header")
            .w_full()
            .h(px(38.))
            .flex_none()
            .items_center()
            .border_b_1()
            .border_color(hsla(220. / 360., 0.20, 0.16, 0.05))
            // The window is moved by this strip. It must not reach under the
            // close button, or the button could never be clicked.
            .child(
                h_flex()
                    .id("quick-drag")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .pl(px(14.))
                    .gap(px(7.))
                    .items_center()
                    .when(!self.embedded, |el| {
                        el.window_control_area(WindowControlArea::Drag)
                    })
                    .child(app_logo(px(16.)))
                    .child(
                        Label::new("快速记事")
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_PRIMARY)),
                    ),
            )
            .child(
                div().pr(px(8.)).flex_none().child(
                    div()
                        .id("quick-close")
                        .size(px(26.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .cursor_pointer()
                        .hover(|s| s.bg(ink(0.055)))
                        .active(|s| s.bg(ink(0.10)))
                        .child(
                            Icon::new(AssetIconName::X)
                                .size(px(13.))
                                .text_color(rgb(INK_SECONDARY)),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.request(QuickAction::Close, cx)
                        })),
                ),
            );

        let body_inner = div()
            .relative()
            .w_full()
            .px(px(8.))
            .pt(px(2.))
            .pb(px(4.))
            .child(note_editor(&self.editor, 6, cx))
            // Painted over the empty editor, where its first line
            // will be; it takes no clicks.
            .when(show_hint, |el| {
                el.child(
                    v_flex()
                        .absolute()
                        .top(px(10.))
                        .left(px(18.))
                        .right(px(18.))
                        .gap(px(4.))
                        .child(
                            div()
                                .line_height(px(24.))
                                .text_color(rgb(0x9BA3B0))
                                .child("写点什么…"),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .line_height(px(16.))
                                .text_color(rgb(0xB0B7C3))
                                .child(
                                    "Enter 完成 · Shift+Enter 换行 · Ctrl+Enter 打开主面板 · Esc 收起",
                                ),
                        ),
                )
            });

        let animated_body: AnyElement = if reduce_motion || self.embedded {
            body_inner.into_any_element()
        } else {
            body_inner
                .with_animation(
                    ("quick-open", self.open_seq as usize),
                    Animation::new(Duration::from_millis(180))
                        .with_easing(cubic_bezier(0.16, 1.0, 0.3, 1.0)),
                    |el, d| el.opacity(0.35 + 0.65 * d).top(px((1. - d) * 5.)),
                )
                .into_any_element()
        };

        let body = div()
            .id("quick-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .text_color(rgb(0x1E2530))
            // A click below the last line still lands in the note.
            .on_click(cx.listener(|this, _, window, cx| this.focus_editor(window, cx)))
            .child(animated_body);

        let (status_icon, status_text, status_color): (Option<AssetIconName>, String, Hsla) =
            match &self.status {
                QuickStatus::Idle => (None, "输入后自动保存".to_string(), rgb(INK_MUTED).into()),
                QuickStatus::Saved(at) => (
                    Some(AssetIconName::Check),
                    format!("已自动保存 · {at}"),
                    rgb(0x2A8555).into(),
                ),
                QuickStatus::Error(message) => (
                    Some(AssetIconName::CircleAlert),
                    message.clone(),
                    rgb(0xC04646).into(),
                ),
            };
        let footer = h_flex()
            .id("quick-footer")
            .w_full()
            .h(px(44.))
            .flex_none()
            .pl(px(16.))
            .pr(px(10.))
            .gap(px(7.))
            .items_center()
            .border_t_1()
            .border_color(hsla(220. / 360., 0.20, 0.16, 0.045))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(4.))
                    .items_center()
                    .when_some(status_icon, |el, icon| {
                        el.child(
                            Icon::new(icon)
                                .size(px(11.))
                                .flex_none()
                                .text_color(status_color),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(status_color)
                            .child(status_text),
                    ),
            )
            .child(paper_pill("quick-open-main", "打开主面板").on_click(cx.listener(
                |this, _, _, cx| this.request(QuickAction::OpenMain, cx),
            )))
            .child(
                primary_pill("quick-done", "完成")
                    .h(px(28.))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.request(QuickAction::Close, cx)
                    })),
            );

        v_flex()
            .id("quick-note")
            .size_full()
            .bg(rgb(paper))
            .when(!self.embedded, |el| {
                el.border_1()
                    .border_color(hsla(220. / 360., 0.20, 0.16, 0.085))
            })
            .when(self.embedded, |el| el.rounded(px(18.)))
            .child(header)
            .child(body)
            .child(footer)
    }
}
