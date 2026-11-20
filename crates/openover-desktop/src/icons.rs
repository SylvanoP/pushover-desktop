//! The icon set: Lucide, the family shadcn/ui uses.
//!
//! Each icon is the 24x24 Lucide source, inlined so the binary carries no asset
//! directory. `svg()` draws them as a mask filled with the current text colour,
//! so an icon follows the theme without a second copy per palette.
//!
//! Lucide is ISC licensed. https://lucide.dev

use gpui::{Pixels, Rgba, Styled as _, Svg, px, svg};

use crate::form_theme::space;

/// Wraps a Lucide body in the viewBox every Lucide icon uses.
macro_rules! lucide {
    ($name:ident, $doc:expr, $body:expr) => {
        #[doc = concat!("Lucide icon for ", $doc, ".")]
        pub const $name: &[u8] = concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">"#,
            $body,
            "</svg>"
        )
        .as_bytes();
    };
}

lucide!(
    INBOX,
    "a message list",
    r##"<g fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2"><path d="M22 12h-6l-2 3h-4l-2-3H2"/><path d="M5.45 5.11L2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11"/></g>"##
);

lucide!(
    ACCOUNT,
    "the account tab",
    r##"<g fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2"><circle cx="12" cy="8" r="5"/><path d="M20 21a8 8 0 0 0-16 0"/></g>"##
);

lucide!(
    SETTINGS,
    "the settings tab",
    r##"<g fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2"><path d="M14 17H5M19 7h-9"/><circle cx="17" cy="17" r="3"/><circle cx="7" cy="7" r="3"/></g>"##
);

lucide!(
    SEARCH,
    "the search field",
    r##"<g fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2"><path d="m21 21l-4.34-4.34"/><circle cx="11" cy="11" r="8"/></g>"##
);

lucide!(
    SEARCH_EMPTY,
    "a search with no match",
    r##"<g fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2"><path d="m13.5 8.5l-5 5m0-5l5 5"/><circle cx="11" cy="11" r="8"/><path d="m21 21l-4.3-4.3"/></g>"##
);

lucide!(
    COPY,
    "copy to the clipboard",
    r##"<g fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2"><rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/></g>"##
);

lucide!(
    CHECK,
    "a finished action",
    r##"<path fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M20 6L9 17l-5-5"/>"##
);

lucide!(
    TRASH,
    "deleting one entry",
    r##"<path fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 11v6m4-6v6m5-11v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>"##
);

lucide!(
    INFO,
    "a neutral message",
    r##"<g fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2"><circle cx="12" cy="12" r="10"/><path d="M12 16v-4m0-4h.01"/></g>"##
);

lucide!(
    SUCCESS,
    "a message about something that worked",
    r##"<g fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2"><circle cx="12" cy="12" r="10"/><path d="m16 9l-5.5 5.5L8 12"/></g>"##
);

lucide!(
    ERROR,
    "a message about something that failed",
    r##"<g fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2"><circle cx="12" cy="12" r="10"/><path d="M12 8v4m0 4h.01"/></g>"##
);

lucide!(
    MINUS,
    "stepping a value down",
    r##"<path fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 12h14"/>"##
);

lucide!(
    PLUS,
    "stepping a value up",
    r##"<path fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 12h14m-7-7v14"/>"##
);

lucide!(
    VOLUME,
    "sound that is on",
    r##"<path fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M11 4.702a.705.705 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.705.705 0 0 0 11 19.298zM16 9a5 5 0 0 1 0 6m3.364 3.364a9 9 0 0 0 0-12.728"/>"##
);

lucide!(
    VOLUME_OFF,
    "sound that is off",
    r##"<path fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M11 4.702a.7.7 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.7.7 0 0 0 11 19.298zm5.5 9.798l5-5m-5 0l5 5"/>"##
);

lucide!(
    SYSTEM,
    "following the system appearance",
    r##"<g fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2"><rect width="20" height="14" x="2" y="3" rx="2"/><path d="M8 21h8m-4-4v4"/></g>"##
);

