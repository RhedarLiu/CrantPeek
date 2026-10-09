//! Full-screen region selection overlay for screenshot translation.
//!
//! The display is captured **before** the overlay opens, and that frozen frame
//! is drawn as the overlay's background, which is how the system screenshot
//! tool behaves. Two problems found on a real machine forced this:
//!
//! * Capturing on release raced the overlay's own removal: macOS removes a
//!   window asynchronously, so the capture usually contained the grey overlay
//!   and OCR found nothing. Only an occasional run won the race.
//! * A "transparent" GPUI window rendered as an opaque grey sheet, so the user
//!   could not see what they were selecting.
//!
//! The crop and OCR therefore use the pre-captured frame, never a second
//! capture. Recognised text goes back through the shell's action channel as
//! [`Action::Recognized`]. Nothing is written to disk.

use std::sync::{Arc, mpsc::Sender};

use gpui_kit::prelude::*;
use gpui_kit::*;

use peek_runtime::action::Action;
use peek_runtime::capture::{self, Rect, Screen};

pub struct Snip {
    /// Where the drag started, in window coordinates.
    anchor: Option<Point<Pixels>>,
    cursor: Option<Point<Pixels>>,
    /// The frame captured before the overlay opened; crop and OCR use it.
    screen: Arc<Screen>,
    /// The same frame converted for display.
    backdrop: Arc<RenderImage>,
    /// Where the recognised text goes.
    result: Sender<Action>,
    /// Keyboard focus, so Esc reaches the overlay.
    focus: FocusHandle,
    /// Guards against a release firing after Esc already closed the overlay.
    done: bool,
}

impl Snip {
    fn new(
        screen: Arc<Screen>,
        backdrop: Arc<RenderImage>,
        result: Sender<Action>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            anchor: None,
            cursor: None,
            screen,
            backdrop,
            result,
            focus: cx.focus_handle(),
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

    /// Crops the frozen frame and recognises it off the UI thread.
    fn recognize(&mut self, window: &mut Window) {
        if self.done {
            return;
        }
        self.done = true;
        window.remove_window();
        let Some(rect) = self.selection() else {
            return;
        };
        let screen = self.screen.clone();
        let result = self.result.clone();
        std::thread::spawn(move || {
            let Some(image) = capture::crop(&screen, rect) else {
                eprintln!("[snip] selection too small: {rect:?}");
                return;
            };
            let image = capture::prepare_region(image);
            match peek_runtime::ocr::recognize(&image) {
                Ok(text) => {
                    eprintln!("[snip] ocr ok: {} chars", text.trim().chars().count());
                    // Sent even when empty, so the panel can say "nothing
                    // recognised" instead of the selection vanishing silently.
                    let _ = result.send(Action::Recognized(text));
                }
                Err(err) => eprintln!("[snip] ocr failed: {err}"),
            }
        });
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
            .track_focus(&self.focus)
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
            // The frozen frame, stretched to the logical window size.
            .child(
                img(self.backdrop.clone())
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .object_fit(ObjectFit::Fill),
            )
            // Dimming over the frame.
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(hsla(0.0, 0.0, 0.0, 0.30)),
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

/// Converts a captured RGBA frame into GPUI's BGRA render image.
fn backdrop(screen: &Screen) -> Arc<RenderImage> {
    // GPUI stores pixels as BGRA, xcap as RGBA, so R and B swap.
    let mut pixels = screen.pixels.clone();
    for pixel in pixels.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Arc::new(RenderImage::new(vec![image::Frame::new(pixels)]))
}

/// Captures the monitor under the cursor, then opens the overlay over it.
///
/// Errors are reported as text for the log; the caller leaves the panel as it
/// was rather than showing a broken overlay.
pub fn open(cx: &mut App, result: Sender<Action>) -> Result<(AnyWindowHandle, Rect), String> {
    let bounds = capture::monitor_bounds().ok_or("no monitor bounds")?;
    let screen = Arc::new(capture::capture().map_err(|err| format!("capture failed: {err}"))?);
    let backdrop = backdrop(&screen);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(bounds.min[0]), px(bounds.min[1])),
            size: size(px(bounds.width()), px(bounds.height())),
        })),
        titlebar: None,
        // `PopUp` is the branch that sets CanJoinAllSpaces, so the overlay
        // covers whichever Space the user is on.
        kind: WindowKind::PopUp,
        show: true,
        focus: true,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    let opened = gpui_kit::open_window(options, cx, move |window, cx| {
        let view = cx.new(|cx| Snip::new(screen, backdrop, result, cx));
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
        crate::native_window::make_capture_overlay(window);
        cx.new(|cx| gpui_kit::base::Root::new(view, window, cx))
    });
    match opened {
        Ok((handle, _)) => Ok((handle, bounds)),
        Err(err) => Err(format!("overlay window failed: {err}")),
    }
}
