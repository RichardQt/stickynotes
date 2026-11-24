use crate::state::{md_command, AppState, MdTip, SyncStatus, ThemeSpec, THEMES};
use gpui_kit::assets::IconName as AssetIconName;
use gpui_kit::base::animation::{cubic_bezier, ease_out_cubic};
use gpui_kit::base::{
    h_flex, spring, transition, v_flex, ElementExt as _, Interpolate, MotionReveal, Presence,
    ReducedMotion, Sequence, Spring, Transition,
};
use gpui_kit::component::input::Input;
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
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use stickynotes_core::config::NoteSort;
use stickynotes_core::shortcuts;
use stickynotes_core::store::model::NoteSummary;
use stickynotes_core::ticks;

const SIDEBAR_W: Pixels = px(206.);
const LIST_PANE_W: Pixels = px(266.);
pub(crate) const TITLEBAR_H: Pixels = px(38.);

/// Note title type size and the line box that holds it. The line box must
/// clear the font's full ascent + descent (about 1.33em for the system UI
/// and CJK fallback fonts), or the input clips the glyphs.
const TITLE_TEXT_SIZE: Pixels = px(22.);
const TITLE_LINE_H: Pixels = px(32.);
/// Horizontal inset the markdown editor applies inside its own viewport.
/// The title field and its rule share it so all three left edges line up.
const EDITOR_TEXT_INSET: Rems = Rems(0.625);

// =========================================================================
// Unified Shell & Crisp Alabaster / Pure-White Surface Tokens
// =========================================================================

/// High-key neutral alabaster mist framing the titlebar, sidebar, and sheets.
pub(crate) const SHELL_BG: u32 = 0xF6F7F9;
/// Snow-porcelain surface for grouped cards inside white dialogs.
pub(crate) const GROUP_BG: u32 = 0xF8F9FB;
/// Primary ink for chrome, cards, and dialogs.
pub(crate) const INK_PRIMARY: u32 = 0x1A1D24;
/// Secondary ink for navigation labels and snippets.
pub(crate) const INK_SECONDARY: u32 = 0x525866;
/// Muted ink for counts, timestamps, and field labels. Dark enough for small
/// text on the shell (`#F6F7F9`: 4.67:1) and on white (5.01:1); the lighter
/// grey this used to be sat at 3.2–3.4:1, below the 4.5:1 AA line.
pub(crate) const INK_MUTED: u32 = 0x667080;
/// Primary button slate fill and states.
pub(crate) const PRIMARY_BTN: u32 = 0x1F2937;
pub(crate) const PRIMARY_BTN_HOVER: u32 = 0x2C3A4E;
pub(crate) const PRIMARY_BTN_ACTIVE: u32 = 0x151D28;

/// Translucent foreground hover fill on the neutral shell.
pub(crate) fn shell_hover() -> Hsla {
    hsla(220. / 360., 0.20, 0.18, 0.048)
}

/// Translucent foreground press/active fill on the neutral shell.
pub(crate) fn shell_active() -> Hsla {
    hsla(220. / 360., 0.22, 0.18, 0.082)
}

/// Ink overlay for the note's paper: translucent black, so one alpha ramp
/// works on every paper colour and always lets it show through instead of
/// painting an opaque grey block. Assumes a light paper, which every theme is.
pub(crate) fn ink(alpha: f32) -> Hsla {
    hsla(0., 0., 0., alpha)
}

/// Crisp capsule fill for floating toolbars on note paper:
/// clean snow-porcelain on `#FFFFFF` paper; luminous translucent white on
/// pastel papers so the tint breathes through.
fn paper_capsule_bg(is_white_paper: bool) -> Hsla {
    if is_white_paper {
        rgb(0xF4F6F9).into()
    } else {
        hsla(0., 0., 1., 0.82)
    }
}

/// RGB-space color wrapper so transitions between warm/cool pastel papers and
/// accent fills blend cleanly without sweeping intermediate hues in HSLA.
#[derive(Clone, Copy, PartialEq)]
struct RgbColor(Rgba);

impl Interpolate for RgbColor {
    fn interpolate(&self, target: &Self, t: f32) -> Self {
        Self(lerp_rgba(self.0, target.0, t))
    }
}

pub(crate) fn lerp_rgba(a: Rgba, b: Rgba, t: f32) -> Rgba {
    Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

/// Soft inner top highlight catching overhead light on elevated iOS surfaces.
pub(crate) fn specular_rim(is_dark: bool, alpha_light: f32) -> BoxShadow {
    BoxShadow {
        inset: true,
        color: hsla(0., 0., 1., if is_dark { 0.13 } else { alpha_light }),
        offset: point(px(0.), px(1.)),
        blur_radius: px(0.),
        spread_radius: px(0.),
    }
}

/// Crisp 1px border color for grouped porcelain cards on `#FFFFFF` surfaces.
pub(crate) fn group_border_color() -> Hsla {
    hsla(220. / 360., 0.20, 0.18, 0.085)
}

/// Crisp 1px border color for `#FFFFFF` controls/inputs inside grouped cards.
pub(crate) fn field_border_color() -> Hsla {
    hsla(220. / 360., 0.20, 0.18, 0.11)
}

/// Crisp floating sheet elevation for the note paper (`editor-pane`).
fn editor_paper_shadow(is_white_paper: bool) -> Vec<BoxShadow> {
    vec![BoxShadow {
        inset: false,
        color: hsla(
            220. / 360.,
            0.22,
            0.12,
            if is_white_paper { 0.045 } else { 0.036 },
        ),
        offset: point(px(0.), px(5.)),
        blur_radius: px(20.),
        spread_radius: px(-4.),
    }]
}

/// Crisp pure-white sheet elevation for the middle note-list column.
fn list_sheet_shadow() -> Vec<BoxShadow> {
    vec![BoxShadow {
        inset: false,
        color: hsla(220. / 360., 0.22, 0.12, 0.038),
        offset: point(px(0.), px(4.)),
        blur_radius: px(16.),
        spread_radius: px(-4.),
    }]
}

/// Feather-light elevation for toolbar & swatch capsules inside the note sheet.
fn paper_capsule_shadow(is_white_paper: bool) -> Vec<BoxShadow> {
    if is_white_paper {
        Vec::new()
    } else {
        vec![
            specular_rim(false, 0.88),
            BoxShadow {
                inset: false,
                color: hsla(220. / 360., 0.16, 0.14, 0.024),
                offset: point(px(0.), px(1.5)),
                blur_radius: px(5.),
                spread_radius: px(-0.5),
            },
        ]
    }
}

/// Lifted white pill for sidebar navigation items, scaled for cross-fades.
/// Outer shadow alpha uses `t^3` because GPUI draws non-inset `BoxShadow`
/// without cutting out the interior, which would otherwise bleed through a
/// semi-transparent white fill mid-transition.
pub(crate) fn nav_pill_shadow_faded(t: f32) -> Vec<BoxShadow> {
    let t = t.clamp(0., 1.);
    let t3 = t * t * t;
    vec![BoxShadow {
        inset: false,
        color: hsla(220. / 360., 0.22, 0.14, 0.045 * t3),
        offset: point(px(0.), px(1.5)),
        blur_radius: px(5.),
        spread_radius: px(-0.5),
    }]
}

/// Subtle ambient drop shadow for `#FFFFFF` controls (paired with `.border_1()`).
pub(crate) fn ios_field_shadow() -> Vec<BoxShadow> {
    ios_field_shadow_faded(1.)
}

fn ios_field_shadow_faded(t: f32) -> Vec<BoxShadow> {
    let t = t.clamp(0., 1.);
    let t3 = t * t * t;
    vec![BoxShadow {
        inset: false,
        color: hsla(220. / 360., 0.20, 0.14, 0.032 * t3),
        offset: point(px(0.), px(1.)),
        blur_radius: px(3.),
        spread_radius: px(0.),
    }]
}

/// Soft diffused elevation for primary dark-slate action capsules (no outer dark ring).
pub(crate) fn primary_pill_shadow() -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            inset: true,
            color: hsla(0., 0., 1., 0.16),
            offset: point(px(0.), px(1.)),
            blur_radius: px(0.),
            spread_radius: px(0.),
        },
        BoxShadow {
            inset: false,
            color: hsla(220. / 360., 0.28, 0.12, 0.11),
            offset: point(px(0.), px(3.)),
            blur_radius: px(10.),
            spread_radius: px(-1.),
        },
    ]
}

/// The same pill while pressed: the ambient lift collapses to a hairline, so
/// the button reads as sitting on the surface instead of floating over it.
pub(crate) fn primary_pill_shadow_pressed() -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            inset: true,
            color: hsla(0., 0., 1., 0.10),
            offset: point(px(0.), px(1.)),
            blur_radius: px(0.),
            spread_radius: px(0.),
        },
        BoxShadow {
            inset: false,
            color: hsla(220. / 360., 0.28, 0.12, 0.07),
            offset: point(px(0.), px(1.)),
            blur_radius: px(4.),
            spread_radius: px(-0.5),
        },
    ]
}

/// The selected note card lifts a little off the list sheet; resting cards
/// lie flat on it, drawn by their hairline alone.
fn note_card_shadow(t: f32) -> Vec<BoxShadow> {
    let t = t.clamp(0., 1.);
    if t <= 0.001 {
        return Vec::new();
    }
    vec![BoxShadow {
        inset: false,
        color: hsla(220. / 360., 0.20, 0.12, 0.055 * t),
        offset: point(px(0.), px(2. * t)),
        blur_radius: px(8.),
        spread_radius: px(-2.),
    }]
}

