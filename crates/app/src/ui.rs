use crate::state::{AppState, MdTip, SyncStatus, ThemeSpec, WebDavTest, THEMES};
use gpui_kit::assets::IconName as AssetIconName;
use gpui_kit::base::{h_flex, v_flex, ElementExt as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use markdown_editor::component::BlockEditor;
use markdown_editor::{
    BlockEditorModel, BlockEditorState, BlockKind, EditorCommand, MarkKind, MarkState,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;
use stickynotes_core::store::model::NoteSummary;
use stickynotes_core::ticks;

const SIDEBAR_W: Pixels = px(206.);
const LIST_PANE_W: Pixels = px(266.);
const TITLEBAR_H: Pixels = px(38.);

/// Note title type size and the line box that holds it. The line box must
/// clear the font's full ascent + descent (about 1.33em for the system UI
/// and CJK fallback fonts), or the input clips the glyphs.
const TITLE_TEXT_SIZE: Pixels = px(22.);
const TITLE_LINE_H: Pixels = px(32.);
/// Horizontal inset the markdown editor applies inside its own viewport.
/// The title field and its rule share it so all three left edges line up.
const EDITOR_TEXT_INSET: Rems = Rems(0.625);

// =========================================================================
// Unified Shell & Tahoe Solid-Glass Tokens
// =========================================================================

/// Outer window chrome (titlebar + sidebar), framing the inset workspace sheet.
const SHELL_BG: u32 = 0xF2F4F7;
/// Secondary surface inside the workspace sheet for the note-card list pane.
const LIST_BG: u32 = 0xF7F8FA;
/// Primary ink for chrome, cards, and dialogs.
const INK_PRIMARY: u32 = 0x1F2937;
/// Secondary ink for navigation labels and snippets.
const INK_SECONDARY: u32 = 0x525E6F;
/// Muted ink for counts, timestamps, and field labels.
const INK_MUTED: u32 = 0x768293;
/// Primary button slate fill and states.
const PRIMARY_BTN: u32 = 0x243042;
const PRIMARY_BTN_HOVER: u32 = 0x324158;
const PRIMARY_BTN_ACTIVE: u32 = 0x1A2330;

/// Translucent foreground hover fill on the neutral shell (Tahoe `fills.hover`).
fn shell_hover() -> Hsla {
    hsla(218. / 360., 0.24, 0.18, 0.055)
}

/// Translucent foreground press/active fill on the neutral shell (Tahoe `fills.active`).
fn shell_active() -> Hsla {
    hsla(218. / 360., 0.26, 0.18, 0.095)
}

/// Translucent structural divider on the neutral shell (Tahoe `fills.line`).
fn shell_hairline() -> Hsla {
    hsla(218. / 360., 0.20, 0.16, 0.065)
}

/// Ink overlay for the note's paper: translucent black, so one alpha ramp
/// works on every paper colour and always lets it show through instead of
/// painting an opaque grey block. Assumes a light paper, which every theme is.
fn ink(alpha: f32) -> Hsla {
    hsla(0., 0., 0., alpha)
}

/// Frosted capsule fill for the colour-swatch track sitting on note paper.
fn swatch_track_bg() -> Hsla {
    hsla(0., 0., 1., 0.78)
}

/// Inner specular highlight rim for Tahoe solid-glass surfaces.
fn specular_rim(is_dark: bool, alpha_light: f32) -> BoxShadow {
    BoxShadow {
        inset: true,
        color: hsla(0., 0., 1., if is_dark { 0.13 } else { alpha_light }),
        offset: point(px(0.), px(0.5)),
        blur_radius: px(0.),
        spread_radius: px(0.5),
    }
}

/// Fine 0.5px outer edge replacing CSS-style 1px borders on raised controls.
fn hairline_edge(is_dark: bool, alpha_light: f32) -> BoxShadow {
    BoxShadow {
        inset: false,
        color: hsla(0., 0., 0., if is_dark { 0.42 } else { alpha_light }),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(0.5),
    }
}

/// Subtle frame around the inset workspace sheet (`note-list` + `editor-pane`).
fn workspace_sheet_shadow() -> Vec<BoxShadow> {
    vec![
        hairline_edge(false, 0.07),
        BoxShadow {
            inset: false,
            color: hsla(220. / 360., 0.25, 0.12, 0.05),
            offset: point(px(0.), px(2.)),
            blur_radius: px(8.),
            spread_radius: px(-1.),
        },
    ]
}

/// Lifted white pill for the active sidebar navigation item.
fn nav_pill_shadow() -> Vec<BoxShadow> {
    vec![
        specular_rim(false, 0.85),
        hairline_edge(false, 0.06),
        BoxShadow {
            inset: false,
            color: hsla(220. / 360., 0.22, 0.14, 0.055),
            offset: point(px(0.), px(1.)),
            blur_radius: px(2.5),
            spread_radius: px(0.),
        },
    ]
}

/// Capsule / input edge for Tahoe geometry (inner top rim, fine edge, contact shadow).
/// Never pair with `.border_1()` — this edge replaces the border cleanly.
fn capsule_control_edge() -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            inset: true,
            color: hsla(0., 0., 1., 0.55),
            offset: point(px(0.), px(1.)),
            blur_radius: px(0.),
            spread_radius: px(0.),
        },
        BoxShadow {
            inset: false,
            color: hsla(0., 0., 0., 0.075),
            offset: point(px(0.), px(0.)),
            blur_radius: px(0.),
            spread_radius: px(0.5),
        },
        BoxShadow {
            inset: false,
            color: hsla(0., 0., 0., 0.055),
            offset: point(px(0.), px(1.)),
            blur_radius: px(2.),
            spread_radius: px(0.),
        },
    ]
}

/// Unselected note card at rest: crisp 0.5px edge and whisper-light lift.
fn card_rest_shadow() -> Vec<BoxShadow> {
    vec![
        specular_rim(false, 0.80),
        hairline_edge(false, 0.055),
        BoxShadow {
            inset: false,
            color: hsla(220. / 360., 0.20, 0.14, 0.035),
            offset: point(px(0.), px(1.)),
            blur_radius: px(2.5),
            spread_radius: px(0.),
        },
    ]
}

/// Selected note card: pairs with `note_theme.border` (so no duplicate outer hairline).
fn card_selected_shadow() -> Vec<BoxShadow> {
    vec![
        specular_rim(false, 0.65),
        BoxShadow {
            inset: false,
            color: hsla(0., 0., 0., 0.075),
            offset: point(px(0.), px(4.)),
            blur_radius: px(12.),
            spread_radius: px(-2.),
        },
    ]
}

/// Floating modal / popover surface (`gpui-liquid-glass` `SurfaceExt` solid form).
fn modal_surface_shadow() -> Vec<BoxShadow> {
    vec![
        specular_rim(false, 0.88),
        hairline_edge(false, 0.10),
        BoxShadow {
            inset: false,
            color: hsla(220. / 360., 0.25, 0.08, 0.20),
            offset: point(px(0.), px(14.)),
            blur_radius: px(34.),
            spread_radius: px(-4.),
        },
    ]
}

/// Soft drop shadow that lifts the selected swatch off the palette track.
/// Selection is expressed by size + elevation only — no ring, no inner dot,
/// no heavy dark circle.
fn lifted_swatch_shadow() -> BoxShadow {
    BoxShadow {
        inset: false,
        color: hsla(0., 0., 0., 0.24),
        offset: point(px(0.), px(1.5)),
        blur_radius: px(3.),
        spread_radius: px(0.),
    }
}

// =========================================================================
// Main App Render
// =========================================================================

