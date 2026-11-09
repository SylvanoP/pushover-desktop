use gpui::{
    App, Bounds, Context, SharedString, TitlebarOptions, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, size,
};

struct SettingsStub {
    status: SharedString,
}

impl Render for SettingsStub {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .p_6()
            .child(div().text_lg().child("Openover — Einstellungen"))
            .child(div().text_sm().child(self.status.clone()))
    }
}

pub fn register(_cx: &mut App) {}

pub fn open_settings_window(cx: &mut App) {
    let view = cx.new(|_| SettingsStub {
        status: "Login und Geräteregistrierung folgen in v0.2.".into(),
    });
    let bounds = Bounds::centered(None, size(px(420.), px(220.)), cx);
    let _ = cx.open_window(
        WindowOptions::default()
            .window_bounds(WindowBounds::Windowed(bounds))
            .titlebar(Some(TitlebarOptions {
                title: Some("Openover — Einstellungen".into()),
                ..Default::default()
            })),
        move |_, _| view,
    );
    cx.activate(true);
}
