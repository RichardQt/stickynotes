//! The settings dialog — a column of pages on the left (通用 / 快捷键 / 关于),
//! the chosen page on the right — and the other surfaces that float over the
//! main window: the "clear all notes" confirmation and the quick note when it
//! has no window of its own.

use crate::quick_note::{QuickAction, QUICK_H, QUICK_W};
use crate::state::{
    AppState, SettingsMenu, SettingsPage, StorageNote, SyncPass, WebDavTest, REPOSITORY_URL,
};
use crate::ui::{
    app_logo, field_border_color, group_border_color, ios_field_shadow, lerp_rgba,
    modal_surface_shadow_faded, nav_pill_shadow_faded, primary_pill_shadow,
    primary_pill_shadow_pressed, secondary_pill,
    settings_pill, settings_switch, shell_active, shell_hover, specular_rim, spinning_icon,
    GROUP_BG, INK_MUTED, INK_PRIMARY, INK_SECONDARY, PRIMARY_BTN, PRIMARY_BTN_ACTIVE,
    PRIMARY_BTN_HOVER, SHELL_BG, TITLEBAR_H,
};
use gpui_kit::assets::IconName as AssetIconName;
use gpui_kit::base::animation::{cubic_bezier, ease_out_cubic};
use gpui_kit::base::{h_flex, spring, transition, v_flex, Presence, Spring, Transition};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::time::Duration;
use stickynotes_core::config::{self, CloseBehavior, SyncDelayError};
use stickynotes_core::shortcuts::{self, ActionDef, Combo, Group};

/// Accent of the settings dialog: switches, selected items, links.
const ACCENT: u32 = 0x3B6EA8;
/// The red of destructive actions and of refused input.
const DANGER: u32 = 0xE5484D;
const DANGER_HOVER: u32 = 0xDC3E42;
const DANGER_ACTIVE: u32 = 0xCE2C31;
const ERROR_INK: u32 = 0xC04646;
const SUCCESS_INK: u32 = 0x2A8555;

// =========================================================================
// Building blocks
// =========================================================================

/// The dimmed backdrop and, below the title bar, the layer that keeps the
/// mouse away from everything behind a dialog. The title bar stays out of it
/// so the window can still be moved, minimized and closed.
fn modal_layers(
    backdrop_id: &'static str,
    shield_id: &'static str,
    dim: f32,
    card: impl IntoElement,
    on_dismiss: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(backdrop_id)
        .absolute()
        .top(px(0.))
        .left(px(0.))
        .size_full()
        .bg(hsla(220. / 360., 0.22, 0.08, dim))
        .child(
            div()
                .id(shield_id)
                .absolute()
                .top(TITLEBAR_H)
                .left(px(0.))
                .right(px(0.))
                .bottom(px(0.))
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                // A click beside the dialog dismisses it.
                .on_click(on_dismiss)
                .child(card),
        )
        .into_any_element()
}

// A `Label` is identified by the line of code that creates it. The helpers
// below are called several times on one page, so each of their labels gets
// an id of its own (its text, unique on its page): labels sharing an id
// would also share one accessibility node, and all but the first be lost.

/// Small caption above a group of settings.
fn section_title(text: &'static str) -> impl IntoElement {
    div().pl(px(4.)).child(
        Label::new(text)
            .id(text)
            .text_size(px(11.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(INK_MUTED)),
    )
}

/// The snow-porcelain rounded card a group of settings sits on, with a crisp
/// 4-sided 1px border (no top-only white inset rim erasing the upper edge).
fn group_card() -> Div {
    v_flex()
        .rounded(px(14.))
        .bg(rgb(GROUP_BG))
        .border_1()
        .border_color(group_border_color())
        .px(px(15.))
}

/// Soft rose-tinted card for destructive settings (`危险操作`), with a crisp
/// continuous 1px rose border around all four sides and corners.
fn danger_group_card() -> Div {
    v_flex()
        .rounded(px(14.))
        .bg(rgb(0xFDF6F7))
        .border_1()
        .border_color(hsla(358. / 360., 0.65, 0.54, 0.28))
        .px(px(15.))
}

/// Hairline between two rows of a group card.
fn row_divider() -> Div {
    div()
        .w_full()
        .h(px(1.))
        .flex_none()
        .bg(hsla(220. / 360., 0.20, 0.18, 0.06))
}

/// Title and explanation of one setting; the control goes to its right.
fn setting_text(title: &'static str, detail: impl Into<SharedString>) -> Div {
    v_flex()
        .flex_1()
        .min_w_0()
        .gap(px(2.))
        .child(
            Label::new(title)
                .id(title)
                .text_size(px(12.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(INK_PRIMARY)),
        )
        .child(
            div()
                .w_full()
                .text_size(px(11.))
                .line_height(px(16.))
                .text_color(rgb(INK_MUTED))
                .child(detail.into()),
        )
}

/// One line of red text under a control whose input was refused.
fn error_line(message: impl Into<SharedString>) -> Div {
    h_flex()
        .items_start()
        .gap(px(5.))
        .child(
            Icon::new(AssetIconName::CircleAlert)
                .size(px(12.))
                .mt(px(2.))
                .flex_none()
                .text_color(rgb(ERROR_INK)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(11.))
                .line_height(px(16.))
                .text_color(rgb(ERROR_INK))
                .child(message.into()),
        )
}

/// Outcome of an action (moved, imported, cleared, …), fading in under the
/// controls that caused it.
fn outcome_note(
    id: &'static str,
    animation: &'static str,
    note: &StorageNote,
    reduce_motion: bool,
) -> AnyElement {
    let (color, icon, message) = match note {
        StorageNote::Info(m) => (rgb(SUCCESS_INK), AssetIconName::CircleCheck, m.clone()),
        StorageNote::Error(m) => (rgb(ERROR_INK), AssetIconName::CircleAlert, m.clone()),
    };
    let row = h_flex()
        .id(id)
        .relative()
        .items_start()
        .gap(px(5.))
        .child(
            Icon::new(icon)
                .size(px(12.))
                .mt(px(2.))
                .flex_none()
                .text_color(color),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(11.))
                .line_height(px(16.))
                .text_color(color)
                .child(message),
        );
    if reduce_motion {
        row.into_any_element()
    } else {
        row.with_animation(
            animation,
            Animation::new(Duration::from_millis(155)).with_easing(ease_out_cubic),
            |el, d| el.opacity(d).top(px((1. - d) * 2.5)),
        )
        .into_any_element()
    }
}

/// Soft outer focus/error halo paired with `.border_1().border_color(rgb(color))`.
fn field_ring(color: u32) -> Vec<BoxShadow> {
    let mut ring = rgb(color);
    ring.a = 0.16;
    vec![BoxShadow {
        inset: false,
        color: ring.into(),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(2.5),
    }]
}

/// Elevation of the drop-down list, scaled for smooth open/close fades.
fn menu_shadow_faded(t: f32) -> Vec<BoxShadow> {
    let t = t.clamp(0., 1.);
    let t3 = t * t * t;
    vec![
        specular_rim(false, 0.9 * t),
        BoxShadow {
            inset: false,
            color: hsla(220. / 360., 0.20, 0.12, 0.10 * t3),
            offset: point(px(0.), px(0.)),
            blur_radius: px(0.),
            spread_radius: px(0.5),
        },
        BoxShadow {
            inset: false,
            color: hsla(222. / 360., 0.22, 0.08, 0.16 * t3),
            offset: point(px(0.), px(8.)),
            blur_radius: px(22.),
            spread_radius: px(-4.),
        },
    ]
}

/// The red capsule of a destructive action. The caller adds `on_click`.
fn danger_pill(id: &'static str, label: &'static str, enabled: bool) -> Stateful<Div> {
    h_flex()
        .id(id)
        .h(px(31.))
        .px(px(14.))
        .gap(px(6.))
        .flex_none()
        .items_center()
        .rounded(px(9999.))
        .bg(rgb(if enabled { DANGER } else { 0xEDC3C5 }))
        .when(enabled, |el| {
            el.cursor_pointer()
                .shadow(vec![
                    BoxShadow {
                        inset: true,
                        color: hsla(0., 0., 1., 0.20),
                        offset: point(px(0.), px(1.)),
                        blur_radius: px(0.),
                        spread_radius: px(0.),
                    },
                    BoxShadow {
                        inset: false,
                        color: hsla(358. / 360., 0.70, 0.40, 0.22),
                        offset: point(px(0.), px(3.)),
                        blur_radius: px(10.),
                        spread_radius: px(-1.),
                    },
                ])
                .hover(|s| s.bg(rgb(DANGER_HOVER)))
                .active(|s| s.bg(rgb(DANGER_ACTIVE)))
        })
        .child(
            Icon::new(AssetIconName::Trash)
                .size(px(12.5))
                .flex_none()
                .text_color(rgb(0xFFFFFF)),
        )
        .child(
            Label::new(label)
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(0xFFFFFF)),
        )
}

