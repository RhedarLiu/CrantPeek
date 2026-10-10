//! Region selection and in-place translation for screenshots.
//!
//! The display is captured **before** the overlay opens and that frozen frame is
//! drawn as the overlay's background, which is how the system screenshot tool
//! behaves. Capturing on release instead raced the overlay's own removal (macOS
//! tears the window down asynchronously), so the crop usually contained the grey
//! overlay and OCR found nothing.
//!
//! After a selection the overlay stays open and shows its own result card, as a
//! translation tool does: the recognised text, its translation, and actions to
//! copy the text or hand it to the panel. The translation runs in the panel's
//! existing pipeline, which the overlay borrows through the action channel, so
//! the network stack and credential handling stay in one place.

use std::sync::{
    Arc,
    mpsc::{Receiver, Sender},
};

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use peek_runtime::action::{Action, OverlayEvent};
use peek_runtime::capture::{self, Rect, Screen};
use peek_runtime::i18n;

/// What the OCR worker reports. Empty and failure stay distinct so a language
/// pack or permission error is not shown as "no text".
enum OcrResult {
    Text(String),
    Empty,
    Failed(String),
}

/// Card geometry, also used to keep it on screen.
const CARD_WIDTH: f32 = 380.;
const CARD_MAX_HEIGHT: f32 = 260.;
const CARD_GAP: f32 = 10.;

pub struct Snip {
    /// Where the drag started, in window coordinates.
    anchor: Option<Point<Pixels>>,
    cursor: Option<Point<Pixels>>,
    /// The frame captured before the overlay opened; crop and OCR use it.
    screen: Arc<Screen>,
    /// The same frame converted for display.
    backdrop: Arc<RenderImage>,
    /// Asks the panel to run a turn for us.
    link: Sender<Action>,
    /// Recognised text, once OCR has finished.
    text: Option<String>,
    /// Streamed translation.
    answer: String,
    status: String,
    busy: bool,
    /// OCR results, delivered by the worker thread.
    ocr_rx: Receiver<OcrResult>,
    ocr_tx: Sender<OcrResult>,
    /// Progress from the panel while it runs our turn.
    events: Option<Receiver<OverlayEvent>>,
    /// Keyboard focus, so Esc reaches the overlay.
    focus: FocusHandle,
    /// A failure waiting for a frame with a Window.
    pending_toast: Option<String>,
    /// The selection is committed; only the result card stays interactive.
    captured: bool,
    /// Close as soon as the text is copied, from the user's settings.
    close_on_copy: bool,
}

