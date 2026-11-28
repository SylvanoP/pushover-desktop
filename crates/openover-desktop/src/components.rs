//! The controls the window is built from.
//!
//! Every one of them carries its own hover, pressed and keyboard-focus state, so
//! that a button looks and behaves the same wherever it appears. Nothing here
//! reaches for a colour or a length directly: the values come from
//! [`crate::form_theme`].

use gpui::{
    AnimationExt as _, AnyElement, App, ClickEvent, ElementId, IntoElement, Motion, SharedString,
    Stateful, Styled, Window, div, ease_out_quint, prelude::*, px,
};
use std::time::Duration;

use crate::form_theme::{
    CONTROL_HEIGHT, CONTROL_HEIGHT_SM, FormTheme, LABEL_WIDTH, MessageKind, RADIUS_MD, RADIUS_SM,
    RING_WIDTH, space,
};
use crate::icons::{icon, icon_size, icon_size_sm};

/// How long a control takes to settle into its hover or pressed look. Short
/// enough to feel immediate, long enough not to flicker when the pointer
/// crosses a row of buttons.
const FEEDBACK: Duration = Duration::from_millis(110);

/// What a button is for, which decides how loudly it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonVariant {
    /// The one action a pane is about. Filled with the accent.
    Primary,
    /// A supporting action. Outlined, quiet until hovered.
    Secondary,
    /// An action that sits inside a row of content. No fill at rest.
    Ghost,
    /// Deletes or discards something. Reads in the error colour.
    Danger,
}

/// Adds hover, pressed and focus-ring behaviour, and transitions between them.
///
/// Used by every control here, which is what keeps them consistent. The ring is
/// drawn only for keyboard focus: showing it after a mouse click looks like a
/// rendering fault.
fn interactive<E>(
    element: E,
    theme: &FormTheme,
    press_bg: gpui::Rgba,
    hover: impl FnOnce(gpui::StyleRefinement) -> gpui::StyleRefinement,
) -> E
where
    E: Styled + InteractiveElement + StatefulInteractiveElement,
{
    let ring = theme.ring;
    element
        .cursor_pointer()
        .tab_stop(true)
        .hover(hover)
        .active(move |style| style.bg(press_bg))
        .focus_visible(move |style| style.ring(RING_WIDTH).ring_color(ring))
        .transitions(|transitions| {
            transitions
                .bg(Motion::new(FEEDBACK).with_easing(ease_out_quint()))
                .border_color(Motion::new(FEEDBACK))
                .text_color(Motion::new(FEEDBACK))
                .ring(Motion::new(FEEDBACK))
        })
}

/// A button with a label. Full width, so the caller decides how wide it is.
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    variant: ButtonVariant,
    theme: &FormTheme,
) -> Stateful<gpui::Div> {
    button_with(id, label, None, variant, theme)
}

/// The same button, with a leading icon.
pub fn button_with(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    glyph: Option<&'static [u8]>,
    variant: ButtonVariant,
    theme: &FormTheme,
) -> Stateful<gpui::Div> {
    let label_color = match variant {
        ButtonVariant::Primary => theme.on_accent,
        ButtonVariant::Danger => theme.stopped,
        _ => theme.text,
    };
    let base = div()
        .id(id)
        .h(CONTROL_HEIGHT)
        .px(space(4))
        .flex()
        .items_center()
        .justify_center()
        .gap(space(2))
        .flex_shrink_0()
        .rounded(RADIUS_MD)
        .text_sm()
        .when_some(glyph, |button, glyph| {
            button.child(icon(glyph, icon_size_sm(), label_color))
        })
        .child(label.into());

    match variant {
        ButtonVariant::Primary => {
            let hover_bg = theme.accent_hover;
            interactive(
                base.bg(theme.accent).text_color(theme.on_accent),
                theme,
                theme.accent_pressed,
                move |style| style.bg(hover_bg),
            )
        }
        ButtonVariant::Secondary => {
            let (hover_bg, hover_border) = (theme.hover, theme.border_hover);
            interactive(
                base.border_1()
                    .border_color(theme.border)
                    .bg(theme.surface)
                    .text_color(theme.text),
                theme,
                theme.pressed,
                move |style| style.bg(hover_bg).border_color(hover_border),
            )
        }
        ButtonVariant::Ghost => {
            let hover_bg = theme.hover;
            interactive(
                base.bg(theme.idle).text_color(theme.text),
                theme,
                theme.pressed,
                move |style| style.bg(hover_bg),
            )
        }
        ButtonVariant::Danger => {
            let hover_bg = theme.hover;
            interactive(
                base.bg(theme.idle).text_color(theme.stopped),
                theme,
                theme.pressed,
                move |style| style.bg(hover_bg),
            )
        }
    }
}

