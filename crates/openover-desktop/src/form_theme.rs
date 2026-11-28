//! Design tokens: the spacing scale, radii, type steps and the palette.
//!
//! Everything visible reads its values from here, so that a change to a token
//! moves the whole window at once instead of one call site at a time.

use gpui::{Hsla, Pixels, Rgba, Styled, Window, WindowAppearance, hsla, px, rgb, rgba};
use openover_core::ThemeChoice;
use std::sync::atomic::{AtomicU8, Ordering};

/// The wish the window is currently drawn with. Child views such as the password
/// field render on their own, so they read it from here instead of being handed
/// the choice through every call.
static WISH: AtomicU8 = AtomicU8::new(0);

fn wish_code(choice: ThemeChoice) -> u8 {
    match choice {
        ThemeChoice::System => 0,
        ThemeChoice::Light => 1,
        ThemeChoice::Dark => 2,
    }
}

fn wish_of(code: u8) -> ThemeChoice {
    match code {
        1 => ThemeChoice::Light,
        2 => ThemeChoice::Dark,
        _ => ThemeChoice::System,
    }
}

pub fn remember_wish(choice: ThemeChoice) {
    WISH.store(wish_code(choice), Ordering::Relaxed);
}

pub fn wish() -> ThemeChoice {
    wish_of(WISH.load(Ordering::Relaxed))
}

/// One step of the spacing scale, in multiples of four pixels.
///
/// Every gap, padding and margin in the window comes from here. Values between
/// the steps are what made the old layout look accidental: 6 next to 8 next to
/// 10 reads as a mistake, 8 next to 12 reads as a decision.
pub const fn space(step: u8) -> Pixels {
    px(4. * step as f32)
}

/// Corner radii. Small for controls inside a field, medium for fields and
/// buttons, large for cards and panels.
pub const RADIUS_SM: Pixels = px(4.);
pub const RADIUS_MD: Pixels = px(6.);
pub const RADIUS_LG: Pixels = px(8.);

/// Height of a field, a button, or anything else that sits in a form row.
pub const CONTROL_HEIGHT: Pixels = px(36.);
/// Height of a control nested inside a field, such as a stepper or a segment.
pub const CONTROL_HEIGHT_SM: Pixels = px(26.);
/// Width of the label column in a settings row.
pub const LABEL_WIDTH: Pixels = px(148.);
/// Width of the focus ring. Drawn outside the border, so it costs no layout.
pub const RING_WIDTH: Pixels = px(2.);

/// What a message to the user is about. Picks its color and its glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Info,
    Success,
    Error,
}

impl MessageKind {
    pub fn color(self, theme: &FormTheme) -> Rgba {
        match self {
            Self::Info => theme.quiet,
            Self::Success => theme.connected,
            Self::Error => theme.stopped,
        }
    }

    /// The icon that makes a message recognisable before it is read.
    pub fn glyph(self) -> &'static [u8] {
        match self {
            Self::Info => crate::icons::INFO,
            Self::Success => crate::icons::SUCCESS,
            Self::Error => crate::icons::ERROR,
        }
    }
}

/// Shared chrome for the whole window. One surface, one field, one accent.
pub struct FormTheme {
    pub surface: Rgba,
    /// A panel that sits on the surface, such as a card or the toast.
    pub raised: Rgba,
    pub field: Rgba,
    pub field_hover: Rgba,
    pub border: Rgba,
    /// Border of a control under the pointer.
    pub border_hover: Rgba,
    /// Fill of a quiet control under the pointer.
    pub hover: Rgba,
    /// Resting fill of a quiet control: the hover colour at zero alpha. A
    /// control without any fill transitions from transparent black, which a
    /// light surface shows as a dark flash on every hover.
    pub idle: Rgba,
    /// Fill of a quiet control being pressed.
    pub pressed: Rgba,
    pub accent: Rgba,
    pub accent_hover: Rgba,
    pub accent_pressed: Rgba,
    /// A tinted fill for a selected segment or a soft accent surface.
    pub accent_soft: Rgba,
    pub on_accent: Rgba,
    /// The focus ring. Only drawn for keyboard focus.
    pub ring: Hsla,
    pub text: Rgba,
    pub quiet: Rgba,
    pub placeholder: Rgba,
    pub selection: Hsla,
    pub connected: Rgba,
    pub pending: Rgba,
    pub stopped: Rgba,
}

impl FormTheme {
    /// Draws with the wish last remembered by the settings window.
    pub fn for_window(window: &Window) -> Self {
        Self::resolve(wish(), window)
    }