impl Snip {
    fn new(
        screen: Arc<Screen>,
        backdrop: Arc<RenderImage>,
        link: Sender<Action>,
        close_on_copy: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let (ocr_tx, ocr_rx) = std::sync::mpsc::channel();
        let snip = Self {
            anchor: None,
            cursor: None,
            screen,
            backdrop,
            link,
            text: None,
            answer: String::new(),
            status: String::new(),
            busy: false,
            ocr_rx,
            ocr_tx,
            events: None,
            focus: cx.focus_handle(),
            pending_toast: None,
            captured: false,
            close_on_copy,
        };
        // The worker thread and the panel's progress channel cannot touch GPUI
        // state, so both are drained here.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
                if this.update(cx, |snip, cx| snip.poll(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        snip
    }

    /// Builds the overlay with a synthetic frame and a sample answer, so the
    /// result card can be checked offscreen without capturing the screen.
    pub fn preview(cx: &mut Context<Self>) -> Self {
        let screen = Arc::new(Screen {
            pixels: image::RgbaImage::from_pixel(1400, 900, image::Rgba([234, 236, 239, 255])),
            origin: [0.0, 0.0],
            scale: 2.0,
        });
        let image = backdrop(&screen);
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut snip = Self::new(screen, image, tx, true, cx);
        snip.captured = true;
        snip.anchor = Some(point(px(180.), px(240.)));
        snip.cursor = Some(point(px(820.), px(380.)));
        snip.text = Some("Screenshot text".into());
        snip.answer = "## 截图翻译\n\n识别到的文字在这里直接翻译，不用先跳回主窗口。".into();
        // The preview has no panel to answer a translate request, so mark the
        // turn as already attached; otherwise it reports a worker failure.
        let (_tx, rx) = std::sync::mpsc::channel();
        snip.events = Some(rx);
        snip
    }

    /// The dragged region, normalised so any drag direction works.
    fn selection(&self) -> Option<Rect> {
        let (anchor, cursor) = (self.anchor?, self.cursor?);
        let raw = Rect::from_drag(
            [anchor.x.into(), anchor.y.into()],
            [cursor.x.into(), cursor.y.into()],
        );
        // Snapped to whole points. The outline and the dimming bands are both
        // derived from this rectangle, and half-point edges left a hairline
        // between the selection and the band that follows it.
        let min_x = raw.min[0].round();
        let min_y = raw.min[1].round();
        let width = (raw.max[0] - min_x).round().max(1.);
        let height = (raw.max[1] - min_y).round().max(1.);
        Some(Rect {
            min: [min_x, min_y],
            max: [min_x + width, min_y + height],
        })
    }

    fn poll(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        let mut recognized = None;
        while let Ok(result) = self.ocr_rx.try_recv() {
            match result {
                OcrResult::Text(text) => recognized = Some(text),
                OcrResult::Empty => {
                    self.status = i18n::tr("status-ocr-empty");
                    self.busy = false;
                    changed = true;
                }
                OcrResult::Failed(err) => {
                    let message = i18n::format("status-ocr-failed", &[("error", &err)]);
                    self.pending_toast = Some(message.clone());
                    self.status = message;
                    self.busy = false;
                    changed = true;
                }
            }
        }
        if let Some(text) = recognized {
            self.text = Some(text);
            changed = true;
        }
        if let Some(events) = &self.events {
            while let Ok(event) = events.try_recv() {
                match event {
                    OverlayEvent::Status(status) => self.status = status,
                    OverlayEvent::Chunk(chunk) => self.answer.push_str(&chunk),
                    // A failure is raised on the next frame, where a Window
                    // exists; the card stays for content, not for errors.
                    OverlayEvent::Failed(message) => {
                        self.pending_toast = Some(message.clone());
                        self.status = i18n::tr("status-failed");
                    }
                    OverlayEvent::Finished => self.busy = false,
                }
                changed = true;
            }
        }
        if changed {
            cx.notify();
        }
        // Started after the drain loops, which borrow other fields.
        if self.text.is_some() && self.events.is_none() {
            self.translate(cx);
        }
    }

    /// Crops the frozen frame and recognises it off the UI thread.
    fn recognize(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.captured {
            return;
        }
        let Some(rect) = self.selection() else {
            return;
        };
        self.captured = true;
        self.busy = true;
        self.status = i18n::tr("status-ocr-running");
        let screen = self.screen.clone();
        let tx = self.ocr_tx.clone();
        std::thread::spawn(move || {
            let Some(image) = capture::crop(&screen, rect) else {
                eprintln!("[snip] selection too small: {rect:?}");
                let _ = tx.send(OcrResult::Empty);
                return;
            };
            let image = capture::prepare_region(image);
            match peek_runtime::ocr::recognize(&image) {
                Ok(text) if !text.trim().is_empty() => {
                    eprintln!("[snip] ocr ok: {} chars", text.trim().chars().count());
                    let _ = tx.send(OcrResult::Text(text));
                }
                Ok(_) => {
                    eprintln!("[snip] ocr found no text");
                    let _ = tx.send(OcrResult::Empty);
                }
                Err(err) => {
                    eprintln!("[snip] ocr failed: {err}");
                    let _ = tx.send(OcrResult::Failed(err));
                }
            }
        });
        cx.notify();
    }

    /// Asks the panel to translate the recognised text, keeping the overlay open.
    fn translate(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.text.clone() else {
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        self.events = Some(rx);
        self.answer.clear();
        self.busy = true;
        self.status = i18n::tr("status-generating");
        if self
            .link
            .send(Action::Translate { text, replies: tx })
            .is_err()
        {
            self.status = i18n::tr("status-worker-crashed");
            self.busy = false;
        }
        cx.notify();
    }

    /// Hands the recognised text to the panel and closes the overlay.
    fn send_to_panel(&mut self, window: &mut Window) {
        if let Some(text) = self.text.clone() {
            let _ = self.link.send(Action::Recognized(text));
        }
        self.close(window);
    }

    /// Copies the recognised text and confirms with a toast, so the card does
    /// not have to grow a status line for a transient message.
    fn copy_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.text.clone() else {
            return;
        };
        let characters = text.chars().count();
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        eprintln!("[snip] copied {characters} chars to the clipboard");
        if self.close_on_copy {
            // A toast would vanish with the window, so there is nothing to show.
            self.close(window);
        } else {
            window.push_notification(
                Notification::success(i18n::tr("snip-copied")).autohide(true),
                cx,
            );
        }
    }

    fn close(&mut self, window: &mut Window) {
        window.remove_window();
    }

    /// Where the result card goes: under the selection, pulled back on screen.
    fn card_origin(&self, rect: Rect, window: &Window) -> Point<Pixels> {
        let view: Point<Pixels> = window.bounds().size.into();
        let width: f32 = view.x.into();
        let height: f32 = view.y.into();
        let x = rect.min[0].min(width - CARD_WIDTH - 12.).max(12.);
        let below = rect.max[1] + CARD_GAP;
        let y = if below + CARD_MAX_HEIGHT > height {
            (rect.min[1] - CARD_MAX_HEIGHT - CARD_GAP).max(12.)
        } else {
            below
        };
        point(px(x), px(y))
    }
}

impl Render for Snip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(message) = self.pending_toast.take() {
            window.push_notification(Notification::error(message).autohide(true), cx);
        }
        let selection = self.selection();
        let accent = hsla(0.58, 0.9, 0.6, 1.0);
        let theme = cx.theme();
        let border = theme.border;
        let card_bg = theme.popover;
        let muted = theme.muted_foreground;

        let card = selection.map(|rect| {
            let origin = self.card_origin(rect, window);
            let body = if !self.answer.is_empty() {
                TextView::markdown("snip_answer", self.answer.clone())
                    .text_size(px(13.))
                    .into_any_element()
            } else {
                div()
                    .text_size(px(13.))
                    .text_color(muted)
                    .child(self.status.clone())
                    .into_any_element()
            };
            div()
                .absolute()
                .left(origin.x)
                .top(origin.y)
                .w(px(CARD_WIDTH))
                .max_h(px(CARD_MAX_HEIGHT))
                .bg(card_bg)
                .border_1()
                .border_color(border)
                .rounded(px(14.))
                .p(px(14.))
                .flex()
                .flex_col()
                .gap(px(10.))
                // The recognised text is deliberately not shown: the card is
                // for the translation, and the text is one press away on the
                // copy button. Printing it made the card a transcript.
                .child(
                    div()
                        .id("snip_scroll")
                        .max_h(px(170.))
                        .overflow_y_scroll()
                        .child(body),
                )
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            Button::new("snip_copy")
                                .icon(IconName::Copy)
                                .label(i18n::tr("snip-copy-text"))
                                .on_click(cx.listener(|this, _event, window, cx| {
                                    this.copy_text(window, cx);
                                })),
                        )
                        .child(
                            Button::new("snip_to_panel")
                                .primary()
                                .icon(IconName::CornerDownLeft)
                                .label(i18n::tr("snip-to-input"))
                                .on_click(cx.listener(|this, _event, window, _cx| {
                                    this.send_to_panel(window);
                                })),
                        )
                        .child(div().flex_1())
                        .child(
                            Button::new("snip_close")
                                .icon(IconName::X)
                                .tooltip(i18n::tr("snip-discard"))
                                .on_click(cx.listener(|this, _event, window, _cx| {
                                    this.close(window);
                                })),
                        ),
                )
                .into_any_element()
        });

        div()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, _cx| {
                if event.keystroke.key == "escape" {
                    this.close(window);
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                    if this.captured {
                        return;
                    }
                    this.anchor = Some(event.position);
                    this.cursor = Some(event.position);
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _window, cx| {
                if this.anchor.is_some() && !this.captured {
                    this.cursor = Some(event.position);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _event: &MouseUpEvent, window, cx| {
                    if this.captured {
                        return;
                    }
                    this.recognize(window, cx);
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
            // Dimming, but never over the selection itself: four bands around
            // it are drawn instead of one sheet, so the chosen region keeps the
            // frame's real brightness.
            .children(dim_bands(selection, window))
            .when(!self.captured, |this| {
                this.child(
                    div()
                        .absolute()
                        .left(px(24.))
                        .top(px(24.))
                        .text_size(px(15.))
                        .text_color(hsla(0.0, 0.0, 1.0, 0.95))
                        .child(i18n::tr("snip-hint")),
                )
            })
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
            .children(card)
    }
}

/// The dimming bands around the selection, or one sheet while nothing is
/// selected yet. Keeping the selection undimmed is what makes it readable.
fn dim_bands(selection: Option<Rect>, window: &Window) -> Vec<AnyElement> {
    let dim = hsla(0.0, 0.0, 0.0, 0.30);
    let view: Point<Pixels> = window.bounds().size.into();
    let width: f32 = view.x.into();
    let height: f32 = view.y.into();
    let Some(rect) = selection else {
        return vec![
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(dim)
                .into_any_element(),
        ];
    };
    let band = |left: f32, top: f32, w: f32, h: f32| {
        div()
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(w.max(0.)))
            .h(px(h.max(0.)))
            .bg(dim)
            .into_any_element()
    };
    vec![
        band(0., 0., width, rect.min[1]),
        band(0., rect.max[1], width, height - rect.max[1]),
        band(0., rect.min[1], rect.min[0], rect.height()),
        band(rect.max[0], rect.min[1], width - rect.max[0], rect.height()),
    ]
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
pub fn open(
    cx: &mut App,
    link: Sender<Action>,
    close_on_copy: bool,
) -> Result<(AnyWindowHandle, Rect), String> {
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
        let view = cx.new(|cx| Snip::new(screen, backdrop, link, close_on_copy, cx));
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
