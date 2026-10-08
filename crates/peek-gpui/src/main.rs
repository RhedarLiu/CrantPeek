//! Crant Peek — GPUI shell.
//!
//! Tray-resident, floating-first: no main window, the peek panel is created
//! hidden at startup and summoned by a global hotkey. This crate replaces the
//! egui/eframe UI layer (`peek-app`) while `peek-core`, `peek-network` and
//! `peek-dict` stay unchanged.
//!
//! Ordering note: the tray icon and the hotkey manager are installed *inside*
//! the `Application::run` callback, not before it. Creating them earlier makes
//! muda build a plain `NSApplication`, after which `MacPlatform::run` panics
//! with "Ivar platform not found on class NSApplication". This is measured in
//! `spikes/gpui-kit/lifecycle-probe-evidence.txt`.
//!
//! Modes (mirroring `peek-app`'s `PEEK_UI_PREVIEW` convention):
//!   PEEK_SELFTEST=1   show the panel, verify visibility, hide it, quit

mod fonts;
mod native_window;

use std::time::Duration;

use gpui_kit::base::{Root, StyledExt as _};
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::*;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tray_icon::Icon as TrayIconImage;
use tray_icon::TrayIconBuilder;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};

/// Poll interval for the tray/hotkey channels. Both crates deliver events on
/// their own channels rather than through GPUI, so they are drained on a timer.
const POLL: Duration = Duration::from_millis(60);

const LA: &str = "The best tools respect your attention.";
const ZH: &str = "需要时出现，看完即走。读取选区并翻译。";

struct Peek {
    set: &'static fonts::FontSet,
    /// Kept alive so the activation observer stays subscribed.
    _activation: Subscription,
}

impl Peek {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Hiding on focus loss. GPUI's callback carries no activation state, so
        // the state is read back from the window itself.
        let activation = cx.observe_window_activation(window, |_this, window, _cx| {
            if !window.is_window_active() {
                native_window::hide(window);
            }
        });
        Self {
            set: &fonts::DEFAULT,
            _activation: activation,
        }
    }
}

impl Render for Peek {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let set = self.set;
        let fg = theme.foreground;
        let bg = theme.background;
        let border = theme.border;
        let muted = theme.muted_foreground;

        v_flex()
            .size_full()
            .bg(bg)
            .text_color(fg)
            .p(px(20.))
            .gap(px(10.))
            // Esc hides the panel — "appear when needed, gone when done".
            .on_key_down(|event, window, _cx| {
                if event.keystroke.key == "escape" {
                    native_window::hide(window);
                }
            })
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex_1()
                            .font_family(set.latin)
                            .text_size(px(15.))
                            .font_semibold()
                            .child("Crant Peek"),
                    )
                    .child(
                        Icon::new(gpui_kit::assets::IconName::Settings)
                            .size(px(16.))
                            .text_color(muted),
                    )
                    .child(
                        Icon::new(gpui_kit::assets::IconName::Close)
                            .size(px(16.))
                            .text_color(muted),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .border_1()
                    .border_color(border)
                    .rounded(px(16.))
                    .p(px(16.))
                    .child(div().font_family(set.latin).text_size(px(14.)).child(LA)),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .font_family(set.latin)
                            .text_size(px(13.))
                            .border_1()
                            .border_color(border)
                            .rounded(px(10.))
                            .px(px(12.))
                            .py(px(6.))
                            .child("Translate  ▾"),
                    )
                    .child(
                        div()
                            .font_family(set.latin)
                            .text_size(px(13.))
                            .bg(fg)
                            .text_color(bg)
                            .rounded(px(999.))
                            .px(px(14.))
                            .py(px(7.))
                            .child("Look up"),
                    ),
            )
            .child(
                v_flex()
                    .w_full()
                    .border_1()
                    .border_color(border)
                    .rounded(px(16.))
                    .p(px(16.))
                    .gap(px(7.))
                    .child(
                        div()
                            .font_family(set.latin)
                            .text_size(px(11.))
                            .text_color(muted)
                            .child("AI answer"),
                    )
                    .child(div().font_family(set.latin).text_size(px(14.)).child(LA))
                    .child(div().font_family(set.sc).text_size(px(14.)).child(ZH))
                    .child(
                        div()
                            .font_family(set.mono)
                            .text_size(px(12.))
                            .text_color(muted)
                            .child("error[E0308]: mismatched types"),
                    ),
            )
            .child(
                div()
                    .font_family(set.latin)
                    .text_size(px(11.))
                    .text_color(muted)
                    .child(format!("font set: {}   ·   Esc 隐藏", set.name)),
            )
    }
}

/// Quits for real.
///
/// `App::quit()` only asks the platform to terminate asynchronously — on macOS
/// it dispatches `NSApplication terminate:` onto the main queue. Measured: the
/// process survived it (still resident 70s later) together with a stale
/// menu-bar icon, because the tray icon is intentionally leaked. Since this app
/// is tray-resident, a quit that leaves the process alive is worse than one
/// that skips destructors, so the exit is explicit.
fn quit_now(cx: &mut App) {
    cx.quit();
    std::process::exit(0);
}

