use gpui::{Hsla, Rgba, Styled, Window, WindowAppearance, hsla, px, rgb};

/// Shared chrome for the login window. One surface, one field, one accent.
pub struct FormTheme {
    pub surface: Rgba,
    pub field: Rgba,
    pub border: Rgba,
    pub accent: Rgba,
    pub on_accent: Rgba,
    pub text: Rgba,
    pub quiet: Rgba,
    pub placeholder: Rgba,
    pub selection: Hsla,
    pub connected: Rgba,
    pub pending: Rgba,
    pub stopped: Rgba,
}

impl FormTheme {
    pub fn for_window(window: &Window) -> Self {
        match window.appearance() {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::dark(),
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::light(),
        }
    }

    fn light() -> Self {
        Self {
            surface: rgb(0xf4f2ee),
            field: rgb(0xfffdfb),
            border: rgb(0xd9d3c8),
            accent: rgb(0x1e4d8c),
            on_accent: rgb(0xfffdfb),
            text: rgb(0x1c1b19),
            quiet: rgb(0x6f6a62),
            placeholder: rgb(0x8a847c),
            selection: hsla(0.08, 0.04, 0.86, 1.0),
            connected: rgb(0x2f7d4a),
            pending: rgb(0xb86e00),
            stopped: rgb(0xb42318),
        }
    }

    fn dark() -> Self {
        Self {
            surface: rgb(0x1c1c1a),
            field: rgb(0x262623),
            border: rgb(0x3e3c38),
            accent: rgb(0x8eb4e8),
            on_accent: rgb(0x12181f),
            text: rgb(0xf3f0ea),
            quiet: rgb(0xa39e94),
            placeholder: rgb(0x8d887f),
            selection: hsla(0.08, 0.03, 0.32, 1.0),
            connected: rgb(0x7dcea0),
            pending: rgb(0xf0c36a),
            stopped: rgb(0xf0a8a0),
        }
    }
}

pub fn field_height() -> gpui::Pixels {
    px(40.)
}

/// Same fill, border, radius, and height for every text control.
pub fn field_chrome<E: Styled>(element: E, theme: &FormTheme, focused: bool) -> E {
    element
        .w_full()
        .h(field_height())
        .px_3()
        .flex()
        .items_center()
        .rounded_md()
        .border_1()
        .border_color(if focused { theme.accent } else { theme.border })
        .bg(theme.field)
        .text_sm()
        .text_color(theme.text)
}
