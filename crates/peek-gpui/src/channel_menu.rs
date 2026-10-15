//! The same call-order editor as Settings, in a separate transparent popup.
use super::*;
use gpui_kit::base::ElementExt as _;
use gpui_kit::component::ThemeStyled as _;

const WIDTH: f32 = 340.;

pub(crate) struct ChannelMenu {
    parent: Entity<Peek>,
    parent_window: AnyWindowHandle,
    focus: FocusHandle,
    closed: bool,
    removed: bool,
    _subscriptions: Vec<Subscription>,
}

pub fn open(parent: Entity<Peek>, position: Point<Pixels>, window: &mut Window, cx: &mut App) {
    motion::sync_system_preference(cx);
    let parent_window = window.window_handle();
    let parent_bounds = window.window_bounds().get_bounds();
    let screen = window.display(cx).map(|display| display.bounds());
    cx.defer(move |cx| {
        // The toolbar listener holds a mutable lease on Peek. Read its config
        // only after that listener returns, just like creating the popup itself.
        let rows = parent
            .read(cx)
            .config
            .quick_channels
            .as_deref()
            .unwrap_or_default()
            .len()
            .min(6);
        let popup_size = size(ui_px(WIDTH), ui_px(100. + 36. * rows as f32));
        let mut origin =
            parent_bounds.origin + position + point(ui_px(16.) - popup_size.width, ui_px(18.));
        if let Some(screen) = screen {
            origin.x = origin.x.clamp(
                screen.left(),
                (screen.right() - popup_size.width).max(screen.left()),
            );
            origin.y = origin.y.clamp(
                screen.top(),
                (screen.bottom() - popup_size.height).max(screen.top()),
            );
        }
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin,
                    size: popup_size,
                })),
                kind: WindowKind::PopUp,
                window_background: WindowBackgroundAppearance::Transparent,
                titlebar: None,
                show: true,
                focus: true,
                is_resizable: false,
                is_movable: false,
                ..Default::default()
            },
            |window, cx| {
                native_window::prepare_popup(window);
                let view = cx.new(|cx| ChannelMenu::new(parent.clone(), parent_window, window, cx));
                cx.new(|cx| Root::new(view, window, cx).bg(transparent_black()))
            },
        );
        if let Err(error) = result {
            parent.update(cx, |peek, cx| {
                peek.channel_menu_open = false;
                peek.notify_error(error.to_string());
                cx.notify();
            });
        }
    });
}

pub(crate) fn preview(
    parent: Entity<Peek>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ChannelMenu> {
    let handle = window.window_handle();
    cx.new(|cx| ChannelMenu::new(parent, handle, window, cx))
}

/// Click the actual toolbar entry, including the parent listener and deferred window creation.
pub(crate) fn verify_popup(
    parent: Entity<Peek>,
    handle: AnyWindowHandle,
    cx: &mut HeadlessAppContext,
) -> anyhow::Result<()> {
    use gpui_kit::test::TestWindowExt as _;
    cx.update(|cx| cx.set_reduce_motion(true));
    parent.update(cx, |peek, cx| {
        peek.pinned = true;
        set_chrome_visible(true);
        cx.notify();
    });
    for _ in 0..5 {
        std::thread::sleep(Duration::from_millis(60));
        cx.advance_clock(Duration::from_millis(60));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))?;
    }
    for entry in ["basic-channel-toggle", "basic-channel-button"] {
        let before = cx.update_window(handle, |_, window, cx| {
            window.click(entry, cx);
            window.bounds()
        })?;
        cx.run_until_parked();
        let popup = cx.update(|cx| {
            let windows = cx.windows();
            assert_eq!(windows.len(), 2, "toolbar must open one independent popup");
            *windows.iter().find(|window| **window != handle).unwrap()
        });
        cx.update_window(popup, |_, window, cx| window.render_frame(cx))?;
        verify_controls(parent.clone(), popup, cx)?;
        cx.update_window(handle, |_, window, _| assert_eq!(window.bounds(), before))?;
        cx.update_window(popup, |_, window, cx| window.press("escape", cx))?;
        cx.advance_clock(Duration::from_millis(300));
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(cx.windows().len(), 1);
            assert!(!parent.read(cx).channel_menu_open);
        });
        parent.update(cx, |peek, cx| {
            peek.config.quick_channels = Some(vec![
                "preview-google".into(),
                "preview-ai".into(),
                "preview-deeplx".into(),
            ]);
            cx.notify();
        });
    }
    println!(
        "[channel-popup-selftest] both toolbar entries, editing, stable parent bounds, Escape and reopen passed"
    );
    Ok(())
}

