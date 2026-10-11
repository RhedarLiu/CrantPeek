//! A component popup in its own transparent window, so it can extend beyond
//! the compact panel without changing the panel's geometry.
use super::*;
use gpui_kit::base::ElementExt as _;
use gpui_kit::component::menu::PopupMenu;

struct TaskMenu {
    menu: Entity<PopupMenu>,
    parent: Entity<Peek>,
    parent_window: AnyWindowHandle,
    closed: bool,
    _subscriptions: Vec<Subscription>,
}

pub fn open(parent: Entity<Peek>, position: Point<Pixels>, window: &mut Window, cx: &mut App) {
    let parent_window = window.window_handle();
    let parent_bounds = window.window_bounds().get_bounds();
    let popup_size = size(ui_px(176.), ui_px(210.));
    let mut origin = parent_bounds.origin + position - point(ui_px(8.), ui_px(8.));
    if let Some(display) = window.display(cx) {
        let screen = display.bounds();
        origin.x = origin.x.clamp(
            screen.left(),
            (screen.right() - popup_size.width).max(screen.left()),
        );
        origin.y = origin.y.clamp(
            screen.top(),
            (screen.bottom() - popup_size.height).max(screen.top()),
        );
    }
    cx.defer(move |cx| {
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
                let view = cx.new(|cx| TaskMenu::new(parent.clone(), parent_window, window, cx));
                cx.new(|cx| Root::new(view, window, cx).bg(transparent_black()))
            },
        );
        if let Err(error) = result {
            parent.update(cx, |peek, cx| {
                peek.task_menu_open = false;
                peek.notify_error(error.to_string());
                cx.notify();
            });
        }
    });
}

impl TaskMenu {
    fn new(
        parent: Entity<Peek>,
        parent_window: AnyWindowHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let current = parent.read(cx).selected_task(cx);
        let menu = PopupMenu::build(window, cx, |mut menu, _, _| {
            menu = menu.min_w(ui_px(160.)).max_w(ui_px(160.)).scrollable(false);
            for (index, key) in TASK_KEYS.iter().enumerate() {
                let task = TASK_ORDER.get(index).copied();
                let target = parent.clone();
                menu = menu.item(
                    PopupMenuItem::new(i18n::tr(key))
                        .icon(task.map(Peek::task_icon).unwrap_or(IconName::Sparkles))
                        .checked(task == current)
                        .on_click(move |_, _, cx| {
                            let _ = cx.update_window(parent_window, |_, window, cx| {
                                target.update(cx, |peek, cx| {
                                    peek.tasks.update(cx, |state, cx| {
                                        state.set_selected_index(
                                            Some(IndexPath::new(index)),
                                            window,
                                            cx,
                                        );
                                    });
                                    cx.notify();
                                });
                            });
                        }),
                );
            }
            menu
        });
        menu.read(cx).focus_handle(cx).focus(window, cx);
        let dismissed = cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, window, cx| {
            this.close(true, window, cx);
        });
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.close(false, window, cx);
            }
        });
        Self {
            menu,
            parent,
            parent_window,
            closed: false,
            _subscriptions: vec![dismissed, activation],
        }
    }

    fn close(&mut self, restore: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.closed {
            return;
        }
        self.closed = true;
        let parent = self.parent.clone();
        let handle = self.parent_window;
        window.remove_window();
        // Wait until native activation events settle before deciding whether
        // the user clicked the parent panel or another application.
        cx.defer(move |cx| {
            let _ = cx.update_window(handle, |_, window, cx| {
                parent.update(cx, |peek, cx| {
                    peek.task_menu_open = false;
                    if restore {
                        native_window::show(window);
                    } else if peek.config.hide_on_blur && !peek.pinned && !window.is_window_active()
                    {
                        native_window::hide(window);
                    }
                    cx.notify();
                });
            });
        });
    }
}

impl Render for TaskMenu {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .p(ui_px(8.))
            .child(self.menu.clone())
            .on_prepaint(|bounds, window, _| {
                // Follow the actual component height, including UI scaling.
                let wanted = size(ui_px(176.), bounds.size.height);
                if window.bounds().size != wanted {
                    window.resize(wanted);
                }
            })
    }
}
