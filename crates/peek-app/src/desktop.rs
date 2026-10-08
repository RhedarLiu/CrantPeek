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
    TogglePause,
    Quit,
}
pub struct Desktop {
    _manager: GlobalHotKeyManager,
    _tray: TrayIcon,
    pub events: Receiver<Action>,
    items: [MenuItem; 5],
}
impl Desktop {
    pub fn refresh_language(&self) {
        for (item, key) in self.items.iter().zip([
            "tray-open",
            "tray-screenshot",
            "tray-settings",
            "tray-pause",
            "tray-quit",
        ]) {
            item.set_text(crate::i18n::tr(key));
        }
    }
    pub fn new(config: &Config, ctx: egui::Context) -> Result<Self, String> {
        let manager = GlobalHotKeyManager::new().map_err(|e| e.to_string())?;
        let blank: HotKey = config.blank_hotkey.parse::<HotKey>().map_err(|e| {
            crate::i18n::format("error-hotkey-blank", &[("detail", &e.to_string())])
        })?;
        let screenshot: HotKey = config.screenshot_hotkey.parse::<HotKey>().map_err(|e| {
            crate::i18n::format("error-hotkey-screenshot", &[("detail", &e.to_string())])
        })?;
        manager.register(blank).map_err(|e| e.to_string())?;
        manager.register(screenshot).map_err(|e| e.to_string())?;
        let menu = Menu::new();
        let open = MenuItem::new(crate::i18n::tr("tray-open"), true, None);
        let snip = MenuItem::new(crate::i18n::tr("tray-screenshot"), true, None);
        let settings = MenuItem::new(crate::i18n::tr("tray-settings"), true, None);
        let pause = MenuItem::new(crate::i18n::tr("tray-pause"), true, None);
        let quit = MenuItem::new(crate::i18n::tr("tray-quit"), true, None);
        menu.append_items(&[&open, &snip, &settings, &pause, &quit])
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
            .with_tooltip(crate::i18n::tr("tray-tooltip"))
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
        let items = [
            open.clone(),
            snip.clone(),
            settings.clone(),
            pause.clone(),
            quit.clone(),
        ];
        let open = open.id().clone();
        let snip = snip.id().clone();
        let settings = settings.id().clone();
        let pause = pause.id().clone();
        let quit = quit.id().clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let action = if event.id == open {
                Some(Action::Blank)
            } else if event.id == snip {
                Some(Action::Screenshot)
            } else if event.id == settings {
                Some(Action::Settings)
            } else if event.id == pause {
                Some(Action::TogglePause)
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
            items,
        })
    }
}
