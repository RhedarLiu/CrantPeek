#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod capture;
mod desktop;
mod ocr;
mod selection;
mod store;
use eframe::egui;
use peek_core::{Config, Message, Protocol, Task, local_route};
use peek_network::{Client, Event};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

struct Peek {
    config: Config,
    draft: Config,
    settings: bool,
    input: String,
    answer: String,
    followup: String,
    messages: Vec<Message>,
    task: Task,
    pinned: bool,
    status: String,
    secret_draft: String,
    runtime: tokio::runtime::Runtime,
    receiver: Option<mpsc::Receiver<Event>>,
    cancel: Option<CancellationToken>,
    busy: bool,
    dictionary: Option<peek_dict::Dict>,
    dictionary_missing: bool,
    dict_entry: Option<peek_dict::Entry>,
    desktop: Option<desktop::Desktop>,
    quit: bool,
    visible: bool,
    snip: Option<capture::Screen>,
    snip_texture: Option<egui::TextureHandle>,
    drag_start: Option<egui::Pos2>,
    capture_rx: Option<std::sync::mpsc::Receiver<Result<capture::Screen, String>>>,
    ocr_rx: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
}
impl Peek {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut fonts = egui::FontDefinitions::default();
        // Load CJK fonts from the OS, never bundle private fonts in the repository.
        let candidates = if cfg!(target_os = "macos") {
            vec![
                "/System/Library/Fonts/PingFang.ttc",
                "/System/Library/Fonts/STHeiti Light.ttc",
            ]
        } else {
            vec![
                "C:\\Windows\\Fonts\\msyh.ttc",
                "C:\\Windows\\Fonts\\msgothic.ttc",
            ]
        };
        for path in candidates {
            if let Ok(bytes) = std::fs::read(path) {
                fonts
                    .font_data
                    .insert("cjk".into(), egui::FontData::from_owned(bytes).into());
                fonts
                    .families
                    .entry(egui::FontFamily::Proportional)
                    .or_default()
                    .push("cjk".into());
                fonts
                    .families
                    .entry(egui::FontFamily::Monospace)
                    .or_default()
                    .push("cjk".into());
                break;
            }
        }
        cc.egui_ctx.set_fonts(fonts);
        let (config, status) = match store::load() {
            Ok(c) => (c, String::new()),
            Err(e) => (Config::default(), format!("Configuration error: {e}")),
        };
        let desktop = desktop::Desktop::new(&config, cc.egui_ctx.clone());
        let (desktop, status) = match desktop {
            Ok(d) => (Some(d), status),
            Err(e) => (None, format!("Desktop integration error: {e}")),
        };
        Self {
            draft: config.clone(),
            config,
            settings: false,
            input: String::new(),
            answer: String::new(),
            followup: String::new(),
            messages: Vec::new(),
            task: Task::Translate,
            pinned: false,
            status,
            secret_draft: String::new(),
            runtime: tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("async runtime"),
            receiver: None,
            cancel: None,
            busy: false,
            dictionary: None,
            dictionary_missing: false,
            dict_entry: None,
            desktop,
            quit: false,
            visible: true,
            snip: None,
            snip_texture: None,
            drag_start: None,
            capture_rx: None,
            ocr_rx: None,
        }
    }
    /// Single words/short tokens only; loads the dictionary file lazily on first use.
    fn lookup_word(&mut self, text: &str) -> Option<peek_dict::Entry> {
        if text.chars().count() > 40 || text.split_whitespace().count() != 1 {
            return None;
        }
        if self.dictionary.is_none() && !self.dictionary_missing {
            self.dictionary = store::dictionary_candidates()
                .into_iter()
                .find_map(|path| peek_dict::Dict::open(&path).ok());
            self.dictionary_missing = self.dictionary.is_none();
        }
        self.dictionary.as_ref()?.lookup(text)
    }
    fn stop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel();
        }
        self.receiver = None;
        self.busy = false;
    }
    fn query(&mut self, ctx: &egui::Context, followup: bool) {
        self.stop();
        let text = if followup {
            self.followup.trim()
        } else {
            self.input.trim()
        }
        .to_owned();
        let text = text.as_str();
        if text.is_empty() {
            return;
        }
        if !followup {
            // Offline dictionary first: instant, and independent of any API key or network.
            self.answer.clear();
            self.dict_entry = self.lookup_word(text);
        }
        let key = match store::secret(&self.config.provider.credential_id) {
            Ok(k) => k,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        if self.config.provider.model.trim().is_empty() {
            self.status = "请先在设置中填写模型".into();
            return;
        }
        if !followup {
            let route = local_route(
                text,
                &self.config.target_language,
                &self.config.chinese_target,
            );
            if self.config.smart_mode {
                self.task = route.task;
            }
            self.messages = vec![Message {
                role: "system".into(),
                content: self.task.instruction(&route.target),
            }];
        }
        self.messages.push(Message {
            role: "user".into(),
            content: text.into(),
        });
        self.followup.clear();
        self.answer.clear();
        self.status = "生成中…".into();
        self.busy = true;
        let (tx, rx) = mpsc::channel(128);
        self.receiver = Some(rx);
        let cancel = CancellationToken::new();
        self.cancel = Some(cancel.clone());
        let provider = self.config.provider.clone();
        let messages = self.messages.clone();
        let ctx = ctx.clone();
        self.runtime.spawn(async move {
            let (net_tx, mut net_rx) = mpsc::channel(32);
            let worker = tokio::spawn(async move {
                Client::default()
                    .stream(&provider, &key, &messages, net_tx, cancel)
                    .await
            });
            while let Some(event) = net_rx.recv().await {
                if tx.send(event).await.is_err() {
                    break;
                }
                ctx.request_repaint();
            }
            if let Ok(Err(e)) = worker.await {
                let _ = tx.send(Event::Failed(e.to_string())).await;
                ctx.request_repaint();
            }
        });
    }
    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("设置");
        ui.label("回答服务（凭据保存在系统安全存储）");
        egui::ComboBox::from_label("协议")
            .selected_text(format!("{:?}", self.draft.provider.protocol))
            .show_ui(ui, |ui| {
                for p in [
                    Protocol::ChatCompletions,
                    Protocol::Responses,
                    Protocol::Anthropic,
                ] {
                    ui.selectable_value(&mut self.draft.provider.protocol, p, format!("{p:?}"));
                }
            });
        ui.label("Base URL（包括 /v1，除非服务要求其他路径）");
        ui.text_edit_singleline(&mut self.draft.provider.base_url);
        ui.label("Model");
        ui.text_edit_singleline(&mut self.draft.provider.model);
        ui.label("API key（留空保留已有凭据）");
        ui.add(egui::TextEdit::singleline(&mut self.secret_draft).password(true));
        ui.label("默认目标语言");
        ui.text_edit_singleline(&mut self.draft.target_language);
        ui.label("中文翻译目标");
        ui.text_edit_singleline(&mut self.draft.chinese_target);
        ui.checkbox(&mut self.draft.smart_mode, "智能判断模式");
        ui.checkbox(&mut self.draft.hide_on_blur, "失焦隐藏（取消固定后生效）");
        ui.horizontal(|ui| {
            if ui.button("保存").clicked() {
                let result = self.draft.validate().map_err(str::to_owned).and_then(|()| {
                    if !self.secret_draft.is_empty() {
                        store::save_secret(&self.draft.provider.credential_id, &self.secret_draft)?;
                    }
                    store::save(&self.draft)
                });
                match result {
                    Ok(()) => {
                        self.config = self.draft.clone();
                        self.secret_draft.clear();
                        self.settings = false;
                        self.status = "已保存".into();
                    }
                    Err(e) => self.status = e,
                }
            }
            if ui.button("取消").clicked() {
                self.draft = self.config.clone();
                self.secret_draft.clear();
                self.settings = false;
            }
        });
    }
}
impl Drop for Peek {
    fn drop(&mut self) {
        self.stop();
    }
}
impl eframe::App for Peek {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        let actions: Vec<_> = self
            .desktop
            .as_ref()
            .map(|d| d.events.try_iter().collect())
            .unwrap_or_default();
        for action in actions {
            match action {
                desktop::Action::Quit => {
                    self.quit = true;
                    self.stop();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                desktop::Action::Blank => {
                    self.stop();
                    self.input.clear();
                    self.answer.clear();
                    self.dict_entry = None;
                    self.followup.clear();
                    self.messages.clear();
                    self.settings = false;
                    self.status.clear();
                    self.visible = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                desktop::Action::Selection(text) => {
                    self.stop();
                    self.input = text;
                    self.answer.clear();
                    self.dict_entry = None;
                    self.followup.clear();
                    self.messages.clear();
                    self.settings = false;
                    self.status.clear();
                    self.visible = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    self.query(&ctx, false);
                }
                desktop::Action::Settings => {
                    self.draft = self.config.clone();
                    self.settings = true;
                    self.visible = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                desktop::Action::Screenshot => {
                    self.stop();
                    self.visible = false;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                    let (tx, rx) = std::sync::mpsc::channel();
                    self.capture_rx = Some(rx);
                    let ctx = ctx.clone();
                    std::thread::spawn(move || {
                        // Allow the window hide to reach the compositor before capture.
                        std::thread::sleep(std::time::Duration::from_millis(200));
                        let _ = tx.send(capture::capture());
                        ctx.request_repaint();
                    });
                }
            }
        }
        if let Some(result) = self.capture_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.capture_rx = None;
            match result {
                Ok(screen) => {
                    let size = [
                        screen.pixels.width() as usize,
                        screen.pixels.height() as usize,
                    ];
                    self.snip_texture = Some(ctx.load_texture(
                        "screen-selection",
                        egui::ColorImage::from_rgba_unmultiplied(size, screen.pixels.as_raw()),
                        egui::TextureOptions::LINEAR,
                    ));
                    self.snip = Some(screen);
                }
                Err(e) => {
                    self.status = format!("截图失败，请检查屏幕录制权限：{e}");
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                }
            }
        }
        if let Some(result) = self.ocr_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.ocr_rx = None;
            self.visible = true;
            self.settings = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            match result {
                Ok(text) if !text.trim().is_empty() => {
                    self.input = text;
                    self.query(&ctx, false);
                }
                Ok(_) => self.status = "未识别到文字，请重选区域".into(),
                Err(e) => self.status = format!("OCR 失败：{e}"),
            }
        }
        if let Some(screen) = &self.snip {
            let size = egui::vec2(
                screen.pixels.width() as f32 / screen.scale,
                screen.pixels.height() as f32 / screen.scale,
            );
            let origin = screen.origin;
            let texture = self.snip_texture.as_ref().unwrap().id();
            let mut selection = None;
            let mut cancelled = false;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("snip"),
                egui::ViewportBuilder::default()
                    .with_position(origin)
                    .with_inner_size(size)
                    .with_decorations(false)
                    .with_always_on_top()
                    .with_resizable(false),
                |ctx, _| {
                    egui::Area::new(egui::Id::new("snip-area"))
                        .fixed_pos(egui::Pos2::ZERO)
                        .show(ctx, |ui| {
                            let (rect, response) =
                                ui.allocate_exact_size(size, egui::Sense::drag());
                            ui.painter().image(
                                texture,
                                rect,
                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                egui::Color32::WHITE,
                            );
                            if response.drag_started() {
                                self.drag_start = response.interact_pointer_pos();
                            }
                            if let (Some(start), Some(end)) =
                                (self.drag_start, response.interact_pointer_pos())
                            {
                                let r = egui::Rect::from_two_pos(start, end);
                                ui.painter().rect_stroke(
                                    r,
                                    0.0,
                                    egui::Stroke::new(2.0, egui::Color32::LIGHT_BLUE),
                                    egui::StrokeKind::Inside,
                                );
                                if response.drag_stopped() {
                                    selection = Some(r);
                                }
                            }
                            ui.painter().text(
                                egui::pos2(24.0, 24.0),
                                egui::Align2::LEFT_TOP,
                                "拖动框选 · Esc 取消",
                                egui::FontId::proportional(18.0),
                                egui::Color32::LIGHT_BLUE,
                            );
                        });
                    cancelled = ctx.input(|i| {
                        i.key_pressed(egui::Key::Escape) || i.viewport().close_requested()
                    });
                },
            );
            if cancelled || selection.is_some() {
                ctx.send_viewport_cmd_to(
                    egui::ViewportId::from_hash_of("snip"),
                    egui::ViewportCommand::Close,
                );
                if let Some(image) = selection.and_then(|rect| capture::crop(screen, rect)) {
                    let (tx, rx) = std::sync::mpsc::channel();
                    self.ocr_rx = Some(rx);
                    let ctx = ctx.clone();
                    std::thread::spawn(move || {
                        let _ = tx.send(ocr::recognize(&image));
                        ctx.request_repaint();
                    });
                }
                self.snip = None;
                self.snip_texture = None;
                self.drag_start = None;
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.quit && self.desktop.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.stop();
            self.visible = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        let mut events = Vec::new();
        if let Some(rx) = &mut self.receiver {
            while let Ok(event) = rx.try_recv() {
                events.push(event);
            }
        }
        for event in events {
            match event {
                Event::Text(text) => self.answer.push_str(&text),
                Event::Done => {
                    self.busy = false;
                    self.status = "完成".into();
                    self.messages.push(Message {
                        role: "assistant".into(),
                        content: self.answer.clone(),
                    });
                }
                Event::Failed(e) => {
                    self.busy = false;
                    self.status = e;
                }
            }
        }
        if self.desktop.is_some() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.stop();
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        if self.desktop.is_some()
            && self.visible
            && !self.pinned
            && self.config.hide_on_blur
            && !self.settings
            && ctx.input(|i| i.viewport().focused == Some(false))
        {
            self.stop();
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        egui::CentralPanel::default().show(root, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Crant Peek");
                ui.checkbox(&mut self.pinned, "固定");
                if ui.button("设置").clicked() {
                    self.draft = self.config.clone();
                    self.settings = true;
                }
                if ui.button("×").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.separator();
            if self.settings {
                self.settings_ui(ui);
            } else {
                ui.add(
                    egui::TextEdit::multiline(&mut self.input)
                        .desired_rows(3)
                        .hint_text("输入或粘贴内容…"),
                );
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("task")
                        .selected_text(self.task.label())
                        .show_ui(ui, |ui| {
                            for task in Task::ALL {
                                if ui
                                    .selectable_value(&mut self.task, task, task.label())
                                    .changed()
                                {
                                    self.config.smart_mode = false;
                                }
                            }
                        });
                    if ui.button("查询").clicked() {
                        self.query(&ctx, false);
                    }
                    if self.busy && ui.button("停止").clicked() {
                        self.stop();
                        self.status = "已停止".into();
                    }
                    if ui.button("复制").clicked() {
                        ctx.copy_text(self.answer.clone());
                    }
                });
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        if let Some(entry) = &self.dict_entry {
                            ui.horizontal_wrapped(|ui| {
                                ui.heading(&entry.word);
                                if !entry.phonetic.is_empty() {
                                    ui.label(format!("/{}/", entry.phonetic));
                                }
                                ui.weak("离线词典");
                            });
                            ui.add(egui::Label::new(&entry.translation).selectable(true).wrap());
                            let forms = entry.forms();
                            if !forms.is_empty() {
                                let line: Vec<String> =
                                    forms.iter().map(|(k, v)| format!("{k} {v}")).collect();
                                ui.weak(line.join("  ·  "));
                            }
                            if let Some(lemma) = entry.lemma() {
                                ui.weak(format!("原形：{lemma}"));
                            }
                            ui.separator();
                        }
                        ui.add(egui::Label::new(&self.answer).selectable(true).wrap());
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.followup).hint_text("继续追问…"));
                    if ui
                        .add_enabled(
                            !self.messages.is_empty() && !self.busy,
                            egui::Button::new("发送"),
                        )
                        .clicked()
                    {
                        self.query(&ctx, true);
                    }
                });
            }
            ui.label(&self.status);
        });
    }
}
fn main() -> eframe::Result {
    eframe::run_native(
        "Crant Peek",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([480.0, 560.0])
                .with_decorations(false)
                .with_always_on_top(),
            renderer: eframe::Renderer::Glow,
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(Peek::new(cc)))),
    )
}
