use gpui::{
    App, Bounds, Context, ElementInputHandler, EntityInputHandler, FocusHandle, Focusable,
    KeyDownEvent, MouseButton, Pixels, Point, Render, UTF16Selection, Window, canvas,
    colors::Colors, div, prelude::*, px,
};
use openover_core::{PASSWORD_BULLET, apply_masked_replace, wipe_secret};
use std::ops::Range;

/// Single-line password field. The secret stays in memory; the glyphs are bullets.
pub struct MaskedPassword {
    focus_handle: FocusHandle,
    secret: String,
    selection: Range<usize>,
    marked: Option<Range<usize>>,
}

impl MaskedPassword {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle().tab_stop(true),
            secret: String::new(),
            selection: 0..0,
            marked: None,
        }
    }

    pub fn secret(&self) -> &str {
        &self.secret
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        wipe_secret(&mut self.secret);
        self.selection = 0..0;
        self.marked = None;
        cx.notify();
    }

    fn edit_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        let changed = if modifiers.secondary() && key == "v" {
            let Some(item) = cx.read_from_clipboard() else {
                return;
            };
            let Some(text) = item.text() else {
                return;
            };
            self.replace(None, &text, cx);
            true
        } else if modifiers.secondary() && key == "a" {
            let len = self.char_len();
            self.selection = 0..len;
            self.marked = None;
            cx.notify();
            true
        } else if modifiers.control || modifiers.platform || modifiers.alt || modifiers.function {
            false
        } else {
            match key {
                "backspace" => {
                    self.backspace(cx);
                    true
                }
                "delete" => {
                    self.delete_forward(cx);
                    true
                }
                "left" => self.move_caret(-1, cx),
                "right" => self.move_caret(1, cx),
                "home" => {
                    self.selection = 0..0;
                    self.marked = None;
                    cx.notify();
                    true
                }
                "end" => {
                    let len = self.char_len();
                    self.selection = len..len;
                    self.marked = None;
                    cx.notify();
                    true
                }
                _ => false,
            }
        };
        if changed {
            window.notify_text_input_changed(&self.focus_handle);
        }
    }

    fn backspace(&mut self, cx: &mut Context<Self>) {
        let range = self.clamp_range(self.selection.clone());
        if range.start != range.end {
            self.replace(Some(range), "", cx);
            return;
        }
        if range.start == 0 {
            return;
        }
        self.replace(Some(range.start - 1..range.start), "", cx);
    }

    fn delete_forward(&mut self, cx: &mut Context<Self>) {
        let range = self.clamp_range(self.selection.clone());
        if range.start != range.end {
            self.replace(Some(range), "", cx);
            return;
        }
        if range.end >= self.char_len() {
            return;
        }
        self.replace(Some(range.end..range.end + 1), "", cx);
    }

    fn move_caret(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        let len = self.char_len() as isize;
        let next = (self.selection.end as isize + delta).clamp(0, len) as usize;
        if self.selection.start == next && self.selection.end == next {
            return false;
        }
        self.selection = next..next;
        self.marked = None;
        cx.notify();
        true
    }

    fn char_len(&self) -> usize {
        self.secret.chars().count()
    }

    fn clamp_range(&self, range: Range<usize>) -> Range<usize> {
        let len = self.char_len();
        let start = range.start.min(range.end).min(len);
        let end = range.start.max(range.end).min(len).max(start);
        start..end
    }

    fn replace(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        cx: &mut Context<Self>,
    ) -> usize {
        let range = self.clamp_range(
            range
                .or_else(|| self.marked.clone())
                .unwrap_or_else(|| self.selection.clone()),
        );
        self.marked = None;
        let caret = apply_masked_replace(&mut self.secret, range, text);
        let caret = caret.min(self.char_len());
        self.selection = caret..caret;
        cx.notify();
        caret
    }
}

impl Focusable for MaskedPassword {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for MaskedPassword {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.clamp_range(range);
        *adjusted_range = Some(range.clone());
        Some(std::iter::repeat_n(PASSWORD_BULLET, range.end - range.start).collect())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let range = self.clamp_range(self.selection.clone());
        self.selection = range.clone();
        Some(UTF16Selection {
            range,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked.clone()
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace(range, text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.clamp_range(
            range
                .or_else(|| self.marked.clone())
                .unwrap_or_else(|| self.selection.clone()),
        );
        let start = range.start;
        let caret = self.replace(Some(range), new_text, cx);
        if caret > start {
            self.marked = Some(start..caret);
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(element_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.char_len())
    }

    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selection = self.clamp_range(range_utf16);
        cx.notify();
    }

    fn text_length_utf16(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.char_len())
    }
}

impl Render for MaskedPassword {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);
        let focused = self.focus_handle.is_focused(window);
        let len = self.char_len();
        let selection = self.clamp_range(self.selection.clone());
        let placeholder = len == 0;
        let shown = if placeholder && !focused {
            String::new()
        } else {
            bullet_line(len, selection.clone(), focused)
        };
        let border = if focused {
            colors.selected
        } else {
            colors.border
        };
        let view = cx.entity();
        let focus_handle = self.focus_handle.clone();

        div()
            .id("openover-password")
            .track_focus(&self.focus_handle)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.focus_handle, cx);
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.edit_key(event, window, cx);
            }))
            .relative()
            .w_full()
            .h(px(40.))
            .px_3()
            .flex()
            .items_center()
            .rounded_md()
            .border_1()
            .border_color(border)
            .bg(colors.container)
            .text_color(colors.text)
            .child(
                div()
                    .when(placeholder && !focused, |line| {
                        line.text_color(colors.disabled).child("Passwort")
                    })
                    .when(!(placeholder && !focused), |line| line.child(shown)),
            )
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        window.handle_input(
                            &focus_handle,
                            ElementInputHandler::new(bounds, view.clone()),
                            cx,
                        );
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0(),
            )
    }
}

fn bullet_line(len: usize, selection: Range<usize>, focused: bool) -> String {
    let mut line = String::new();
    for index in 0..len {
        if focused && selection.start == selection.end && index == selection.end {
            line.push('\u{2502}');
        }
        if index >= selection.start && index < selection.end {
            line.push('\u{25e6}');
        } else {
            line.push(PASSWORD_BULLET);
        }
    }
    if focused && selection.end >= len {
        line.push('\u{2502}');
    }
    line
}