/// Floating modal sheet shadow with progressive depth bloom during arrival,
/// using `t^3` outer shadow scaling so blur ink never bleeds through the
/// translucent white card during enter/exit fades.
pub(crate) fn modal_surface_shadow_faded(t: f32) -> Vec<BoxShadow> {
    let t = t.clamp(0., 1.);
    let t3 = t * t * t;
    vec![
        specular_rim(false, 0.94 * t),
        BoxShadow {
            inset: false,
            color: hsla(220. / 360., 0.22, 0.12, 0.075 * t3),
            offset: point(px(0.), px(0.)),
            blur_radius: px(0.),
            spread_radius: px(0.5),
        },
        BoxShadow {
            inset: false,
            color: hsla(220. / 360., 0.24, 0.10, 0.06 * t3),
            offset: point(px(0.), px(3. + 3. * t)),
            blur_radius: px(12.),
            spread_radius: px(-2.),
        },
        BoxShadow {
            inset: false,
            color: hsla(222. / 360., 0.26, 0.08, 0.15 * t3),
            offset: point(px(0.), px(8. + 12. * t)),
            blur_radius: px(24. + 22. * t),
            spread_radius: px(-6.),
        },
    ]
}

/// Dark solid-glass tooltip shadow scaled for enter/exit fades.
fn tooltip_surface_shadow_faded(t: f32) -> Vec<BoxShadow> {
    let t = t.clamp(0., 1.);
    let t2 = t * t;
    let t3 = t2 * t;
    vec![
        BoxShadow {
            inset: true,
            color: hsla(0., 0., 1., 0.15 * t),
            offset: point(px(0.), px(0.5)),
            blur_radius: px(0.),
            spread_radius: px(0.5),
        },
        BoxShadow {
            inset: false,
            color: hsla(0., 0., 0., 0.44 * t2),
            offset: point(px(0.), px(0.)),
            blur_radius: px(0.),
            spread_radius: px(0.5),
        },
        BoxShadow {
            inset: false,
            color: hsla(0., 0., 0., 0.24 * t3),
            offset: point(px(0.), px(6.)),
            blur_radius: px(14.),
            spread_radius: px(-2.),
        },
    ]
}

/// The focus ring for keyboard navigation: 2px of accent around the control,
/// no transition (a ring that fades in is a ring the keyboard user waits for).
fn focus_ring() -> Vec<BoxShadow> {
    vec![BoxShadow {
        inset: false,
        color: hsla(212. / 360., 0.72, 0.45, 0.55),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(2.),
    }]
}

/// Soft bead highlight + subtle diffuse lift for an active/hovered colour swatch.
fn lifted_swatch_shadow_faded(t: f32) -> Vec<BoxShadow> {
    let t = t.clamp(0., 1.);
    vec![
        BoxShadow {
            inset: true,
            color: hsla(0., 0., 1., 0.28 + 0.16 * t),
            offset: point(px(0.), px(0.5)),
            blur_radius: px(0.),
            spread_radius: px(0.),
        },
        BoxShadow {
            inset: false,
            color: hsla(220. / 360., 0.18, 0.14, 0.14 * t * t),
            offset: point(px(0.), px(1.5 * t)),
            blur_radius: px(3.5),
            spread_radius: px(-0.5),
        },
    ]
}

/// Multi-colour app logo rasterized from `assets/app-icon.svg` via `resvg`.
/// (`svg()` in GPUI renders a single-colour alpha mask, whereas `ImageFormat::Svg`
/// preserves the SVG's `#293354` navy strokes and `#FBD68B` warm gold fill.)
pub(crate) fn app_logo(size: Pixels) -> Img {
    static APP_ICON: LazyLock<Arc<Image>> = LazyLock::new(|| {
        Arc::new(Image::from_bytes(
            ImageFormat::Svg,
            include_bytes!("../assets/app-icon.svg").to_vec(),
        ))
    });
    img(APP_ICON.clone()).size(size).flex_none()
}

/// Smoothly rotating activity icon (`RefreshCw`) that respects reduced motion.
pub(crate) fn spinning_icon(
    id: &'static str,
    icon: AssetIconName,
    size: Pixels,
    color: impl Into<Hsla>,
    reduce_motion: bool,
) -> AnyElement {
    let base = Icon::new(icon).size(size).flex_none().text_color(color);
    if reduce_motion {
        base.into_any_element()
    } else {
        base.with_animation(
            id,
            Animation::new(Duration::from_millis(950)).repeat(),
            |ic, delta| ic.rotate(percentage(delta)),
        )
        .into_any_element()
    }
}

// =========================================================================
// Main App Render
// =========================================================================

