use crate::state::{AppState, MdTip, StorageNote, SyncStatus, ThemeSpec, WebDavTest, THEMES};
use gpui_kit::assets::IconName as AssetIconName;
use gpui_kit::base::animation::ease_out_cubic;
use gpui_kit::base::{
    h_flex, spring, transition, v_flex, ElementExt as _, Interpolate, MotionReveal, Presence,
    ReducedMotion, Sequence, Spring, Transition,
};
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
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
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
// Unified Shell & iOS / iPadOS Airy Surface Tokens
// =========================================================================

/// iOS `systemGroupedBackground` cool mist framing the sidebar, card stream, and note sheet.
const SHELL_BG: u32 = 0xF2F2F7;
/// Primary ink for chrome, cards, and dialogs.
const INK_PRIMARY: u32 = 0x1C1C1E;
/// Secondary ink for navigation labels and snippets.
const INK_SECONDARY: u32 = 0x54565F;
/// Muted ink for counts, timestamps, and field labels.
const INK_MUTED: u32 = 0x80838F;
/// Primary button slate fill and states.
const PRIMARY_BTN: u32 = 0x243042;
const PRIMARY_BTN_HOVER: u32 = 0x324158;
const PRIMARY_BTN_ACTIVE: u32 = 0x1A2330;

/// Translucent foreground hover fill on the neutral shell.
fn shell_hover() -> Hsla {
    hsla(222. / 360., 0.18, 0.20, 0.05)
}

/// Translucent foreground press/active fill on the neutral shell.
fn shell_active() -> Hsla {
    hsla(222. / 360., 0.20, 0.20, 0.085)
}

/// Ink overlay for the note's paper: translucent black, so one alpha ramp
/// works on every paper colour and always lets it show through instead of
/// painting an opaque grey block. Assumes a light paper, which every theme is.
fn ink(alpha: f32) -> Hsla {
    hsla(0., 0., 0., alpha)
}