    /// The stored wish wins. `System` follows the window appearance.
    pub fn resolve(choice: ThemeChoice, window: &Window) -> Self {
        remember_wish(choice);
        let dark = match choice {
            ThemeChoice::Dark => true,
            ThemeChoice::Light => false,
            ThemeChoice::System => matches!(
                window.appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
        };
        if dark { Self::dark() } else { Self::light() }
    }

    /// shadcn "zinc" in light: white surface, zinc borders, one blue accent.
    /// Interactive states darken, because the surface is already at white.
    fn light() -> Self {
        Self {
            surface: rgb(0xffffff),
            raised: rgb(0xffffff),
            field: rgb(0xfafafa),
            field_hover: rgb(0xf4f4f5),
            border: rgb(0xe4e4e7),
            border_hover: rgb(0xd4d4d8),
            hover: rgb(0xf4f4f5),
            idle: rgba(0xf4f4f500),
            pressed: rgb(0xe4e4e7),
            accent: rgb(0x2563eb),
            accent_hover: rgb(0x1d4ed8),
            accent_pressed: rgb(0x1e40af),
            accent_soft: rgb(0xeff6ff),
            on_accent: rgb(0xfafafa),
            ring: hsla(217. / 360., 0.91, 0.60, 1.0),
            text: rgb(0x09090b),
            quiet: rgb(0x71717a),
            placeholder: rgb(0xa1a1aa),
            selection: hsla(217. / 360., 0.91, 0.88, 1.0),
            connected: rgb(0x16a34a),
            pending: rgb(0xd97706),
            stopped: rgb(0xdc2626),
        }
    }

    /// shadcn "zinc" in dark: near-black surface, layered card, lifted accent.
    /// Interactive states brighten, because depth comes from light here.
    fn dark() -> Self {
        Self {
            surface: rgb(0x0a0a0a),
            raised: rgb(0x18181b),
            field: rgb(0x18181b),
            field_hover: rgb(0x1f1f23),
            border: rgb(0x27272a),
            border_hover: rgb(0x3f3f46),
            hover: rgb(0x1f1f23),
            idle: rgba(0x1f1f2300),
            pressed: rgb(0x27272a),
            accent: rgb(0x60a5fa),
            accent_hover: rgb(0x93c5fd),
            accent_pressed: rgb(0x3b82f6),
            accent_soft: rgb(0x172554),
            on_accent: rgb(0x09090b),
            ring: hsla(213. / 360., 0.94, 0.68, 1.0),
            text: rgb(0xfafafa),
            quiet: rgb(0xa1a1aa),
            placeholder: rgb(0x71717a),
            selection: hsla(217. / 360., 0.5, 0.28, 1.0),
            connected: rgb(0x4ade80),
            pending: rgb(0xfbbf24),
            stopped: rgb(0xf87171),
        }
    }
}

/// Same fill, border, radius, and height for every text control.
///
/// A focused field is outlined in the accent. An unfocused one reacts to the
/// pointer, so a field reads as something you can type into before you click it.
pub fn field_chrome<E>(element: E, theme: &FormTheme, focused: bool) -> E
where
    E: Styled + gpui::InteractiveElement,
{
    let (border, hover_border, hover_bg) = if focused {
        (theme.accent, theme.accent, theme.field)
    } else {
        (theme.border, theme.border_hover, theme.field_hover)
    };
    element
        .w_full()
        .h(CONTROL_HEIGHT)
        .px(space(3))
        .flex()
        .items_center()
        .rounded(RADIUS_MD)
        .border_1()
        .border_color(border)
        .bg(theme.field)
        .text_sm()
        .text_color(theme.text)
        .hover(move |style| style.border_color(hover_border).bg(hover_bg))
        .transitions(|transitions| {
            transitions
                .border_color(gpui::Motion::new(std::time::Duration::from_millis(110)))
                .bg(gpui::Motion::new(std::time::Duration::from_millis(110)))
        })
}

#[cfg(test)]
mod tests {
    use super::{CONTROL_HEIGHT, CONTROL_HEIGHT_SM, space};
    use gpui::px;

    #[test]
    fn the_scale_steps_in_fours() {
        assert_eq!(space(0), px(0.));
        assert_eq!(space(1), px(4.));
        assert_eq!(space(2), px(8.));
        assert_eq!(space(3), px(12.));
        assert_eq!(space(4), px(16.));
        assert_eq!(space(6), px(24.));
    }

    #[test]
    fn a_nested_control_fits_inside_a_field() {
        // Otherwise a stepper or a segment would push its own field open.
        assert!(CONTROL_HEIGHT_SM < CONTROL_HEIGHT);
    }
}