impl Render for AppState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Without a database there is nothing to list or edit: show only how
        // to get it back.
        if let Some(error) = self.startup_error.clone() {
            return self.render_startup_error(error, window, cx);
        }

        let current_theme = self.current_theme();

        if let Some(tip) = self.md_tip {
            self.md_tip_last = Some(tip);
        }
        let tip_presence = Presence::new("md-tip-overlay", self.md_tip.is_some())
            .transition(Transition::new(Duration::from_millis(135)))
            .sample(window, cx);
        let tip_overlay = tip_presence
            .should_render()
            .then_some(self.md_tip.or(self.md_tip_last))
            .flatten()
            .map(|tip| {
                // The keys are looked up now, not when the pointer arrived:
                // they are the user's to change.
                let keys = self.keymap.label(tip.action);
                md_tip_overlay(tip, keys, tip_presence.progress, window, cx)
            });

        let settings_transition = if self.show_settings {
            Transition::new(Duration::from_millis(245)).ease(cubic_bezier(0.16, 1.0, 0.3, 1.0))
        } else {
            Transition::new(Duration::from_millis(150)).ease(ease_out_cubic)
        };
        let settings_presence = Presence::new("settings-modal", self.show_settings)
            .transition(settings_transition)
            .sample(window, cx);
        let settings_modal = settings_presence
            .should_render()
            .then(|| self.render_settings_modal(settings_presence.progress, window, cx));

        // The question before "clear all notes", on top of the settings.
        let confirm_transition = if self.confirm_clear.is_some() {
            Transition::new(Duration::from_millis(220)).ease(cubic_bezier(0.16, 1.0, 0.3, 1.0))
        } else {
            Transition::new(Duration::from_millis(140)).ease(ease_out_cubic)
        };
        let confirm_presence = Presence::new("clear-confirm", self.confirm_clear.is_some())
            .transition(confirm_transition)
            .sample(window, cx);
        let confirm_modal = confirm_presence
            .should_render()
            .then(|| self.render_clear_confirm(confirm_presence.progress, cx));

        // The quick note, when it has no window of its own.
        let quick_transition = if self.quick_overlay_open {
            Transition::new(Duration::from_millis(215)).ease(cubic_bezier(0.16, 1.0, 0.3, 1.0))
        } else {
            Transition::new(Duration::from_millis(140)).ease(ease_out_cubic)
        };
        let quick_presence = Presence::new("quick-overlay", self.quick_overlay_open)
            .transition(quick_transition)
            .sample(window, cx);
        let quick_overlay = quick_presence
            .should_render()
            .then(|| self.render_quick_overlay(quick_presence.progress, cx))
            .flatten();

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
                    // Crisp 2-sheet workspace over the alabaster `#F6F7F9` shell:
                    // pure `#FFFFFF` rounded note-list sheet in the middle +
                    // 20px-rounded note paper sheet on the right.
                    // Avoiding a shared `.overflow_hidden()` wrapper prevents
                    // GPUI's rectangular `ContentMask` from clipping diffuse
                    // shadows or squaring off rounded corners.
                    .child(
                        h_flex()
                            .id("workspace-area")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .pl(px(2.))
                            .pr(px(10.))
                            .pb(px(10.))
                            .gap(px(8.))
                            .child(self.render_note_list(window, cx))
                            .child(self.render_editor_area(current_theme, window, cx)),
                    ),
            )
            // Hover hint for the markdown bar, painted last so it floats over
            // the title below the toolbar.
            .when_some(tip_overlay, |el, overlay| el.child(overlay))
            .when_some(settings_modal, |el, modal| el.child(modal))
            .when_some(confirm_modal, |el, modal| el.child(modal))
            .when_some(quick_overlay, |el, overlay| el.child(overlay))
            .into_any_element()
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
#[allow(clippy::too_many_arguments)]
fn md_tb_icon(
    id: &'static str,
    label: &'static str,
    icon: AssetIconName,
    active: bool,
    enabled: bool,
    bounds: Rc<RefCell<HashMap<&'static str, Bounds<Pixels>>>>,
    on_hover: impl Fn(&bool, &mut Window, &mut App) + 'static,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let active_t = transition(
        ("md-tb-active", id),
        if active { 1.0_f32 } else { 0.0_f32 },
        Transition::new(Duration::from_millis(125)),
        window,
        cx,
    );
    let fg = if !enabled {
        ink(0.24)
    } else {
        ink(0.58 + 0.34 * active_t)
    };
    let fg_hover = if active { ink(0.95) } else { ink(0.86) };
    let bg_hover = if active { ink(0.12) } else { ink(0.065) };
    let bg_press = ink(0.16);

    div()
        .id(id)
        .group(id)
        .role(Role::Button)
        .aria_label(label)
        .size(px(23.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .text_color(fg)
        .when(active_t > 0.005, |el| {
            el.bg(ink(0.105 * active_t))
                .shadow(vec![specular_rim(false, 0.36 * active_t)])
        })
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

/// Tahoe solid-surface hint shown under a markdown toolbar button, with smooth
/// enter/exit fade and spring horizontal glide across neighboring buttons.
fn md_tip_overlay(
    tip: MdTip,
    keys: Option<String>,
    progress: f32,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let x = spring(
        "md-tip-x",
        tip.x.as_f32(),
        Spring::new(Duration::from_millis(170))
            .with_damping(0.86)
            .with_epsilon(0.15)
            .with_travel(tip.glide),
        window,
        cx,
    );
    let progress = progress.clamp(0., 1.);

    h_flex()
        .absolute()
        .left(px(x))
        .top(tip.y + px(6. - (1. - progress) * 3.))
        .opacity(progress)
        .items_center()
        .gap(px(6.))
        .px(px(9.5))
        .py(px(4.5))
        .rounded(px(8.5))
        .bg(rgb(0x1E232B))
        .shadow(tooltip_surface_shadow_faded(progress))
        .child(
            Label::new(tip.label)
                .text_size(px(11.5))
                .text_color(rgb(0xF8FAFC)),
        )
        // An action without a shortcut shows its name alone.
        .when_some(keys, |el, keys| {
            el.child(
                div()
                    .px(px(4.5))
                    .py(px(0.5))
                    .rounded(px(4.5))
                    .bg(hsla(0., 0., 1., 0.11))
                    .child(
                        Label::new(keys)
                            .text_size(px(10.5))
                            .text_color(hsla(0., 0., 1., 0.74)),
                    ),
            )
        })
}

impl AppState {
    // =========================================================================
    // Recovery screen: the note database could not be opened
    // =========================================================================
    fn render_startup_error(
        &mut self,
        error: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let custom_location = !self.config.uses_default_data_dir();
        let advice = if custom_location {
            "数据库所在的磁盘或文件夹可能暂时不可用，重新连接后点“重试”。也可以改用默认位置：原位置的文件不会被改动，默认位置如果还没有数据库，会从空白开始。"
        } else {
            "可以稍后重试。应用定期保存的数据库备份在备份文件夹里。"
        };

        let enter_t = *Sequence::new("startup-error-enter", 0.0_f32)
            .with_step(1.0_f32, Transition::new(Duration::from_millis(210)))
            .sample(window, cx)
            .value();

        let card = v_flex()
            .id("startup-error-card")
            .w(px(480.))
            .relative()
            .top(px((1. - enter_t) * 8.))
            .opacity(enter_t)
            .rounded(px(22.))
            .bg(rgb(0xFFFFFF))
            .shadow(modal_surface_shadow_faded(enter_t))
            .p(px(22.))
            .gap(px(12.))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Icon::new(AssetIconName::TriangleAlert)
                            .size(px(16.))
                            .flex_none()
                            .text_color(rgb(0xC08A1E)),
                    )
                    .child(
                        Label::new("无法打开便签数据库")
                            .text_size(px(15.5))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(INK_PRIMARY)),
                    ),
            )
            .child(
                div()
                    .id("startup-error-path")
                    .w_full()
                    .text_size(px(12.))
                    .line_height(px(18.))
                    .text_color(rgb(INK_SECONDARY))
                    .child(self.config.database_path().display().to_string()),
            )
            .child(
                div()
                    .id("startup-error-reason")
                    .w_full()
                    .text_size(px(11.5))
                    .line_height(px(17.))
                    .text_color(rgb(0xC04646))
                    .child(error),
            )
            .child(
                div()
                    .id("startup-error-advice")
                    .w_full()
                    .text_size(px(11.5))
                    .line_height(px(17.))
                    .text_color(rgb(INK_MUTED))
                    .child(advice),
            )
            .child(
                h_flex()
                    .pt(px(2.))
                    .items_center()
                    .gap_2()
                    .flex_wrap()
                    .child(primary_pill("btn-startup-retry", "重试").on_click(cx.listener(
                        |this, _, window, cx| this.retry_open_database(window, cx),
                    )))
                    .when(custom_location, |el| {
                        el.child(secondary_pill("btn-startup-default", "改用默认位置", true).on_click(
                            cx.listener(|this, _, window, cx| {
                                this.use_default_database(window, cx)
                            }),
                        ))
                    })
                    .child(secondary_pill("btn-startup-backups", "打开备份文件夹", true).on_click(
                        cx.listener(|this, _, _window, cx| this.reveal_backups(cx)),
                    )),
            );

        v_flex()
            .id("app-root")
            .size_full()
            .overflow_hidden()
            .bg(rgb(SHELL_BG))
            .child(self.render_titlebar(window, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p(px(24.))
                    .child(card),
            )
            .into_any_element()
    }

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
                    .pl(px(13.))
                    .items_center()
                    .gap_1p5()
                    .window_control_area(WindowControlArea::Drag)
                    .child(app_logo(px(18.)))
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let all_count = self.notes.len() as u32;
        let reduce_motion = cx.reduce_motion();

        // --- Capsule "New Note" Button (optically aligned with 46px workspace header) ---
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
                    .active(|el| {
                        el.bg(rgb(PRIMARY_BTN_ACTIVE))
                            .shadow(primary_pill_shadow_pressed())
                    })
                    .shadow(primary_pill_shadow())
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
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label("新建便签")
                    .focus_visible(|s| s.shadow(focus_ring()))
                    .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                        if matches!(ev.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            this.new_note(window, cx);
                        }
                    }))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.new_note(window, cx);
                    })),
            );

        // --- Navigation Items (All Notes + Tags) ---
        let mut nav_rows = v_flex().px_2p5().py_0p5().gap(px(2.));

        // "All Notes" item
        let is_all_selected = self.tag_filter.is_none();
        let all_sel_t = transition(
            "nav-pill-all",
            if is_all_selected { 1.0_f32 } else { 0.0_f32 },
            Transition::new(Duration::from_millis(155)),
            window,
            cx,
        );
        nav_rows = nav_rows.child(
            h_flex()
                .id("nav-all")
                .h(px(32.))
                .px_2p5()
                .rounded(px(10.))
                .items_center()
                .justify_between()
                .cursor_pointer()
                .when(all_sel_t > 0.005, |el| {
                    el.bg(hsla(0., 0., 1., all_sel_t))
                        .shadow(nav_pill_shadow_faded(all_sel_t))
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
                                .text_color(lerp_rgba(
                                    rgb(INK_MUTED),
                                    rgb(0x3B6EA8),
                                    all_sel_t,
                                )),
                        )
                        .child(
                            Label::new("全部便签")
                                .text_size(px(12.5))
                                .font_weight(if is_all_selected {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(lerp_rgba(
                                    rgb(INK_SECONDARY),
                                    rgb(INK_PRIMARY),
                                    all_sel_t,
                                )),
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
                                .text_size(px(11.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(lerp_rgba(
                                    rgb(INK_MUTED),
                                    rgb(0x3B6EA8),
                                    all_sel_t,
                                )),
                        ),
                )
                .tab_index(0)
                .role(Role::Button)
                .aria_label(format!("全部便签，{} 条", all_count))
                .focus_visible(|s| s.shadow(focus_ring()))
                .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                    if matches!(ev.keystroke.key.as_str(), "enter" | "space") {
                        cx.stop_propagation();
                        this.tag_filter = None;
                        this.ensure_selection_after_filter(window, cx);
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.tag_filter = None;
                    this.ensure_selection_after_filter(window, cx);
                    cx.notify();
                })),
        );

        // "Tags" Section Header with Dedicated "+" Button (rotates 45° into "×" when open)
        let plus_rot = spring(
            "sidebar-add-tag-rot",
            if self.show_sidebar_add_tag {
                0.125_f32
            } else {
                0.0_f32
            },
            Spring::new(Duration::from_millis(210))
                .with_damping(0.78)
                .with_epsilon(0.002),
            window,
            cx,
        );
        let tags_header = h_flex()
            .pl_2p5()
            .pr_1p5()
            .pt_3p5()
            .pb_1()
            .items_center()
            .justify_between()
            .child(
                Label::new("标签分类")
                    .text_size(px(11.5))
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
                    .when(self.show_sidebar_add_tag, |el| el.bg(shell_hover()))
                    .hover(|el| el.bg(shell_hover()))
                    .active(|el| el.bg(shell_active()))
                    .child(
                        Icon::new(IconName::Plus)
                            .size(px(12.))
                            // The spring swings a little past its target, on
                            // the way back to 0 into negative turns; as an
                            // angle that is the same as just under one turn.
                            .rotate(percentage(plus_rot.rem_euclid(1.0)))
                            .text_color(if self.show_sidebar_add_tag {
                                rgb(INK_PRIMARY)
                            } else {
                                rgb(INK_SECONDARY)
                            }),
                    )
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label("新建标签")
                    .focus_visible(|s| s.shadow(focus_ring()))
                    .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                        if matches!(ev.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            this.toggle_sidebar_add_tag(window, cx);
                        }
                    }))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_sidebar_add_tag(window, cx);
                    })),
            );
        nav_rows = nav_rows.child(tags_header);

        // Inline Add Tag Form (smooth vertical reveal + unclipped capsule shadow)
        let add_tag_presence = Presence::new(
            "sidebar-add-tag",
            self.show_sidebar_add_tag && self.new_tag_input.is_some(),
        )
        .transition(Transition::new(Duration::from_millis(175)))
        .sample(window, cx);

        if add_tag_presence.should_render() {
            if let Some(inp) = &self.new_tag_input {
                let p = add_tag_presence.progress;
                let inline_add_row = div()
                    .id("sidebar-add-tag-row")
                    // Breathing room inside `MotionReveal`'s `ContentMask` so the
                    // white field's diffuse shadow is never clipped.
                    .mx(px(1.5))
                    .py(px(4.))
                    .opacity(p)
                    .child(
                        h_flex()
                            .pl_2p5()
                            .pr_1()
                            .h(px(31.))
                            .gap_1()
                            .items_center()
                            .rounded(px(10.))
                            .bg(rgb(0xFFFFFF))
                            .shadow(ios_field_shadow_faded(p))
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
                            ),
                    );
                nav_rows = nav_rows.child(MotionReveal::new(
                    "sidebar-add-tag-reveal",
                    p,
                    inline_add_row.into_any_element(),
                ));
            }
        }

        // User Tags list (Zero SQLite queries - in-memory cache)
        if self.tags_list.is_empty() && !add_tag_presence.should_render() {
            nav_rows = nav_rows.child(
                div().px_2p5().py_2().child(
                    Label::new("暂无标签，点击 ＋ 创建")
                        .text_size(px(11.5))
                        .text_color(rgb(INK_MUTED)),
                ),
            );
        }

        for t in self.tags_list.iter() {
            let tid = t.id;
            let count = self.tag_counts.get(&tid).copied().unwrap_or(0);
            let is_selected = self.tag_filter == Some(tid);
            let row_id = ElementId::Name(format!("nav-tag-{tid}").into());
            let sel_t = transition(
                (row_id.clone(), "pill"),
                if is_selected { 1.0_f32 } else { 0.0_f32 },
                Transition::new(Duration::from_millis(155)),
                window,
                cx,
            );
            // Unique group per row so the delete button only fades in for the
            // row the pointer is actually over.
            let row_group = SharedString::from(format!("tag-row-{tid}"));

            nav_rows = nav_rows.child(
                h_flex()
                    .id(row_id)
                    .group(row_group.clone())
                    .h(px(31.))
                    .px_2p5()
                    .rounded(px(10.))
                    .items_center()
                    .justify_between()
                    .cursor_pointer()
                    .when(sel_t > 0.005, |el| {
                        el.bg(hsla(0., 0., 1., sel_t))
                            .shadow(nav_pill_shadow_faded(sel_t))
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
                                            .text_color(lerp_rgba(
                                                rgb(INK_MUTED),
                                                rgb(0x3B6EA8),
                                                sel_t,
                                            )),
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
                                    .text_color(lerp_rgba(
                                        rgb(INK_SECONDARY),
                                        rgb(INK_PRIMARY),
                                        sel_t,
                                    )),
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
                                            .text_size(px(11.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(lerp_rgba(
                                                rgb(INK_MUTED),
                                                rgb(0x3B6EA8),
                                                sel_t,
                                            )),
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
                                    .role(Role::Button)
                                    .aria_label(format!("删除标签 #{}", t.name))
                                    .on_click(cx.listener(move |this, _, _window, cx| {
                                        this.delete_tag(tid, cx);
                                    })),
                            ),
                    )
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label(format!("#{}，{} 条", t.name, count))
                    .focus_visible(|s| s.shadow(focus_ring()))
                    .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
                        if matches!(ev.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            this.tag_filter = Some(tid);
                            this.ensure_selection_after_filter(window, cx);
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.tag_filter = Some(tid);
                        this.ensure_selection_after_filter(window, cx);
                        cx.notify();
                    })),
            );
        }

        let webdav_configured = self.config.webdav_configured();
        let webdav_on = self.config.webdav_auto && webdav_configured;
        // A pass in flight always shows itself: the spinner is the promise
        // that something is happening. A background poll says "checking", so
        // it is not mistaken for a full sync.
        let webdav_working = self.webdav_syncing;
        let webdav_flash = self
            .webdav_flash
            .as_ref()
            .filter(|(_, until)| Instant::now() < *until)
            .map(|(text, _)| text.as_str());
        let (dot_hex, webdav_label) = if webdav_working {
            (
                0x4F86C6,
                if self.webdav_quiet { "正在检查更新…" } else { "正在同步…" },
            )
        } else if self.webdav_failed && webdav_on {
            // Said until a pass succeeds: a down server is not retried on
            // every window switch, so the dot is where it stays visible.
            (0xD2615C, "WebDAV 同步失败")
        } else if let Some(text) = webdav_flash {
            (0x52A876, text)
        } else if webdav_on {
            (0x52A876, "自动同步已开启")
        } else if webdav_configured {
            (0xD49B3E, "自动同步已关闭")
        } else {
            (0xAAB3C0, "本地存储")
        };
        let dot_color = transition(
            "sidebar-webdav-dot",
            RgbColor(rgb(dot_hex)),
            Transition::new(Duration::from_millis(220))
                .reduced_motion(ReducedMotion::Continue),
            window,
            cx,
        )
        .0;

        // One 11px slot for every state, so the label never shifts sideways
        // when the dot turns into a spinner or a check.
        let status_lead: AnyElement = {
            let lead: AnyElement = if webdav_working {
                spinning_icon(
                    "sidebar-webdav-spin",
                    AssetIconName::RefreshCw,
                    px(10.5),
                    rgb(0x4F86C6),
                    reduce_motion,
                )
            } else if webdav_flash.is_some() && !self.webdav_failed {
                Icon::new(AssetIconName::CircleCheck)
                    .size(px(10.5))
                    .flex_none()
                    .text_color(rgb(0x2A8555))
                    .into_any_element()
            } else if !webdav_configured {
                Icon::new(AssetIconName::HardDrive)
                    .size(px(10.5))
                    .flex_none()
                    .text_color(rgb(INK_MUTED))
                    .into_any_element()
            } else {
                div()
                    .size(px(6.5))
                    .rounded_full()
                    .bg(dot_color)
                    .into_any_element()
            };
            div()
                .size(px(11.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(lead)
                .into_any_element()
        };

        let gear_rot = spring(
            "sidebar-gear-rot",
            if self.show_settings { 0.1667_f32 } else { 0.0_f32 },
            Spring::new(Duration::from_millis(240))
                .with_damping(0.76)
                .with_epsilon(0.002),
            window,
            cx,
        );

        // The footer always says where the notes live: "本地存储" without a
        // WebDAV account, then the sync state once there is one. It is a
        // status, not a control: settings live behind the gear next to it.
        let status_group: AnyElement = h_flex()
            .flex_1()
            .items_center()
            .gap_2()
            .min_w_0()
            .px_1p5()
            .py(px(3.))
            .child(status_lead)
            .child(if self.webdav_failed && webdav_on && !webdav_working {
                // The failure is the thing to read here, not a muted
                // footnote: it stays until sync recovers.
                div()
                    .id("webdav-failed-label")
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        Label::new(webdav_label)
                            .text_size(px(11.5))
                            .text_color(rgb(0xB45B67))
                            .whitespace_nowrap()
                            .text_ellipsis(),
                    )
                    .into_any_element()
            } else {
                let label_color = if webdav_working {
                    rgb(0x3B6EA8)
                } else if webdav_flash.is_some() {
                    rgb(0x2A8555)
                } else {
                    rgb(INK_SECONDARY)
                };
                div()
                    .id("webdav-state-label")
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        Label::new(webdav_label)
                            .text_size(px(11.5))
                            .text_color(label_color)
                            .whitespace_nowrap()
                            .text_ellipsis(),
                    )
                    .into_any_element()
            })
            .into_any_element();

        let footer_status = div()
            .px_2p5()
            .pb(px(8.))
            .pt_1()
            .child(
                h_flex()
                    .h(px(32.))
                    .items_center()
                    .gap_2()
                    .justify_between()
                    .child(status_group)
                    .child(
                        div()
                            .id("btn-settings")
                            .flex_none()
                            .size(px(28.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(8.))
                            .cursor_pointer()
                            .when(self.show_settings, |s| s.bg(shell_hover()))
                            .text_color(rgb(if self.show_settings {
                                0x3B6EA8
                            } else {
                                INK_MUTED
                            }))
                            .hover(|s| s.bg(shell_hover()).text_color(rgb(INK_PRIMARY)))
                            .active(|s| s.bg(shell_active()))
                            .child(
                                Icon::new(AssetIconName::Settings)
                                    .size(px(14.))
                                    .rotate(percentage(gear_rot.rem_euclid(1.0))),
                            )
                            .tab_index(0)
                            .role(Role::Button)
                            .aria_label("设置")
                            .focus_visible(|s| s.shadow(focus_ring()))
                            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                                if matches!(ev.keystroke.key.as_str(), "enter" | "space") {
                                    cx.stop_propagation();
                                    this.open_settings(window, cx);
                                }
                            }))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_settings(window, cx);
                            })),
                    ),
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let notes = self.filtered_notes();
        let reduce_motion = cx.reduce_motion();
        let clear_search_presence = Presence::new("search-clear", !self.search_query.is_empty())
            .transition(Transition::new(Duration::from_millis(135)))
            .sample(window, cx);

        // Soft porcelain search capsule inside the pure-white `#FFFFFF` list sheet;
        // lifts to `#FFFFFF` with a crisp accent ring when focused.
        let search_box = match &self.search_input {
            Some(inp) => {
                let focused = inp.read(cx).focus_handle(cx).is_focused(window);
                h_flex()
                    .h(px(48.))
                    .px(px(11.))
                    .pt(px(2.))
                    .flex_none()
                    .items_center()
                    .child(
                        h_flex()
                            .id("search-capsule")
                            .w_full()
                            .h(px(33.))
                            .pl_3()
                            .pr_1p5()
                            .gap_1p5()
                            .items_center()
                            .rounded(px(11.))
                            .bg(rgb(if focused { 0xFFFFFF } else { 0xF3F5F8 }))
                            .when(!focused, |el| el.hover(|s| s.bg(rgb(0xECEFF4))))
                            .when(focused, |el| {
                                el.shadow(vec![BoxShadow {
                                    inset: false,
                                    color: hsla(212. / 360., 0.52, 0.45, 0.55),
                                    offset: point(px(0.), px(0.)),
                                    blur_radius: px(0.),
                                    spread_radius: px(1.25),
                                }])
                            })
                            .child(
                                Icon::new(IconName::Search)
                                    .size(px(13.5))
                                    .flex_none()
                                    .text_color(rgb(if focused { 0x3B6EA8 } else { INK_MUTED })),
                            )
                            .child(
                                div().flex_1().min_w_0().child(
                                    Input::new(inp)
                                        .appearance(false)
                                        .bordered(false)
                                        .focus_bordered(false),
                                ),
                            )
                            .when(clear_search_presence.should_render(), |el| {
                                let p = clear_search_presence.progress;
                                el.child(
                                    div()
                                        .id("btn-clear-search")
                                        .size(px(18.))
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .opacity(p)
                                        .hover(|s| s.bg(shell_hover()))
                                        .active(|s| s.bg(shell_active()))
                                        .cursor_pointer()
                                        .child(
                                            Icon::new(IconName::Close)
                                                .size(px(10.))
                                                .text_color(rgb(INK_MUTED)),
                                        )
                                        .tab_index(0)
                                        .role(Role::Button)
                                        .aria_label("清除搜索")
                                        .focus_visible(|s| s.shadow(focus_ring()))
                                        .on_key_down(cx.listener(
                                            |this, ev: &KeyDownEvent, window, cx| {
                                                if matches!(
                                                    ev.keystroke.key.as_str(),
                                                    "enter" | "space"
                                                ) {
                                                    cx.stop_propagation();
                                                    this.clear_search(window, cx);
                                                }
                                            },
                                        ))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.clear_search(window, cx);
                                        })),
                                )
                            }),
                    )
                    .into_any_element()
            }
            None => div().into_any_element(),
        };

        // Note Count & Filter Banner
        let active_tag = self.tag_filter.and_then(|tid| {
            self.tags_list
                .iter()
                .find(|t| t.id == tid)
                .map(|t| (t.id, t.name.clone()))
        });

        // Two icon buttons instead of words: the clock is "recently
        // modified", the calendar "recently created". Icons alone would be a
        // guess, so each shows its meaning on hover.
        let sort_mode = self.config.note_sort;
        let sort_bounds = self.md_tip_bounds.clone();
        let sort_button =
            |id: &'static str,
             icon: AssetIconName,
             label: &'static str,
             mode: NoteSort,
             cx: &mut Context<Self>| {
                let active = sort_mode == mode;
                h_flex()
                    .id(id)
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(|s| s.bg(hsla(220. / 360., 0.20, 0.18, 0.055)))
                    .active(|s| s.bg(hsla(220. / 360., 0.20, 0.18, 0.09)))
                    .child(
                        Icon::new(icon)
                            .size(px(13.5))
                            .text_color(if active {
                                rgb(INK_PRIMARY)
                            } else {
                                // Light, but still past the 3:1 floor for
                                // non-text controls on white.
                                rgb(0x8A94A6)
                            }),
                    )
                    .tab_index(0)
                    .role(Role::RadioButton)
                    .aria_label(label)
                    .aria_selected(active)
                    .focus_visible(|s| s.shadow(focus_ring()))
                    .on_prepaint({
                        let sort_bounds = sort_bounds.clone();
                        move |bounds, _, _| {
                            sort_bounds.borrow_mut().insert(id, bounds);
                        }
                    })
                    .on_hover({
                        let sort_bounds = sort_bounds.clone();
                        cx.listener(move |this, hovered: &bool, _window, cx| {
                            if *hovered {
                                if let Some(b) = sort_bounds.borrow().get(id).copied() {
                                    this.md_tip = Some(MdTip {
                                        id,
                                        label,
                                        action: "",
                                        x: b.left(),
                                        y: b.bottom(),
                                        glide: false,
                                    });
                                    this.md_tip_closed_at = None;
                                }
                            } else if this.md_tip.is_some_and(|tip| tip.id == id) {
                                this.md_tip = None;
                                this.md_tip_closed_at = Some(Instant::now());
                            }
                            cx.notify();
                        })
                    })
                    .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _window, cx| {
                        if matches!(ev.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            this.set_note_sort(mode, cx);
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_note_sort(mode, cx)))
            };

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
                    .when_some(active_tag, |el, (tid, name)| {
                        let chip = h_flex()
                            .id("btn-clear-tag-filter")
                            .relative()
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
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(0x3B6EA8)),
                            )
                            .child(
                                Icon::new(IconName::Close)
                                    .size(px(9.))
                                    .text_color(rgb(0x3B6EA8)),
                            )
                            .tab_index(0)
                            .role(Role::Button)
                            .aria_label(format!("清除标签筛选 #{}", name))
                            .focus_visible(|s| s.shadow(focus_ring()))
                            .on_key_down(cx.listener(
                                |this, ev: &KeyDownEvent, window, cx| {
                                    if matches!(ev.keystroke.key.as_str(), "enter" | "space") {
                                        cx.stop_propagation();
                                        this.clear_tag_filter(window, cx);
                                    }
                                },
                            ))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.clear_tag_filter(window, cx);
                            }));
                        if reduce_motion {
                            el.child(chip)
                        } else {
                            el.child(chip.with_animation(
                                ElementId::Name(format!("tag-chip-enter-{tid}").into()),
                                Animation::new(Duration::from_millis(150))
                                    .with_easing(ease_out_cubic),
                                |c, d| c.opacity(d).left(px((1. - d) * -4.)),
                            ))
                        }
                    })
                    .child(
                        Label::new(if self.tag_filter.is_some() || !self.search_query.is_empty() {
                            format!("筛选出 {} 条", notes.len())
                        } else {
                            format!("共 {} 条便签", notes.len())
                        })
                        .text_size(px(11.5))
                        .text_color(rgb(INK_MUTED)),
                    ),
            )
            .child(
                h_flex()
                    .flex_none()
                    .items_center()
                    .gap(px(1.))
                    .child(sort_button(
                        "sort-updated",
                        AssetIconName::Clock,
                        "按修改时间排序",
                        NoteSort::RecentUpdated,
                        cx,
                    ))
                    .child(sort_button(
                        "sort-created",
                        AssetIconName::CalendarDays,
                        "按创建时间排序",
                        NoteSort::RecentCreated,
                        cx,
                    )),
            );

        // Framed cards with a little air between them, on the white sheet.
        let filtering = self.tag_filter.is_some() || !self.search_query.trim().is_empty();
        let mut card_list = v_flex().px(px(10.)).pt_0p5().pb_3().gap(px(6.));
        if notes.is_empty() {
            // The empty canvas on the right is the primary empty state, with
            // the one "新建便签" action. The list only says quietly that there
            // is nothing here; a second instruction and CTA would compete
            // with the one on the canvas.
            let (empty_icon, empty_title, empty_hint) = if filtering {
                (
                    IconName::Search,
                    "没有匹配的便签",
                    Some("试试其他关键词或清除筛选条件"),
                )
            } else {
                (IconName::Inbox, "还没有便签", None)
            };
            let empty_state = v_flex()
                .py_12()
                .px_4()
                .relative()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .child(
                    div()
                        .size(px(40.))
                        .rounded(px(12.))
                        .bg(rgb(0xF4F6FA))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::new(empty_icon)
                                .size(px(17.))
                                .text_color(rgb(INK_MUTED)),
                        ),
                )
                .child(
                    Label::new(empty_title)
                        .text_size(px(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(INK_SECONDARY)),
                )
                .when_some(empty_hint, |el, hint| {
                    el.child(
                        Label::new(hint)
                            .text_size(px(11.5))
                            .text_color(rgb(INK_MUTED)),
                    )
                });
            card_list = if reduce_motion {
                card_list.child(empty_state)
            } else {
                card_list.child(empty_state.with_animation(
                    "empty-notes-enter",
                    Animation::new(Duration::from_millis(180))
                        .with_easing(cubic_bezier(0.16, 1.0, 0.3, 1.0)),
                    |el, d| el.opacity(d).top(px((1. - d) * 5.)),
                ))
            };
        } else {
            for (i, note) in notes.iter().enumerate() {
                card_list = card_list.child(self.render_note_card(i, note, window, cx));
            }
        }

        // Pure `#FFFFFF` rounded list sheet so blank space below cards stays
        // bright and crisp instead of exposing a grey column void.
        v_flex()
            .id("note-list-pane")
            .w(LIST_PANE_W)
            .h_full()
            .flex_none()
            .rounded(px(18.))
            .bg(rgb(0xFFFFFF))
            .border_1()
            .border_color(hsla(220. / 360., 0.20, 0.16, 0.065))
            .shadow(list_sheet_shadow())
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_selected = self.selected_id.as_deref() == Some(note.id.as_str());
        let is_pinned = self.is_pinned(&note.id);
        let updated = ticks::format_friendly_ticks(note.updated_at);
        let highlight = self.search_query.trim().to_string();
        let has_highlight = !highlight.is_empty();
        let card_title = if note.title.trim().is_empty() {
            "(无标题便签)"
        } else {
            note.title.as_str()
        };
        let note_theme = crate::state::get_theme(&note.theme);
        let sel_t = transition(
            (ElementId::Name(format!("card-sel-{}", note.id).into()), "t"),
            if is_selected { 1.0_f32 } else { 0.0_f32 },
            Transition::new(Duration::from_millis(160)),
            window,
            cx,
        );
        // Each card is a soft porcelain tile with a hairline frame: the fill
        // keeps it from reading as an empty wireframe box on the white sheet,
        // the frame gives it a clean edge. The selected card takes its own
        // paper's tint and frame colour.
        let mut selected_border = rgb(note_theme.border);
        selected_border.a = 0.9;
        let rest_bg = rgb(0xF9FAFB);
        let rest_border: Rgba = hsla(220. / 360., 0.20, 0.18, 0.085).into();
        let hover_bg = rgb(0xF4F6F9);
        let hover_border: Rgba = hsla(220. / 360., 0.20, 0.18, 0.13).into();
        let bg_color = lerp_rgba(rest_bg, rgb(note_theme.card_active_bg), sel_t);
        let border_color = lerp_rgba(rest_border, selected_border, sel_t);

        let card = h_flex()
            .id(ElementId::Name(format!("note-card-{idx}").into()))
            .w_full()
            .pl_3()
            .pr(px(13.))
            .py(px(10.))
            .gap_2()
            .rounded(px(12.))
            .cursor_pointer()
            .bg(bg_color)
            .border_1()
            .border_color(border_color)
            .shadow(note_card_shadow(sel_t))
            .when(!is_selected, |el| {
                el.hover(move |s| s.bg(hover_bg).border_color(hover_border))
                    .active(move |s| {
                        s.bg(lerp_rgba(hover_bg, rgb(0x1A1D24), 0.04))
                            .border_color(hover_border)
                    })
            })
            // A selected card keeps its tint, one step deeper while pressed.
            .when(is_selected, |el| {
                el.active(move |s| s.bg(lerp_rgba(bg_color, rgb(0x1A1D24), 0.05)))
            })
            .tab_index(0)
            .role(Role::Button)
            .aria_label(format!("{}，修改于 {}", card_title, updated))
            .focus_visible(|s| s.shadow(focus_ring()))
            .on_key_down(cx.listener({
                let id = note.id.clone();
                move |this, ev: &KeyDownEvent, window, cx| {
                    if matches!(ev.keystroke.key.as_str(), "enter" | "space") {
                        cx.stop_propagation();
                        this.select_note(&id, window, cx);
                    }
                }
            }));

        // Card body content
        let mut content = v_flex().flex_1().min_w_0().gap(px(3.));

        // Title row: theme color dot + Title + Pin + Relative Timestamp
        let title_text = card_title;
        let dot_size = px(7.5 + 1.0 * sel_t);
        let is_white_note = note_theme.name == "White";

        let title_row = h_flex()
            .items_center()
            .gap_1p5()
            .child(
                div()
                    .size(px(9.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .size(dot_size)
                            .rounded_full()
                            .bg(rgb(note_theme.accent))
                            .map(|dot| {
                                if is_white_note {
                                    dot.border_1().border_color(rgb(if is_selected {
                                        0x6B90BA
                                    } else {
                                        0xAEB8C6
                                    }))
                                } else {
                                    dot.shadow(vec![specular_rim(false, 0.35)])
                                }
                            }),
                    ),
            )
            .child(
                Label::new(title_text.to_string())
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
                    .when(has_highlight, |el| el.highlights(highlight.clone())),
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
                    .text_size(px(11.))
                    .flex_none()
                    .text_color(rgb(INK_MUTED)),
            );
        content = content.child(title_row);

        // Snippet (two brief lines, plain text — markdown already stripped at store level)
        if !note.snippet.is_empty() {
            content = content.child(
                Label::new(note.snippet.clone())
                    .text_size(px(11.5))
                    .line_height(px(16.5))
                    .line_clamp(2)
                    .text_ellipsis()
                    .text_color(rgb(INK_SECONDARY))
                    .when(has_highlight, |el| el.highlights(highlight.clone())),
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
                menu = menu.submenu_with_icon(
                    Some(Icon::new(AssetIconName::Tag)),
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

    /// Floating iOS capsule header at the top of the rounded note sheet:
    /// borderless translucent Markdown formatting pill on the left and colour
    /// swatch pill on the right. Transparent container (no `.bg(paper_bg)` and no
    /// wall-to-wall `border_b_1`) so the parent sheet's 20px rounded corners
    /// and continuous paper surface stay completely intact.
    fn render_editor_header(
        &self,
        editor: &Entity<BlockEditorState>,
        theme: &'static ThemeSpec,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_white_paper = theme.name == "White";
        let md_strip = self.render_md_toolbar(editor, is_white_paper, window, cx);
        let palette = self.render_palette_capsule(theme, window, cx);

        h_flex()
            .id("canvas-toolbar")
            .w_full()
            .h(px(46.))
            .flex_none()
            .px_4()
            .gap_2p5()
            .items_center()
            .justify_between()
            .child(md_strip)
            .child(palette)
    }

    /// Flat, single-row markdown bar: every high-frequency format is a 23×23
    /// icon button with a hover hint. Borderless translucent iOS material lets
    /// the note's paper colour breathe through without any dark perimeter line.
    fn render_md_toolbar(
        &self,
        editor: &Entity<BlockEditorState>,
        is_white_paper: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // Below this width the full row no longer fits beside the palette
        // capsule (measured at the minimum window size).
        let narrow = window.viewport_size().width < px(1080.);
        let (kind, in_code_block, bold, italic, strike, code, highlight, in_quote) = {
            let e = editor.read(cx);
            let m = e.model();
            let kind = e.block_kind_at_caret();
            (
                kind,
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

        let sep_bg = ink(0.06);
        let app = cx.entity();
        let bounds = self.md_tip_bounds.clone();

        let hover_for = |id: &'static str, label: &'static str, action: &'static str| {
            let app = app.clone();
            move |hovered: &bool, _window: &mut Window, cx: &mut App| {
                if !*hovered {
                    app.update(cx, |this, cx| {
                        if this.md_tip_hover == Some(id) {
                            this.md_tip_hover = None;
                        }
                        if this.md_tip.is_some_and(|t| t.id == id) {
                            this.md_tip = None;
                            this.md_tip_closed_at = Some(Instant::now());
                            cx.notify();
                        }
                    });
                    return;
                }
                app.update(cx, |this, cx| {
                    this.md_tip_hover = Some(id);
                    let warm = this.md_tip.is_some()
                        || this
                            .md_tip_closed_at
                            .is_some_and(|t| t.elapsed() < Duration::from_millis(260));
                    if warm {
                        if let Some(b) = this.md_tip_bounds.borrow().get(id).copied() {
                            this.md_tip = Some(MdTip {
                                id,
                                label,
                                action,
                                x: b.left(),
                                y: b.bottom(),
                                glide: true,
                            });
                            cx.notify();
                            return;
                        }
                    }
                    cx.spawn(async move |weak, cx| {
                        cx.background_executor()
                            .timer(Duration::from_millis(320))
                            .await;
                        let _ = weak.update(cx, |this, cx| {
                            if this.md_tip_hover != Some(id) {
                                return;
                            }
                            if let Some(b) = this.md_tip_bounds.borrow().get(id).copied() {
                                this.md_tip = Some(MdTip {
                                    id,
                                    label,
                                    action,
                                    x: b.left(),
                                    y: b.bottom(),
                                    glide: false,
                                });
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                });
            }
        };

        let run = |cmd: Option<EditorCommand>| {
            let ed = editor.clone();
            let app = app.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                let Some(cmd) = cmd.clone() else {
                    return;
                };
                app.update(cx, |this, cx| {
                    this.md_tip = None;
                    this.md_tip_closed_at = None;
                    cx.notify();
                });
                ed.update(cx, |s, cx| {
                    let _ = s.run(cmd, cx);
                    s.focus(window, cx);
                });
            }
        };

        // Every button is one of the formatting actions of the shortcut
        // settings. The action decides what the button does and whether that
        // applies at the caret — exactly as its shortcut would — and gives
        // the hint its name and its keys.
        let mut btn = |id: &'static str, icon: AssetIconName, active: bool, action: &'static str| {
            let cmd = md_command(action, kind);
            let label = shortcuts::action_def(action)
                .map(|def| def.label)
                .unwrap_or_default();
            md_tb_icon(
                id,
                label,
                icon,
                active,
                cmd.is_some(),
                bounds.clone(),
                hover_for(id, label, action),
                run(cmd),
                window,
                cx,
            )
        };
        let sep = || div().w(px(1.)).h(px(12.)).mx(px(2.)).flex_none().bg(sep_bg);

        let mut row = h_flex().h_full().flex_none().items_center().gap(px(1.));
        // 1. Heading level.
        row = row.child(btn(
            "md-para",
            AssetIconName::Pilcrow,
            false,
            shortcuts::MD_PARAGRAPH,
        ));
        row = row.child(btn(
            "md-h1",
            AssetIconName::Heading1,
            heading(1),
            shortcuts::MD_H1,
        ));
        row = row.child(btn(
            "md-h2",
            AssetIconName::Heading2,
            heading(2),
            shortcuts::MD_H2,
        ));
        row = row.child(btn(
            "md-h3",
            AssetIconName::Heading3,
            heading(3),
            shortcuts::MD_H3,
        ));
        row = row.child(sep());
        // 2. Inline styles.
        row = row.child(btn(
            "md-bold",
            AssetIconName::Bold,
            bold,
            shortcuts::MD_BOLD,
        ));
        row = row.child(btn(
            "md-italic",
            AssetIconName::Italic,
            italic,
            shortcuts::MD_ITALIC,
        ));
        row = row.child(btn(
            "md-strike",
            AssetIconName::Strikethrough,
            strike,
            shortcuts::MD_STRIKE,
        ));
        row = row.child(btn(
            "md-code",
            AssetIconName::Code,
            code,
            shortcuts::MD_CODE,
        ));
        row = row.child(btn(
            "md-highlight",
            AssetIconName::Highlighter,
            highlight,
            shortcuts::MD_HIGHLIGHT,
        ));
        row = row.child(sep());
        // 3. Lists and block structure.
        row = row.child(btn(
            "md-bullet",
            AssetIconName::List,
            in_bullet,
            shortcuts::MD_BULLET,
        ));
        row = row.child(btn(
            "md-ordered",
            AssetIconName::ListOrdered,
            in_ordered,
            shortcuts::MD_ORDERED,
        ));
        row = row.child(btn(
            "md-task",
            AssetIconName::ListChecks,
            in_task,
            shortcuts::MD_TASK,
        ));
        // The block tools are the least used group. A narrow window keeps them
        // (their shortcuts still work) but drops their buttons, so the same
        // pill always shows a complete set of icons instead of a cut-off row.
        if !narrow {
            row = row.child(btn(
                "md-quote",
                AssetIconName::TextQuote,
                in_quote,
                shortcuts::MD_QUOTE,
            ));
            row = row.child(btn(
                "md-codeblock",
                AssetIconName::SquareCode,
                in_code_block,
                shortcuts::MD_CODE_BLOCK,
            ));
            row = row.child(btn(
                "md-hr",
                AssetIconName::Minus,
                false,
                shortcuts::MD_RULE,
            ));
            row = row.child(btn(
                "md-table",
                AssetIconName::Table,
                false,
                shortcuts::MD_TABLE,
            ));
        }

        // Borderless iOS translucent capsule: outer pill owns the rounded fill
        // and whisper elevation; the inner div clips instead, so `ContentMask`
        // never rounds off the pill's own corners.
        h_flex()
            .min_w_0()
            .h(px(30.))
            .px(px(5.))
            .items_center()
            .rounded(px(9999.))
            .bg(paper_capsule_bg(is_white_paper))
            .shadow(paper_capsule_shadow(is_white_paper))
            .child(
                div()
                    .id("md-toolbar-row")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_hidden()
                    .child(row),
            )
    }

    /// Borderless iOS capsule holding the paper-theme swatches.
    /// Each swatch springs inside a fixed 16×16 cell so scaling the active bead
    /// never jostles neighboring colours.
    fn render_palette_capsule(
        &self,
        current_theme: &'static ThemeSpec,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_white_paper = current_theme.name == "White";
        let tip_bounds = self.md_tip_bounds.clone();
        let mut palette = h_flex()
            .h(px(30.))
            .items_center()
            .gap(px(2.5))
            .px(px(7.))
            .rounded(px(9999.))
            .bg(paper_capsule_bg(is_white_paper))
            .shadow(paper_capsule_shadow(is_white_paper))
            .flex_none();

        for t in THEMES {
            let theme_name = t.name.to_string();
            let tip_bounds = tip_bounds.clone();
            let is_cur = t.name == current_theme.name;
            let is_white_swatch = t.name == "White";
            let sel_t = spring(
                ("palette-swatch", t.name),
                if is_cur { 1.0_f32 } else { 0.0_f32 },
                Spring::new(Duration::from_millis(190))
                    .with_damping(0.74)
                    .with_epsilon(0.005),
                window,
                cx,
            );
            let diameter = px((12.0 + 3.5 * sel_t).clamp(11.0, 16.0));
            let base_alpha = (0.88 + 0.12 * sel_t).clamp(0., 1.);

            let mut circle = div()
                .size(diameter)
                .flex_none()
                .rounded_full()
                .bg(rgb(t.accent))
                .opacity(base_alpha)
                .when(is_white_swatch, |el| {
                    el.border_1()
                        .border_color(rgb(if is_cur { 0x4F7CB0 } else { 0xAAB4C3 }))
                })
                .shadow(lifted_swatch_shadow_faded(sel_t));

            if !is_cur {
                circle = circle.group_hover(t.name, |s| {
                    s.opacity(1.).shadow(lifted_swatch_shadow_faded(0.85))
                });
            }

            let cell = div()
                .id(ElementId::Name(format!("palette-{}", t.name).into()))
                .group(t.name)
                .size(px(16.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .active(|s| s.opacity(0.76))
                .on_prepaint(move |b, _, _| {
                    tip_bounds.borrow_mut().insert(t.name, b);
                })
                .on_hover(cx.listener(move |this, hovered: &bool, _window, cx| {
                    if *hovered {
                        if let Some(b) = this.md_tip_bounds.borrow().get(t.name).copied() {
                            this.md_tip = Some(MdTip {
                                id: t.name,
                                label: t.label,
                                // No shortcut: the hint shows the name alone.
                                action: "",
                                x: b.left(),
                                y: b.bottom(),
                                glide: false,
                            });
                            this.md_tip_closed_at = None;
                        }
                    } else if this.md_tip.is_some_and(|tip| tip.id == t.name) {
                        this.md_tip = None;
                        this.md_tip_closed_at = Some(Instant::now());
                    }
                    cx.notify();
                }))
                .tab_index(0)
                .role(Role::RadioButton)
                .aria_label(format!("纸张颜色：{}", t.label))
                .aria_selected(is_cur)
                .focus_visible(|s| s.shadow(focus_ring()))
                .on_key_down(cx.listener({
                    let theme_name = theme_name.clone();
                    move |this, ev: &KeyDownEvent, _window, cx| {
                        if matches!(ev.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            this.set_theme(&theme_name, cx);
                        }
                    }
                }))
                .on_click(cx.listener(move |this, _, _window, cx| {
                    this.set_theme(&theme_name, cx);
                }))
                .child(circle);

            palette = palette.child(cell);
        }

        palette
    }

    fn render_editor_area(
        &mut self,
        theme: &'static ThemeSpec,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let reduce_motion = cx.reduce_motion();
        let is_white_paper = theme.name == "White";
        let paper_bg = transition(
            "editor-paper-bg",
            RgbColor(rgb(theme.paper_bg)),
            Transition::new(Duration::from_millis(220))
                .reduced_motion(ReducedMotion::Continue),
            window,
            cx,
        )
        .0;

        let Some((editor, title_input)) = self.editor.clone().zip(self.title_input.clone()) else {
            return v_flex()
                .id("editor-pane")
                .flex_1()
                .min_w_0()
                .h_full()
                .rounded(px(20.))
                .bg(rgb(0xFFFFFF))
                .border_1()
                .border_color(hsla(220. / 360., 0.20, 0.16, 0.065))
                .shadow(editor_paper_shadow(true))
                .items_center()
                .justify_center()
                .child(
                    v_flex()
                        .items_center()
                        .gap(px(10.))
                        .child(
                            div()
                                .size(px(56.))
                                .rounded(px(17.))
                                .bg(rgb(0xF5F7FA))
                                .border_1()
                                .border_color(group_border_color())
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(app_logo(px(30.))),
                        )
                        .child(
                            div().pt(px(2.)).child(
                                Label::new("选择或新建一条便签")
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_PRIMARY)),
                            ),
                        )
                        .child(
                            Label::new("支持 Markdown 排版、标签筛选与托盘快速记事")
                                .text_size(px(11.5))
                                .text_color(rgb(INK_MUTED)),
                        )
                        .child(
                            div().pt(px(4.)).child(
                                h_flex()
                                    .id("btn-empty-new-note")
                                    .h(px(32.))
                                    .px_4()
                                    .gap_1p5()
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
                                        Icon::new(IconName::Plus)
                                            .size(px(13.))
                                            .text_color(rgb(0xFFFFFF)),
                                    )
                                    .child(
                                        Label::new("新建便签")
                                            .text_size(px(12.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(rgb(0xFFFFFF)),
                                    )
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.new_note(window, cx);
                                    })),
                            ),
                        ),
                );
        };

        let header = self.render_editor_header(&editor, theme, window, cx);

        let sheet_body = div()
            .w_full()
            .relative()
            // Title: borderless single-line field, visually part of the note sheet.
            .child(
                div()
                    .id("editor-title-row")
                    .w_full()
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
            // Whisper ink rule separating the note title from the markdown body.
            .child(
                div()
                    .w_full()
                    .px_8()
                    .child(
                        div()
                            .w_full()
                            .px(EDITOR_TEXT_INSET)
                            .child(div().w_full().h(px(1.)).bg(ink(0.042))),
                    ),
            )
            // Body: markdown editor with comfortable measure and breathing room.
            .child(
                div()
                    .id("editor-body-row")
                    .w_full()
                    .px_8()
                    .pt(px(16.))
                    .pb_6()
                    .child(BlockEditor::new(&editor).auto_grow(12, 60)),
            );

        let animated_sheet: AnyElement = match (&self.selected_id, reduce_motion) {
            (Some(sel_id), false) => sheet_body
                .with_animation(
                    ElementId::Name(format!("note-switch-{sel_id}").into()),
                    Animation::new(Duration::from_millis(135)).with_easing(ease_out_cubic),
                    |el, d| el.opacity(0.86 + 0.14 * d).top(px((1. - d) * 2.5)),
                )
                .into_any_element(),
            _ => sheet_body.into_any_element(),
        };

        let editor_content = div()
            .id("editor-canvas")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .text_color(rgb(0x232B36))
            .child(animated_sheet);

        // Footer left: the current note's tags + only busy/error states, all
        // rendered through `ink(alpha)` so every paper colour reads cleanly.
        let footer_tags: Vec<(i64, String)> = self
            .selected_id
            .as_deref()
            .map(|id| {
                self.tags_for_note(id)
                    .iter()
                    .map(|t| (t.id, t.name.clone()))
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
            .map(|n| ticks::format_ticks(n.updated_at))
            .unwrap_or_default();

        // 4.5:1 is the floor for small text; the old 0.45 sat at about 3.4:1
        // on the note papers.
        let meta_color = ink(0.58);
        let pill_bg = ink(0.055);
        let pill_fg = ink(0.62);

        // The footer shows the note's tags; creating and editing tags belongs
        // to the sidebar's tag section, where they are managed. A tag here is
        // still a shortcut: clicking it filters the list by that tag.
        let mut tag_chips: Vec<AnyElement> = Vec::new();
        for (idx, (tag_id, name)) in footer_tags.iter().enumerate() {
            let tag_id = *tag_id;
            tag_chips.push(
                div()
                    .id(ElementId::Name(format!("footer-tag-{idx}").into()))
                    .px_2()
                    .py(px(1.))
                    .rounded_full()
                    .bg(pill_bg)
                    .cursor_pointer()
                    .hover(|s| s.bg(ink(0.10)))
                    .active(|s| s.bg(ink(0.15)))
                    .child(
                        Label::new(format!("#{name}"))
                            .text_size(px(11.))
                            .text_color(pill_fg),
                    )
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label(format!("按标签 #{} 筛选", name))
                    .focus_visible(|s| s.shadow(focus_ring()))
                    .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
                        if matches!(ev.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            this.tag_filter = if this.tag_filter == Some(tag_id) {
                                None
                            } else {
                                Some(tag_id)
                            };
                            this.ensure_selection_after_filter(window, cx);
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.tag_filter = if this.tag_filter == Some(tag_id) {
                            None
                        } else {
                            Some(tag_id)
                        };
                        this.ensure_selection_after_filter(window, cx);
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }

        let tags_row = h_flex()
            .flex_1()
            .items_center()
            .gap_1p5()
            .min_w_0()
            .child(
                div()
                    .id("footer-tags-scroll")
                    .flex_1()
                    .min_w_0()
                    .overflow_x_scroll()
                    .scrollbar_width(px(0.))
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1p5()
                            .children(tag_chips),
                    ),
            );

        let footer = h_flex()
            .id("editor-footer")
            .w_full()
            .h(px(30.))
            .flex_none()
            .px_5()
            .pb(px(2.))
            .items_center()
            .justify_between()
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .child(tags_row)
                    .when_some(status_badge, |el, (is_err, text)| {
                        let color: Hsla = if is_err {
                            rgb(0xC04646).into()
                        } else {
                            meta_color
                        };
                        let icon_el: AnyElement = if is_err {
                            Icon::new(AssetIconName::CircleAlert)
                                .size(px(10.5))
                                .flex_none()
                                .text_color(color)
                                .into_any_element()
                        } else {
                            spinning_icon(
                                "footer-sync-spin",
                                AssetIconName::RefreshCw,
                                px(10.5),
                                color,
                                reduce_motion,
                            )
                        };
                        // The message can be long (a failing server's error);
                        // it must end in "…" before it reaches "修改于", never
                        // run under it. The flex below can only give the label
                        // the space that is left over; hiding the overflow turns
                        // that limit into the truncation point.
                        el.child(
                            h_flex()
                                .id("footer-status")
                                .min_w_0()
                                .overflow_hidden()
                                .items_center()
                                .gap_1()
                                .child(icon_el)
                                .child(
                                    div()
                                        .id("footer-status-text")
                                        .min_w_0()
                                        .overflow_hidden()
                                        .child(
                                            Label::new(text)
                                                .text_size(px(11.))
                                                .text_color(color)
                                                .whitespace_nowrap()
                                                .text_ellipsis(),
                                        ),
                                ),
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
                                .text_size(px(11.))
                                .text_color(meta_color),
                        )
                    })
                    .when(!current_note_updated.is_empty(), |el| {
                        el.child(
                            Label::new(format!("修改于 {current_note_updated}"))
                                .text_size(px(11.))
                                .text_color(meta_color),
                        )
                    }),
            );

        v_flex()
            .id("editor-pane")
            .flex_1()
            .min_w_0()
            .h_full()
            .rounded(px(20.))
            .bg(paper_bg)
            .border_1()
            .border_color(hsla(
                220. / 360.,
                0.20,
                0.16,
                if is_white_paper { 0.065 } else { 0.045 },
            ))
            .shadow(editor_paper_shadow(is_white_paper))
            .child(header)
            .child(editor_content)
            .child(footer)
    }
}

/// Crisp `#FFFFFF` capsule button inside grouped cards of the settings dialog.
/// It carries its hover and pressed styles (GPUI accepts each only once per
/// element); the caller adds `on_click`.
pub(crate) fn settings_pill(id: &'static str, label: &'static str, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(29.))
        .px(px(13.))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(9999.))
        .bg(rgb(0xFFFFFF))
        .border_1()
        .border_color(field_border_color())
        .shadow(ios_field_shadow())
        .when(enabled, |el| {
            el.cursor_pointer()
                .hover(|s| s.bg(rgb(0xFAFBFD)))
                .active(|s| s.bg(rgb(0xECEEF4)))
        })
        .child(
            Label::new(label)
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(if enabled { INK_PRIMARY } else { INK_MUTED })),
        )
}

/// Secondary porcelain capsule button for actions resting directly on a
/// `#FFFFFF` modal surface.
pub(crate) fn secondary_pill(id: &'static str, label: &'static str, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(31.))
        .px(px(14.))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(9999.))
        .bg(rgb(0xF3F5F8))
        .when(enabled, |el| {
            el.cursor_pointer()
                .hover(|s| s.bg(rgb(0xEAECEF)))
                .active(|s| s.bg(rgb(0xDFE3EC)))
        })
        .child(
            Label::new(label)
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(if enabled { INK_PRIMARY } else { INK_MUTED })),
        )
}

/// Slate capsule for the one primary action of a surface.
pub(crate) fn primary_pill(id: &'static str, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(31.))
        .px_4()
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(9999.))
        .bg(rgb(PRIMARY_BTN))
        .shadow(primary_pill_shadow())
        .cursor_pointer()
        .hover(|s| s.bg(rgb(PRIMARY_BTN_HOVER)))
        .active(|s| s.bg(rgb(PRIMARY_BTN_ACTIVE)))
        .child(
            Label::new(label)
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(0xFFFFFF)),
        )
}

/// Compact iOS pill switch used in the settings dialog, with spring thumb
/// travel and RGB track color transition.
pub(crate) fn settings_switch(
    id: &'static str,
    on: bool,
    accent: Rgba,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    let off_bg = rgb(0xD8DAE2);
    let off_hover = rgb(0xCBCDD7);
    let on_hover = rgb(0x336094);
    let track_bg = transition(
        (id, "track-bg"),
        RgbColor(if on { accent } else { off_bg }),
        Transition::new(Duration::from_millis(165)).reduced_motion(ReducedMotion::Continue),
        window,
        cx,
    )
    .0;
    let thumb_x = spring(
        (id, "thumb-x"),
        if on { 18.0_f32 } else { 2.0_f32 },
        Spring::new(Duration::from_millis(185))
            .with_damping(0.78)
            .with_epsilon(0.05),
        window,
        cx,
    );

    div()
        .id(id)
        .w(px(38.))
        .h(px(22.))
        .flex_none()
        .rounded_full()
        .bg(track_bg)
        .hover(move |s| s.bg(if on { on_hover } else { off_hover }))
        .active(|s| s.opacity(0.88))
        .cursor_pointer()
        .relative()
        .child(
            div()
                .absolute()
                .top(px(2.))
                .left(px(thumb_x))
                .size(px(18.))
                .rounded_full()
                .bg(rgb(0xFFFFFF))
                .shadow(vec![
                    specular_rim(false, 0.85),
                    BoxShadow {
                        inset: false,
                        color: hsla(225. / 360., 0.18, 0.12, 0.14),
                        offset: point(px(0.), px(1.5)),
                        blur_radius: px(3.5),
                        spread_radius: px(-0.5),
                    },
                ]),
        )
}