/// A compact action that sits inside a row of content, such as "Kopieren" next
/// to a message. Same states as a button, just smaller and without a fill.
pub fn inline_action(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    variant: ButtonVariant,
    theme: &FormTheme,
) -> Stateful<gpui::Div> {
    inline_action_with(id, label, None, variant, theme)
}

/// The same action, with a leading icon.
pub fn inline_action_with(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    glyph: Option<&'static [u8]>,
    variant: ButtonVariant,
    theme: &FormTheme,
) -> Stateful<gpui::Div> {
    let color = match variant {
        ButtonVariant::Danger => theme.stopped,
        ButtonVariant::Primary => theme.accent,
        _ => theme.quiet,
    };
    let hover_bg = theme.hover;
    interactive(
        div()
            .id(id)
            .h(CONTROL_HEIGHT_SM)
            .px(space(2))
            .flex()
            .items_center()
            .gap(space(1))
            .flex_shrink_0()
            .rounded(RADIUS_SM)
            .bg(theme.idle)
            .text_xs()
            .text_color(color)
            .when_some(glyph, |action, glyph| {
                action.child(icon(glyph, px(12.), color))
            })
            .child(label.into()),
        theme,
        theme.pressed,
        move |style| style.bg(hover_bg),
    )
}

/// A square button holding a single glyph, for stepping a value up or down.
///
/// A disabled stepper keeps its space but drops every interactive state, so it
/// cannot be hovered, pressed or focused into.
pub fn icon_button(
    id: impl Into<ElementId>,
    glyph: &'static [u8],
    enabled: bool,
    theme: &FormTheme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let base = div()
        .id(id)
        .size(CONTROL_HEIGHT_SM)
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .rounded(RADIUS_SM)
        .border_1();
    if !enabled {
        return base
            .border_color(theme.border)
            .child(icon(glyph, icon_size_sm(), theme.placeholder))
            .into_any_element();
    }
    let base = base.child(icon(glyph, icon_size_sm(), theme.text));
    let (hover_bg, hover_border) = (theme.hover, theme.border_hover);
    interactive(
        base.bg(theme.idle)
            .border_color(theme.border)
            .text_color(theme.text),
        theme,
        theme.pressed,
        move |style| style.bg(hover_bg).border_color(hover_border),
    )
    .on_click(on_click)
    .into_any_element()
}

/// An external link. Underlined on hover only, so a row of links does not read
/// as a block of underlines.
pub fn link(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    url: &'static str,
    theme: &FormTheme,
) -> Stateful<gpui::Div> {
    let hover_bg = theme.hover;
    interactive(
        div()
            .id(id)
            .h(CONTROL_HEIGHT_SM)
            .px(space(1))
            .flex()
            .items_center()
            .rounded(RADIUS_SM)
            .bg(theme.idle)
            .text_color(theme.accent)
            .child(label.into()),
        theme,
        theme.pressed,
        move |style| style.bg(hover_bg).underline(),
    )
    .on_click(move |_, _, cx| cx.open_url(url))
}

/// One tab in the window's tab strip.
///
/// The active marker is its own square bar at the bottom edge, not a rounded
/// border: a border follows the element's corner radius and ends up looking bent
/// at both ends.
pub fn tab(
    id: impl Into<ElementId>,
    label: &'static str,
    glyph: &'static [u8],
    active: bool,
    index: isize,
    theme: &FormTheme,
) -> Stateful<gpui::Div> {
    let ring = theme.ring;
    let hover_bg = theme.hover;
    let hover_text = theme.text;
    div()
        .id(id)
        .tab_stop(true)
        .tab_index(index)
        .cursor_pointer()
        .relative()
        .h(CONTROL_HEIGHT)
        .px(space(3))
        .flex()
        .items_center()
        .gap(space(2))
        .bg(theme.idle)
        .text_color(if active { theme.text } else { theme.quiet })
        .hover(move |style| style.text_color(hover_text).bg(hover_bg))
        .active(move |style| style.bg(theme.pressed))
        .focus_visible(move |style| style.ring(RING_WIDTH).ring_color(ring))
        .transitions(|transitions| {
            transitions
                .bg(Motion::new(FEEDBACK))
                .text_color(Motion::new(FEEDBACK))
                .ring(Motion::new(FEEDBACK))
        })
        .child(icon(
            glyph,
            icon_size_sm(),
            if active { theme.text } else { theme.quiet },
        ))
        .child(label)
        .child(
            div()
                .absolute()
                .bottom(px(-1.))
                .left_0()
                .right_0()
                .h(px(2.))
                .bg(if active {
                    theme.accent
                } else {
                    gpui::Rgba {
                        alpha: 0.,
                        ..theme.accent
                    }
                })
                .transitions(|transitions| transitions.bg(Motion::new(FEEDBACK))),
        )
}