/// Replay native-style focus handoffs around both independent popup windows.
pub(crate) fn verify_focus(
    parent: Entity<Peek>,
    handle: AnyWindowHandle,
    cx: &mut HeadlessAppContext,
) -> anyhow::Result<()> {
    use gpui_kit::test::TestWindowExt as _;
    parent.update(cx, |peek, cx| {
        peek.pinned = false;
        peek.config.hide_on_blur = true;
        set_chrome_visible(true);
        cx.notify();
    });
    SHOWN_AT.with(|shown| shown.set(None));
    let settle = |cx: &mut HeadlessAppContext| {
        cx.run_until_parked();
        cx.advance_clock(Duration::from_millis(300));
        cx.run_until_parked();
    };
    for _ in 0..5 {
        std::thread::sleep(Duration::from_millis(60));
        cx.advance_clock(Duration::from_millis(60));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))?;
    }
    for entry in ["basic-channel-toggle", "task-picker"] {
        for exit in ["escape", "parent", "outside", "select"] {
            if exit == "select" && entry != "task-picker" {
                continue;
            }
            cx.update_window(handle, |_, window, _| window.activate_window())?;
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| window.click(entry, cx))?;
            cx.run_until_parked();
            let popup = cx.update(|cx| {
                let windows = cx.windows();
                assert_eq!(windows.len(), 2);
                *windows.iter().find(|window| **window != handle).unwrap()
            });
            cx.update_window(popup, |_, window, _| window.activate_window())?;
            settle(cx);
            cx.update_window(handle, |_, window, cx| {
                assert!(!window.is_window_active());
                assert!(!parent.read(cx).should_hide_on_blur(window));
            })?;

            // Replay the out-of-order key-window handoff seen during menu clicks.
            cx.update_window(handle, |_, window, _| window.activate_window())?;
            cx.update_window(popup, |_, window, _| window.activate_window())?;
            settle(cx);
            cx.update(|cx| assert_eq!(cx.windows().len(), 2));
            cx.update_window(handle, |_, window, cx| {
                assert!(!parent.read(cx).should_hide_on_blur(window));
            })?;
            cx.update_window(popup, |_, window, cx| window.render_frame(cx))?;
            if entry == "basic-channel-toggle" && exit == "escape" {
                verify_controls(parent.clone(), popup, cx)?;
            }
            let outside = if exit == "outside" {
                let other =
                    cx.open_window(size(px(80.), px(80.)), |_, cx| cx.new(|_| gpui_kit::Empty))?;
                cx.update_window(other.into(), |_, window, _| window.activate_window())?;
                Some(other)
            } else {
                if exit == "select" {
                    cx.update_window(popup, |_, window, cx| {
                        window.within("popup-menu").click(0usize, cx);
                        assert_eq!(parent.read(cx).selected_task(cx), Some(Task::Translate));
                    })?;
                } else if exit == "parent" {
                    cx.update_window(handle, |_, window, _| window.activate_window())?;
                } else {
                    cx.update_window(popup, |_, window, cx| window.press("escape", cx))?;
                }
                None
            };
            settle(cx);
            settle(cx);
            cx.update_window(handle, |_, window, cx| {
                let peek = parent.read(cx);
                assert!(!peek.task_menu_open && !peek.channel_menu_open);
                if exit == "outside" {
                    assert!(peek.should_hide_on_blur(window));
                } else if exit == "parent" {
                    assert!(!peek.should_hide_on_blur(window));
                }
            })?;
            if let Some(outside) = outside {
                cx.update_window(outside.into(), |_, window, _| window.remove_window())?;
            }
            settle(cx);
            cx.update(|cx| assert_eq!(cx.windows().len(), 1));
        }
    }
    println!(
        "[popup-focus-selftest] both menus protect parent during editing and transient focus loss; selection, Escape, parent click and outside blur passed"
    );
    Ok(())
}