/// A labelled `#FFFFFF` text field inside the WebDAV sync card, with live
/// accent focus ring feedback.
///
/// Each field carries its own element id: every text editor has the same
/// internal id, and the accessibility node id is hashed from the chain of
/// enclosing ids. Without a unique id per field they collapse onto one node
/// and the toolkit panics with "Duplicate a11y node id" on a focus change.
fn form_field(
    key: &'static str,
    label: &'static str,
    input: &Entity<InputState>,
    password: bool,
    window: &Window,
    cx: &App,
) -> Stateful<Div> {
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    v_flex()
        .id(key)
        .gap(px(5.))
        .child(
            Label::new(label)
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(if focused { ACCENT } else { INK_SECONDARY })),
        )
        .child(
            div()
                .h(px(33.))
                .pl(px(3.))
                .pr(px(if password { 6. } else { 3. }))
                .flex()
                .items_center()
                .rounded(px(9.5))
                .bg(rgb(0xFFFFFF))
                .border_1()
                .border_color(if focused {
                    rgb(ACCENT).into()
                } else {
                    field_border_color()
                })
                .shadow(if focused {
                    field_ring(ACCENT)
                } else {
                    ios_field_shadow()
                })
                .child(div().flex_1().min_w_0().child({
                    let mut widget = Input::new(input)
                        .appearance(false)
                        .bordered(false)
                        .focus_bordered(false);
                    if password {
                        widget = widget.mask_toggle();
                    }
                    widget
                })),
        )
}

/// Split a `Combo` into individual keycap labels (`["Ctrl", "Shift", "N"]`),
/// preserving literal `"+"` if the bound key itself is `"+"`.
fn combo_keycaps(combo: &Combo) -> Vec<String> {
    let mut full = combo.to_string();
    let mut caps = Vec::with_capacity(4);
    for (held, name) in [
        (combo.ctrl, "Ctrl"),
        (combo.alt, "Alt"),
        (combo.shift, "Shift"),
        (combo.win, "Win"),
    ] {
        if held {
            caps.push(name.to_string());
            if let Some(rest) = full.strip_prefix(name).and_then(|s| s.strip_prefix('+')) {
                full = rest.to_string();
            }
        }
    }
    if !full.is_empty() {
        caps.push(full);
    }
    caps
}