/// A group of settings under a heading.
pub fn section(title: &'static str, theme: &FormTheme) -> gpui::Div {
    div().flex().flex_col().gap(space(3)).child(
        div()
            .text_xs()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(theme.quiet)
            .child(title),
    )
}

/// One settings row: label on the left in a fixed column, control on the right,
/// optional hint underneath.
///
/// The fixed label column is what gives the pane a vertical edge. Every row uses
/// it, so the controls line up no matter how long their labels are.
pub fn setting_row(
    label: &'static str,
    control: impl IntoElement,
    hint: Option<SharedString>,
    theme: &FormTheme,
) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(space(1))
        .child(
            div()
                .flex()
                .items_center()
                .gap(space(3))
                .child(
                    div()
                        .w(LABEL_WIDTH)
                        .flex_shrink_0()
                        .text_color(theme.text)
                        .child(label),
                )
                .child(div().flex_1().child(control)),
        )
        .when_some(hint, |row, hint| {
            row.child(
                div()
                    .pl(LABEL_WIDTH + space(3))
                    .text_xs()
                    .text_color(theme.quiet)
                    .child(hint),
            )
        })
}

/// A label above a field, with the field's error beneath it when there is one.
///
/// The error stays at the field rather than going to the toast area: a message
/// that says "enter an address" is no help if it does not say where.
pub fn field_group(
    label: &'static str,
    field: impl IntoElement,
    error: Option<SharedString>,
    theme: &FormTheme,
) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(space(2))
        .child(div().text_color(theme.text).child(label))
        .child(field)
        .when_some(error, |group, error| {
            group.child(message_line(error, MessageKind::Error, theme))
        })
}

/// A line of feedback: a glyph, then the text, in the colour of its kind.
pub fn message_line(
    text: impl Into<SharedString>,
    kind: MessageKind,
    theme: &FormTheme,
) -> gpui::Div {
    let color = kind.color(theme);
    div()
        .flex()
        .items_start()
        .gap(space(2))
        .text_xs()
        .text_color(color)
        .child(icon(kind.glyph(), px(14.), color).mt(px(1.)))
        .child(div().flex_1().whitespace_normal().child(text.into()))
}