fn window_options() -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(320.), px(200.)),
            size: size(px(480.), px(560.)),
        })),
        titlebar: Some(TitlebarOptions {
            title: None,
            appears_transparent: true,
            traffic_light_position: None,
        }),
        // Documented as "appears above all other windows"; also the branch that
        // sets CanJoinAllSpaces, which a globally summoned popup needs.
        kind: WindowKind::PopUp,
        // Created hidden: the panel only appears when summoned.
        show: false,
        focus: true,
        is_resizable: false,
        ..Default::default()
    }
}

fn main() -> anyhow::Result<()> {
    let selftest = std::env::var("PEEK_SELFTEST").is_ok();

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);

        if let Err(err) = fonts::register(cx) {
            eprintln!("font registration failed: {err}");
        }
        let missing = fonts::missing(cx);
        if missing.is_empty() {
            println!("bundled font families registered");
        } else {
            eprintln!(
                "bundled fonts MISSING (file truncated?): {}",
                missing.join(", ")
            );
        }
        fonts::apply(cx, &fonts::DEFAULT);

        let handle = match gpui_kit::open_window(window_options(), cx, |window, cx| {
            let view = cx.new(|cx| Peek::new(window, cx));
            cx.new(|cx| Root::new(view, window, cx))
        }) {
            Ok((handle, _)) => handle,
            Err(err) => {
                eprintln!("open_window failed: {err}");
                cx.quit();
                return;
            }
        };

        // Tray icon — installed here, see the ordering note at the top.
        let menu = Menu::new();
        let show_item = MenuItem::new("显示 Peek", true, None);
        let quit_item = MenuItem::new("退出", true, None);
        let show_id = show_item.id().clone();
        let quit_id = quit_item.id().clone();
        let _ = menu.append(&show_item);
        let _ = menu.append(&quit_item);
        match TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Crant Peek")
            .with_icon(
                TrayIconImage::from_rgba(vec![0x28, 0x28, 0x2c, 0xff], 1, 1)
                    .expect("1x1 tray icon"),
            )
            .build()
        {
            Ok(tray) => {
                println!("tray icon created");
                // Kept alive for the process lifetime; dropping it removes the icon.
                std::mem::forget(tray);
            }
            Err(err) => eprintln!("tray icon failed: {err}"),
        }

        let hotkey = HotKey::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::KeyA);
        match GlobalHotKeyManager::new() {
            Ok(manager) => match manager.register(hotkey) {
                Ok(()) => {
                    println!("global hotkey registered");
                    std::mem::forget(manager);
                }
                Err(err) => eprintln!("hotkey register failed: {err}"),
            },
            Err(err) => eprintln!("hotkey manager failed: {err}"),
        }

        // Automated check of the show/hide path, so the native shim does not
        // depend on a human pressing the hotkey. Briefly shows one 480x560
        // window and exits on its own.
        if selftest {
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                let before = cx
                    .update_window(handle, |_, window, _| native_window::is_visible(window))
                    .unwrap_or(false);
                println!("[selftest] visible at startup : {before}  <- want false");

                cx.update_window(handle, |_, window, _| native_window::show(window))
                    .ok();
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                let shown = cx
                    .update_window(handle, |_, window, _| native_window::is_visible(window))
                    .unwrap_or(false);
                println!("[selftest] visible after show  : {shown}  <- want true");

                cx.update_window(handle, |_, window, _| native_window::hide(window))
                    .ok();
                cx.background_executor()
                    .timer(Duration::from_millis(400))
                    .await;
                let hidden = cx
                    .update_window(handle, |_, window, _| native_window::is_visible(window))
                    .unwrap_or(false);
                println!("[selftest] visible after hide  : {hidden}  <- want false");

                cx.update(quit_now);
            })
            .detach();
            return;
        }

        // Drain the tray/hotkey channels. These are separate ecosystems from
        // GPUI, so they are polled rather than wired into GPUI's dispatcher.
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(POLL).await;

                let mut toggle = false;
                let mut quit = false;

                while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
                    if event.state == HotKeyState::Pressed {
                        toggle = true;
                    }
                }
                while let Ok(event) = MenuEvent::receiver().try_recv() {
                    if event.id == show_id {
                        toggle = true;
                    } else if event.id == quit_id {
                        quit = true;
                    }
                }

                if toggle {
                    let visible = cx
                        .update_window(handle, |_, window, _| native_window::is_visible(window))
                        .unwrap_or(false);
                    if visible {
                        let _ =
                            cx.update_window(handle, |_, window, _| native_window::hide(window));
                    } else {
                        let _ =
                            cx.update_window(handle, |_, window, _| native_window::show(window));
                    }
                }

                if quit {
                    cx.update(quit_now);
                    return;
                }
            }
        })
        .detach();
    });

    Ok(())
}