/// Exercise the production editor without saving preferences or calling providers.
pub(crate) fn verify_controls(
    parent: Entity<Peek>,
    handle: AnyWindowHandle,
    cx: &mut HeadlessAppContext,
) -> anyhow::Result<()> {
    use gpui_kit::test::TestWindowExt as _;
    let settle = |cx: &mut HeadlessAppContext| -> anyhow::Result<()> {
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))?;
        Ok(())
    };
    for (control, expected) in [
        (
            "quick-up-preview-google",
            vec!["preview-google", "preview-ai", "preview-deeplx"],
        ),
        (
            "quick-down-preview-google",
            vec!["preview-ai", "preview-google", "preview-deeplx"],
        ),
        (
            "quick-up-preview-deeplx",
            vec!["preview-ai", "preview-deeplx", "preview-google"],
        ),
        (
            "quick-remove-preview-ai",
            vec!["preview-deeplx", "preview-google"],
        ),
    ] {
        settle(cx)?;
        cx.update_window(handle, |_, window, cx| {
            window.click(SharedString::from(control), cx);
            assert_eq!(
                parent.read(cx).config.quick_channels.as_deref().unwrap(),
                expected
            );
        })?;
    }
    settle(cx)?;
    cx.update_window(handle, |_, window, cx| window.click("quick-add", cx))?;
    settle(cx)?;
    cx.advance_clock(Duration::from_millis(200));
    settle(cx)?;
    cx.update_window(handle, |_, window, cx| {
        window.within("popup-menu").click(0usize, cx);
        assert_eq!(
            parent.read(cx).config.quick_channels.as_deref().unwrap(),
            ["preview-deeplx", "preview-google", "preview-ai"]
        );
    })?;
    settle(cx)?;
    cx.update_window(handle, |_, window, _cx| {
        // The order list must remain mounted after each editing action.
        assert!(
            window
                .within("quick-priority-list")
                .find("quick-up-preview-ai")
                .bounds()
                .size
                .height
                > px(0.)
        );
    })?;
    println!("[channel-order-selftest] boundary, reorder, remove, add and live repaint passed");
    Ok(())
}

impl ChannelMenu {
    fn new(
        parent: Entity<Peek>,
        parent_window: AnyWindowHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        // Reordering, removing and adding keep this editor open and repaint it
        // from the parent config, including changes made through Settings.
        let changes = cx.observe(&parent, |_, _, cx| cx.notify());
        let activation = cx.observe_window_activation(window, |_this, window, cx| {
            if !window.is_window_active() {
                cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(POPUP_FOCUS_SETTLE).await;
                    let _ = this.update_in(cx, |this, window, cx| {
                        if !window.is_window_active() {
                            this.close(false, window, cx);
                        }
                    });
                })
                .detach();
            }
        });
        Self {
            parent,
            parent_window,
            focus,
            closed: false,
            removed: false,
            _subscriptions: vec![changes, activation],
        }
    }

    fn close(&mut self, restore: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.closed {
            if !restore {
                self.finish_close(false, window, cx);
            }
            return;
        }
        self.closed = true;
        if !restore || cx.reduce_motion() {
            self.finish_close(restore, window, cx);
            return;
        }
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(motion::MENU_EXIT).await;
            let _ = this.update_in(cx, |this, window, cx| this.finish_close(true, window, cx));
        })
        .detach();
    }

    fn finish_close(&mut self, restore: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.removed {
            return;
        }
        self.removed = true;
        let parent = self.parent.clone();
        let handle = self.parent_window;
        window.remove_window();
        cx.defer(move |cx| {
            let _ = cx.update_window(handle, |_, window, cx| {
                parent.update(cx, |peek, cx| {
                    peek.channel_menu_open = false;
                    if restore {
                        native_window::show(window);
                    } else {
                        peek.hide_after_focus_settles(window, cx);
                    }
                    cx.notify();
                });
            });
        });
    }
}

impl Render for ChannelMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list = self
            .parent
            .update(cx, |peek, cx| peek.quick_channel_list(cx));
        let surface = div()
            .relative()
            .w(ui_px(WIDTH - 16.))
            .popover_style(cx)
            .font_family(self.parent.read(cx).set.latin)
            .p(ui_px(12.))
            .child(list);
        let surface = if self.closed {
            surface
                .with_animation(
                    "channel-menu-exit",
                    motion::menu_exit(),
                    |surface, delta| surface.top(ui_px(-2. * delta)).opacity(1. - delta),
                )
                .into_any_element()
        } else {
            surface
                .with_animation(
                    "channel-menu-enter",
                    motion::menu_enter(),
                    |surface, delta| surface.top(ui_px(-6. * (1. - delta))).opacity(delta),
                )
                .into_any_element()
        };
        div()
            .id("channel-menu-surface")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    this.close(true, window, cx);
                }
            }))
            .when(self.closed, |root| {
                root.capture_any_mouse_down(|_, _, cx| cx.stop_propagation())
                    .capture_any_mouse_up(|_, _, cx| cx.stop_propagation())
            })
            .p(ui_px(8.))
            .child(surface)
            .on_prepaint(|bounds, window, _| {
                let wanted = size(ui_px(WIDTH), bounds.size.height);
                if window.bounds().size != wanted {
                    window.resize(wanted);
                }
            })
    }
}