/// What is shown where content would be, when there is none.
///
/// Carries an accent glyph and a hint, because a lone grey sentence reads like
/// something failed rather than like nothing is there yet.
pub fn empty_state(
    glyph: &'static [u8],
    title: impl Into<SharedString>,
    hint: Option<SharedString>,
    theme: &FormTheme,
) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(space(2))
        .py(space(8))
        .child(
            div()
                .size(px(44.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(theme.accent_soft)
                .text_color(theme.accent)
                .child(icon(glyph, icon_size(), theme.accent)),
        )
        .child(div().text_color(theme.text).child(title.into()))
        .when_some(hint, |state, hint| {
            state.child(
                div()
                    .text_xs()
                    .text_color(theme.quiet)
                    .whitespace_normal()
                    .child(hint),
            )
        })
}

/// An on/off switch. The knob slides, so the state is readable from the shape
/// alone rather than only from the colour.
pub fn switch(
    id: &'static str,
    on: bool,
    index: isize,
    theme: &FormTheme,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let ring = theme.ring;
    let track = div()
        .id(SharedString::from(id))
        .tab_stop(true)
        .tab_index(index)
        .cursor_pointer()
        .w(px(40.))
        .h(px(22.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .p(px(2.))
        .rounded_full()
        .border_1()
        .border_color(if on { theme.accent } else { theme.border })
        .bg(if on { theme.accent } else { theme.field })
        .focus_visible(move |style| style.ring(RING_WIDTH).ring_color(ring))
        .transitions(|transitions| {
            transitions
                .bg(Motion::new(FEEDBACK))
                .border_color(Motion::new(FEEDBACK))
                .ring(Motion::new(FEEDBACK))
                .pl(Motion::new(FEEDBACK).with_easing(ease_out_quint()))
        })
        .pl(if on { px(20.) } else { px(2.) })
        .child(div().size(px(14.)).rounded_full().bg(if on {
            theme.on_accent
        } else {
            theme.placeholder
        }));
    track.on_click(on_toggle)
}

/// A switch that shows a fixed state and cannot be changed, such as the
/// notification switch of an emergency.
pub fn switch_locked(on: bool, theme: &FormTheme) -> impl IntoElement {
    div()
        .w(px(40.))
        .h(px(22.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .p(px(2.))
        .rounded_full()
        .border_1()
        .border_color(if on { theme.accent } else { theme.border })
        .bg(if on { theme.accent } else { theme.field })
        .opacity(0.45)
        .pl(if on { px(20.) } else { px(2.) })
        .child(div().size(px(14.)).rounded_full().bg(if on {
            theme.on_accent
        } else {
            theme.placeholder
        }))
}

/// The closed state of a drop-down: the chosen value and a chevron.
pub fn select_trigger(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    open: bool,
    enabled: bool,
    theme: &FormTheme,
) -> Stateful<gpui::Div> {
    let base = div()
        .id(id)
        .w_full()
        .h(CONTROL_HEIGHT)
        .px(space(3))
        .flex()
        .items_center()
        .gap(space(2))
        .rounded(RADIUS_MD)
        .border_1()
        .border_color(if open { theme.accent } else { theme.border })
        .bg(theme.field)
        .text_sm()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(label.into()),
        )
        .child(icon(
            crate::icons::CHEVRON_DOWN,
            icon_size_sm(),
            if enabled {
                theme.quiet
            } else {
                theme.placeholder
            },
        ));
    if !enabled {
        return base.text_color(theme.placeholder);
    }
    let (hover_bg, hover_border) = (theme.field_hover, theme.border_hover);
    let hover_border = if open { theme.accent } else { hover_border };
    interactive(
        base.text_color(theme.text),
        theme,
        theme.pressed,
        move |style| style.bg(hover_bg).border_color(hover_border),
    )
}

/// The floating panel a drop-down opens into.
pub fn menu_panel(id: impl Into<ElementId>, theme: &FormTheme) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap(px(1.))
        .p(space(1))
        .rounded(RADIUS_MD)
        .border_1()
        .border_color(theme.border)
        .bg(theme.raised)
        .shadow_lg()
        .overflow_y_scroll()
}

/// One choice in a drop-down. The selected one carries a check mark, so the
/// choice reads without relying on colour.
pub fn menu_item(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    theme: &FormTheme,
) -> Stateful<gpui::Div> {
    let hover_bg = theme.hover;
    interactive(
        div()
            .id(id)
            .h(CONTROL_HEIGHT_SM + space(1))
            .pl(space(2))
            .pr(space(1))
            .flex()
            .items_center()
            .gap(space(2))
            .flex_shrink_0()
            .rounded(RADIUS_SM)
            .bg(theme.idle)
            .text_sm()
            .text_color(theme.text)
            .child(div().w(px(14.)).flex_shrink_0().when(selected, |slot| {
                slot.child(icon(crate::icons::CHECK, px(14.), theme.accent))
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(label.into()),
            ),
        theme,
        theme.pressed,
        move |style| style.bg(hover_bg),
    )
}

/// What a sound button shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayState {
    /// Ready to play.
    Idle,
    /// Downloading the sound before it can play. Clicking cancels.
    Loading,
    /// Playing right now. Clicking stops.
    Playing,
    /// The last attempt failed. Clicking tries again.
    Failed,
    /// Nothing to play.
    Disabled,
}

/// A small round button that plays a sound and shows where that sound is:
/// loading, playing (as a stop button on an accent fill), or failed.
///
/// Every state stays clickable except `Disabled`, so a user running through
/// the list never has to wait for one sound before trying the next.
pub fn play_button(
    id: impl Into<ElementId>,
    state: PlayState,
    theme: &FormTheme,
) -> Stateful<gpui::Div> {
    let base = div()
        .id(id)
        .size(CONTROL_HEIGHT_SM)
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .rounded_full();
    let label = match state {
        PlayState::Idle => "Anhören",
        PlayState::Loading => "Lädt, klicken zum Abbrechen",
        PlayState::Playing => "Stoppen",
        PlayState::Failed => "Fehlgeschlagen, erneut versuchen",
        PlayState::Disabled => "Kein Ton",
    };
    let base = base.aria_label(label);
    match state {
        PlayState::Disabled => base.child(icon(crate::icons::PLAY, px(12.), theme.placeholder)),
        PlayState::Playing => {
            let hover_bg = theme.accent_hover;
            interactive(
                base.bg(theme.accent)
                    .child(icon(crate::icons::STOP, px(10.), theme.on_accent)),
                theme,
                theme.accent_pressed,
                move |style| style.bg(hover_bg),
            )
        }
        PlayState::Loading => {
            let hover_bg = theme.accent_soft;
            interactive(
                base.bg(theme.idle).child(spinner(theme.accent)),
                theme,
                theme.pressed,
                move |style| style.bg(hover_bg),
            )
        }
        PlayState::Failed => {
            let hover_bg = theme.hover;
            interactive(
                base.bg(theme.idle)
                    .child(icon(crate::icons::RETRY, px(12.), theme.stopped)),
                theme,
                theme.pressed,
                move |style| style.bg(hover_bg),
            )
        }
        PlayState::Idle => {
            let hover_bg = theme.accent_soft;
            interactive(
                base.bg(theme.idle)
                    .child(icon(crate::icons::PLAY, px(12.), theme.accent)),
                theme,
                theme.pressed,
                move |style| style.bg(hover_bg),
            )
        }
    }
}

/// Three bars that rise and fall while a sound plays, so a playing row reads
/// as playing even when the pointer is elsewhere.
pub fn equalizer(color: gpui::Rgba) -> impl IntoElement {
    const BARS: [(f32, f32); 3] = [(0.0, 0.9), (0.33, 0.55), (0.66, 0.75)];
    const PERIOD: Duration = Duration::from_millis(900);
    let bar_height = px(12.);
    div()
        .flex()
        .items_end()
        .gap(px(2.))
        .h(bar_height)
        .flex_shrink_0()
        .children(BARS.iter().enumerate().map(move |(index, (phase, peak))| {
            let (phase, peak) = (*phase, *peak);
            div()
                .w(px(2.))
                .h(bar_height)
                .rounded(px(1.))
                .bg(color)
                .with_animation(
                    ("eq-bar", index),
                    gpui::Animation::new(PERIOD).repeat().with_max_fps(30.),
                    move |bar, delta| {
                        let t = (delta + phase).fract();
                        let wave = 0.5 - 0.5 * (t * std::f32::consts::TAU).cos();
                        bar.h(bar_height * (0.25 + (peak - 0.25) * wave))
                    },
                )
        }))
}

/// A turning ring, for work the user is waiting on.
pub fn spinner(color: gpui::Rgba) -> impl IntoElement {
    icon(crate::icons::LOADER, px(14.), color).with_animation(
        "spinner",
        gpui::Animation::new(Duration::from_millis(800)).repeat(),
        |glyph, delta| {
            glyph.with_transformation(gpui::Transformation::rotate(gpui::percentage(delta)))
        },
    )
}

/// A row of mutually exclusive options, drawn as one control.
pub fn segmented<T: PartialEq + Copy + 'static>(
    id: &'static str,
    options: &[(T, &'static str, &'static str, &'static [u8])],
    active: T,
    theme: &FormTheme,
    on_pick: impl Fn(T, &mut Window, &mut App) + Clone + 'static,
) -> Stateful<gpui::Div> {
    let mut group = div()
        .id(SharedString::from(id))
        .flex()
        .items_center()
        .gap(space(1))
        .p(space(1))
        .rounded(RADIUS_MD)
        .border_1()
        .border_color(theme.border)
        .bg(theme.field);
    for (value, label, segment_id, glyph) in options {
        let (value, label, glyph) = (*value, *label, *glyph);
        let selected = value == active;
        let pick = on_pick.clone();
        let segment = div()
            .id(SharedString::from(*segment_id))
            .flex_1()
            .h(CONTROL_HEIGHT_SM)
            .flex()
            .items_center()
            .justify_center()
            .gap(space(1))
            .rounded(RADIUS_SM)
            .text_xs()
            .child(icon(
                glyph,
                px(12.),
                if selected {
                    theme.on_accent
                } else {
                    theme.quiet
                },
            ))
            .child(label);
        let segment = if selected {
            segment.bg(theme.accent).text_color(theme.on_accent)
        } else {
            let (hover_bg, hover_text) = (theme.hover, theme.text);
            interactive(
                segment.bg(theme.idle).text_color(theme.quiet),
                theme,
                theme.pressed,
                move |style| style.bg(hover_bg).text_color(hover_text),
            )
        };
        group = group.child(segment.cursor_pointer().on_click(
            move |_: &ClickEvent, window, cx| {
                pick(value, window, cx);
            },
        ));
    }
    group
}

#[cfg(test)]
mod tests {
    use super::ButtonVariant;

    #[test]
    fn danger_is_its_own_variant() {
        // A delete action must not be able to share a look with a safe one:
        // that was the old behaviour, where both were plain clickable text.
        assert_ne!(ButtonVariant::Danger, ButtonVariant::Ghost);
        assert_ne!(ButtonVariant::Danger, ButtonVariant::Primary);
    }
}