/// Tactile miniature keycap badge (rendered with `div()`, never `Label::new`,
/// so repeated keycaps inside shortcut rows never collide in AccessKit).
fn keycap_chip(label: String) -> Div {
    div()
        .h(px(20.))
        .min_w(px(22.))
        .px(px(6.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .bg(rgb(0xF3F5F9))
        .border_1()
        .border_color(hsla(220. / 360., 0.20, 0.18, 0.11))
        .shadow(vec![BoxShadow {
            inset: false,
            color: hsla(220. / 360., 0.22, 0.14, 0.06),
            offset: point(px(0.), px(1.)),
            blur_radius: px(0.),
            spread_radius: px(0.),
        }])
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(INK_PRIMARY))
        .child(label)
}

fn delay_error_text(error: SyncDelayError, in_effect: &str) -> String {
    let what = match error {
        SyncDelayError::Empty => "请输入秒数（0.5 – 600）",
        SyncDelayError::NotANumber => "只能输入数字，例如 2.5",
        SyncDelayError::TooPrecise => "最多保留一位小数，例如 2.5",
        SyncDelayError::OutOfRange => "范围是 0.5 – 600 秒",
    };
    format!("{what}。这个值没有保存，仍按 {in_effect} 秒同步。")
}

impl AppState {
    // =========================================================================
    // The dialog
    // =========================================================================
    pub(crate) fn render_settings_modal(
        &mut self,
        progress: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let progress = progress.clamp(0., 1.);
        let reduce_motion = cx.reduce_motion();
        let page = self.settings_page;

        let (title, subtitle, body_id) = match page {
            SettingsPage::General => (
                "通用",
                "启动、窗口行为、同步与数据存储",
                "settings-body-general",
            ),
            SettingsPage::Shortcuts => (
                "快捷键",
                "点击按键框，再按下新的组合键",
                "settings-body-shortcuts",
            ),
            SettingsPage::About => ("关于", "版本与开源信息", "settings-body-about"),
        };
        let content = match page {
            SettingsPage::General => self.render_general_page(window, cx).into_any_element(),
            SettingsPage::Shortcuts => self.render_shortcuts_page(cx).into_any_element(),
            SettingsPage::About => self.render_about_page(cx).into_any_element(),
        };
        // Crisp tab-switch settle: animate ONLY when `settings_page_seq > 0`
        // (i.e. the user actually switched tabs inside Settings, never on initial
        // modal open), with a high opacity floor (`0.65 -> 1.0`) and subtle `4px`
        // directional settle over `135ms` so the right pane never flashes blank.
        let page_dir = self.settings_page_dir;
        let page_seq = self.settings_page_seq;
        let content = if reduce_motion || page_seq == 0 {
            content
        } else {
            div()
                .relative()
                .child(content)
                .with_animation(
                    ("settings-page", page_seq),
                    Animation::new(Duration::from_millis(135)).with_easing(ease_out_cubic),
                    move |el, d| {
                        el.opacity(0.65 + 0.35 * d)
                            .top(px((1. - d) * 4. * page_dir))
                    },
                )
                .into_any_element()
        };

        // Shared critically-damped (`damping = 1.0`, zero bounce/overshoot)
        // `#FFFFFF` indicator pill gliding cleanly across the 3 nav items
        // (`34px` row height + `4px` gap = `38px` stride), with a subtle left
        // accent bar inside.
        let pill_y = spring(
            "settings-nav-pill-y",
            (page.index() as f32) * 38.0,
            Spring::new(Duration::from_millis(150)).with_epsilon(0.05),
            window,
            cx,
        );
        let nav_items = v_flex()
            .relative()
            .gap(px(4.))
            .child(
                div()
                    .absolute()
                    .left(px(0.))
                    .right(px(0.))
                    .top(px(if reduce_motion {
                        (page.index() as f32) * 38.0
                    } else {
                        pill_y
                    }))
                    .h(px(34.))
                    .rounded(px(10.))
                    .bg(rgb(0xFFFFFF))
                    .border_1()
                    .border_color(hsla(220. / 360., 0.20, 0.16, 0.07))
                    .shadow(nav_pill_shadow_faded(1.0))
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .ml(px(4.))
                            .w(px(2.5))
                            .h(px(14.))
                            .rounded_full()
                            .bg(rgb(ACCENT)),
                    ),
            )
            .child(self.settings_nav_item(
                "settings-nav-general",
                AssetIconName::Settings,
                "通用",
                SettingsPage::General,
                window,
                cx,
            ))
            .child(self.settings_nav_item(
                "settings-nav-shortcuts",
                AssetIconName::Keyboard,
                "快捷键",
                SettingsPage::Shortcuts,
                window,
                cx,
            ))
            .child(self.settings_nav_item(
                "settings-nav-about",
                AssetIconName::Info,
                "关于",
                SettingsPage::About,
                window,
                cx,
            ));

        let nav = v_flex()
            .id("settings-nav")
            .w(px(172.))
            .h_full()
            .flex_none()
            .rounded_l(px(22.))
            .bg(rgb(0xF7F8FA))
            .border_r_1()
            .border_color(hsla(220. / 360., 0.20, 0.16, 0.065))
            .px(px(11.))
            .pt(px(18.))
            .pb(px(14.))
            .child(
                div().pl(px(8.)).pb(px(12.)).child(
                    Label::new("设置")
                        .text_size(px(16.5))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(INK_PRIMARY)),
                ),
            )
            .child(nav_items)
            .child(div().flex_1())
            .child(
                div().pl(px(8.)).child(
                    Label::new(concat!("便签 v", env!("CARGO_PKG_VERSION")))
                        .text_size(px(10.5))
                        .text_color(rgb(INK_MUTED)),
                ),
            );

        let header_text = v_flex()
            .gap(px(2.))
            .child(
                Label::new(title)
                    .text_size(px(16.5))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(INK_PRIMARY)),
            )
            .child(
                Label::new(subtitle)
                    .text_size(px(11.))
                    .text_color(rgb(INK_MUTED)),
            );
        let header_title: AnyElement = if reduce_motion || page_seq == 0 {
            header_text.into_any_element()
        } else {
            header_text
                .relative()
                .with_animation(
                    ("settings-header", page_seq),
                    Animation::new(Duration::from_millis(135)).with_easing(ease_out_cubic),
                    move |el, d| {
                        el.opacity(0.68 + 0.32 * d)
                            .top(px((1. - d) * 2.5 * page_dir))
                    },
                )
                .into_any_element()
        };

        let header = h_flex()
            .pl(px(24.))
            .pr(px(16.))
            .pb(px(12.))
            .border_b_1()
            .border_color(hsla(220. / 360., 0.20, 0.16, 0.055))
            .justify_between()
            .items_center()
            .child(header_title)
            .child(
                div()
                    .id("btn-settings-close")
                    .size(px(28.))
                    .flex()
                    .flex_none()
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
                    .on_click(cx.listener(|this, _, window, cx| this.close_settings(window, cx))),
            );

        let right = v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .pt(px(16.))
            .child(header)
            .child(
                // The padding lives inside the scrolling body, not on the
                // card: the scroll area clips to its own box, and controls at
                // its very edge would lose their soft shadows there.
                div()
                    .id(body_id)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.settings_scroll)
                    // The drop-down list hangs from its button; once the
                    // page scrolls it would drift away with it.
                    .on_scroll(cx.listener(|this, _: &ScrollEvent, _, cx| {
                        if this.settings_menu.take().is_some() {
                            cx.notify();
                        }
                    }))
                    .child(
                        div()
                            .pl(px(24.))
                            .pr(px(22.))
                            .pt(px(14.))
                            .pb(px(22.))
                            .child(content),
                    ),
            );

        // Never larger than the window: the page scrolls instead.
        // Interpolate subtle centered expansion (`744->760` × `536->548`) +
        // `14px` upward settle so the dialog blooms smoothly into focus.
        let viewport = window.viewport_size();
        let max_w = px((viewport.width.as_f32() - 32.).max(420.));
        let max_h = px((viewport.height.as_f32() - TITLEBAR_H.as_f32() - 28.).max(300.));
        let (card_w, card_h, card_top) = if reduce_motion {
            (px(760.), px(548.), px(0.))
        } else {
            (
                px(744. + 16. * progress),
                px(536. + 12. * progress),
                px((1. - progress) * 14.),
            )
        };
        let card = h_flex()
            .id("settings-card")
            .w(card_w)
            .h(card_h)
            .max_w(max_w)
            .max_h(max_h)
            .relative()
            .top(card_top)
            .opacity(progress)
            .rounded(px(22.))
            .bg(rgb(0xFFFFFF))
            .shadow(modal_surface_shadow_faded(progress))
            // The page inside can grow past the card (it did once its sync
            // group gained rows). Whatever the flex chain decides, nothing may
            // draw or be clickable outside the card: the card is the frame,
            // and its scrolling body is the only thing that moves.
            .overflow_hidden()
            // Key presses go to the dialog, not to the note behind it.
            .track_focus(&self.modal_focus)
            // A click inside the dialog never reaches the layer that closes
            // it. It does end whatever was waiting for one: the open
            // drop-down list, a shortcut being recorded.
            .on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                if this.settings_menu.take().is_some() {
                    cx.notify();
                }
                this.cancel_shortcut_recording(cx);
            }))
            .child(nav)
            .child(right);

        modal_layers(
            "settings-backdrop",
            "settings-shield",
            0.24 * progress,
            card,
            cx.listener(|this, _, window, cx| {
                if this.show_settings {
                    this.close_settings(window, cx);
                }
            }),
        )
    }

    /// One entry of the dialog's left column (resting over the shared
    /// spring-gliding `#FFFFFF` indicator pill).
    fn settings_nav_item(
        &self,
        id: &'static str,
        icon: AssetIconName,
        label: &'static str,
        target: SettingsPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let selected = self.settings_page == target;
        let t = transition(
            (id, "selected"),
            if selected { 1.0_f32 } else { 0.0_f32 },
            Transition::new(Duration::from_millis(135)),
            window,
            cx,
        );
        h_flex()
            .id(id)
            .relative()
            .h(px(34.))
            .pl(px(14.))
            .pr(px(10.))
            .gap(px(9.))
            .items_center()
            .rounded(px(10.))
            .cursor_pointer()
            .when(!selected, |el| {
                el.hover(|s| s.bg(shell_hover()))
                    .active(|s| s.bg(shell_active()))
            })
            .child(
                Icon::new(icon)
                    .size(px(14.5))
                    .flex_none()
                    .text_color(lerp_rgba(rgb(INK_MUTED), rgb(ACCENT), t)),
            )
            .child(
                Label::new(label)
                    .text_size(px(12.5))
                    .font_weight(if t > 0.5 {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(lerp_rgba(rgb(INK_SECONDARY), rgb(INK_PRIMARY), t)),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.set_settings_page(target, window, cx)
            }))
    }

    // =========================================================================
    // 通用
    // =========================================================================
    fn render_general_page(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        v_flex()
            .gap(px(18.))
            .child(
                v_flex()
                    .gap(px(7.))
                    .child(section_title("启动与窗口"))
                    .child(self.render_window_group(window, cx)),
            )
            .child(
                v_flex()
                    .gap(px(7.))
                    .child(section_title("同步"))
                    .child(self.render_sync_group(window, cx)),
            )
            .child(
                v_flex()
                    .gap(px(7.))
                    .child(section_title("数据存储"))
                    .child(self.render_storage_group(cx)),
            )
            .child(
                v_flex()
                    .gap(px(7.))
                    .child(section_title("危险操作"))
                    .child(self.render_danger_group(cx)),
            )
    }

    /// Starting with Windows, what closing the window does, and the way out
    /// of the app.
    fn render_window_group(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let tray = self.tray_available();
        let behavior = if tray {
            self.config.close_behavior
        } else {
            // Without an icon to come back from, closing can only quit.
            CloseBehavior::Quit
        };
        let (current, detail) = match behavior {
            CloseBehavior::Tray => (
                "留在托盘",
                "关闭后留在托盘并继续同步；左键点托盘图标快速记事，右键打开菜单。",
            ),
            CloseBehavior::Quit if tray => ("直接退出", "关闭窗口即退出便签，后台同步随之停止。"),
            CloseBehavior::Quit => (
                "直接退出",
                "当前无法使用系统托盘图标，关闭窗口时会直接退出。",
            ),
        };

        let open = self.settings_menu == Some(SettingsMenu::CloseBehavior) && tray;
        let chevron_rot = spring(
            "close-chevron-rot",
            if open { 0.5_f32 } else { 0.0_f32 },
            Spring::new(Duration::from_millis(190))
                .with_damping(0.78)
                .with_epsilon(0.002),
            window,
            cx,
        );
        let menu_transition = if open {
            Transition::new(Duration::from_millis(165)).ease(cubic_bezier(0.16, 1.0, 0.3, 1.0))
        } else {
            Transition::new(Duration::from_millis(115)).ease(ease_out_cubic)
        };
        let menu_presence = Presence::new("close-behavior-menu-presence", open)
            .transition(menu_transition)
            .sample(window, cx);

        let option = |id: &'static str, label: &'static str, value: CloseBehavior| {
            let selected = behavior == value;
            h_flex()
                .id(id)
                .h(px(32.))
                .px(px(9.))
                .gap(px(7.))
                .items_center()
                .rounded(px(8.))
                .cursor_pointer()
                .when(selected, |el| el.bg(hsla(212. / 360., 0.52, 0.48, 0.08)))
                .hover(|s| s.bg(shell_hover()))
                .active(|s| s.bg(shell_active()))
                .child(
                    div()
                        .w(px(13.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(selected, |el| {
                            el.child(
                                Icon::new(IconName::Check)
                                    .size(px(12.))
                                    .text_color(rgb(ACCENT)),
                            )
                        }),
                )
                .child(
                    Label::new(label)
                        .text_size(px(12.))
                        .font_weight(if selected {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(rgb(if selected { ACCENT } else { INK_PRIMARY })),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.set_close_behavior(value, cx);
                }))
        };

        let dropdown = div()
            .relative()
            .flex_none()
            .child(
                h_flex()
                    .id("close-behavior")
                    .h(px(30.))
                    .pl(px(12.))
                    .pr(px(9.))
                    .gap(px(7.))
                    .items_center()
                    .rounded(px(9.))
                    .bg(rgb(0xFFFFFF))
                    .border_1()
                    .border_color(if open {
                        rgb(ACCENT).into()
                    } else {
                        field_border_color()
                    })
                    .shadow(if open {
                        field_ring(ACCENT)
                    } else {
                        ios_field_shadow()
                    })
                    .when(tray, |el| {
                        el.cursor_pointer()
                            .hover(|s| s.bg(rgb(0xFAFBFD)))
                            .active(|s| s.bg(rgb(0xECEEF4)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.cancel_shortcut_recording(cx);
                                this.settings_menu =
                                    match this.settings_menu {
                                        Some(SettingsMenu::CloseBehavior) => None,
                                        _ => Some(SettingsMenu::CloseBehavior),
                                    };
                                cx.notify();
                            }))
                    })
                    .child(
                        Label::new(current)
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(if tray { INK_PRIMARY } else { INK_MUTED })),
                    )
                    .child(
                        Icon::new(AssetIconName::ChevronDown)
                            .size(px(12.))
                            .flex_none()
                            .rotate(percentage(chevron_rot.rem_euclid(1.0)))
                            .text_color(rgb(if open { ACCENT } else { INK_MUTED })),
                    ),
            )
            .when(menu_presence.should_render(), |el| {
                let mp = menu_presence.progress;
                // Drawn after everything else, so neither the scrolling page
                // nor the groups below can cover or clip it.
                el.child(
                    deferred(
                        v_flex()
                            .id("close-behavior-menu")
                            .absolute()
                            .top(px(29. + 6. * mp))
                            .right(px(0.))
                            .w(px(148.))
                            .p(px(4.))
                            .gap(px(2.))
                            .opacity(mp)
                            .rounded(px(12.))
                            .bg(rgb(0xFFFFFF))
                            .border_1()
                            .border_color(hsla(220. / 360., 0.20, 0.16, 0.10))
                            .shadow(menu_shadow_faded(mp))
                            .occlude()
                            .child(option("close-option-tray", "留在托盘", CloseBehavior::Tray))
                            .child(option("close-option-quit", "直接退出", CloseBehavior::Quit)),
                    )
                    .with_priority(1),
                )
            });

        let autostart_detail = match behavior {
            CloseBehavior::Tray => "登录 Windows 后自动启动，直接留在托盘，不打开窗口。",
            CloseBehavior::Quit if tray => {
                "登录 Windows 后自动启动并打开窗口；“关闭窗口时”选“留在托盘”后，改为直接留在托盘。"
            }
            CloseBehavior::Quit => "登录 Windows 后自动启动并打开窗口。",
        };

        group_card()
            .child(
                v_flex()
                    .py(px(12.))
                    .gap(px(7.))
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .gap_3()
                            .child(setting_text("开机自启动", autostart_detail))
                            .child(
                                settings_switch(
                                    "sw-autostart",
                                    self.autostart,
                                    rgb(ACCENT),
                                    window,
                                    cx,
                                )
                                .on_click(cx.listener(|this, _, _window, cx| {
                                    let on = !this.autostart;
                                    this.set_autostart(on, cx);
                                })),
                            ),
                    )
                    .when_some(self.autostart_error.clone(), |el, message| {
                        el.child(error_line(message).id("settings-autostart-error"))
                    }),
            )
            .child(row_divider())
            .child(
                h_flex()
                    .py(px(12.))
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .child(setting_text("关闭窗口时", detail))
                    .child(dropdown),
            )
            .child(row_divider())
            .child(
                h_flex()
                    .py(px(12.))
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .child(setting_text(
                        "退出应用",
                        "完全退出便签，托盘图标和后台同步也一并结束。",
                    ))
                    .child(
                        settings_pill("btn-quit-app", "退出应用", true).on_click(cx.listener(
                            |this, _, window, cx| this.quit_app(window, cx),
                        )),
                    ),
            )
    }

    /// Automatic sync, debounce delay, pull interval, WebDAV credentials, and
    /// test/sync actions unified inside one snow-porcelain card.
    fn render_sync_group(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let in_effect = config::format_sync_delay_secs(self.config.sync_delay_ms);
        let refused = self.delay_error;
        let interval_secs = config::clamp_pull_interval_secs(self.config.pull_interval_secs);

        let mut delay_row = h_flex()
            .justify_between()
            .items_center()
            .gap_3()
            .child(setting_text(
                "编辑后上传延迟",
                "停止编辑后等待这么久再上传。范围 0.5 – 600 秒，可带一位小数，默认 2.5 秒。",
            ));
        if let Some(inputs) = &self.settings {
            let delay_focused = inputs.delay.read(cx).focus_handle(cx).is_focused(window);
            delay_row = delay_row.child(
                h_flex()
                    .flex_none()
                    .items_center()
                    .gap(px(7.))
                    .child(
                        div()
                            .id("settings-delay")
                            .w(px(78.))
                            .h(px(30.))
                            .flex()
                            .items_center()
                            .rounded(px(9.))
                            .bg(rgb(0xFFFFFF))
                            .border_1()
                            .border_color(if refused.is_some() {
                                rgb(DANGER).into()
                            } else if delay_focused {
                                rgb(ACCENT).into()
                            } else {
                                field_border_color()
                            })
                            .shadow(if refused.is_some() {
                                field_ring(DANGER)
                            } else if delay_focused {
                                field_ring(ACCENT)
                            } else {
                                ios_field_shadow()
                            })
                            .child(
                                div().flex_1().min_w_0().child(
                                    Input::new(&inputs.delay)
                                        .appearance(false)
                                        .bordered(false)
                                        .focus_bordered(false),
                                ),
                            ),
                    )
                    .child(
                        Label::new("秒")
                            .text_size(px(12.))
                            .text_color(rgb(INK_SECONDARY)),
                    ),
            );
        }

        group_card()
            .child(
                h_flex()
                    .py(px(12.))
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .child(setting_text(
                        "自动同步（WebDAV）",
                        format!(
                            "编辑后约 {in_effect} 秒上传；每 {} 检查其他设备的更新，回到窗口时立即检查",
                            config::format_pull_interval(interval_secs)
                        ),
                    ))
                    .child(
                        settings_switch(
                            "sw-webdav",
                            self.config.webdav_auto,
                            rgb(ACCENT),
                            window,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _window, cx| {
                            let on = !this.config.webdav_auto;
                            this.set_webdav_auto(on, cx);
                        })),
                    ),
            )
            .child(row_divider())
            .child(self.render_pull_interval_row(interval_secs, window, cx))
            .child(row_divider())
            .child(
                v_flex()
                    .py(px(12.))
                    .gap(px(7.))
                    .child(delay_row)
                    .when_some(refused, |el, error| {
                        el.child(
                            error_line(delay_error_text(error, &in_effect)).id("settings-delay-error"),
                        )
                    }),
            )
            .child(row_divider())
            .child(self.render_webdav_form(window, cx))
    }

    /// "检查其他设备更新": how often the background loop looks at the server.
    fn render_pull_interval_row(
        &self,
        interval_secs: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let open = self.settings_menu == Some(SettingsMenu::PullInterval);
        let chevron_rot = spring(
            "pull-chevron-rot",
            if open { 0.5_f32 } else { 0.0_f32 },
            Spring::new(Duration::from_millis(190))
                .with_damping(0.78)
                .with_epsilon(0.002),
            window,
            cx,
        );
        let menu_transition = if open {
            Transition::new(Duration::from_millis(165)).ease(cubic_bezier(0.16, 1.0, 0.3, 1.0))
        } else {
            Transition::new(Duration::from_millis(115)).ease(ease_out_cubic)
        };
        let menu_presence = Presence::new("pull-interval-menu-presence", open)
            .transition(menu_transition)
            .sample(window, cx);

        let mut menu = v_flex()
            .id("pull-interval-menu")
            .absolute()
            .top(px(29. + 6. * menu_presence.progress))
            .right(px(0.))
            .w(px(148.))
            .p(px(4.))
            .gap(px(2.))
            .opacity(menu_presence.progress)
            .rounded(px(12.))
            .bg(rgb(0xFFFFFF))
            .border_1()
            .border_color(hsla(220. / 360., 0.20, 0.16, 0.10))
            .shadow(menu_shadow_faded(menu_presence.progress))
            .occlude();
        for secs in config::PULL_INTERVAL_CHOICES_SECS {
            let selected = interval_secs == secs;
            let label = config::format_pull_interval(secs);
            menu = menu.child(
                h_flex()
                    .id(ElementId::Name(format!("pull-option-{secs}").into()))
                    .h(px(32.))
                    .px(px(9.))
                    .gap(px(7.))
                    .items_center()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .when(selected, |el| el.bg(hsla(212. / 360., 0.52, 0.48, 0.08)))
                    .hover(|s| s.bg(shell_hover()))
                    .active(|s| s.bg(shell_active()))
                    .child(
                        div()
                            .w(px(13.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(selected, |el| {
                                el.child(
                                    Icon::new(IconName::Check)
                                        .size(px(12.))
                                        .text_color(rgb(ACCENT)),
                                )
                            }),
                    )
                    .child(
                        Label::new(label)
                            .text_size(px(12.))
                            .font_weight(if selected {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(rgb(if selected { ACCENT } else { INK_PRIMARY })),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.set_pull_interval(secs, cx);
                    })),
            );
        }

        let dropdown = div()
            .relative()
            .flex_none()
            .child(
                h_flex()
                    .id("pull-interval")
                    .h(px(30.))
                    .pl(px(12.))
                    .pr(px(9.))
                    .gap(px(7.))
                    .items_center()
                    .rounded(px(9.))
                    .bg(rgb(0xFFFFFF))
                    .border_1()
                    .border_color(if open {
                        rgb(ACCENT).into()
                    } else {
                        field_border_color()
                    })
                    .shadow(if open {
                        field_ring(ACCENT)
                    } else {
                        ios_field_shadow()
                    })
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(0xFAFBFD)))
                    .active(|s| s.bg(rgb(0xECEEF4)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.cancel_shortcut_recording(cx);
                        this.settings_menu = match this.settings_menu {
                            Some(SettingsMenu::PullInterval) => None,
                            _ => Some(SettingsMenu::PullInterval),
                        };
                        cx.notify();
                    }))
                    .child(
                        Label::new(config::format_pull_interval(interval_secs))
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(INK_PRIMARY)),
                    )
                    .child(
                        Icon::new(AssetIconName::ChevronDown)
                            .size(px(12.))
                            .flex_none()
                            .rotate(percentage(chevron_rot.rem_euclid(1.0)))
                            .text_color(rgb(if open { ACCENT } else { INK_MUTED })),
                    ),
            )
            .when(menu_presence.should_render(), |el| {
                el.child(deferred(menu).with_priority(1))
            });

        h_flex()
            .py(px(12.))
            .justify_between()
            .items_center()
            .gap_3()
            .child(setting_text(
                "检查其他设备更新",
                "后台按此间隔查看服务器上的改动；窗口藏在托盘时放慢到至少 10 分钟。",
            ))
            .child(dropdown)
    }

    /// Server, credentials and the two buttons that try them out.
    fn render_webdav_form(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let reduce_motion = cx.reduce_motion();
        let text = rgb(INK_PRIMARY);
        let muted = rgb(INK_MUTED);

        let test_status: Option<(Rgba, Option<IconName>, String)> = match &self.webdav_test {
            WebDavTest::Idle => None,
            WebDavTest::Testing => Some((muted, None, "正在连接…".to_string())),
            WebDavTest::Ok(msg) => Some((rgb(SUCCESS_INK), Some(IconName::Check), msg.clone())),
            WebDavTest::Err(msg) => Some((rgb(ERROR_INK), Some(IconName::Close), msg.clone())),
        };
        let testing = self.webdav_test == WebDavTest::Testing;
        // A quiet background pull is none of this button's business.
        let syncing = self.webdav_syncing && !self.webdav_quiet;

        let mut form = v_flex().py(px(13.)).gap(px(11.));
        if let Some(inputs) = &self.settings {
            form = form
                .child(form_field(
                    "settings-url",
                    "服务器地址（URL）",
                    &inputs.url,
                    false,
                    window,
                    cx,
                ))
                .child(
                    h_flex()
                        .items_start()
                        .gap(px(11.))
                        .child(
                            form_field("settings-user", "用户名", &inputs.user, false, window, cx)
                                .flex_1()
                                .min_w_0(),
                        )
                        .child(
                            form_field("settings-pass", "密码", &inputs.pass, true, window, cx)
                                .flex_1()
                                .min_w_0(),
                        ),
                )
                .child(form_field(
                    "settings-dir",
                    "远端同步目录",
                    &inputs.dir,
                    false,
                    window,
                    cx,
                ));
        }

        let mut actions = h_flex()
            .pt(px(2.))
            .items_center()
            .gap_2p5()
            .flex_wrap()
            .child(
                h_flex()
                    .id("btn-webdav-test")
                    .h(px(30.))
                    .px_4()
                    .gap_1p5()
                    .items_center()
                    .rounded(px(9999.))
                    .bg(if testing {
                        rgb(SHELL_BG)
                    } else {
                        rgb(PRIMARY_BTN)
                    })
                    .when(!testing, |el| {
                        el.shadow(primary_pill_shadow())
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(PRIMARY_BTN_HOVER)))
                            .active(|s| {
                                s.bg(rgb(PRIMARY_BTN_ACTIVE))
                                    .shadow(primary_pill_shadow_pressed())
                            })
                            .on_click(cx.listener(|this, _, window, cx| this.test_webdav(window, cx)))
                    })
                    .when(testing, |el| {
                        el.child(spinning_icon(
                            "webdav-test-spin",
                            AssetIconName::RefreshCw,
                            px(11.5),
                            muted,
                            reduce_motion,
                        ))
                    })
                    .child(
                        Label::new(if testing { "测试中…" } else { "测试连接" })
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if testing { muted } else { rgb(0xFFFFFF) }),
                    ),
            )
            .child(
                h_flex()
                    .id("btn-webdav-sync")
                    .h(px(30.))
                    .px_4()
                    .gap_1p5()
                    .items_center()
                    .rounded(px(9999.))
                    .bg(rgb(if syncing { 0xF1F3F7 } else { 0xFFFFFF }))
                    .border_1()
                    .border_color(field_border_color())
                    .when(!syncing, |el| {
                        el.shadow(ios_field_shadow())
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0xFAFBFD)))
                            .active(|s| s.bg(rgb(0xECEEF4)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_webdav_sync(window, cx, SyncPass::Forced)
                            }))
                    })
                    .when(syncing, |el| {
                        el.child(spinning_icon(
                            "webdav-sync-spin",
                            AssetIconName::RefreshCw,
                            px(11.5),
                            muted,
                            reduce_motion,
                        ))
                    })
                    .child(
                        Label::new(if syncing { "同步中…" } else { "立即同步" })
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if syncing { muted } else { text }),
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
            // Same reading as the test line above: green when it worked, red
            // when it failed, muted while it is still going.
            let (color, maybe_icon) = if msg.starts_with("同步完成") {
                (rgb(SUCCESS_INK), Some(IconName::Check))
            } else if msg.starts_with("同步失败") {
                (rgb(ERROR_INK), Some(IconName::Close))
            } else {
                (muted, None)
            };
            actions = actions.child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .when_some(maybe_icon, |el, ic| {
                        el.child(Icon::new(ic).size(px(11.)).text_color(color))
                    })
                    .child(Label::new(msg.clone()).text_size(px(11.)).text_color(color)),
            );
        }

        form.child(actions).child(
            Label::new(
                "每条便签保存为 <id>.md，带 YAML Frontmatter，可在任意 Markdown 编辑器中阅读。",
            )
            .text_size(px(10.5))
            .text_color(muted),
        )
    }

    /// Where the database lives, and the import from Windows Sticky Notes.
    fn render_storage_group(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let reduce_motion = cx.reduce_motion();
        let muted = rgb(INK_MUTED);
        let default_location = self.config.uses_default_data_dir();
        let picking = self.picking_folder;

        let mut storage = group_card()
            .id("settings-storage")
            .py(px(12.))
            .gap(px(9.))
            .child(
                v_flex()
                    .gap(px(3.))
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .gap_3()
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(
                                        Icon::new(AssetIconName::Database)
                                            .size(px(13.))
                                            .flex_none()
                                            .text_color(rgb(ACCENT)),
                                    )
                                    .child(
                                        Label::new("便签数据库")
                                            .text_size(px(12.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(rgb(INK_PRIMARY)),
                                    ),
                            )
                            .child(
                                Label::new(format!(
                                    "共 {} 条便签 · {}",
                                    self.notes.len(),
                                    if default_location {
                                        "默认位置"
                                    } else {
                                        "自定义位置"
                                    }
                                ))
                                .text_size(px(11.))
                                .flex_none()
                                .text_color(muted),
                            ),
                    )
                    // The full path, wrapping instead of truncating: this is
                    // the one place that says where the notes actually are.
                    .child(
                        div()
                            .id("settings-db-path")
                            .w_full()
                            .text_size(px(11.5))
                            .line_height(px(17.))
                            .text_color(rgb(INK_SECONDARY))
                            .child(self.database_path().display().to_string()),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        settings_pill(
                            "btn-db-move",
                            if picking { "正在选择…" } else { "更改位置…" },
                            !picking,
                        )
                        .when(!picking, |el| {
                            el.on_click(cx.listener(|this, _, window, cx| {
                                this.choose_data_folder(window, cx)
                            }))
                        }),
                    )
                    .child(settings_pill("btn-db-reveal", "打开文件夹", true).on_click(
                        cx.listener(|this, _, _window, cx| this.reveal_database(cx)),
                    ))
                    .when(!default_location, |el| {
                        el.child(
                            settings_pill("btn-db-default", "恢复默认", !picking).when(
                                !picking,
                                |el| {
                                    el.on_click(cx.listener(|this, _, window, cx| {
                                        this.restore_default_data_folder(window, cx)
                                    }))
                                },
                            ),
                        )
                    })
                    .child(settings_pill("btn-db-backups", "备份文件夹", true).on_click(
                        cx.listener(|this, _, _window, cx| this.reveal_backups(cx)),
                    ))
                    // Location actions sit left, the import on the right.
                    .child(div().flex_1())
                    .child(
                        settings_pill("btn-import-windows", "导入 Windows 便签", true).on_click(
                            cx.listener(|this, _, window, cx| {
                                this.import_from_windows(window, cx)
                            }),
                        ),
                    ),
            )
            .child(
                Label::new("导入只读取 Windows 便签并逐条合并，不会改动它，也不会删除这里的便签。")
                    .text_size(px(11.))
                    .text_color(muted),
            );

        if let Some(dir) = &self.pending_adopt {
            // The picked folder already holds a database. It is never
            // overwritten; switching to it needs an explicit yes.
            let adopt_box = v_flex()
                .id("settings-adopt")
                .relative()
                .gap(px(9.))
                .px(px(11.))
                .py(px(10.))
                .rounded(px(11.))
                .bg(rgb(0xFFF7E3))
                .border_1()
                .border_color(hsla(39. / 360., 0.65, 0.50, 0.25))
                .child(
                    div()
                        .text_size(px(11.5))
                        .line_height(px(17.))
                        .text_color(rgb(0x7A5A12))
                        .child(format!(
                            "“{}”里已经有一个便签数据库。要改用它吗？当前的数据库会留在原位置，不会被覆盖或删除。",
                            dir.display()
                        )),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(settings_pill("btn-adopt-yes", "使用该数据库", true).on_click(
                            cx.listener(|this, _, window, cx| this.confirm_adopt(window, cx)),
                        ))
                        .child(settings_pill("btn-adopt-no", "取消", true).on_click(
                            cx.listener(|this, _, _window, cx| this.cancel_adopt(cx)),
                        )),
                );
            storage = if reduce_motion {
                storage.child(adopt_box)
            } else {
                storage.child(adopt_box.with_animation(
                    "settings-adopt-enter",
                    Animation::new(Duration::from_millis(160)).with_easing(ease_out_cubic),
                    |el, d| el.opacity(d).top(px((1. - d) * 3.)),
                ))
            };
        } else if let Some(note) = &self.storage_note {
            storage = storage.child(outcome_note(
                "settings-storage-note",
                "settings-storage-note-enter",
                note,
                reduce_motion,
            ));
        }
        storage
    }

    /// "Clear all notes": the red button; the question comes afterwards.
    fn render_danger_group(&self, cx: &mut Context<Self>) -> Div {
        let reduce_motion = cx.reduce_motion();
        let has_notes = !self.notes.is_empty();
        let detail = if self.config.webdav_configured() {
            "删除所有便签，标签会保留。清除前自动备份数据库；云端和其他设备上的便签也会随同步被清除。"
        } else {
            "删除所有便签，标签会保留。清除前自动备份数据库。"
        };
        danger_group_card()
            .py(px(12.))
            .gap(px(9.))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .child(setting_text("清除全部便签", detail))
                    .child(
                        danger_pill("btn-clear-all", "清除全部便签", has_notes).when(
                            has_notes,
                            |el| {
                                el.on_click(cx.listener(|this, _, window, cx| {
                                    this.request_clear_all(window, cx)
                                }))
                            },
                        ),
                    ),
            )
            .when_some(self.danger_note.as_ref(), |el, note| {
                el.child(outcome_note(
                    "settings-danger-note",
                    "settings-danger-note-enter",
                    note,
                    reduce_motion,
                ))
            })
    }

    // =========================================================================
    // 快捷键
    // =========================================================================
    fn render_shortcuts_page(&self, cx: &mut Context<Self>) -> Div {
        let mut page = v_flex().gap(px(16.)).child(
            div()
                .w_full()
                .text_size(px(11.5))
                .line_height(px(17.))
                .text_color(rgb(INK_MUTED))
                .child("修改后自动保存。录制时按 Esc 取消，按 Backspace 清除该快捷键。"),
        );

        let groups = [
            (Group::Global, "全局（便签运行时，在任何程序里都生效）"),
            (Group::General, "通用"),
            (Group::Markdown, "Markdown 格式（在正文编辑区生效）"),
        ];
        for (group, title) in groups {
            let mut card = group_card();
            for (index, def) in shortcuts::ACTIONS
                .iter()
                .filter(|def| def.group == group)
                .enumerate()
            {
                if index > 0 {
                    card = card.child(row_divider());
                }
                card = card.child(self.render_shortcut_row(def, cx));
            }
            page = page.child(
                v_flex()
                    .gap(px(7.))
                    .child(section_title(title))
                    .child(card),
            );
        }

        // Only the changes are stored, so "nothing stored" is "all default".
        let changed = !self.config.shortcuts.is_empty();
        page.child(
            h_flex().child(
                secondary_pill("btn-shortcuts-reset-all", "恢复默认快捷键", changed).when(
                    changed,
                    |el| {
                        el.on_click(cx.listener(|this, _, _, cx| this.reset_all_shortcuts(cx)))
                    },
                ),
            ),
        )
    }

    /// One action: its name, the tactile keycap recorder box, and a reset
    /// action shown only when modified so the list stays clean.
    fn render_shortcut_row(&self, def: &'static ActionDef, cx: &mut Context<Self>) -> Div {
        let action = def.id;
        let recording = self.recording_shortcut == Some(action);
        let is_default = self.keymap.is_default(action);
        let refused = self
            .shortcut_error
            .as_ref()
            .filter(|(owner, _)| *owner == action)
            .map(|(_, message)| message.clone())
            // The system-wide key is set, but Windows would not register it.
            .or_else(|| {
                let global = def.group == Group::Global && !recording;
                self.hotkey_problems
                    .get(action)
                    .filter(|_| global)
                    .map(|message| (*message).to_string())
            });

        let key_content: AnyElement = if recording {
            div()
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(ACCENT))
                .child("请按下快捷键…")
                .into_any_element()
        } else {
            match self.keymap.combo(action) {
                Some(combo) => {
                    let mut row = h_flex().items_center().gap(px(4.));
                    for (idx, cap) in combo_keycaps(combo).into_iter().enumerate() {
                        if idx > 0 {
                            row = row.child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(INK_MUTED))
                                    .child("+"),
                            );
                        }
                        row = row.child(keycap_chip(cap));
                    }
                    row.into_any_element()
                }
                None => div()
                    .text_size(px(11.5))
                    .text_color(rgb(INK_MUTED))
                    .child("未设置")
                    .into_any_element(),
            }
        };

        let recorder = div()
            .id(ElementId::Name(format!("shortcut-keys-{action}").into()))
            .min_w(px(148.))
            .h(px(30.))
            .px(px(10.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(9.))
            .cursor_pointer()
            .border_1()
            .map(|el| {
                if recording {
                    el.bg(rgb(0xEEF4FB))
                        .border_color(rgb(ACCENT))
                        .shadow(field_ring(ACCENT))
                } else {
                    el.bg(rgb(0xFFFFFF))
                        .border_color(field_border_color())
                        .shadow(ios_field_shadow())
                        .hover(|s| s.bg(rgb(0xFAFBFD)))
                        .active(|s| s.bg(rgb(0xECEEF4)))
                }
            })
            .child(key_content)
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.settings_menu = None;
                this.toggle_shortcut_recording(action, window, cx);
            }));

        v_flex()
            .py(px(7.))
            .gap(px(5.))
            .child(
                h_flex()
                    .min_h(px(30.))
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div().flex_1().min_w_0().child(
                            Label::new(def.label)
                                .id(ElementId::Name(format!("shortcut-name-{action}").into()))
                                .text_size(px(12.5))
                                .text_color(rgb(INK_PRIMARY)),
                        ),
                    )
                    .when(!is_default, |el| {
                        el.child(
                            div()
                                .id(ElementId::Name(format!("shortcut-reset-{action}").into()))
                                .h(px(26.))
                                .px(px(9.))
                                .flex()
                                .flex_none()
                                .items_center()
                                .justify_center()
                                .rounded(px(8.))
                                .cursor_pointer()
                                .bg(hsla(212. / 360., 0.52, 0.48, 0.08))
                                .hover(|s| s.bg(hsla(212. / 360., 0.52, 0.48, 0.14)))
                                .active(|s| s.bg(hsla(212. / 360., 0.52, 0.48, 0.20)))
                                .child(
                                    div()
                                        .text_size(px(11.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgb(ACCENT))
                                        .child("重置"),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.reset_shortcut(action, cx);
                                })),
                        )
                    })
                    .child(recorder),
            )
            .when_some(refused, |el, message| el.child(error_line(message)))
    }

    // =========================================================================
    // 关于
    // =========================================================================
    fn render_about_page(&self, cx: &mut Context<Self>) -> Div {
        let copied = self.about_copied;
        v_flex()
            .gap(px(18.))
            .child(
                v_flex()
                    .pt(px(16.))
                    .pb(px(4.))
                    .items_center()
                    .gap(px(5.))
                    .child(app_logo(px(62.)))
                    .child(
                        div().pt(px(6.)).child(
                            Label::new("便签 Sticky Notes")
                                .text_size(px(17.))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(INK_PRIMARY)),
                        ),
                    )
                    .child(
                        Label::new(concat!("版本 ", env!("CARGO_PKG_VERSION")))
                            .text_size(px(11.5))
                            .text_color(rgb(INK_MUTED)),
                    )
                    .child(
                        div().pt(px(4.)).child(
                            Label::new("轻量的本地便签：Markdown 编辑、标签整理、WebDAV 同步。")
                                .text_size(px(12.))
                                .text_color(rgb(INK_SECONDARY)),
                        ),
                    ),
            )
            .child(
                group_card().py(px(13.)).child(
                    h_flex()
                        .items_center()
                        .gap(px(12.))
                        .child(
                            div()
                                .size(px(38.))
                                .flex()
                                .flex_none()
                                .items_center()
                                .justify_center()
                                .rounded(px(11.))
                                .bg(rgb(0xFFFFFF))
                                .border_1()
                                .border_color(field_border_color())
                                .shadow(ios_field_shadow())
                                .child(
                                    Icon::new(AssetIconName::Github)
                                        .size(px(18.))
                                        .text_color(rgb(INK_PRIMARY)),
                                ),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap(px(2.))
                                .child(
                                    Label::new("开源地址")
                                        .text_size(px(12.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgb(INK_PRIMARY)),
                                )
                                .child(
                                    div()
                                        .id("about-repository")
                                        .w_full()
                                        .text_size(px(11.5))
                                        .line_height(px(17.))
                                        .text_color(rgb(ACCENT))
                                        .child(REPOSITORY_URL),
                                ),
                        )
                        .child(
                            settings_pill(
                                "btn-about-copy",
                                if copied { "已复制" } else { "复制" },
                                true,
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.copy_repository_url(window, cx)
                            })),
                        )
                        .child(
                            h_flex()
                                .id("btn-about-open")
                                .h(px(29.))
                                .px(px(13.))
                                .gap(px(5.))
                                .flex_none()
                                .items_center()
                                .rounded(px(9999.))
                                .bg(rgb(PRIMARY_BTN))
                                .shadow(primary_pill_shadow())
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(PRIMARY_BTN_HOVER)))
                                .active(|s| {
                                s.bg(rgb(PRIMARY_BTN_ACTIVE))
                                    .shadow(primary_pill_shadow_pressed())
                            })
                                .child(
                                    Label::new("打开")
                                        .text_size(px(12.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgb(0xFFFFFF)),
                                )
                                .child(
                                    Icon::new(AssetIconName::ExternalLink)
                                        .size(px(11.5))
                                        .flex_none()
                                        .text_color(rgb(0xFFFFFF)),
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.open_repository(cx))),
                        ),
                ),
            )
            .child(
                div()
                    .px(px(4.))
                    .w_full()
                    .text_size(px(11.))
                    .line_height(px(17.))
                    .text_color(rgb(INK_MUTED))
                    .child(
                        "便签保存在本机的数据库里；只有填写了 WebDAV 服务器，才会同步到你自己的服务器上。",
                    ),
            )
    }

    // =========================================================================
    // "Clear all notes": the second question
    // =========================================================================
    pub(crate) fn render_clear_confirm(
        &mut self,
        progress: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let progress = progress.clamp(0., 1.);
        let count = self.confirm_clear_count;
        let cloud = self.config.webdav_configured();
        let line = |text: &'static str, color: u32| {
            div()
                .w_full()
                .text_size(px(12.))
                .line_height(px(18.5))
                .text_color(rgb(color))
                .child(text)
        };

        let reduce_motion = cx.reduce_motion();
        let (card_w, card_top) = if reduce_motion {
            (px(412.), px(0.))
        } else {
            (px(402. + 10. * progress), px((1. - progress) * 12.))
        };
        let card = v_flex()
            .id("clear-confirm-card")
            .w(card_w)
            .relative()
            .top(card_top)
            .opacity(progress)
            .rounded(px(20.))
            .bg(rgb(0xFFFFFF))
            .shadow(modal_surface_shadow_faded(progress))
            .p(px(22.))
            .gap(px(13.))
            // Clicks inside the card must not reach the layer that cancels.
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                h_flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .size(px(34.))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(hsla(358. / 360., 0.75, 0.59, 0.13))
                            .child(
                                Icon::new(AssetIconName::TriangleAlert)
                                    .size(px(16.))
                                    .text_color(rgb(DANGER)),
                            ),
                    )
                    .child(
                        Label::new(format!("清除全部 {count} 条便签？"))
                            .text_size(px(15.5))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(INK_PRIMARY)),
                    ),
            )
            .child(
                v_flex()
                    .gap(px(5.))
                    .child(line(
                        "此操作不可撤销：所有便签都会被删除，标签会保留。",
                        INK_SECONDARY,
                    ))
                    .when(cloud, |el| {
                        el.child(line(
                            "已配置 WebDAV 同步：云端的便签文件也会被删除，其他设备同步后同样会被清空。",
                            ERROR_INK,
                        ))
                    })
                    .child(line(
                        "清除前会自动备份数据库，备份在“通用 › 备份文件夹”里。",
                        INK_SECONDARY,
                    )),
            )
            .child(
                h_flex()
                    .pt(px(3.))
                    .justify_end()
                    .items_center()
                    .gap_2()
                    .child(secondary_pill("btn-clear-cancel", "取消", true).on_click(
                        cx.listener(|this, _, window, cx| this.cancel_clear_all(window, cx)),
                    ))
                    .child(
                        danger_pill("btn-clear-confirm", "清除全部便签", true).on_click(
                            cx.listener(|this, _, window, cx| this.confirm_clear_all(window, cx)),
                        ),
                    ),
            );

        modal_layers(
            "clear-confirm-backdrop",
            "clear-confirm-shield",
            0.22 * progress,
            card,
            cx.listener(|this, _, window, cx| this.cancel_clear_all(window, cx)),
        )
    }

    // =========================================================================
    // The quick note as a card — when it has no window of its own
    // =========================================================================
    pub(crate) fn render_quick_overlay(
        &mut self,
        progress: f32,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let view = self
            .quick
            .as_ref()
            .filter(|host| host.window.is_none())
            .map(|host| host.view.clone())?;
        let progress = progress.clamp(0., 1.);
        let reduce_motion = cx.reduce_motion();
        let card = div()
            .id("quick-card")
            .w(QUICK_W)
            .h(QUICK_H)
            .relative()
            .top(px(if reduce_motion {
                0.
            } else {
                (1. - progress) * 12.
            }))
            .opacity(progress)
            .rounded(px(18.))
            .shadow(modal_surface_shadow_faded(progress))
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(view);
        Some(modal_layers(
            "quick-backdrop",
            "quick-shield",
            0.22 * progress,
            card,
            // A click beside the card is "done", like the button.
            cx.listener(|this, _, _, cx| this.quick_request(QuickAction::Close, cx)),
        ))
    }
}
