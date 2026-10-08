use eframe::egui;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use peek_core::Config;
use std::sync::mpsc::{self, Receiver};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem},
};

#[derive(Debug)]
pub enum Action {
    Blank,
    Selection(String),
    Screenshot,
    Settings,
    Quit,
}
pub struct Desktop {
    _manager: GlobalHotKeyManager,
    _tray: TrayIcon,
    pub events: Receiver<Action>,
}
impl Desktop {
    pub fn new(config: &Config, ctx: egui::Context) -> Result<Self, String> {
        let manager = GlobalHotKeyManager::new().map_err(|e| e.to_string())?;
        let blank: HotKey = config
            .blank_hotkey
            .parse()
            .map_err(|e| format!("Invalid blank shortcut: {e}"))?;
        let screenshot: HotKey = config
            .screenshot_hotkey
            .parse()
            .map_err(|e| format!("Invalid screenshot shortcut: {e}"))?;
        manager.register(blank).map_err(|e| e.to_string())?;
        manager.register(screenshot).map_err(|e| e.to_string())?;
        let menu = Menu::new();
        let open = MenuItem::new("打开空白 Peek", true, None);
        let snip = MenuItem::new("截图", true, None);
        let settings = MenuItem::new("设置", true, None);
        let quit = MenuItem::new("退出", true, None);
        menu.append_items(&[&open, &snip, &settings, &quit])
            .map_err(|e| e.to_string())?;
        let mut pixels = vec![0_u8; 16 * 16 * 4];
        for y in 2..14 {
            for x in 2..14 {
                let i = (y * 16 + x) * 4;
                pixels[i..i + 4].copy_from_slice(&[95, 160, 240, 255]);
            }
        }
        let icon = Icon::from_rgba(pixels, 16, 16).map_err(|e| e.to_string())?;
        let tray = TrayIconBuilder::new()
            .with_tooltip("Crant Peek")
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .build()
            .map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::channel();
        crate::selection::listen(config.double_ctrl_ms, tx.clone(), ctx.clone());
        let hot_tx = tx.clone();
        let hot_ctx = ctx.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state != HotKeyState::Pressed {
                return;
            }
            let action = if event.id == blank.id() {
                Some(Action::Blank)
            } else if event.id == screenshot.id() {
                Some(Action::Screenshot)
            } else {
                None
            };
            if let Some(action) = action {
                let _ = hot_tx.send(action);
                hot_ctx.request_repaint();
            }
        }));
        let open = open.id().clone();
        let snip = snip.id().clone();
        let settings = settings.id().clone();
        let quit = quit.id().clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let action = if event.id == open {
                Some(Action::Blank)
            } else if event.id == snip {
                Some(Action::Screenshot)
            } else if event.id == settings {
                Some(Action::Settings)
            } else if event.id == quit {
                Some(Action::Quit)
            } else {
                None
            };
            if let Some(action) = action {
                let _ = tx.send(action);
                ctx.request_repaint();
            }
        }));
        Ok(Self {
            _manager: manager,
            _tray: tray,
            events: rx,
        })
    }
}