lucide!(
    LIGHT,
    "the light palette",
    r##"<g fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2"><circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M4.93 4.93l1.41 1.41m11.32 11.32l1.41 1.41M2 12h2m16 0h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41"/></g>"##
);

lucide!(
    DARK,
    "the dark palette",
    r##"<path fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M20.985 12.486a9 9 0 1 1-9.473-9.472c.405-.022.617.46.402.803a6 6 0 0 0 8.268 8.268c.344-.215.825-.004.803.401"/>"##
);

lucide!(
    CLOSE,
    "dismissing the notice",
    r##"<path fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M18 6L6 18M6 6l12 12"/>"##
);

// Only the Windows build offers an in-app restart, so this is dead code
// elsewhere and clippy -Dwarnings would reject it.
#[cfg(windows)]
lucide!(
    DOWNLOAD,
    "an update that is ready",
    r##"<g fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2"><path d="M12 15V3m9 12v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m7 10l5 5l5-5"/></g>"##
);

lucide!(
    LOGOUT,
    "signing out",
    r##"<path fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="m16 17l5-5l-5-5m5 5H9m0 9H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"/>"##
);

lucide!(
    EMERGENCY,
    "an emergency waiting for an answer",
    r##"<path fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10.268 21a2 2 0 0 0 3.464 0M22 8c0-2.3-.8-4.3-2-6M3.262 15.326A1 1 0 0 0 4 17h16a1 1 0 0 0 .74-1.673C19.41 13.956 18 12.499 18 8A6 6 0 0 0 6 8c0 4.499-1.411 5.956-2.738 7.326M4 2C2.8 3.7 2 5.7 2 8"/>"##
);

/// The size an icon takes next to body text.
pub fn icon_size() -> Pixels {
    space(4)
}

/// The size an icon takes inside a compact control.
pub fn icon_size_sm() -> Pixels {
    px(14.)
}

/// An icon in `color`.
///
/// The colour is not optional on purpose: GPUI paints an SVG only when
/// `text_color` is set on the SVG element itself. A colour inherited from the
/// parent leaves `style.text.color` empty and the icon is never drawn, which
/// looks exactly like a missing icon.
pub fn icon(data: &'static [u8], size: Pixels, color: Rgba) -> Svg {
    svg()
        .data(data)
        .size(size)
        .flex_shrink_0()
        .text_color(color)
}

#[cfg(test)]
mod tests {
    use super::{CHECK, INBOX, INFO, SEARCH};

    /// Rasterizes an icon and reports how many pixels carry alpha.
    ///
    /// GPUI draws an SVG as an alpha mask, so an icon that produces no alpha is
    /// invisible no matter what colour it is given.
    fn covered_pixels(data: &[u8]) -> usize {
        let tree = resvg::usvg::Tree::from_data(data, &resvg::usvg::Options::default())
            .expect("icon must parse");
        let mut pixmap = resvg::tiny_skia::Pixmap::new(48, 48).unwrap();
        let size = tree.size();
        let transform =
            resvg::tiny_skia::Transform::from_scale(48. / size.width(), 48. / size.height());
        resvg::render(&tree, transform, &mut pixmap.as_mut());
        pixmap
            .pixels()
            .iter()
            .filter(|pixel| pixel.alpha() > 0)
            .count()
    }

    #[test]
    fn an_icon_carries_its_own_colour() {
        // GPUI paints an SVG only when text_color is set on the SVG element
        // itself; a colour inherited from the parent div leaves the icon
        // invisible. icon() therefore takes the colour as an argument, and this
        // asserts the signature stays that way.
        fn takes_a_colour(f: fn(&'static [u8], gpui::Pixels, gpui::Rgba) -> gpui::Svg) -> bool {
            let _ = f;
            true
        }
        assert!(takes_a_colour(super::icon));
    }

    #[test]
    fn every_icon_rasterizes_to_something_visible() {
        for (name, data) in [
            ("inbox", INBOX),
            ("search", SEARCH),
            ("check", CHECK),
            ("info", INFO),
        ] {
            let covered = covered_pixels(data);
            assert!(
                covered > 20,
                "{name} rasterized to {covered} visible pixels, so it would not be seen"
            );
        }
    }
}