impl Render for AppState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current_theme = self.current_theme();
        let settings_modal = self
            .show_settings
            .then(|| self.render_settings_modal(window, cx));

        v_flex()
            .id("app-root")
            .size_full()
            .overflow_hidden()
            .bg(rgb(SHELL_BG))
            .child(self.render_titlebar(window, cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(window, cx))
                    // Inset rounded workspace sheet: frames the list + paper
                    // cleanly inside the shell so no paper colour collides
                    // with the neutral titlebar or window buttons.
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .pr(px(8.))
                            .pb(px(8.))
                            .child(
                                h_flex()
                                    .id("workspace-sheet")
                                    .size_full()
                                    .rounded(px(12.))
                                    .overflow_hidden()
                                    .bg(rgb(LIST_BG))
                                    .shadow(workspace_sheet_shadow())
                                    .child(self.render_note_list(window, cx))
                                    .child(self.render_editor_area(current_theme, window, cx)),
                            ),
                    ),
            )
            // Hover hint for the markdown bar, painted last so it floats over
            // the title below the toolbar.
            .when_some(self.md_tip, |el, tip| el.child(md_tip_overlay(tip)))
            .when_some(settings_modal, |el, modal| el.child(modal))
    }
}

/// A single window control button drawn on the shell titlebar.
/// The OS handles the actual click via `window_control_area` hit testing;
/// Close adopts Windows-standard red hover/press feedback.
fn titlebar_control(
    id: &'static str,
    area: WindowControlArea,
    icon: IconName,
    is_close: bool,
) -> impl IntoElement {
    div()
        .id(id)
        .group(id)
        .w(px(44.))
        .h(TITLEBAR_H)
        .flex()
        .items_center()
        .justify_center()
        .window_control_area(area)
        .map(|el| {
            if is_close {
                el.hover(|s| s.bg(rgb(0xE81123)))
                    .active(|s| s.bg(rgb(0xC50F1F)))
            } else {
                el.hover(|s| s.bg(shell_hover()))
                    .active(|s| s.bg(shell_active()))
            }
        })
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(INK_SECONDARY))
                .when(is_close, |el| {
                    el.group_hover(id, |s| s.text_color(rgb(0xFFFFFF)))
                })
                .child(Icon::new(icon).size(px(13.5))),
        )
}

/// Whether the caret sits in a list, and which kind: (bullet, ordered, task).
fn list_state(model: &BlockEditorModel) -> (bool, bool, bool) {
    let Some(caret) = model.caret() else {
        return (false, false, false);
    };
    let doc = model.document();
    let mut parent = doc.block(caret.block).and_then(|b| b.parent());
    let mut task = false;
    while let Some(block) = parent {
        match block.kind() {
            BlockKind::ListItem { checked } => task = checked.is_some(),
            BlockKind::List { ordered } => return (!ordered && !task, ordered, task),
            _ => {}
        }
        parent = block.parent();
    }
    (false, false, false)
}

/// One icon button of the markdown toolbar: a 24×24 transparent square that
/// only ever paints translucent ink, so it blends into any paper colour.
/// Its painted bounds are recorded so the hover hint can sit under it.
fn md_tb_icon(
    id: &'static str,
    icon: AssetIconName,
    active: bool,
    enabled: bool,
    bounds: Rc<RefCell<HashMap<&'static str, Bounds<Pixels>>>>,
    on_hover: impl Fn(&bool, &mut Window, &mut App) + 'static,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let fg = if !enabled {
        ink(0.24)
    } else if active {
        ink(0.92)
    } else {
        ink(0.58)
    };
    let fg_hover = if active {
        ink(0.94)
    } else {
        ink(0.86)
    };
    let bg_hover = if active {
        ink(0.11)
    } else {
        ink(0.06)
    };
    let bg_press = ink(0.15);
    let bg_active = ink(0.10);

    div()
        .id(id)
        .group(id)
        .size(px(24.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .text_color(fg)
        .when(active, |el| el.bg(bg_active))
        .when(enabled, |el| {
            el.cursor_pointer()
                .hover(move |s| s.bg(bg_hover))
                .active(move |s| s.bg(bg_press))
        })
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .when(enabled, |el| {
                    el.group_hover(id, move |s| s.text_color(fg_hover))
                })
                .child(Icon::new(icon).size(px(14.5))),
        )
        .on_prepaint(move |b, _, _| {
            bounds.borrow_mut().insert(id, b);
        })
        .on_hover(on_hover)
        .on_click(move |ev, window, cx| {
            if enabled {
                on_click(ev, window, cx);
            }
        })
}

/// Tahoe solid-surface hint shown under a markdown toolbar button.
fn md_tip_overlay(tip: MdTip) -> impl IntoElement {
    h_flex()
        .absolute()
        .left(tip.x)
        .top(tip.y + px(6.))
        .items_center()
        .gap(px(6.))
        .px(px(9.))
        .py(px(4.5))
        .rounded(px(8.))
        .bg(rgb(0x1E232B))
        .shadow(vec![
            specular_rim(true, 0.14),
            hairline_edge(true, 0.45),
            BoxShadow {
                inset: false,
                color: hsla(0., 0., 0., 0.22),
                offset: point(px(0.), px(6.)),
                blur_radius: px(14.),
                spread_radius: px(-2.),
            },
        ])
        .child(
            Label::new(tip.label)
                .text_size(px(11.5))
                .text_color(rgb(0xF8FAFC)),
        )
        .when(!tip.keys.is_empty(), |el| {
            el.child(
                div()
                    .px(px(4.5))
                    .py(px(0.5))
                    .rounded(px(4.))
                    .bg(hsla(0., 0., 1., 0.11))
                    .child(
                        Label::new(tip.keys)
                            .text_size(px(10.5))
                            .text_color(hsla(0., 0., 1., 0.72)),
                    ),
            )
        })
}