/// Borderless iOS material fill for floating capsules on note paper:
/// milky translucent white on tinted papers; subtle `quaternarySystemFill` on
/// pure `#FFFFFF` paper so the capsule remains legible without any dark border.
fn paper_capsule_bg(is_white_paper: bool) -> Hsla {
    if is_white_paper {
        hsla(222. / 360., 0.14, 0.22, 0.045)
    } else {
        hsla(0., 0., 1., 0.78)
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

fn lerp_rgba(a: Rgba, b: Rgba, t: f32) -> Rgba {
    Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

/// Soft inner top highlight catching overhead light on elevated iOS surfaces.
fn specular_rim(is_dark: bool, alpha_light: f32) -> BoxShadow {
    BoxShadow {
        inset: true,
        color: hsla(0., 0., 1., if is_dark { 0.13 } else { alpha_light }),
        offset: point(px(0.), px(1.)),
        blur_radius: px(0.),
        spread_radius: px(0.),
    }
}

/// Borderless iOS floating sheet elevation for the 4-corner-rounded note paper:
/// zero outer dark stroke — only an inner white top highlight and a wide,
/// negative-spread diffuse shadow so no dark halo appears on the perimeter.
fn editor_paper_shadow(is_white_paper: bool) -> Vec<BoxShadow> {
    vec![
        specular_rim(false, 0.90),
        BoxShadow {
            inset: false,
            color: hsla(
                225. / 360.,
                0.18,
                0.12,
                if is_white_paper { 0.048 } else { 0.036 },
            ),
            offset: point(px(0.), px(5.)),
            blur_radius: px(22.),
            spread_radius: px(-3.),
        },
    ]
}

/// Feather-light elevation for toolbar & swatch capsules inside the note sheet,
/// with zero dark perimeter hairline.
fn paper_capsule_shadow(is_white_paper: bool) -> Vec<BoxShadow> {
    if is_white_paper {
        Vec::new()
    } else {
        vec![
            specular_rim(false, 0.88),
            BoxShadow {
                inset: false,
                color: hsla(220. / 360., 0.16, 0.14, 0.020),
                offset: point(px(0.), px(1.5)),
                blur_radius: px(5.),
                spread_radius: px(-0.5),
            },
        ]
    }
}

/// Lifted borderless white pill for sidebar navigation items, scaled for cross-fades.
/// Outer shadow alpha uses `t^3` because GPUI draws non-inset `BoxShadow`
/// without cutting out the interior, which would otherwise bleed through a
/// semi-transparent white fill mid-transition.
fn nav_pill_shadow_faded(t: f32) -> Vec<BoxShadow> {
    let t = t.clamp(0., 1.);
    let t3 = t * t * t;
    vec![
        specular_rim(false, 0.90 * t),
        BoxShadow {
            inset: false,
            color: hsla(225. / 360., 0.18, 0.14, 0.032 * t3),
            offset: point(px(0.), px(1.5)),
            blur_radius: px(5.),
            spread_radius: px(-0.5),
        },
    ]
}

/// Borderless iOS white control / search field elevation over `#F2F2F7` grouped surfaces.
fn ios_field_shadow() -> Vec<BoxShadow> {
    ios_field_shadow_faded(1.)
}

fn ios_field_shadow_faded(t: f32) -> Vec<BoxShadow> {
    let t = t.clamp(0., 1.);
    let t3 = t * t * t;
    vec![
        specular_rim(false, 0.85 * t),
        BoxShadow {
            inset: false,
            color: hsla(225. / 360., 0.18, 0.14, 0.028 * t3),
            offset: point(px(0.), px(1.5)),
            blur_radius: px(6.),
            spread_radius: px(-0.5),
        },
    ]
}

/// Soft diffused elevation for primary dark-slate action capsules (no outer dark ring).
fn primary_pill_shadow() -> Vec<BoxShadow> {
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

/// Borderless iOS `InsetGrouped` card surface inside floating dialogs (`settings-storage`, etc.).
fn inset_group_shadow() -> Vec<BoxShadow> {
    vec![specular_rim(false, 0.72)]
}

/// Smooth iOS shadow interpolation for note cards (`t = 0` rest, `t = 1` selected).
/// Rest cards are completely borderless `#FFFFFF` islands on `#F2F2F7`; selected
/// cards gain a soft hue-matched pastel rim (`0.50` alpha, never mixed with black)
/// and gentle ambient lift without any layout jump.
fn card_interpolated_shadow(t: f32, active_border_hex: u32) -> Vec<BoxShadow> {
    let t = t.clamp(0., 1.);
    let mut active_edge = rgb(active_border_hex);
    active_edge.a = 0.50 * t;

    vec![
        specular_rim(false, 0.86 - 0.12 * t),
        BoxShadow {
            inset: false,
            color: active_edge.into(),
            offset: point(px(0.), px(0.)),
            blur_radius: px(0.),
            spread_radius: px(0.75 * t),
        },
        BoxShadow {
            inset: false,
            color: hsla(225. / 360., 0.18, 0.12, 0.025 + 0.020 * t),
            offset: point(px(0.), px(2. + 2. * t)),
            blur_radius: px(8. + 6. * t),
            spread_radius: px(-1. - 1. * t),
        },
    ]
}

/// Floating iOS modal sheet shadow (borderless, diffuse ambient cushion),
/// using `t^3` outer shadow scaling so blur ink never bleeds through the
/// translucent white card during enter/exit fades.
fn modal_surface_shadow_faded(t: f32) -> Vec<BoxShadow> {
    let t = t.clamp(0., 1.);
    let t3 = t * t * t;
    vec![
        specular_rim(false, 0.92 * t),
        BoxShadow {
            inset: false,
            color: hsla(225. / 360., 0.22, 0.08, 0.14 * t3),
            offset: point(px(0.), px(16.)),
            blur_radius: px(42.),
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
fn app_logo(size: Pixels) -> Img {
    static APP_ICON: LazyLock<Arc<Image>> = LazyLock::new(|| {
        Arc::new(Image::from_bytes(
            ImageFormat::Svg,
            include_bytes!("../assets/app-icon.svg").to_vec(),
        ))
    });
    img(APP_ICON.clone()).size(size).flex_none()
}

/// Smoothly rotating activity icon (`RefreshCw`) that respects reduced motion.
fn spinning_icon(
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
            .map(|tip| md_tip_overlay(tip, tip_presence.progress, window, cx));

        let settings_presence = Presence::new("settings-modal", self.show_settings)
            .transition(Transition::new(Duration::from_millis(185)))
            .sample(window, cx);
        let settings_modal = settings_presence
            .should_render()
            .then(|| self.render_settings_modal(settings_presence.progress, window, cx));

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
                    // Seamless iOS `#F2F2F7` canvas: borderless note-card stream
                    // in the middle + single 20px-rounded hero note sheet on the
                    // right, separated by luminance and whitespace rather than lines.
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
                            .gap(px(6.))
                            .child(self.render_note_list(window, cx))
                            .child(self.render_editor_area(current_theme, window, cx)),
                    ),
            )
            // Hover hint for the markdown bar, painted last so it floats over
            // the title below the toolbar.
            .when_some(tip_overlay, |el, overlay| el.child(overlay))
            .when_some(settings_modal, |el, modal| el.child(modal))
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
fn md_tip_overlay(tip: MdTip, progress: f32, window: &mut Window, cx: &mut App) -> impl IntoElement {
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
        .when(!tip.keys.is_empty(), |el| {
            el.child(
                div()
                    .px(px(4.5))
                    .py(px(0.5))
                    .rounded(px(4.5))
                    .bg(hsla(0., 0., 1., 0.11))
                    .child(
                        Label::new(tip.keys)
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
                    .active(|el| el.bg(rgb(PRIMARY_BTN_ACTIVE)))
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
                                .text_size(px(10.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(lerp_rgba(
                                    rgb(INK_MUTED),
                                    rgb(0x3B6EA8),
                                    all_sel_t,
                                )),
                        ),
                )
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
                    .when(self.show_sidebar_add_tag, |el| el.bg(shell_hover()))
                    .hover(|el| el.bg(shell_hover()))
                    .active(|el| el.bg(shell_active()))
                    .child(
                        Icon::new(IconName::Plus)
                            .size(px(12.))
                            .rotate(percentage(plus_rot))
                            .text_color(if self.show_sidebar_add_tag {
                                rgb(INK_PRIMARY)
                            } else {
                                rgb(INK_SECONDARY)
                            }),
                    )
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
                        .text_size(px(11.))
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
                                            .text_size(px(10.5))
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
        let (dot_hex, webdav_label) = if self.webdav_syncing {
            (0x4F86C6, "WebDAV 正在同步…")
        } else if webdav_on {
            (0x52A876, "WebDAV 自动同步")
        } else if webdav_configured {
            (0xD49B3E, "WebDAV 已配置")
        } else {
            (0xAAB3C0, "WebDAV 未配置")
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

        let status_dot: AnyElement = {
            let base = div()
                .size(px(6.5))
                .flex_none()
                .rounded_full()
                .bg(dot_color);
            if self.webdav_syncing && !reduce_motion {
                base.with_animation(
                    "sidebar-webdav-pulse",
                    Animation::new(Duration::from_millis(1300))
                        .repeat()
                        .with_easing(pulsating_between(0.38, 1.0)),
                    |el, alpha| el.opacity(alpha),
                )
                .into_any_element()
            } else {
                base.into_any_element()
            }
        };

        let footer_status = div()
            .px_2p5()
            .pb(px(8.))
            .pt_1()
            .child(
                h_flex()
                    .id("btn-settings")
                    .group("btn-settings")
                    .h(px(32.))
                    .px_2p5()
                    .rounded(px(8.5))
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
                            .child(status_dot)
                            .child(
                                Label::new(webdav_label)
                                    .text_size(px(11.))
                                    .text_color(rgb(INK_SECONDARY)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(rgb(INK_MUTED))
                            .group_hover("btn-settings", |s| s.text_color(rgb(INK_PRIMARY)))
                            .child(Icon::new(AssetIconName::Settings).size(px(13.5))),
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let notes = self.filtered_notes();
        let reduce_motion = cx.reduce_motion();
        let clear_search_presence = Presence::new("search-clear", !self.search_query.is_empty())
            .transition(Transition::new(Duration::from_millis(135)))
            .sample(window, cx);

        // Borderless iOS search field (`rounded(12)` `#FFFFFF` over `#F2F2F7`)
        // aligned to the same 46px header band as the note sheet toolbar.
        let search_box = match &self.search_input {
            Some(inp) => h_flex()
                .h(px(46.))
                .px_2p5()
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
                        .rounded(px(11.5))
                        .bg(rgb(0xFFFFFF))
                        .shadow(ios_field_shadow())
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
        let active_tag = self.tag_filter.and_then(|tid| {
            self.tags_list
                .iter()
                .find(|t| t.id == tid)
                .map(|t| (t.id, t.name.clone()))
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
                        .text_size(px(11.))
                        .text_color(rgb(INK_MUTED)),
                    ),
            );

        // Cards list: padding stays inside `cards-scroll` so GPUI's scroll
        // `ContentMask` never slices off the cards' diffuse ambient shadow.
        let mut card_list = v_flex().px_2p5().pt_1().pb_3().gap(px(7.));
        if notes.is_empty() {
            let empty_state = v_flex()
                .py_12()
                .relative()
                .items_center()
                .justify_center()
                .gap_2p5()
                .child(app_logo(px(38.)).opacity(0.85))
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
                );
            card_list = if reduce_motion {
                card_list.child(empty_state)
            } else {
                card_list.child(empty_state.with_animation(
                    "empty-notes-enter",
                    Animation::new(Duration::from_millis(170)).with_easing(ease_out_cubic),
                    |el, d| el.opacity(d).top(px((1. - d) * 4.)),
                ))
            };
        } else {
            for (i, note) in notes.iter().enumerate() {
                card_list = card_list.child(self.render_note_card(i, note, window, cx));
            }
        }

        // Borderless iOS `InsetGrouped` column directly on `SHELL_BG` (`#F2F2F7`):
        // removing the outer boxed wrapper lets `#FFFFFF` search & note cards
        // separate cleanly by pure luminance without stacked container outlines.
        v_flex()
            .id("note-list-pane")
            .w(LIST_PANE_W)
            .h_full()
            .flex_none()
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
        let note_theme = crate::state::get_theme(&note.theme);
        let sel_t = transition(
            (ElementId::Name(format!("card-sel-{}", note.id).into()), "t"),
            if is_selected { 1.0_f32 } else { 0.0_f32 },
            Transition::new(Duration::from_millis(160)),
            window,
            cx,
        );
        let bg_color = lerp_rgba(rgb(0xFFFFFF), rgb(note_theme.card_active_bg), sel_t);

        let card = h_flex()
            .id(ElementId::Name(format!("note-card-{idx}").into()))
            .w_full()
            .pl_3()
            .pr(px(13.))
            .py(px(10.))
            .gap_2()
            .rounded(px(13.5))
            .cursor_pointer()
            .bg(bg_color)
            .shadow(card_interpolated_shadow(sel_t, note_theme.border))
            .when(!is_selected, |el| {
                el.hover(|s| s.bg(rgb(0xFAFBFD)))
                    .active(|s| s.bg(rgb(0xEFEFF4)))
            });

        // Card body content
        let mut content = v_flex().flex_1().min_w_0().gap(px(3.));

        // Title row: theme color dot + Title + Pin + Relative Timestamp
        let title_text = if note.title.trim().is_empty() {
            "(无标题便签)"
        } else {
            &note.title
        };
        let dot_size = px(7.5 + 1.0 * sel_t);

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
                            .shadow(vec![specular_rim(false, 0.35)]),
                    ),
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
                    .text_color(rgb(INK_SECONDARY))
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

        let sep_bg = ink(0.06);
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
                                keys,
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
                                    keys,
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

        let run = |cmd: EditorCommand| {
            let ed = editor.clone();
            let app = app.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                app.update(cx, |this, cx| {
                    this.md_tip = None;
                    this.md_tip_closed_at = None;
                    cx.notify();
                });
                ed.update(cx, |s, cx| {
                    let _ = s.run(cmd.clone(), cx);
                    s.focus(window, cx);
                });
            }
        };

        let mut btn = |id: &'static str,
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
                window,
                cx,
            )
        };
        let sep = || div().w(px(1.)).h(px(12.)).mx(px(2.)).flex_none().bg(sep_bg);

        let code_block_cmd = if in_code_block {
            EditorCommand::SetBlockKind(BlockKind::Paragraph)
        } else {
            EditorCommand::SetCodeBlock { lang: None }
        };

        let mut row = h_flex().h_full().flex_none().items_center().gap(px(1.));
        // 1. Heading level.
        row = row.child(btn(
            "md-para",
            AssetIconName::Pilcrow,
            false,
            heading_enabled,
            "正文",
            "Ctrl+Alt+0",
            EditorCommand::SetBlockKind(BlockKind::Paragraph),
        ));
        row = row.child(btn(
            "md-h1",
            AssetIconName::Heading1,
            heading(1),
            heading_enabled,
            "一级标题",
            "",
            EditorCommand::SetBlockKind(BlockKind::Heading { level: 1 }),
        ));
        row = row.child(btn(
            "md-h2",
            AssetIconName::Heading2,
            heading(2),
            heading_enabled,
            "二级标题",
            "",
            EditorCommand::SetBlockKind(BlockKind::Heading { level: 2 }),
        ));
        row = row.child(btn(
            "md-h3",
            AssetIconName::Heading3,
            heading(3),
            heading_enabled,
            "三级标题",
            "",
            EditorCommand::SetBlockKind(BlockKind::Heading { level: 3 }),
        ));
        row = row.child(sep());
        // 2. Inline styles.
        row = row.child(btn(
            "md-bold",
            AssetIconName::Bold,
            bold,
            marks_enabled,
            "加粗",
            "Ctrl+B",
            EditorCommand::ToggleMark(MarkKind::Bold),
        ));
        row = row.child(btn(
            "md-italic",
            AssetIconName::Italic,
            italic,
            marks_enabled,
            "斜体",
            "Ctrl+I",
            EditorCommand::ToggleMark(MarkKind::Italic),
        ));
        row = row.child(btn(
            "md-strike",
            AssetIconName::Strikethrough,
            strike,
            marks_enabled,
            "删除线",
            "",
            EditorCommand::ToggleMark(MarkKind::Strikethrough),
        ));
        row = row.child(btn(
            "md-code",
            AssetIconName::Code,
            code,
            marks_enabled,
            "行内代码",
            "Ctrl+E",
            EditorCommand::ToggleMark(MarkKind::Code),
        ));
        row = row.child(btn(
            "md-highlight",
            AssetIconName::Highlighter,
            highlight,
            marks_enabled,
            "文本高亮",
            "",
            EditorCommand::ToggleMark(MarkKind::Highlight),
        ));
        row = row.child(sep());
        // 3. Lists and block structure.
        row = row.child(btn(
            "md-bullet",
            AssetIconName::List,
            in_bullet,
            true,
            "无序列表",
            "",
            EditorCommand::ToggleList { ordered: false },
        ));
        row = row.child(btn(
            "md-ordered",
            AssetIconName::ListOrdered,
            in_ordered,
            true,
            "有序列表",
            "",
            EditorCommand::ToggleList { ordered: true },
        ));
        row = row.child(btn(
            "md-task",
            AssetIconName::ListChecks,
            in_task,
            true,
            "待办清单",
            "",
            EditorCommand::ToggleTask,
        ));
        row = row.child(btn(
            "md-quote",
            AssetIconName::TextQuote,
            in_quote,
            true,
            "引用块",
            "",
            EditorCommand::ToggleBlockquote,
        ));
        row = row.child(btn(
            "md-codeblock",
            AssetIconName::SquareCode,
            in_code_block,
            true,
            "代码块",
            "",
            code_block_cmd,
        ));
        row = row.child(btn(
            "md-hr",
            AssetIconName::Minus,
            false,
            true,
            "分割线",
            "",
            EditorCommand::InsertBlock(BlockKind::HorizontalRule),
        ));
        row = row.child(btn(
            "md-table",
            AssetIconName::Table,
            false,
            true,
            "插入表格",
            "",
            EditorCommand::InsertTable { rows: 3, cols: 3 },
        ));

        // Borderless iOS translucent capsule: outer pill owns the rounded fill
        // and whisper elevation; inner div handles horizontal scroll if the
        // window is narrowed, so `ContentMask` never clips the pill's corners.
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
                    .id("md-toolbar-scroll")
                    .min_w_0()
                    .h_full()
                    .overflow_x_scroll()
                    .scrollbar_width(px(0.))
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
            let is_cur = t.name == current_theme.name;
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
            let base_alpha = (0.86 + 0.14 * sel_t).clamp(0., 1.);

            let mut circle = div()
                .size(diameter)
                .flex_none()
                .rounded_full()
                .bg(rgb(t.accent))
                .opacity(base_alpha)
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
                .bg(paper_bg)
                .shadow(editor_paper_shadow(is_white_paper))
                .items_center()
                .justify_center()
                .child(
                    v_flex()
                        .items_center()
                        .gap_3()
                        .child(app_logo(px(46.)).opacity(0.88))
                        .child(
                            Label::new("选择左侧便签，或点击“新建便签”")
                                .text_size(px(13.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(ink(0.45)),
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
            // Whisper ink rule separating the note title from the markdown body.
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
                            .child(div().w_full().h(px(1.)).bg(ink(0.042))),
                    ),
            )
            // Body: markdown editor with comfortable measure and breathing room.
            .child(
                div()
                    .id("editor-body-row")
                    .w_full()
                    .max_w(px(740.))
                    .mx_auto()
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
            .map(|n| ticks::format_ticks(n.updated_at))
            .unwrap_or_default();

        let meta_color = ink(0.45);
        let pill_bg = ink(0.055);
        let pill_fg = ink(0.62);

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
                    .items_center()
                    .gap_1p5()
                    .min_w_0()
                    .children(footer_tags.iter().enumerate().map(|(idx, name)| {
                        div()
                            .id(ElementId::Name(format!("footer-tag-{idx}").into()))
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
                        let color: Hsla = if is_err {
                            rgb(0xC04646).into()
                        } else {
                            meta_color
                        };
                        let icon_el: AnyElement = if is_err {
                            Icon::new(AssetIconName::CircleAlert)
                                .size(px(10.5))
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
                        el.child(
                            h_flex()
                                .items_center()
                                .gap_1()
                                .child(icon_el)
                                .child(Label::new(text).text_size(px(10.5)).text_color(color)),
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
            .rounded(px(20.))
            .bg(paper_bg)
            .shadow(editor_paper_shadow(is_white_paper))
            .child(header)
            .child(editor_content)
            .child(footer)
    }

    // =========================================================================
    // Settings modal (iOS `InsetGrouped` surface + diffuse elevation)
    // =========================================================================
    fn render_settings_modal(
        &mut self,
        progress: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let progress = progress.clamp(0., 1.);
        let reduce_motion = cx.reduce_motion();
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
                        .h(px(34.))
                        .px_3()
                        .flex()
                        .items_center()
                        .rounded(px(10.5))
                        .bg(rgb(SHELL_BG))
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

        // Keep horizontal padding (20px) and vertical breathing room (4px)
        // inside `settings-body` (`overflow_y_scroll`) rather than on the outer
        // card, so GPUI's scroll `ContentMask` never slices off outer shadows
        // on the left/right edges of nested controls.
        let mut rows = v_flex().px(px(20.)).py(px(4.)).gap(px(14.));

        // --- Data storage (concentric rounded(14) inside rounded(22) modal) ---
        let default_location = self.config.uses_default_data_dir();
        let picking = self.picking_folder;
        let mut storage = v_flex()
            .id("settings-storage")
            .rounded(px(14.))
            .bg(rgb(SHELL_BG))
            .shadow(inset_group_shadow())
            .px(px(14.))
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
                                            .text_color(accent),
                                    )
                                    .child(
                                        Label::new("便签数据库")
                                            .text_size(px(12.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(text),
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
                .shadow(vec![specular_rim(false, 0.75)])
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
            let (color, icon, message) = match note {
                StorageNote::Info(m) => (rgb(0x2A8555), AssetIconName::CircleCheck, m.clone()),
                StorageNote::Error(m) => (rgb(0xC04646), AssetIconName::CircleAlert, m.clone()),
            };
            let note_row = h_flex()
                .id("settings-storage-note")
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
            storage = if reduce_motion {
                storage.child(note_row)
            } else {
                storage.child(note_row.with_animation(
                    "settings-storage-note-enter",
                    Animation::new(Duration::from_millis(155)).with_easing(ease_out_cubic),
                    |el, d| el.opacity(d).top(px((1. - d) * 2.5)),
                ))
            };
        }
        rows = rows.child(storage);

        // --- Sync strategy switch ---
        let webdav_on = self.config.webdav_auto;
        rows = rows.child(
            div()
                .rounded(px(14.))
                .bg(rgb(SHELL_BG))
                .shadow(inset_group_shadow())
                .px(px(14.))
                .py(px(12.))
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
                            settings_switch("sw-webdav", webdav_on, accent, window, cx).on_click(
                                cx.listener(|this, _, _window, cx| {
                                    let on = !this.config.webdav_auto;
                                    this.set_webdav_auto(on, cx);
                                }),
                            ),
                        ),
                ),
        );

        // --- WebDAV form ---
        let mut form = v_flex().gap(px(11.));
        if let Some(inputs) = &self.settings {
            form = form
                .child(field(
                    "settings-url",
                    "服务器地址（URL）",
                    &inputs.url,
                    false,
                    muted,
                ))
                // Credentials share a row so the dialog fits the default
                // window height without scrolling.
                .child(
                    h_flex()
                        .items_start()
                        .gap(px(11.))
                        .child(
                            field("settings-user", "用户名", &inputs.user, false, muted)
                                .flex_1()
                                .min_w_0(),
                        )
                        .child(
                            field("settings-pass", "密码", &inputs.pass, true, muted)
                                .flex_1()
                                .min_w_0(),
                        ),
                )
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
                h_flex()
                    .id("btn-webdav-test")
                    .h(px(31.))
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
                            .active(|s| s.bg(rgb(PRIMARY_BTN_ACTIVE)))
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
                    .h(px(31.))
                    .px_4()
                    .gap_1p5()
                    .items_center()
                    .rounded(px(9999.))
                    .bg(rgb(SHELL_BG))
                    .when(!self.webdav_syncing, |el| {
                        el.cursor_pointer()
                            .hover(|s| s.bg(rgb(0xE8E9F0)))
                            .active(|s| s.bg(rgb(0xDDDFE8)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_webdav_sync(window, cx, true)
                            }))
                    })
                    .when(self.webdav_syncing, |el| {
                        el.child(spinning_icon(
                            "webdav-sync-spin",
                            AssetIconName::RefreshCw,
                            px(11.5),
                            muted,
                            reduce_motion,
                        ))
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

        // Never taller than the window: the body scrolls instead.
        let max_height = px((window.viewport_size().height.as_f32() - 44.).max(320.));
        let card = v_flex()
            .id("settings-card")
            .w(px(510.))
            .max_h(max_height)
            .relative()
            .top(px((1. - progress) * 8.))
            .opacity(progress)
            .rounded(px(22.))
            .bg(rgb(0xFFFFFF))
            .shadow(modal_surface_shadow_faded(progress))
            .pt(px(20.))
            .pb(px(17.))
            .gap(px(13.))
            .child(
                h_flex()
                    .px(px(20.))
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
                                Label::new("数据存储与同步")
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
            .bg(hsla(220. / 360., 0.22, 0.08, 0.36 * progress))
            .on_click(cx.listener(|this, _, window, cx| {
                if this.show_settings {
                    this.close_settings(window, cx);
                }
            }))
            .on_key_down(
                cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if this.show_settings && event.keystroke.key.eq_ignore_ascii_case("escape") {
                        this.close_settings(window, cx);
                    }
                }),
            )
            .child(card)
            .into_any_element()
    }
}

/// Borderless white capsule button inside `#F2F2F7` grouped cards of the settings
/// dialog. It carries its hover and pressed styles (GPUI accepts each only once
/// per element); the caller adds `on_click`.
fn settings_pill(id: &'static str, label: &'static str, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(29.))
        .px(px(13.))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(9999.))
        .bg(rgb(0xFFFFFF))
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

/// Borderless `#F2F2F7` secondary capsule button for actions resting directly
/// on a `#FFFFFF` modal surface.
fn secondary_pill(id: &'static str, label: &'static str, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(31.))
        .px(px(14.))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(9999.))
        .bg(rgb(SHELL_BG))
        .when(enabled, |el| {
            el.cursor_pointer()
                .hover(|s| s.bg(rgb(0xE8E9F0)))
                .active(|s| s.bg(rgb(0xDDDFE8)))
        })
        .child(
            Label::new(label)
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(if enabled { INK_PRIMARY } else { INK_MUTED })),
        )
}

/// Slate capsule for the one primary action of a surface.
fn primary_pill(id: &'static str, label: &'static str) -> Stateful<Div> {
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
fn settings_switch(
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
