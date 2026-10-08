//! Full-screen region selection overlay for screenshot translation.
//!
//! The overlay deliberately does **not** pre-capture the display: it opens a
//! transparent always-on-top window covering the monitor, the user drags a
//! region, and the capture happens on release. Enumerating monitors needs no
//! system permission (only `capture_image` does), so the overlay is fully
//! testable before Screen Recording has been granted — and it lets the user aim
//! at live, up-to-date screen content instead of a frozen frame.
//!
//! Recognised text is reported back through the shell's action channel as
//! [`Action::Recognized`]. Nothing is written to disk.

use std::sync::mpsc::Sender;

use gpui_kit::prelude::*;
use gpui_kit::*;

use peek_runtime::action::Action;
use peek_runtime::capture::{self, Rect};

pub struct Snip {
    /// Where the drag started, in window coordinates.
    anchor: Option<Point<Pixels>>,
    cursor: Option<Point<Pixels>>,
    /// Where the recognised text goes.
    result: Sender<Action>,
    /// Guards against a release firing after Esc already closed the overlay.
    done: bool,
}

impl Snip {
    pub fn new(result: Sender<Action>) -> Self {
        Self {
            anchor: None,
            cursor: None,
            result,
            done: false,
        }
    }

    /// The dragged region, normalised so any drag direction works.
    fn selection(&self) -> Option<Rect> {
        let (anchor, cursor) = (self.anchor?, self.cursor?);
        Some(Rect::from_drag(
            [anchor.x.into(), anchor.y.into()],
            [cursor.x.into(), cursor.y.into()],
        ))
    }

    /// Captures, crops and recognises off the UI thread: capture can block and
    /// OCR takes tens of milliseconds.
    fn recognize(&mut self, window: &mut Window) {
        if self.done {
            return;
        }
        self.done = true;
        let Some(rect) = self.selection() else {
            window.remove_window();
            return;
        };
        let result = self.result.clone();
        std::thread::spawn(move || {
            let Ok(screen) = capture::capture() else {
                return;
            };
            let Some(image) = capture::crop(&screen, rect) else {
                return;
            };
            let image = capture::prepare_region(image);
            if let Ok(text) = peek_runtime::ocr::recognize(&image)
                && !text.trim().is_empty()
            {
                let _ = result.send(Action::Recognized(text));
            }
        });
        window.remove_window();
    }

    fn cancel(&mut self, window: &mut Window) {
        self.done = true;
        window.remove_window();
    }
}

impl Render for Snip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selection = self.selection();
        let accent = hsla(0.58, 0.9, 0.6, 1.0);

        div()
            .size_full()
            // Dimming is alpha over a transparent window, so the live desktop
            // stays visible underneath.
            .bg(hsla(0.0, 0.0, 0.0, 0.35))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, _cx| {
                if event.keystroke.key == "escape" {
                    this.cancel(window);
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                    this.anchor = Some(event.position);
                    this.cursor = Some(event.position);
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _window, cx| {
                if this.anchor.is_some() {
                    this.cursor = Some(event.position);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _event: &MouseUpEvent, window, _cx| {
                    this.recognize(window);
                }),
            )
            .child(
                div()
                    .absolute()
                    .left(px(24.))
                    .top(px(24.))
                    .text_size(px(15.))
                    .text_color(hsla(0.0, 0.0, 1.0, 0.95))
                    .child(peek_runtime::i18n::tr("snip-hint")),
            )
            .when_some(selection, |this, rect| {
                this.child(
                    div()
                        .absolute()
                        .left(px(rect.min[0]))
                        .top(px(rect.min[1]))
                        .w(px(rect.width()))
                        .h(px(rect.height()))
                        .border_2()
                        .border_color(accent),
                )
            })
    }
}

/// Opens the overlay covering the monitor that holds the cursor.
///
/// Returns `None` when the monitor geometry is unavailable; the caller then
/// leaves the panel as it was rather than showing a broken overlay.
pub fn open(cx: &mut App, result: Sender<Action>) -> Option<(AnyWindowHandle, Rect)> {
    let bounds = capture::monitor_bounds()?;
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(bounds.min[0]), px(bounds.min[1])),
            size: size(px(bounds.width()), px(bounds.height())),
        })),
        titlebar: Some(TitlebarOptions {
            title: None,
            appears_transparent: true,
            traffic_light_position: None,
        }),
        // `PopUp` is the branch that sets CanJoinAllSpaces, so the overlay
        // covers whichever Space the user is on.
        kind: WindowKind::PopUp,
        show: true,
        focus: true,
        is_movable: false,
        is_resizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    let opened = gpui_kit::open_window(options, cx, move |window, cx| {
        let view = cx.new(|_| Snip::new(result));
        cx.new(|cx| gpui_kit::base::Root::new(view, window, cx))
    });
    match opened {
        Ok((handle, _)) => Some((handle, bounds)),
        Err(err) => {
            eprintln!("overlay window failed: {err}");
            None
        }
    }
}