impl AppState {
    // =========================================================================
    // 0. Immersive shell titlebar (merged seamlessly with sidebar & frame)
    // =========================================================================
    fn render_titlebar(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .id("app-titlebar")
            .w_full()
            .h(TITLEBAR_H)
            .flex_none()
            .items_center()
            .bg(rgb(SHELL_BG))
            // The drag area must NOT cover the window buttons. Its hitbox is
            // painted before its children and the platform picks the first
            // window-control hitbox under the cursor, so a full-width Drag
            // area would swallow every click on min/max/close.
            .child(
                h_flex()
                    .id("titlebar-drag-region")
                    .flex_1()
                    .h_full()
                    .pl(px(14.))
                    .items_center()
                    .gap_2()
                    .window_control_area(WindowControlArea::Drag)
                    .child(
                        div()
                            .size(px(12.))
                            .rounded(px(3.5))
                            .bg(rgb(0xF0B443))
                            .shadow(capsule_control_edge()),
                    )
                    .child(
                        Label::new("便签")
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_PRIMARY)),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .flex_none()
                    .child(titlebar_control(
                        "tb-min",
                        WindowControlArea::Min,
                        IconName::WindowMinimize,
                        false,
                    ))
                    .child(titlebar_control(
                        "tb-max",
                        WindowControlArea::Max,
                        IconName::WindowMaximize,
                        false,
                    ))
                    .child(titlebar_control(
                        "tb-close",
                        WindowControlArea::Close,
                        IconName::WindowClose,
                        true,
                    )),
            )
    }

    // =========================================================================
    // 1. Left Navigation Sidebar
    // =========================================================================
    fn render_sidebar(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let all_count = self.notes.len() as u32;

        // --- Tahoe Capsule "New Note" Button (optically aligned with 46px workspace header) ---
        let new_button = div()
            .px_3()
            .pt(px(7.))
            .pb_2()
            .child(
                h_flex()
                    .id("btn-new-note")
                    .w_full()
                    .h(px(32.))
                    .px_3p5()
                    .items_center()
                    .justify_center()
                    .gap_1p5()
                    .rounded(px(9999.))
                    .bg(rgb(PRIMARY_BTN))
                    .hover(|el| el.bg(rgb(PRIMARY_BTN_HOVER)))
                    .active(|el| el.bg(rgb(PRIMARY_BTN_ACTIVE)))
                    .shadow(capsule_control_edge())
                    .cursor_pointer()
                    .child(
                        Icon::new(IconName::Plus)
                            .size(px(13.5))
                            .text_color(rgb(0xFFFFFF)),
                    )
                    .child(
                        Label::new("新建便签")
                            .text_size(px(12.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(0xFFFFFF)),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.new_note(window, cx);
                    })),
            );

        // --- Navigation Items (All Notes + Tags) ---
        let mut nav_rows = v_flex().px_2p5().py_0p5().gap(px(2.));

        // "All Notes" item
        let is_all_selected = self.tag_filter.is_none();
        nav_rows = nav_rows.child(
            h_flex()
                .id("nav-all")
                .h(px(32.))
                .px_2p5()
                .rounded(px(8.))
                .items_center()
                .justify_between()
                .cursor_pointer()
                .when(is_all_selected, |el| {
                    el.bg(rgb(0xFFFFFF)).shadow(nav_pill_shadow())
                })
                .when(!is_all_selected, |el| {
                    el.hover(|s| s.bg(shell_hover()))
                        .active(|s| s.bg(shell_active()))
                })
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Icon::new(IconName::Inbox)
                                .size(px(14.))
                                .text_color(if is_all_selected {
                                    rgb(0x3B6EA8)
                                } else {
                                    rgb(INK_MUTED)
                                }),
                        )
                        .child(
                            Label::new("全部便签")
                                .text_size(px(12.5))
                                .font_weight(if is_all_selected {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(if is_all_selected {
                                    rgb(INK_PRIMARY)
                                } else {
                                    rgb(INK_SECONDARY)
                                }),
                        ),
                )
                .child(
                    div()
                        .px_2()
                        .py(px(1.))
                        .rounded_full()
                        .bg(if is_all_selected {
                            hsla(214. / 360., 0.45, 0.45, 0.11)
                        } else {
                            shell_hover()
                        })
                        .child(
                            Label::new(format!("{all_count}"))
                                .text_size(px(10.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(if is_all_selected {
                                    rgb(0x3B6EA8)
                                } else {
                                    rgb(INK_MUTED)
                                }),
                        ),
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.tag_filter = None;
                    this.ensure_selection_after_filter(window, cx);
                    cx.notify();
                })),
        );

        // "Tags" Section Header with Dedicated "+" Button
        let tags_header = h_flex()
            .pl_2p5()
            .pr_1p5()
            .pt_3p5()
            .pb_1()
            .items_center()
            .justify_between()
            .child(
                Label::new("标签分类")
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(INK_MUTED)),
            )
            .child(
                div()
                    .id("btn-toggle-sidebar-add-tag")
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .cursor_pointer()
                    .hover(|el| el.bg(shell_hover()))
                    .active(|el| el.bg(shell_active()))
                    .child(
                        Icon::new(IconName::Plus)
                            .size(px(12.))
                            .text_color(rgb(INK_SECONDARY)),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_sidebar_add_tag(window, cx);
                    })),
            );
        nav_rows = nav_rows.child(tags_header);

        // Inline Add Tag Form (toggled by "+")
        if self.show_sidebar_add_tag {
            if let Some(inp) = &self.new_tag_input {
                let inline_add_row = h_flex()
                    .my_1()
                    .pl_2p5()
                    .pr_1()
                    .h(px(31.))
                    .gap_1()
                    .items_center()
                    .rounded(px(8.5))
                    .bg(rgb(0xFFFFFF))
                    .shadow(capsule_control_edge())
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(inp)
                                .xsmall()
                                .appearance(false)
                                .bordered(false)
                                .focus_bordered(false),
                        ),
                    )
                    .child(
                        div()
                            .id("btn-confirm-add-tag")
                            .size(px(22.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(hsla(214. / 360., 0.45, 0.45, 0.12))
                            .hover(|el| el.bg(hsla(214. / 360., 0.45, 0.45, 0.20)))
                            .active(|el| el.bg(hsla(214. / 360., 0.45, 0.45, 0.28)))
                            .cursor_pointer()
                            .child(
                                Icon::new(IconName::Check)
                                    .size(px(11.))
                                    .text_color(rgb(0x3B6EA8)),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_tag(window, cx);
                            })),
                    )
                    .child(
                        div()
                            .id("btn-cancel-add-tag")
                            .size(px(22.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .hover(|el| el.bg(shell_hover()))
                            .active(|el| el.bg(shell_active()))
                            .cursor_pointer()
                            .child(
                                Icon::new(IconName::Close)
                                    .size(px(10.))
                                    .text_color(rgb(INK_MUTED)),
                            )
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.show_sidebar_add_tag = false;
                                cx.notify();
                            })),
                    );
                nav_rows = nav_rows.child(inline_add_row);
            }
        }

        // User Tags list (Zero SQLite queries - in-memory cache)
        if self.tags_list.is_empty() && !self.show_sidebar_add_tag {
            nav_rows = nav_rows.child(
                div().px_2p5().py_2().child(
                    Label::new("暂无标签，点击 ＋ 创建")
                        .text_size(px(11.))
                        .text_color(rgb(INK_MUTED)),
                ),
            );
        }

        for t in self.tags_list.iter() {
            let tid = t.id;
            let count = self.tag_counts.get(&tid).copied().unwrap_or(0);
            let is_selected = self.tag_filter == Some(tid);
            // Unique group per row so the delete button only fades in for the
            // row the pointer is actually over.
            let row_group = SharedString::from(format!("tag-row-{tid}"));

            nav_rows = nav_rows.child(
                h_flex()
                    .id(ElementId::Name(format!("nav-tag-{tid}").into()))
                    .group(row_group.clone())
                    .h(px(31.))
                    .px_2p5()
                    .rounded(px(8.))
                    .items_center()
                    .justify_between()
                    .cursor_pointer()
                    .when(is_selected, |el| {
                        el.bg(rgb(0xFFFFFF)).shadow(nav_pill_shadow())
                    })
                    .when(!is_selected, |el| {
                        el.hover(|s| s.bg(shell_hover()))
                            .active(|s| s.bg(shell_active()))
                    })
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .min_w_0()
                            .child(
                                div()
                                    .w(px(14.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        Icon::new(AssetIconName::Hash)
                                            .size(px(12.5))
                                            .text_color(if is_selected {
                                                rgb(0x3B6EA8)
                                            } else {
                                                rgb(INK_MUTED)
                                            }),
                                    ),
                            )
                            .child(
                                Label::new(t.name.clone())
                                    .text_size(px(12.5))
                                    .font_weight(if is_selected {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .truncate()
                                    .text_color(if is_selected {
                                        rgb(INK_PRIMARY)
                                    } else {
                                        rgb(INK_SECONDARY)
                                    }),
                            ),
                    )
                    .child(
                        // Count badge aligns flush right with "全部便签"; hovering
                        // swaps it in-place for the delete button with zero layout shift.
                        div()
                            .relative()
                            .h(px(20.))
                            .min_w(px(22.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_end()
                            .child(
                                div()
                                    .px_2()
                                    .py(px(1.))
                                    .rounded_full()
                                    .bg(if is_selected {
                                        hsla(214. / 360., 0.45, 0.45, 0.11)
                                    } else {
                                        shell_hover()
                                    })
                                    .group_hover(row_group.clone(), |el| el.opacity(0.))
                                    .child(
                                        Label::new(format!("{count}"))
                                            .text_size(px(10.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(if is_selected {
                                                rgb(0x3B6EA8)
                                            } else {
                                                rgb(INK_MUTED)
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .id(ElementId::Name(format!("del-tag-{tid}").into()))
                                    .absolute()
                                    .right_0()
                                    .top_0()
                                    .size(px(20.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .cursor_pointer()
                                    .opacity(0.)
                                    .group_hover(row_group, |el| el.opacity(1.))
                                    .hover(|el| el.bg(hsla(352. / 360., 0.65, 0.55, 0.14)))
                                    .active(|el| el.bg(hsla(352. / 360., 0.65, 0.55, 0.24)))
                                    .child(
                                        Icon::new(IconName::Close)
                                            .size(px(9.5))
                                            .text_color(rgb(0xB45B67)),
                                    )
                                    .on_click(cx.listener(move |this, _, _window, cx| {
                                        this.delete_tag(tid, cx);
                                    })),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.tag_filter = Some(tid);
                        this.ensure_selection_after_filter(window, cx);
                        cx.notify();
                    })),
            );
        }

        let webdav_configured = self.config.webdav_configured();
        let webdav_on = self.config.webdav_auto && webdav_configured;
        let (dot, webdav_label) = if self.webdav_syncing {
            (0x4F86C6, "WebDAV 正在同步…")
        } else if webdav_on {
            (0x52A876, "WebDAV 自动同步")
        } else if webdav_configured {
            (0xD49B3E, "WebDAV 已配置")
        } else {
            (0xAAB3C0, "WebDAV 未配置")
        };

        let footer_status = div()
            .px_2p5()
            .pb(px(8.))
            .pt_1()
            .child(
                h_flex()
                    .id("btn-settings")
                    .h(px(32.))
                    .px_2p5()
                    .rounded(px(8.))
                    .items_center()
                    .gap_2()
                    .justify_between()
                    .cursor_pointer()
                    .hover(|el| el.bg(shell_hover()))
                    .active(|el| el.bg(shell_active()))
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .min_w_0()
                            .child(
                                div()
                                    .size(px(6.5))
                                    .flex_none()
                                    .rounded_full()
                                    .bg(rgb(dot)),
                            )
                            .child(
                                Label::new(webdav_label)
                                    .text_size(px(11.))
                                    .text_color(rgb(INK_SECONDARY)),
                            ),
                    )
                    .child(
                        Icon::new(AssetIconName::Settings)
                            .size(px(13.5))
                            .flex_none()
                            .text_color(rgb(INK_MUTED)),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_settings(window, cx);
                    })),
            );

        v_flex()
            .id("sidebar")
            .w(SIDEBAR_W)
            .h_full()
            .flex_none()
            .bg(rgb(SHELL_BG))
            .child(new_button)
            .child(
                v_flex()
                    .id("sidebar-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(nav_rows),
            )
            .child(footer_status)
    }

    // =========================================================================
    // 2. Middle Note Cards List Pane
    // =========================================================================
    fn render_note_list(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let notes = self.filtered_notes();

        // Tahoe capsule search bar aligned to the same 46px header band as the editor bar
        let search_box = match &self.search_input {
            Some(inp) => h_flex()
                .h(px(46.))
                .px_3()
                .flex_none()
                .items_center()
                .child(
                    h_flex()
                        .w_full()
                        .h(px(32.))
                        .pl_3()
                        .pr_1p5()
                        .gap_1p5()
                        .items_center()
                        .rounded(px(9999.))
                        .bg(rgb(0xFFFFFF))
                        .shadow(capsule_control_edge())
                        .child(
                            Icon::new(IconName::Search)
                                .size(px(13.5))
                                .flex_none()
                                .text_color(rgb(INK_MUTED)),
                        )
                        .child(
                            div().flex_1().min_w_0().child(
                                Input::new(inp)
                                    .appearance(false)
                                    .bordered(false)
                                    .focus_bordered(false),
                            ),
                        )
                        .when(!self.search_query.is_empty(), |el| {
                            el.child(
                                div()
                                    .id("btn-clear-search")
                                    .size(px(18.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .hover(|s| s.bg(shell_hover()))
                                    .active(|s| s.bg(shell_active()))
                                    .cursor_pointer()
                                    .child(
                                        Icon::new(IconName::Close)
                                            .size(px(10.))
                                            .text_color(rgb(INK_MUTED)),
                                    )
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.clear_search(window, cx);
                                    })),
                            )
                        }),
                )
                .into_any_element(),
            None => div().into_any_element(),
        };

        // Note Count & Filter Banner
        let active_tag_name = self.tag_filter.and_then(|tid| {
            self.tags_list
                .iter()
                .find(|t| t.id == tid)
                .map(|t| t.name.clone())
        });

        let subheader = h_flex()
            .px_3p5()
            .py_1()
            .flex_none()
            .items_center()
            .justify_between()
            .child(
                h_flex()
                    .items_center()
                    .gap_1p5()
                    .when_some(active_tag_name, |el, name| {
                        el.child(
                            h_flex()
                                .id("btn-clear-tag-filter")
                                .pl_2()
                                .pr_1p5()
                                .py(px(1.))
                                .rounded_full()
                                .bg(hsla(214. / 360., 0.45, 0.45, 0.12))
                                .hover(|s| s.bg(hsla(214. / 360., 0.45, 0.45, 0.18)))
                                .active(|s| s.bg(hsla(214. / 360., 0.45, 0.45, 0.25)))
                                .items_center()
                                .gap_1()
                                .cursor_pointer()
                                .child(
                                    Label::new(format!("#{name}"))
                                        .text_size(px(10.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgb(0x3B6EA8)),
                                )
                                .child(
                                    Icon::new(IconName::Close)
                                        .size(px(9.))
                                        .text_color(rgb(0x3B6EA8)),
                                )
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.clear_tag_filter(window, cx);
                                })),
                        )
                    })
                    .child(
                        Label::new(if self.tag_filter.is_some() || !self.search_query.is_empty() {
                            format!("筛选出 {} 条", notes.len())
                        } else {
                            format!("共 {} 条便签", notes.len())
                        })
                        .text_size(px(11.))
                        .text_color(rgb(INK_MUTED)),
                    ),
            );

        // Cards list
        let mut card_list = v_flex().px_2p5().pt_1().pb_3().gap(px(6.));
        if notes.is_empty() {
            card_list = card_list.child(
                v_flex()
                    .py_12()
                    .items_center()
                    .justify_center()
                    .gap_2p5()
                    .child(
                        div()
                            .size(px(38.))
                            .rounded(px(10.))
                            .bg(shell_hover())
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap(px(3.))
                            .child(
                                div()
                                    .w(px(18.))
                                    .h(px(2.))
                                    .rounded_full()
                                    .bg(hsla(218. / 360., 0.20, 0.35, 0.35)),
                            )
                            .child(
                                div()
                                    .w(px(18.))
                                    .h(px(2.))
                                    .rounded_full()
                                    .bg(hsla(218. / 360., 0.20, 0.35, 0.25)),
                            )
                            .child(
                                div()
                                    .w(px(11.))
                                    .h(px(2.))
                                    .rounded_full()
                                    .bg(hsla(218. / 360., 0.20, 0.35, 0.18)),
                            ),
                    )
                    .child(
                        Label::new("暂无匹配便签")
                            .text_size(px(12.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(INK_SECONDARY)),
                    )
                    .child(
                        Label::new("点击左侧“新建便签”添加第一条笔记")
                            .text_size(px(11.))
                            .text_color(rgb(INK_MUTED)),
                    ),
            );
        } else {
            for (i, note) in notes.iter().enumerate() {
                card_list = card_list.child(self.render_note_card(i, note, cx));
            }
        }

        v_flex()
            .id("note-list-pane")
            .w(LIST_PANE_W)
            .h_full()
            .flex_none()
            .bg(rgb(LIST_BG))
            .border_r_1()
            .border_color(shell_hairline())
            .child(search_box)
            .child(subheader)
            .child(
                v_flex()
                    .id("cards-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(card_list),
            )
    }

    fn render_note_card(
        &self,
        idx: usize,
        note: &NoteSummary,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_selected = self.selected_id.as_deref() == Some(note.id.as_str());
        let is_pinned = self.is_pinned(&note.id);
        let updated = note
            .updated_at
            .map(ticks::format_friendly_ticks)
            .unwrap_or_default();
        let note_theme = crate::state::get_theme(note.theme.as_deref());

        let mut card = h_flex()
            .id(ElementId::Name(format!("note-card-{idx}").into()))
            .w_full()
            .pl_3()
            .pr(px(13.))
            .py(px(8.5))
            .gap_2()
            .rounded(px(10.))
            .cursor_pointer();

        if is_selected {
            card = card
                .bg(rgb(note_theme.card_active_bg))
                .border_1()
                .border_color(rgb(note_theme.border))
                .shadow(card_selected_shadow());
        } else {
            card = card
                .bg(rgb(0xFFFFFF))
                .shadow(card_rest_shadow())
                .hover(|el| el.bg(rgb(0xFCFDFF)))
                .active(|el| el.bg(rgb(0xF3F5F9)));
        }

        // Card body content
        let mut content = v_flex().flex_1().min_w_0().gap(px(3.));

        // Title row: theme color dot + Title + Pin + Relative Timestamp
        let title_text = if note.title.trim().is_empty() {
            "(无标题便签)"
        } else {
            &note.title
        };

        let title_row = h_flex()
            .items_center()
            .gap_1p5()
            .child(
                div()
                    .size(px(7.5))
                    .flex_none()
                    .rounded_full()
                    .bg(rgb(note_theme.accent)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.5))
                    .font_weight(if is_selected {
                        FontWeight::BOLD
                    } else {
                        FontWeight::SEMIBOLD
                    })
                    .text_color(rgb(INK_PRIMARY))
                    .child(title_text.to_string()),
            )
            .when(is_pinned, |el| {
                el.child(
                    Icon::new(AssetIconName::Pin)
                        .size(px(10.5))
                        .flex_none()
                        .text_color(rgb(0x5B7A9D)),
                )
            })
            .child(
                Label::new(updated)
                    .text_size(px(10.5))
                    .flex_none()
                    .text_color(rgb(INK_MUTED)),
            );
        content = content.child(title_row);

        // Snippet (two brief lines, plain text — markdown already stripped at store level)
        if !note.snippet.is_empty() {
            content = content.child(
                div()
                    .text_size(px(11.5))
                    .line_height(px(16.5))
                    .line_clamp(2)
                    .text_ellipsis()
                    .text_color(rgb(0x647082))
                    .child(note.snippet.clone()),
            );
        }

        // Right-click menu data, captured once per render. The menu operates
        // on this card's note even when it is not the selected one.
        let note_id = note.id.clone();
        let menu_note_id = note.id.clone();
        let attached: Vec<i64> = self
            .tags_for_note(&note.id)
            .iter()
            .map(|t| t.id)
            .collect();
        let all_tags: Vec<(i64, String)> = self
            .tags_list
            .iter()
            .map(|t| (t.id, t.name.clone()))
            .collect();
        let weak = cx.weak_entity();
        let window_handle = self.window_handle;

        card.child(content)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_note(&note_id, window, cx);
            }))
            .context_menu(move |mut menu, window, cx| {
                // 1. Pin / unpin — the most frequent action goes first.
                let weak_pin = weak.clone();
                let id_pin = menu_note_id.clone();
                menu = menu.item(
                    PopupMenuItem::new(if is_pinned { "取消置顶" } else { "置顶" })
                        .icon(Icon::new(if is_pinned {
                            AssetIconName::PinOff
                        } else {
                            AssetIconName::Pin
                        }))
                        .on_click(move |_, _window, cx| {
                            let _ = weak_pin.update(cx, |this, cx| {
                                this.toggle_pin(&id_pin, cx);
                            });
                        }),
                );

                // 2. Tags > submenu: ✓ marks attached tags; one click toggles
                //    the tag without opening the note.
                menu = menu.separator();
                let weak_tags = weak.clone();
                let id_tags = menu_note_id.clone();
                let attached = attached.clone();
                let all_tags = all_tags.clone();
                menu = menu.submenu(
                    "添加 / 管理标签",
                    window,
                    cx,
                    move |mut sub, _window, _cx| {
                        if all_tags.is_empty() {
                            sub = sub.item(
                                PopupMenuItem::new("暂无标签，先在左侧新建").disabled(true),
                            );
                        }
                        for (tid, name) in all_tags.iter() {
                            let tid = *tid;
                            let weak = weak_tags.clone();
                            let id = id_tags.clone();
                            sub = sub.item(
                                PopupMenuItem::new(format!("#{name}"))
                                    .checked(attached.contains(&tid))
                                    .on_click(move |_, _window, cx| {
                                        let _ = weak.update(cx, |this, cx| {
                                            this.toggle_tag_for(&id, tid, cx);
                                        });
                                    }),
                            );
                        }
                        sub = sub.separator();
                        let weak = weak_tags.clone();
                        let id = id_tags.clone();
                        let handle = window_handle;
                        sub.item(
                            PopupMenuItem::new("新建标签…")
                                .icon(Icon::new(IconName::Plus))
                                .on_click(move |_, _window, cx| {
                                    let weak = weak.clone();
                                    let id = id.clone();
                                    cx.defer(move |cx| {
                                        let _ = handle.update(cx, |_, window, cx| {
                                            let _ = weak.update(cx, |this, cx| {
                                                this.select_note(&id, window, cx);
                                                this.toggle_sidebar_add_tag(window, cx);
                                            });
                                        });
                                    });
                                }),
                        )
                    },
                );

                // 3. Delete this note (not necessarily the selected one).
                let weak_del = weak.clone();
                let id_del = menu_note_id.clone();
                let handle_del = window_handle;
                menu.separator().item(
                    PopupMenuItem::new("删除便签")
                        .icon(Icon::new(AssetIconName::Trash))
                        .on_click(move |_, _window, cx| {
                            let weak = weak_del.clone();
                            let id = id_del.clone();
                            cx.defer(move |cx| {
                                let _ = handle_del.update(cx, |_, window, cx| {
                                    let _ = weak.update(cx, |this, cx| {
                                        this.delete_note_by_id(&id, window, cx);
                                    });
                                });
                            });
                        }),
                )
            })
    }

    // =========================================================================
    // 3. Right Editor Paper Canvas Area
    // =========================================================================

    /// Unified single-row paper header: Markdown formatting strip on the left
    /// and the frosted Tahoe colour-swatch capsule on the right, separated from
    /// the note canvas by one delicate ink hairline.
    fn render_editor_header(
        &self,
        editor: &Entity<BlockEditorState>,
        theme: &'static ThemeSpec,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let md_strip = self.render_md_toolbar(editor, cx);
        let palette = self.render_palette_capsule(theme, cx);

        h_flex()
            .id("canvas-toolbar")
            .w_full()
            .h(px(46.))
            .flex_none()
            .px_5()
            .gap_3()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(ink(0.06))
            .bg(rgb(theme.paper_bg))
            .child(md_strip)
            .child(palette)
    }

    /// Flat, single-row markdown bar: every high-frequency format is a 24×24
    /// icon button with a hover hint. No dropdowns and no fixed grey — the
    /// paper colour shows through every state.
    fn render_md_toolbar(
        &self,
        editor: &Entity<BlockEditorState>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (kind, in_text, in_code_block, bold, italic, strike, code, highlight, in_quote) = {
            let e = editor.read(cx);
            let m = e.model();
            let kind = e.block_kind_at_caret();
            (
                kind,
                kind.is_some(),
                kind == Some(BlockKind::CodeBlock),
                m.mark_state(MarkKind::Bold) == MarkState::On,
                m.mark_state(MarkKind::Italic) == MarkState::On,
                m.mark_state(MarkKind::Strikethrough) == MarkState::On,
                m.mark_state(MarkKind::Code) == MarkState::On,
                m.mark_state(MarkKind::Highlight) == MarkState::On,
                m.in_blockquote(),
            )
        };
        let (in_bullet, in_ordered, in_task) = {
            let e = editor.read(cx);
            list_state(e.model())
        };

        let heading =
            |level: u8| matches!(kind, Some(BlockKind::Heading { level: l }) if l == level);
        let heading_enabled = in_text && !in_code_block;
        let marks_enabled = in_text && !in_code_block;

        let sep_bg = ink(0.08);
        let app = cx.entity();
        let bounds = self.md_tip_bounds.clone();

        let hover_for = |id: &'static str, label: &'static str, keys: &'static str| {
            let app = app.clone();
            move |hovered: &bool, _window: &mut Window, cx: &mut App| {
                if !*hovered {
                    app.update(cx, |this, cx| {
                        if this.md_tip_hover == Some(id) {
                            this.md_tip_hover = None;
                        }
                        if this.md_tip.is_some_and(|t| t.id == id) {
                            this.md_tip = None;
                            cx.notify();
                        }
                    });
                    return;
                }
                app.update(cx, |this, cx| {
                    this.md_tip_hover = Some(id);
                    cx.spawn(async move |weak, cx| {
                        cx.background_executor()
                            .timer(Duration::from_millis(350))
                            .await;
                        let _ = weak.update(cx, |this, cx| {
                            if this.md_tip_hover != Some(id) {
                                return;
                            }
                            if let Some(b) = this.md_tip_bounds.borrow().get(id).copied() {
                                this.md_tip = Some(MdTip {
                                    id,
                                    label,
                                    keys,
                                    x: b.left(),
                                    y: b.bottom(),
                                });
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                });
            }
        };

        let run = |cmd: EditorCommand| {
            let ed = editor.clone();
            let app = app.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                app.update(cx, |this, cx| {
                    this.md_tip = None;
                    cx.notify();
                });
                ed.update(cx, |s, cx| {
                    let _ = s.run(cmd.clone(), cx);
                    s.focus(window, cx);
                });
            }
        };

        let btn = |id: &'static str,
                   icon: AssetIconName,
                   active: bool,
                   enabled: bool,
                   label: &'static str,
                   keys: &'static str,
                   cmd: EditorCommand| {
            md_tb_icon(
                id,
                icon,
                active,
                enabled,
                bounds.clone(),
                hover_for(id, label, keys),
                run(cmd),
            )
        };
        let sep = || div().w(px(1.)).h(px(12.)).mx(px(2.5)).flex_none().bg(sep_bg);

        let code_block_cmd = if in_code_block {
            EditorCommand::SetBlockKind(BlockKind::Paragraph)
        } else {
            EditorCommand::SetCodeBlock { lang: None }
        };

        div()
            .id("md-toolbar-scroll")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_x_scroll()
            .scrollbar_width(px(0.))
            .child(
                h_flex()
                    .h_full()
                    .flex_none()
                    .items_center()
                    .gap(px(1.5))
                    // 1. Heading level.
                    .child(btn(
                        "md-para",
                        AssetIconName::Pilcrow,
                        false,
                        heading_enabled,
                        "正文",
                        "Ctrl+Alt+0",
                        EditorCommand::SetBlockKind(BlockKind::Paragraph),
                    ))
                    .child(btn(
                        "md-h1",
                        AssetIconName::Heading1,
                        heading(1),
                        heading_enabled,
                        "一级标题",
                        "",
                        EditorCommand::SetBlockKind(BlockKind::Heading { level: 1 }),
                    ))
                    .child(btn(
                        "md-h2",
                        AssetIconName::Heading2,
                        heading(2),
                        heading_enabled,
                        "二级标题",
                        "",
                        EditorCommand::SetBlockKind(BlockKind::Heading { level: 2 }),
                    ))
                    .child(btn(
                        "md-h3",
                        AssetIconName::Heading3,
                        heading(3),
                        heading_enabled,
                        "三级标题",
                        "",
                        EditorCommand::SetBlockKind(BlockKind::Heading { level: 3 }),
                    ))
                    .child(sep())
                    // 2. Inline styles.
                    .child(btn(
                        "md-bold",
                        AssetIconName::Bold,
                        bold,
                        marks_enabled,
                        "加粗",
                        "Ctrl+B",
                        EditorCommand::ToggleMark(MarkKind::Bold),
                    ))
                    .child(btn(
                        "md-italic",
                        AssetIconName::Italic,
                        italic,
                        marks_enabled,
                        "斜体",
                        "Ctrl+I",
                        EditorCommand::ToggleMark(MarkKind::Italic),
                    ))
                    .child(btn(
                        "md-strike",
                        AssetIconName::Strikethrough,
                        strike,
                        marks_enabled,
                        "删除线",
                        "",
                        EditorCommand::ToggleMark(MarkKind::Strikethrough),
                    ))
                    .child(btn(
                        "md-code",
                        AssetIconName::Code,
                        code,
                        marks_enabled,
                        "行内代码",
                        "Ctrl+E",
                        EditorCommand::ToggleMark(MarkKind::Code),
                    ))
                    .child(btn(
                        "md-highlight",
                        AssetIconName::Highlighter,
                        highlight,
                        marks_enabled,
                        "文本高亮",
                        "",
                        EditorCommand::ToggleMark(MarkKind::Highlight),
                    ))
                    .child(sep())
                    // 3. Lists and block structure.
                    .child(btn(
                        "md-bullet",
                        AssetIconName::List,
                        in_bullet,
                        true,
                        "无序列表",
                        "",
                        EditorCommand::ToggleList { ordered: false },
                    ))
                    .child(btn(
                        "md-ordered",
                        AssetIconName::ListOrdered,
                        in_ordered,
                        true,
                        "有序列表",
                        "",
                        EditorCommand::ToggleList { ordered: true },
                    ))
                    .child(btn(
                        "md-task",
                        AssetIconName::ListChecks,
                        in_task,
                        true,
                        "待办清单",
                        "",
                        EditorCommand::ToggleTask,
                    ))
                    .child(btn(
                        "md-quote",
                        AssetIconName::TextQuote,
                        in_quote,
                        true,
                        "引用块",
                        "",
                        EditorCommand::ToggleBlockquote,
                    ))
                    .child(btn(
                        "md-codeblock",
                        AssetIconName::SquareCode,
                        in_code_block,
                        true,
                        "代码块",
                        "",
                        code_block_cmd,
                    ))
                    .child(btn(
                        "md-hr",
                        AssetIconName::Minus,
                        false,
                        true,
                        "分割线",
                        "",
                        EditorCommand::InsertBlock(BlockKind::HorizontalRule),
                    ))
                    .child(btn(
                        "md-table",
                        AssetIconName::Table,
                        false,
                        true,
                        "插入表格",
                        "",
                        EditorCommand::InsertTable { rows: 3, cols: 3 },
                    )),
            )
    }

    /// Frosted Tahoe capsule holding the paper-theme swatches.
    /// Selection is expressed purely by size + soft elevation — no outer ring,
    /// no inner dot, and no dark circle.
    fn render_palette_capsule(
        &self,
        current_theme: &'static ThemeSpec,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut palette = h_flex()
            .h(px(26.))
            .items_center()
            .gap(px(4.5))
            .px(px(7.))
            .rounded(px(9999.))
            .bg(swatch_track_bg())
            .shadow(capsule_control_edge())
            .flex_none();

        for t in THEMES {
            let theme_name = t.name.to_string();
            let is_cur = t.name == current_theme.name;

            let mut circle = div()
                .id(ElementId::Name(format!("palette-{}", t.name).into()))
                .size(if is_cur { px(15.) } else { px(12.) })
                .flex_none()
                .rounded_full()
                .cursor_pointer()
                .bg(rgb(t.accent));

            if is_cur {
                circle = circle.shadow(vec![lifted_swatch_shadow()]);
            } else {
                circle = circle
                    .opacity(0.88)
                    .hover(|s| s.opacity(1.).shadow(vec![lifted_swatch_shadow()]))
                    .active(|s| s.opacity(0.75));
            }

            circle = circle.on_click(cx.listener(move |this, _, _window, cx| {
                this.set_theme(&theme_name, cx);
            }));

            palette = palette.child(circle);
        }

        palette
    }

    fn render_editor_area(
        &mut self,
        theme: &'static ThemeSpec,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let Some((editor, title_input)) = self.editor.clone().zip(self.title_input.clone()) else {
            return v_flex()
                .id("editor-pane")
                .flex_1()
                .min_w_0()
                .h_full()
                .bg(rgb(theme.paper_bg))
                .items_center()
                .justify_center()
                .child(
                    v_flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .size(px(46.))
                                .rounded(px(12.))
                                .bg(ink(0.05))
                                .flex()
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap(px(3.5))
                                .child(div().w(px(22.)).h(px(2.5)).rounded_full().bg(ink(0.22)))
                                .child(div().w(px(22.)).h(px(2.5)).rounded_full().bg(ink(0.16)))
                                .child(div().w(px(13.)).h(px(2.5)).rounded_full().bg(ink(0.12))),
                        )
                        .child(
                            Label::new("选择左侧便签，或点击“新建便签”")
                                .text_size(px(13.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(ink(0.45)),
                        ),
                );
        };

        let header = self.render_editor_header(&editor, theme, cx);

        let editor_content = div()
            .id("editor-canvas")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .text_color(rgb(0x232B36))
            // Title: borderless single-line field, visually part of the note sheet.
            .child(
                div()
                    .w_full()
                    .max_w(px(740.))
                    .mx_auto()
                    .px_8()
                    .pt(px(22.))
                    .pb(px(10.))
                    .child(
                        // The stock input frame is a 32px control with 8px of
                        // vertical padding and a 20px line box, which clips
                        // 22px glyphs. The frame hugs one full title line.
                        Input::new(&title_input)
                            .appearance(false)
                            .bordered(false)
                            .focus_bordered(false)
                            .h(TITLE_LINE_H)
                            .py_0()
                            .px(EDITOR_TEXT_INSET)
                            .line_height(TITLE_LINE_H)
                            .text_size(TITLE_TEXT_SIZE)
                            .font_weight(FontWeight::BOLD),
                    ),
            )
            // Delicate ink rule separating the note title from the markdown body.
            .child(
                div()
                    .w_full()
                    .max_w(px(740.))
                    .mx_auto()
                    .px_8()
                    .child(
                        div()
                            .w_full()
                            .px(EDITOR_TEXT_INSET)
                            .child(div().w_full().h(px(1.)).bg(ink(0.055))),
                    ),
            )
            // Body: markdown editor with comfortable measure and breathing room.
            .child(
                div()
                    .w_full()
                    .max_w(px(740.))
                    .mx_auto()
                    .px_8()
                    .pt(px(16.))
                    .pb_6()
                    .child(BlockEditor::new(&editor).auto_grow(12, 60)),
            );

        // Footer left: the current note's tags + only busy/error states, all
        // rendered through `ink(alpha)` so every paper colour reads cleanly.
        let footer_tags: Vec<String> = self
            .selected_id
            .as_deref()
            .map(|id| {
                self.tags_for_note(id)
                    .iter()
                    .map(|t| t.name.clone())
                    .collect()
            })
            .unwrap_or_default();

        let status_badge = match &self.status {
            SyncStatus::Error(e) => Some((true, e.clone())),
            SyncStatus::Busy(s) => Some((false, s.clone())),
            _ => None,
        };

        let (chars, lines) = self.current_note_stats(cx);
        let current_note_updated = self
            .current_note()
            .and_then(|n| n.updated_at)
            .map(ticks::format_ticks)
            .unwrap_or_default();

        let meta_color = ink(0.45);
        let pill_bg = ink(0.055);
        let pill_fg = ink(0.62);

        let footer = h_flex()
            .id("editor-footer")
            .w_full()
            .h(px(29.))
            .flex_none()
            .px_5()
            .border_t_1()
            .border_color(ink(0.055))
            .bg(rgb(theme.paper_bg))
            .items_center()
            .justify_between()
            .child(
                h_flex()
                    .items_center()
                    .gap_1p5()
                    .min_w_0()
                    .children(footer_tags.iter().map(|name| {
                        div()
                            .px_2()
                            .py(px(1.))
                            .rounded_full()
                            .bg(pill_bg)
                            .child(
                                Label::new(format!("#{name}"))
                                    .text_size(px(10.5))
                                    .text_color(pill_fg),
                            )
                    }))
                    .when_some(status_badge, |el, (is_err, text)| {
                        let color = if is_err {
                            rgb(0xC04646)
                        } else {
                            meta_color.into()
                        };
                        el.child(
                            h_flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    Icon::new(if is_err {
                                        AssetIconName::CircleAlert
                                    } else {
                                        AssetIconName::RefreshCw
                                    })
                                    .size(px(10.5))
                                    .text_color(color),
                                )
                                .child(Label::new(text).text_size(px(10.5)).text_color(color)),
                        )
                    })
                    .when(self.native_app_running, |el| {
                        el.child(
                            Label::new("Windows 便签正在运行，修改可能被其覆盖")
                                .text_size(px(10.5))
                                .text_color(rgb(0xA67316)),
                        )
                    }),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_3()
                    .flex_none()
                    .when(chars > 0, |el| {
                        el.child(
                            Label::new(format!("共 {chars} 字 · {lines} 行"))
                                .text_size(px(10.5))
                                .text_color(meta_color),
                        )
                    })
                    .when(!current_note_updated.is_empty(), |el| {
                        el.child(
                            Label::new(format!("修改于 {current_note_updated}"))
                                .text_size(px(10.5))
                                .text_color(meta_color),
                        )
                    }),
            );

        v_flex()
            .id("editor-pane")
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(rgb(theme.paper_bg))
            .child(header)
            .child(editor_content)
            .child(footer)
    }

    // =========================================================================
    // Settings modal (Tahoe solid-glass surface + concentric geometry)
    // =========================================================================
    fn render_settings_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent = rgb(0x3B6EA8);
        let text = rgb(INK_PRIMARY);
        let muted = rgb(INK_MUTED);

        let test_status: Option<(Rgba, Option<IconName>, String)> = match &self.webdav_test {
            WebDavTest::Idle => None,
            WebDavTest::Testing => Some((muted, None, "正在连接…".to_string())),
            WebDavTest::Ok(msg) => Some((rgb(0x2A8555), Some(IconName::Check), msg.clone())),
            WebDavTest::Err(msg) => Some((rgb(0xC04646), Some(IconName::Close), msg.clone())),
        };
        let testing = self.webdav_test == WebDavTest::Testing;

        // Each field carries its own element id: gpui-base gives every text
        // editor the same internal id ("input-state"), and the accessibility
        // node id is hashed from the chain of enclosing ids. Without a unique
        // id per row all four inputs collapse onto one a11y node and gpui
        // panics with "Duplicate a11y node id" on the first focus change.
        let field = |key: &'static str,
                     label: &str,
                     input: &Entity<InputState>,
                     password: bool,
                     muted: Rgba| {
            v_flex()
                .id(key)
                .gap(px(5.))
                .child(
                    Label::new(label.to_string())
                        .text_size(px(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(muted),
                )
                .child(
                    div()
                        .h(px(33.))
                        .px_2p5()
                        .flex()
                        .items_center()
                        .rounded(px(8.5))
                        .bg(rgb(0xFFFFFF))
                        .shadow(capsule_control_edge())
                        .child(
                            div().flex_1().min_w_0().child({
                                let mut widget = Input::new(input)
                                    .appearance(false)
                                    .bordered(false)
                                    .focus_bordered(false);
                                if password {
                                    widget = widget.mask_toggle();
                                }
                                widget
                            }),
                        ),
                )
        };

        let mut rows = v_flex().gap(px(16.));

        // --- Sync strategy switches (concentric rounded(12) inside rounded(18) modal) ---
        let webdav_on = self.config.webdav_auto;
        let ms_on = self.config.microsoft;
        rows = rows.child(
            div()
                .rounded(px(12.))
                .bg(rgb(0xF6F8FA))
                .shadow(vec![hairline_edge(false, 0.06)])
                .px(px(14.))
                .py(px(12.))
                .flex()
                .flex_col()
                .gap(px(11.))
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .gap_3()
                        .child(
                            v_flex()
                                .gap(px(2.))
                                .child(
                                    Label::new("自动同步（WebDAV）")
                                        .text_size(px(12.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(text),
                                )
                                .child(
                                    Label::new("编辑后约 2.5 秒上传；启动与后台定时拉取")
                                        .text_size(px(11.))
                                        .text_color(muted),
                                ),
                        )
                        .child(
                            settings_switch("sw-webdav", webdav_on, accent).on_click(
                                cx.listener(|this, _, _window, cx| {
                                    let on = !this.config.webdav_auto;
                                    this.set_webdav_auto(on, cx);
                                }),
                            ),
                        ),
                )
                .child(div().w_full().h(px(1.)).bg(shell_hairline()))
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .gap_3()
                        .child(
                            v_flex()
                                .gap(px(2.))
                                .child(
                                    Label::new("同步微软账户（Windows 便签）")
                                        .text_size(px(12.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(text),
                                )
                                .child(
                                    Label::new(
                                        "可选；跟随原生便签与云端（纯文本，不含 Markdown 格式）",
                                    )
                                    .text_size(px(11.))
                                    .text_color(muted),
                                ),
                        )
                        .child(
                            settings_switch("sw-microsoft", ms_on, accent).on_click(
                                cx.listener(|this, _, _window, cx| {
                                    let on = !this.config.microsoft;
                                    this.set_microsoft_sync(on, cx);
                                }),
                            ),
                        ),
                ),
        );

        // --- WebDAV form ---
        let mut form = v_flex().gap(px(11.));
        form = form.child(
            Label::new("WebDAV 服务器")
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(text),
        );
        if let Some(inputs) = &self.settings {
            form = form
                .child(field(
                    "settings-url",
                    "服务器地址（URL）",
                    &inputs.url,
                    false,
                    muted,
                ))
                .child(field("settings-user", "用户名", &inputs.user, false, muted))
                .child(field("settings-pass", "密码", &inputs.pass, true, muted))
                .child(field(
                    "settings-dir",
                    "远端同步目录",
                    &inputs.dir,
                    false,
                    muted,
                ));
        }

        let sync_label = if self.webdav_syncing {
            "同步中…"
        } else {
            "立即同步"
        };
        let mut actions = h_flex()
            .pt(px(2.))
            .items_center()
            .gap_2p5()
            .flex_wrap()
            .child(
                div()
                    .id("btn-webdav-test")
                    .h(px(31.))
                    .px_4()
                    .flex()
                    .items_center()
                    .rounded(px(9999.))
                    .bg(if testing {
                        rgb(0xE8EDF3)
                    } else {
                        rgb(PRIMARY_BTN)
                    })
                    .shadow(capsule_control_edge())
                    .when(!testing, |el| {
                        el.cursor_pointer()
                            .hover(|s| s.bg(rgb(PRIMARY_BTN_HOVER)))
                            .active(|s| s.bg(rgb(PRIMARY_BTN_ACTIVE)))
                            .on_click(cx.listener(|this, _, window, cx| this.test_webdav(window, cx)))
                    })
                    .child(
                        Label::new(if testing { "测试中…" } else { "测试连接" })
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if testing { muted } else { rgb(0xFFFFFF) }),
                    ),
            )
            .child(
                div()
                    .id("btn-webdav-sync")
                    .h(px(31.))
                    .px_4()
                    .flex()
                    .items_center()
                    .rounded(px(9999.))
                    .bg(rgb(0xFFFFFF))
                    .shadow(capsule_control_edge())
                    .when(!self.webdav_syncing, |el| {
                        el.cursor_pointer()
                            .hover(|s| s.bg(rgb(0xF6F8FB)))
                            .active(|s| s.bg(rgb(0xECEFF5)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_webdav_sync(window, cx, true)
                            }))
                    })
                    .child(
                        Label::new(sync_label)
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if self.webdav_syncing { muted } else { text }),
                    ),
            );

        if let Some((color, maybe_icon, msg)) = test_status {
            actions = actions.child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .when_some(maybe_icon, |el, ic| {
                        el.child(Icon::new(ic).size(px(11.)).text_color(color))
                    })
                    .child(Label::new(msg).text_size(px(11.)).text_color(color)),
            );
        } else if let Some(msg) = &self.webdav_message {
            actions = actions.child(Label::new(msg.clone()).text_size(px(11.)).text_color(muted));
        }

        form = form.child(actions);
        form = form.child(
            Label::new(
                "每条便签保存为 <id>.md，带 YAML Frontmatter，可在任意 Markdown 编辑器中阅读。",
            )
            .text_size(px(10.5))
            .text_color(muted),
        );
        rows = rows.child(form);

        let card = v_flex()
            .id("settings-card")
            .w(px(510.))
            .max_h(px(660.))
            .rounded(px(18.))
            .bg(rgb(0xFFFFFF))
            .shadow(modal_surface_shadow())
            .p(px(20.))
            .gap(px(16.))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        v_flex()
                            .gap(px(2.))
                            .child(
                                Label::new("设置")
                                    .text_size(px(16.5))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(text),
                            )
                            .child(
                                Label::new("数据同步与 WebDAV")
                                    .text_size(px(11.))
                                    .text_color(muted),
                            ),
                    )
                    .child(
                        div()
                            .id("btn-settings-close")
                            .size(px(28.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .cursor_pointer()
                            .hover(|el| el.bg(shell_hover()))
                            .active(|el| el.bg(shell_active()))
                            .child(
                                Icon::new(AssetIconName::X)
                                    .size(px(14.))
                                    .text_color(rgb(INK_SECONDARY)),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_settings(window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .id("settings-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(rows),
            )
            // Clicks inside the card must not reach the backdrop.
            .on_click(|_, _, cx| cx.stop_propagation());

        div()
            .id("settings-backdrop")
            .absolute()
            .top(px(0.))
            .left(px(0.))
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(hsla(220. / 360., 0.22, 0.08, 0.36))
            .on_click(cx.listener(|this, _, window, cx| this.close_settings(window, cx)))
            .on_key_down(
                cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key.eq_ignore_ascii_case("escape") {
                        this.close_settings(window, cx);
                    }
                }),
            )
            .child(card)
            .into_any_element()
    }
}

/// Compact Tahoe pill switch used in the settings dialog.
fn settings_switch(id: &'static str, on: bool, accent: Rgba) -> Stateful<Div> {
    let off_bg = rgb(0xD2D8E2);
    let off_hover = rgb(0xC3CBD8);
    let on_hover = rgb(0x336094);
    div()
        .id(id)
        .w(px(38.))
        .h(px(22.))
        .flex_none()
        .rounded_full()
        .bg(if on { accent } else { off_bg })
        .hover(move |s| s.bg(if on { on_hover } else { off_hover }))
        .active(|s| s.opacity(0.88))
        .cursor_pointer()
        .relative()
        .child(
            div()
                .absolute()
                .top(px(2.))
                .left(if on { px(18.) } else { px(2.) })
                .size(px(18.))
                .rounded_full()
                .bg(rgb(0xFFFFFF))
                .shadow(vec![BoxShadow {
                    inset: false,
                    color: hsla(0., 0., 0., 0.20),
                    offset: point(px(0.), px(1.)),
                    blur_radius: px(2.5),
                    spread_radius: px(0.),
                }]),
        )
}
