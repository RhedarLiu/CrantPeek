#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod desktop;
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
    desktop: Option<desktop::Desktop>,
    quit: bool,
    visible: bool,
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
            desktop,
            quit: false,
            visible: true,
        }
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
        };
        if text.is_empty() {
            return;
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
                    self.status = "截图选区正在开发中".into();
                    self.visible = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
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
