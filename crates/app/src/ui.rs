use crate::state::{AppState, MdTip, SyncStatus, ThemeSpec, WebDavTest, THEMES};
use gpui_kit::base::{h_flex, v_flex, ElementExt as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::Sizable as _;
use gpui_kit::assets::IconName as AssetIconName;
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

const SIDEBAR_W: Pixels = px(220.);
const LIST_PANE_W: Pixels = px(272.);
const TITLEBAR_H: Pixels = px(40.);

// =========================================================================
// Tahoe / Liquid Glass Surface & Shadow Helpers (solid glass form)
// =========================================================================

/// Inner specular highlight rim for Tahoe solid glass form.
fn specular_rim(is_dark: bool) -> BoxShadow {
    BoxShadow {
        inset: true,
        color: hsla(0., 0., 1., if is_dark { 0.12 } else { 0.75 }),
        offset: point(px(0.), px(0.5)),
        blur_radius: px(0.),
        spread_radius: px(0.5),
    }
}

/// Fine hairline outer edge.
fn hairline_edge(is_dark: bool) -> BoxShadow {
    BoxShadow {
        inset: false,
        color: hsla(0., 0., 0., if is_dark { 0.40 } else { 0.07 }),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(0.5),
    }
}

/// Multi-layered surface shadow with specular rim, fine edge, and soft depth.
fn surface_shadow(is_dark: bool, elevated: bool) -> Vec<BoxShadow> {
    let depth_alpha = if is_dark { 0.32 } else { 0.07 };
    let (y, blur, spread) = if elevated {
        (px(5.), px(10.), px(-2.))
    } else {
        (px(1.5), px(3.), px(0.))
    };
    vec![
        specular_rim(is_dark),
        hairline_edge(is_dark),
        BoxShadow {
            inset: false,
            color: hsla(0., 0., 0., depth_alpha),
            offset: point(px(0.), y),
            blur_radius: blur,
            spread_radius: spread,
        },
    ]
}

/// Capsule button edge for Tahoe geometry (inner top rim, fine edge, contact shadow).
fn capsule_control_edge(is_dark: bool) -> Vec<BoxShadow> {
    let rim = if is_dark { 0.14 } else { 0.45 };
    let edge = if is_dark { 0.35 } else { 0.08 };
    let contact = if is_dark { 0.25 } else { 0.08 };
    vec![
        BoxShadow {
            inset: true,
            color: hsla(0., 0., 1., rim),
            offset: point(px(0.), px(1.)),
            blur_radius: px(0.),
            spread_radius: px(0.),
        },
        BoxShadow {
            inset: false,
            color: hsla(0., 0., 0., edge),
            offset: point(px(0.), px(0.)),
            blur_radius: px(0.),
            spread_radius: px(0.5),
        },
        BoxShadow {
            inset: false,
            color: hsla(0., 0., 0., contact),
            offset: point(px(0.), px(1.)),
            blur_radius: px(2.),
            spread_radius: px(0.),
        },
    ]
}

/// Soft drop shadow that lifts the selected swatch off the palette track.
/// Selection is expressed by size + elevation only — no ring, no inner dot,
/// no heavy dark circle.
fn lifted_swatch_shadow(dark: bool) -> BoxShadow {
    BoxShadow {
        inset: false,
        color: hsla(0., 0., 0., if dark { 0.45 } else { 0.22 }),
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
            .bg(rgb(0xF7F8FA))
            .child(self.render_titlebar(window, cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(window, cx))
                    .child(self.render_note_list(window, cx))
                    .child(self.render_editor_area(current_theme, window, cx)),
            )
            // Hover hint for the markdown bar, painted last so it floats over
            // the title below the toolbar.
            .when_some(self.md_tip, |el, tip| el.child(md_tip_overlay(tip)))
            .when_some(settings_modal, |el, modal| el.child(modal))
    }
}

/// A single window control button drawn on the light titlebar.
/// The OS handles the actual click via `window_control_area` hit testing.
fn titlebar_control(id: &'static str, area: WindowControlArea, icon: IconName) -> impl IntoElement {
    div()
        .id(id)
        .size(px(40.))
        .flex()
        .items_center()
        .justify_center()
        .window_control_area(area)
        .hover(|el| el.bg(rgb(0xE8EBF0)))
        .child(
            Icon::new(icon)
                .size(px(14.))
                .text_color(rgb(0x5A6472)),
        )
}

/// Ink overlay for the note's paper: pure black on light papers, pure white on
/// dark ones, so a single alpha ramp works on every note theme and always lets
/// the paper colour show through instead of painting a fixed grey.
fn ink(theme: &ThemeSpec, alpha: f32) -> Hsla {
    if theme.is_dark {
        hsla(0., 0., 1., alpha)
    } else {
        hsla(0., 0., 0., alpha)
    }
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

/// One icon button of the markdown toolbar: a 26×26 transparent square that
/// only ever paints black/white alpha, so it blends into any paper colour.
/// Its painted bounds are recorded so the hover hint can sit under it.
fn md_tb_icon(
    id: &'static str,
    icon: AssetIconName,
    active: bool,
    enabled: bool,
    theme: &'static ThemeSpec,
    bounds: Rc<RefCell<HashMap<&'static str, Bounds<Pixels>>>>,
    on_hover: impl Fn(&bool, &mut Window, &mut App) + 'static,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let fg = if !enabled {
        ink(theme, 0.24)
    } else if active {
        ink(theme, 0.90)
    } else {
        ink(theme, 0.58)
    };
    let fg_hover = if active { ink(theme, 0.90) } else { ink(theme, 0.85) };
    let bg_hover = if active { ink(theme, 0.09) } else { ink(theme, 0.05) };
    let bg_active = ink(theme, 0.09);

    div()
        .id(id)
        .group(id)
        .size(px(26.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .text_color(fg)
        .when(active, |el| el.bg(bg_active))
        .when(enabled, |el| {
            el.cursor_pointer().hover(move |el| el.bg(bg_hover))
        })
        // The icon inherits the text colour, so hover only has to lift the
        // wrapper's text colour — no second icon element or colour plumbing.
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .group_hover(id, move |el| el.text_color(fg_hover))
                .child(Icon::new(icon).size(px(15.))),
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

/// The dark capsule hint shown under a toolbar button.
fn md_tip_overlay(tip: MdTip) -> impl IntoElement {
    h_flex()
        .absolute()
        .left(tip.x)
        .top(tip.y + px(6.))
        .items_center()
        .gap(px(6.))
        .px(px(8.))
        .py(px(4.))
        .rounded(px(5.))
        .bg(rgb(0x1F2329))
        .shadow(vec![BoxShadow {
            inset: false,
            color: hsla(0., 0., 0., 0.15),
            offset: point(px(0.), px(4.)),
            blur_radius: px(12.),
            spread_radius: px(0.),
        }])
        .child(
            Label::new(tip.label)
                .text_size(px(11.5))
                .text_color(rgb(0xFFFFFF)),
        )
        .when(!tip.keys.is_empty(), |el| {
            el.child(
                Label::new(tip.keys)
                    .text_size(px(11.5))
                    .text_color(hsla(0., 0., 1., 0.55)),
            )
        })
}

impl AppState {
    // =========================================================================
    // 0. Immersive light titlebar (merged with the app surface)
    // =========================================================================
    fn render_titlebar(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .id("app-titlebar")
            .w_full()
            .h(TITLEBAR_H)
            .flex_none()
            .items_center()
            .bg(rgb(0xF7F8FA))
            // The drag area must NOT cover the window buttons. Its hitbox is
            // painted before its children and the platform picks the first
            // window-control hitbox under the cursor, so a full-width Drag
            // area would swallow every click on min/max/close.
            .child(
                h_flex()
                    .id("titlebar-drag-region")
                    .flex_1()
                    .h_full()
                    .pl(px(12.))
                    .items_center()
                    .gap_1p5()
                    .window_control_area(WindowControlArea::Drag)
                    .child(
                        div()
                            .size(px(10.))
                            .rounded(px(2.5))
                            .bg(rgb(0xF5B84C))
                            .shadow(capsule_control_edge(false)),
                    )
                    .child(
                        Label::new("便签")
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(0x3D4756)),
                    )
                    .child(
                        Label::new("Sticky Notes")
                            .text_size(px(10.5))
                            .text_color(rgb(0x9AA3B2)),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .flex_none()
                    .child(titlebar_control("tb-min", WindowControlArea::Min, IconName::WindowMinimize))
                    .child(titlebar_control("tb-max", WindowControlArea::Max, IconName::WindowMaximize))
                    .child(titlebar_control("tb-close", WindowControlArea::Close, IconName::WindowClose)),
            )
    }

    // =========================================================================
    // 1. Left Navigation Sidebar
    // =========================================================================
    fn render_sidebar(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let all_count = self.notes.len() as u32;

        // --- Tahoe Capsule "New Note" Button ---
        let new_button = div()
            .px_3()
            .pb_2p5()
            .child(
                h_flex()
                    .id("btn-new-note")
                    .w_full()
                    .h(px(34.))
                    .px_4()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .rounded(px(9999.))
                    .bg(rgb(0x334155))
                    .hover(|el| el.bg(rgb(0x475569)))
                    .shadow(capsule_control_edge(false))
                    .cursor_pointer()
                    .child(
                        Label::new("＋  新建便签")
                            .text_size(px(12.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(0xFFFFFF)),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.new_note(window, cx);
                    })),
            );

        // --- Navigation Items (All Notes + Tags) ---
        let mut nav_rows = v_flex().px_2().py_1().gap(px(2.));

        // "All Notes" item
        let is_all_selected = self.tag_filter.is_none();
        nav_rows = nav_rows.child(
            h_flex()
                .id("nav-all")
                .px_3()
                .py_2()
                .rounded(px(8.))
                .items_center()
                .justify_between()
                .cursor_pointer()
                .when(is_all_selected, |el| {
                    el.bg(rgb(0xEAEFF6))
                })
                .when(!is_all_selected, |el| el.hover(|el| el.bg(rgb(0xF0F2F5))))
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Icon::new(IconName::Inbox)
                                .size(px(14.))
                                .text_color(if is_all_selected {
                                    rgb(0x5B7A9D)
                                } else {
                                    rgb(0x8B94A3)
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
                                    rgb(0x3D5166)
                                } else {
                                    rgb(0x3D4756)
                                }),
                        ),
                )
                .child(
                    div()
                        .px_2()
                        .py(px(1.))
                        .rounded_full()
                        .bg(if is_all_selected {
                            rgb(0xD9E2EC)
                        } else {
                            rgb(0xE9EDF2)
                        })
                        .child(
                            Label::new(format!("{all_count}"))
                                .text_size(px(10.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(if is_all_selected {
                                    rgb(0x5B7A9D)
                                } else {
                                    rgb(0x8B94A3)
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
            .px_3()
            .pt_3()
            .pb_1()
            .items_center()
            .justify_between()
            .child(
                Label::new("标签分类")
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(0x8B94A3)),
            )
            .child(
                div()
                    .id("btn-toggle-sidebar-add-tag")
                    .size(px(19.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .cursor_pointer()
                    .hover(|el| el.bg(rgb(0xE4E8EE)))
                    .child(
                        Icon::new(IconName::Plus)
                            .size(px(11.5))
                            .text_color(rgb(0x6B7480)),
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
                    .mx_1()
                    .my_1()
                    .pl_2p5()
                    .pr_1()
                    .h(px(32.))
                    .gap_1()
                    .items_center()
                    .rounded(px(9.))
                    .bg(rgb(0xFFFFFF))
                    .border_1()
                    .border_color(rgb(0xE4E8EE))
                    .shadow(surface_shadow(false, false))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
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
                            .bg(rgb(0xE7EEF6))
                            .hover(|el| el.bg(rgb(0xD5E2F0)))
                            .cursor_pointer()
                            .child(
                                Icon::new(IconName::Check)
                                    .size(px(11.))
                                    .text_color(rgb(0x4A6889)),
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
                            .hover(|el| el.bg(rgb(0xF0F2F5)))
                            .cursor_pointer()
                            .child(
                                Icon::new(IconName::Close)
                                    .size(px(10.))
                                    .text_color(rgb(0x9AA3B2)),
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
                div()
                    .px_3()
                    .py_2()
                    .child(
                        Label::new("暂无标签，点击 ＋ 创建")
                            .text_size(px(11.))
                            .text_color(rgb(0x9AA3B2)),
                    ),
            );
        }

        for t in self.tags_list.iter() {
            let tid = t.id;
            let count = self.tag_counts.get(&tid).copied().unwrap_or(0);
            let is_selected = self.tag_filter == Some(tid);
            // Unique group per row so the delete button only fades in for the
            // row the pointer is actually over.
            let row_group = format!("tag-row-{tid}");

            nav_rows = nav_rows.child(
                h_flex()
                    .id(ElementId::Name(format!("nav-tag-{tid}").into()))
                    .group(row_group.clone())
                    .h(px(30.))
                    .px_3()
                    .rounded(px(8.))
                    .items_center()
                    .justify_between()
                    .cursor_pointer()
                    .when(is_selected, |el| {
                        el.bg(rgb(0xEAEFF6))
                    })
                    .when(!is_selected, |el| el.hover(|el| el.bg(rgb(0xF0F2F5))))
                    .child(
                        h_flex()
                            .items_center()
                            .min_w_0()
                            .child(
                                Label::new("#")
                                    .text_size(px(11.5))
                                    .text_color(if is_selected {
                                        rgb(0x7E93A8)
                                    } else {
                                        rgb(0x9AA3B2)
                                    }),
                            )
                            .child(
                                Label::new(t.name.clone())
                                    .text_size(px(12.))
                                    .truncate()
                                    .text_color(if is_selected {
                                        rgb(0x3D5166)
                                    } else {
                                        rgb(0x556070)
                                    }),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .px_1p5()
                                    .py(px(0.5))
                                    .rounded_full()
                                    .bg(if is_selected {
                                        rgb(0xD9E2EC)
                                    } else {
                                        rgb(0xEDF0F4)
                                    })
                                    .child(
                                        Label::new(format!("{count}"))
                                            .text_size(px(10.))
                                            .text_color(if is_selected {
                                                rgb(0x5B7A9D)
                                            } else {
                                                rgb(0x8B94A3)
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .id(ElementId::Name(format!("del-tag-{tid}").into()))
                                    .size(px(18.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .cursor_pointer()
                                    .opacity(0.)
                                    .group_hover(row_group, |el| el.opacity(1.))
                                    .hover(|el| el.bg(rgb(0xF6E3E7)))
                                    .child(
                                        Icon::new(IconName::Close)
                                            .size(px(10.))
                                            .text_color(rgb(0xB98A94)),
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
        let (dot, webdav_label) = if webdav_on {
            (0x74B58B, "WebDAV 自动同步")
        } else if webdav_configured {
            (0xD8A24A, "WebDAV 已配置")
        } else {
            (0xBFC6D1, "WebDAV 未配置")
        };
        let footer_status = h_flex()
            .px_3()
            .py_2p5()
            .border_t_1()
            .border_color(rgb(0xE7EAF0))
            .items_center()
            .gap_2()
            .justify_between()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .child(
                        div()
                            .size(px(7.))
                            .flex_none()
                            .rounded_full()
                            .bg(rgb(dot)),
                    )
                    .child(
                        Label::new(webdav_label)
                            .text_size(px(10.5))
                            .text_color(rgb(0x8B94A3)),
                    ),
            )
            .child(
                div()
                    .id("btn-settings")
                    .size(px(24.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(|el| el.bg(rgb(0xEDF0F4)))
                    .child(
                        Icon::new(AssetIconName::Settings)
                            .size(px(14.))
                            .text_color(rgb(0x6B7480)),
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
            .bg(rgb(0xF7F8FA))
            .border_r_1()
            .border_color(rgb(0xE7EAF0))
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
    fn render_note_list(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let notes = self.filtered_notes();

        // Spotlight-style Search Bar at top of list
        let search_box = match &self.search_input {
            Some(inp) => h_flex()
                .px_2p5()
                .h(px(52.))
                .flex_none()
                .items_center()
                .bg(rgb(0xF7F8FA))
                // Same height and hairline as the canvas palette bar, so the
                // two dividers read as one continuous line across the panes.
                .border_b_1()
                .border_color(rgb(0xE7EAF0))
                .child(
                    h_flex()
                        .w_full()
                        .h(px(30.))
                        .pl_2p5()
                        .pr_1()
                        .gap_1p5()
                        .items_center()
                        .rounded(px(15.))
                        .bg(rgb(0xFFFFFF))
                        .border_1()
                        .border_color(rgb(0xE4E8EE))
                        .shadow(surface_shadow(false, false))
                        .child(
                            Icon::new(IconName::Search)
                                .size(px(13.))
                                .text_color(rgb(0x9AA3B2)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    Input::new(inp)
                                        .appearance(false)
                                        .focus_bordered(false),
                                ),
                        )
                        .when(!self.search_query.is_empty(), |el| {
                            el.child(
                                div()
                                    .id("btn-clear-search")
                                    .size(px(16.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .hover(|s| s.bg(rgb(0xE9EDF2)))
                                    .cursor_pointer()
                                    .child(
                                        Icon::new(IconName::Close)
                                            .size(px(10.))
                                            .text_color(rgb(0x8B94A3)),
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
            self.tags_list.iter().find(|t| t.id == tid).map(|t| t.name.clone())
        });

        let subheader = h_flex()
            .px_3()
            .py_1p5()
            .items_center()
            .justify_between()
            .bg(rgb(0xF7F8FA))
            .child(
                h_flex()
                    .items_center()
                    .gap_1p5()
                    .when_some(active_tag_name, |el, name| {
                        el.child(
                            h_flex()
                                .px_1p5()
                                .py(px(0.5))
                                .rounded_full()
                                .bg(rgb(0xE3EAF2))
                                .items_center()
                                .gap_1()
                                .child(
                                    Label::new(format!("#{name}"))
                                        .text_size(px(10.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgb(0x5B7A9D)),
                                )
                                .child(
                                    div()
                                        .id("btn-clear-tag-filter")
                                        .size(px(12.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .child(
                                            Icon::new(IconName::Close)
                                                .size(px(9.))
                                                .text_color(rgb(0x5B7A9D)),
                                        )
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.clear_tag_filter(window, cx);
                                        })),
                                ),
                        )
                    })
                    .child(
                        Label::new(if self.tag_filter.is_some() || !self.search_query.is_empty() {
                            format!("筛选出 {} 条", notes.len())
                        } else {
                            format!("共 {} 条便签", notes.len())
                        })
                        .text_size(px(11.))
                        .text_color(rgb(0x9AA3B2)),
                    ),
            );

        // Cards list
        let mut card_list = v_flex().p_2().gap_1p5();
        if notes.is_empty() {
            card_list = card_list.child(
                v_flex()
                    .py_12()
                    .items_center()
                    .justify_center()
                    .gap_2p5()
                    .child(
                        div()
                            .size(px(36.))
                            .rounded(px(10.))
                            .bg(rgb(0xE7EAF0))
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap(px(2.5))
                            .child(div().w(px(18.)).h(px(2.)).rounded_full().bg(rgb(0xB4BCC8)))
                            .child(div().w(px(18.)).h(px(2.)).rounded_full().bg(rgb(0xC6CDD7)))
                            .child(div().w(px(10.)).h(px(2.)).rounded_full().bg(rgb(0xD5DAE2))),
                    )
                    .child(
                        Label::new("暂无匹配便签")
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(0x6B7480)),
                    )
                    .child(
                        Label::new("点击左侧“＋ 新建便签”添加第一条笔记")
                            .text_size(px(11.))
                            .text_color(rgb(0x9AA3B2)),
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
            .bg(rgb(0xF1F3F6))
            .border_r_1()
            .border_color(rgb(0xE7EAF0))
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
            // Slightly deeper right padding keeps the timestamp off the edge
            // and gives truncated titles/snippets room to breathe.
            .pr(px(14.))
            .py(px(7.))
            .gap_2()
            .rounded(px(10.))
            .cursor_pointer();

        if is_selected {
            card = card
                .bg(rgb(note_theme.card_active_bg))
                .border_1()
                .border_color(rgb(note_theme.border))
                .shadow(surface_shadow(note_theme.is_dark, true));
        } else {
            card = card
                .bg(rgb(0xFFFFFF))
                .border_1()
                .border_color(rgb(0xE7EAF0))
                .shadow(surface_shadow(false, false))
                .hover(|el| {
                    el.bg(rgb(0xFCFDFE))
                        .border_color(rgb(0xD9DEE6))
                });
        }

        // Card body content
        let mut content = v_flex().flex_1().min_w_0().gap(px(2.));

        // Title row: theme color dot + Title + Relative Timestamp
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
                    .size(px(7.))
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
                    .text_color(if note_theme.is_dark && is_selected {
                        rgb(0xF3F4F6)
                    } else if is_selected {
                        rgb(0x2B3442)
                    } else {
                        rgb(0x3D4756)
                    })
                    .child(title_text.to_string()),
            )
            .when(is_pinned, |el| {
                el.child(
                    Icon::new(AssetIconName::Pin)
                        .size(px(10.))
                        .flex_none()
                        .text_color(rgb(0x8B94A3)),
                )
            })
            .child(
                Label::new(updated)
                    .text_size(px(10.))
                    .flex_none()
                    .text_color(if note_theme.is_dark && is_selected {
                        rgb(0x9AA3B2)
                    } else {
                        rgb(0x9AA3B2)
                    }),
            );
        content = content.child(title_row);

        // Snippet (two brief lines, plain text — markdown already stripped at store level)
        if !note.snippet.is_empty() {
            content = content.child(
                div()
                    .text_size(px(11.5))
                    .line_height(px(17.))
                    .line_clamp(2)
                    .text_ellipsis()
                    .text_color(if note_theme.is_dark && is_selected {
                        rgb(0xCBD5E1)
                    } else {
                        rgb(0x7B8494)
                    })
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
                menu = menu.submenu("添加 / 管理标签", window, cx, move |mut sub, _window, _cx| {
                    if all_tags.is_empty() {
                        sub = sub.item(PopupMenuItem::new("暂无标签，先在左侧新建").disabled(true));
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
                                // Menu callbacks only get `&mut App` and run while
                                // the window is borrowed; defer, then go through
                                // the window handle to reach a `&mut Window`.
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
                });

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
    // 3. Right Editor Canvas Area
    // =========================================================================

    /// Flat, single-row markdown bar: every high-frequency format is a 26×26
    /// icon button with a hover hint. No dropdowns and no fixed grey — the
    /// paper colour shows through every state.
    fn render_md_toolbar(
        &self,
        editor: &Entity<BlockEditorState>,
        theme: &'static ThemeSpec,
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

        let heading = |level: u8| matches!(kind, Some(BlockKind::Heading { level: l }) if l == level);
        // Headings and inline marks cannot apply inside a code block.
        let heading_enabled = in_text && !in_code_block;
        let marks_enabled = in_text && !in_code_block;

        // Everything is a black (or white, on dark paper) alpha overlay: no
        // fixed grey, so the bar picks up whatever paper colour is behind it.
        let sep_bg = ink(theme, 0.08);
        let divider = ink(theme, 0.06);

        let app = cx.entity();
        let bounds = self.md_tip_bounds.clone();

        // Hover hint: appears after a short rest, anchored under the button's
        // painted bounds, and cleared as soon as the pointer leaves it.
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
                            // The pointer may have moved on while we waited.
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
                theme,
                bounds.clone(),
                hover_for(id, label, keys),
                run(cmd),
            )
        };
        let sep = || div().w(px(1.)).h(px(12.)).mx(px(3.)).flex_none().bg(sep_bg);

        let code_block_cmd = if in_code_block {
            EditorCommand::SetBlockKind(BlockKind::Paragraph)
        } else {
            EditorCommand::SetCodeBlock { lang: None }
        };

        // Transparent 32px strip with a single 1px hairline underneath. It
        // scrolls horizontally (scrollbar hidden) instead of wrapping when the
        // pane gets narrow.
        div()
            .id("md-toolbar-scroll")
            .w_full()
            .h(px(32.))
            .flex_none()
            .overflow_x_scroll()
            .scrollbar_width(px(0.))
            .border_b_1()
            .border_color(divider)
            .child(
                h_flex()
                    .h_full()
                    .flex_none()
                    .items_center()
                    .gap(px(2.))
                    // 1. Heading level. 正文 is a plain action, never shown as
                    // selected — a normal paragraph is the default, not a mode.
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

    fn render_editor_area(
        &mut self,
        theme: &'static ThemeSpec,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let toolbar = self.render_canvas_toolbar(theme, cx);

        let editor_content = match (&self.editor, &self.title_input) {
            (Some(editor), Some(title_input)) => div()
                .id("editor-canvas")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .text_color(if theme.is_dark {
                    rgb(0xF3F4F6)
                } else {
                    rgb(0x2B3442)
                })
                // Markdown formatting bar first, right under the palette bar.
                // Its own 32px row with a single hairline underneath.
                .child(
                    div()
                        .w_full()
                        .max_w(px(760.))
                        .mx_auto()
                        .px_6()
                        .pt(px(6.))
                        .child(self.render_md_toolbar(editor, theme, cx)),
                )
                // Title: a borderless single-line field, visually part of the page.
                .child(
                    div()
                        .w_full()
                        .max_w(px(760.))
                        .mx_auto()
                        .px_6()
                        .pt(px(14.))
                        .pb(px(10.))
                        .child(
                            Input::new(title_input)
                                .appearance(false)
                                .bordered(false)
                                .focus_bordered(false)
                                .text_size(px(21.))
                                .font_weight(FontWeight::SEMIBOLD),
                        ),
                )
                // Hairline between title and body, same width as the toolbar's.
                .child(
                    div()
                        .w_full()
                        .max_w(px(760.))
                        .mx_auto()
                        .px_6()
                        .child(div().w_full().h(px(1.)).bg(ink(theme, 0.08))),
                )
                // Body: markdown editor, 14px below the divider.
                .child(
                    div()
                        .w_full()
                        .max_w(px(760.))
                        .mx_auto()
                        .px_6()
                        .pt(px(14.))
                        .pb_5()
                        .child(BlockEditor::new(editor).auto_grow(12, 60)),
                )
                .into_any_element(),
            _ => div()
                .flex_1()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    v_flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .size(px(48.))
                                .rounded(px(12.))
                                .bg(rgb(0xE7EAF0))
                                .flex()
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap(px(3.))
                                .child(div().w(px(24.)).h(px(2.5)).rounded_full().bg(rgb(0xB4BCC8)))
                                .child(div().w(px(24.)).h(px(2.5)).rounded_full().bg(rgb(0xC6CDD7)))
                                .child(div().w(px(14.)).h(px(2.5)).rounded_full().bg(rgb(0xD5DAE2))),
                        )
                        .child(
                            Label::new("选择左侧便签，或点击“＋ 新建便签”")
                                .text_size(px(14.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(0x9AA3B2)),
                        ),
                )
                .into_any_element(),
        };

        // Footer left: the current note's tags, if it has any. Only error and
        // busy states are surfaced as text, so one-off messages like
        // “标签已删除” no longer linger under every note.
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
        let status_text = match &self.status {
            SyncStatus::Error(e) => Some(format!("⚠ {e}")),
            SyncStatus::Busy(s) => Some(format!("⏳ {s}")),
            _ => None,
        };

        let (chars, lines) = self.current_note_stats(cx);
        let current_note_updated = self
            .current_note()
            .and_then(|n| n.updated_at)
            .map(ticks::format_ticks)
            .unwrap_or_default();

        let footer = h_flex()
            .id("editor-footer")
            .w_full()
            .h(px(30.))
            .flex_none()
            .px_4()
            .border_t_1()
            .border_color(rgb(theme.border))
            .bg(rgb(theme.paper_bg))
            .items_center()
            .justify_between()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .children(footer_tags.iter().map(|name| {
                        div()
                            .px_2()
                            .py(px(1.))
                            .rounded_full()
                            .bg(rgb(if theme.is_dark { 0x3A4048 } else { 0xEDF0F4 }))
                            .child(
                                Label::new(format!("#{name}"))
                                    .text_size(px(10.))
                                    .text_color(if theme.is_dark {
                                        rgb(0xCBD5E1)
                                    } else {
                                        rgb(0x7B8494)
                                    }),
                            )
                    }))
                    .when_some(status_text, |el, text| {
                        el.child(
                            Label::new(text)
                                .text_size(px(11.))
                                .text_color(if theme.is_dark {
                                    rgb(0xD1D5DB)
                                } else {
                                    rgb(0x8B94A3)
                                }),
                        )
                    })
                    .when(self.native_app_running, |el| {
                        el.child(
                            Label::new("Windows 便签正在运行，修改可能被其覆盖")
                                .text_size(px(10.5))
                                .text_color(if theme.is_dark {
                                    rgb(0xE5B567)
                                } else {
                                    rgb(0xB07D1E)
                                }),
                        )
                    }),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_3()
                    .when(chars > 0, |el| {
                        el.child(
                            Label::new(format!("共 {chars} 字 · {lines} 行"))
                                .text_size(px(10.5))
                                .text_color(rgb(0x9AA3B2)),
                        )
                    })
                    .child(
                        Label::new(if current_note_updated.is_empty() {
                            String::new()
                        } else {
                            format!("修改于 {current_note_updated}")
                        })
                        .text_size(px(10.5))
                        .text_color(rgb(0x9AA3B2)),
                    ),
            );

        v_flex()
            .id("editor-pane")
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(rgb(theme.paper_bg))
            .child(toolbar)
            .child(editor_content)
            .child(footer)
    }

    // Canvas top toolbar: the palette only, plus the overflow menu.
    // Tagging lives in the sidebar and in the note-card right-click menu.
    fn render_canvas_toolbar(
        &mut self,
        current_theme: &'static ThemeSpec,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // --- 1. Color Palette Swatches (Capsule track) ---
        let mut palette = h_flex()
            .items_center()
            .gap(px(5.))
            .px_2()
            .py_1()
            .rounded(px(9999.))
            .bg(if current_theme.is_dark {
                rgb(0x27272A)
            } else {
                rgb(0xFFFFFF)
            })
            .border_1()
            .border_color(rgb(current_theme.border))
            .shadow(capsule_control_edge(current_theme.is_dark))
            .flex_none();

        for t in THEMES {
            let theme_name = t.name.to_string();
            let is_cur = t.name == current_theme.name;

            // Pure color dot. The selected one is slightly larger and lifted
            // by a soft shadow — no ring, no center dot, no dark circle.
            let mut circle = div()
                .id(ElementId::Name(format!("palette-{}", t.name).into()))
                .size(if is_cur { px(17.) } else { px(14.) })
                .flex_none()
                .rounded_full()
                .cursor_pointer()
                .bg(rgb(t.accent));

            if is_cur {
                circle = circle.shadow(vec![lifted_swatch_shadow(t.is_dark)]);
            }

            circle = circle.on_click(cx.listener(move |this, _, _window, cx| {
                this.set_theme(&theme_name, cx);
            }));

            palette = palette.child(circle);
        }

        h_flex()
            .id("canvas-toolbar")
            .w_full()
            .h(px(52.))
            .flex_none()
            .px_4()
            .border_b_1()
            .border_color(rgb(current_theme.border))
            .bg(rgb(current_theme.paper_bg))
            .items_center()
            .child(palette)
    }

    // =========================================================================
    // Settings modal (WebDAV + sync strategy)
    // =========================================================================
    fn render_settings_modal(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let accent = rgb(0x4F7DA8);
        let text = rgb(0x2B3440);
        let muted = rgb(0x8B94A3);
        let border = rgb(0xE4E8EE);

        let test_status = match &self.webdav_test {
            WebDavTest::Idle => None,
            WebDavTest::Testing => Some((rgb(0x8B94A3), "正在连接…".to_string())),
            WebDavTest::Ok(msg) => Some((rgb(0x2F8F5B), format!("✓ {msg}"))),
            WebDavTest::Err(msg) => Some((rgb(0xC04646), format!("✕ {msg}"))),
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
                .gap(px(6.))
                .child(Label::new(label.to_string()).text_size(px(11.5)).text_color(muted))
                .child(
                    div()
                        .h(px(34.))
                        .px_2p5()
                        .flex()
                        .items_center()
                        .rounded(px(8.))
                        .bg(rgb(0xFFFFFF))
                        .border_1()
                        .border_color(border)
                        .child(
                            div().flex_1().min_w_0().child({
                                let mut widget =
                                    Input::new(input).appearance(false).focus_bordered(false);
                                if password {
                                    widget = widget.mask_toggle();
                                }
                                widget
                            }),
                        ),
                )
        };

        let mut rows = v_flex().gap(px(16.));

        // --- Sync strategy switches ---
        let webdav_on = self.config.webdav_auto;
        let ms_on = self.config.microsoft;
        rows = rows.child(
            div()
                .rounded(px(10.))
                .bg(rgb(0xF7F9FB))
                .border_1()
                .border_color(border)
                .p(px(14.))
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .gap_3()
                        .child(
                            v_flex()
                                .gap(px(2.))
                                .child(Label::new("自动同步（WebDAV）").text_size(px(12.5)).text_color(text))
                                .child(
                                    Label::new("编辑后约 2.5 秒上传；启动与后台定时拉取")
                                        .text_size(px(10.5))
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
                                        .text_color(text),
                                )
                                .child(
                                    Label::new("可选；开启后会跟随原生便签与云端的变化")
                                        .text_size(px(10.5))
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
        let mut form = v_flex().gap(px(12.));
        form = form.child(Label::new("WebDAV 服务器").text_size(px(12.)).text_color(text));
        if let Some(inputs) = &self.settings {
            form = form
                .child(field("settings-url", "服务器地址（URL）", &inputs.url, false, muted))
                .child(field("settings-user", "用户名", &inputs.user, false, muted))
                .child(field("settings-pass", "密码", &inputs.pass, true, muted))
                .child(field("settings-dir", "远端同步目录", &inputs.dir, false, muted));
        }

        let sync_label = if self.webdav_syncing { "同步中…" } else { "立即同步" };
        let mut actions = h_flex().items_center().gap_3().child(
            div()
                .id("btn-webdav-test")
                .h(px(32.))
                .px_4()
                .flex()
                .items_center()
                .rounded(px(8.))
                .bg(if testing { rgb(0xE8EDF3) } else { accent })
                .cursor_pointer()
                .child(
                    Label::new(if testing { "测试中…" } else { "测试连接" })
                        .text_size(px(12.))
                        .text_color(if testing { rgb(0x6B7480) } else { rgb(0xFFFFFF) }),
                )
                .when(!testing, |el| {
                    el.on_click(cx.listener(|this, _, window, cx| this.test_webdav(window, cx)))
                }),
        );
        actions = actions.child(
            div()
                .id("btn-webdav-sync")
                .h(px(32.))
                .px_4()
                .flex()
                .items_center()
                .rounded(px(8.))
                .bg(rgb(0xFFFFFF))
                .border_1()
                .border_color(border)
                .cursor_pointer()
                .hover(|el| el.bg(rgb(0xF2F5F9)))
                .child(
                    Label::new(sync_label)
                        .text_size(px(12.))
                        .text_color(if self.webdav_syncing { muted } else { text }),
                )
                .when(!self.webdav_syncing, |el| {
                    el.on_click(cx.listener(|this, _, window, cx| {
                        this.start_webdav_sync(window, cx)
                    }))
                }),
        );
        if let Some((color, msg)) = test_status {
            actions = actions.child(
                Label::new(msg)
                    .text_size(px(11.))
                    .text_color(color),
            );
        } else if let Some(msg) = &self.webdav_message {
            actions = actions.child(Label::new(msg.clone()).text_size(px(11.)).text_color(muted));
        }
        form = form.child(actions);
        form = form.child(
            Label::new("每条便签保存为 <id>.md，带 YAML Frontmatter，可在任意 Markdown 编辑器中阅读。")
                .text_size(px(10.5))
                .text_color(muted),
        );
        rows = rows.child(form);

        let card = v_flex()
            .id("settings-card")
            .w(px(540.))
            .max_h(px(660.))
            .rounded(px(14.))
            .bg(rgb(0xFFFFFF))
            .shadow(vec![BoxShadow {
                inset: false,
                color: hsla(0., 0., 0., 0.24),
                offset: point(px(0.), px(18.)),
                blur_radius: px(48.),
                spread_radius: px(0.),
            }])
            .p(px(22.))
            .gap(px(16.))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        v_flex()
                            .gap(px(2.))
                            .child(Label::new("设置").text_size(px(17.)).text_color(text))
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
                            .rounded(px(6.))
                            .cursor_pointer()
                            .hover(|el| el.bg(rgb(0xF0F3F7)))
                            .child(
                                Icon::new(AssetIconName::X)
                                    .size(px(14.))
                                    .text_color(rgb(0x6B7480)),
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
            .bg(hsla(0., 0., 0., 0.34))
            .on_click(cx.listener(|this, _, window, cx| this.close_settings(window, cx)))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key.eq_ignore_ascii_case("escape") {
                    this.close_settings(window, cx);
                }
            }))
            .child(card)
            .into_any_element()
    }
}

/// Compact on/off switch used in the settings dialog.
fn settings_switch(id: &'static str, on: bool, accent: Rgba) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(38.))
        .h(px(22.))
        .flex_none()
        .rounded(px(11.))
        .bg(if on { accent } else { rgb(0xD5DAE2) })
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
                    color: hsla(0., 0., 0., 0.18),
                    offset: point(px(0.), px(1.)),
                    blur_radius: px(2.),
                    spread_radius: px(0.),
                }]),
        )
}
