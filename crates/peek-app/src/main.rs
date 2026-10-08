#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod capture;
mod desktop;
mod instance;
mod ocr;
mod permissions;
mod selection;
mod store;
use eframe::egui;
use peek_core::{Config, Message, Protocol, Task, local_route};
use peek_network::{Client, Event};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn apply_appearance(ctx: &egui::Context, config: &Config) {
    let preference = match config.theme.as_str() {
        "light" => egui::ThemePreference::Light,
        "dark" => egui::ThemePreference::Dark,
        _ => egui::ThemePreference::System,
    };
    ctx.set_theme(preference);
    ctx.set_zoom_factor(config.zoom);
}

struct Peek {
    config: Config,
    draft: Config,
    settings: bool,
    input: String,
    answer: String,
    followup: String,
    messages: Vec<Message>,
    task: Task,
    manual_task: bool,
    target_override: String,
    pinned: bool,
    status: String,
    secret_draft: String,
    decision_secret_draft: String,
    route_note: String,
    runtime: tokio::runtime::Runtime,
    client: Client,
    test_rx: Option<mpsc::Receiver<Event>>,
    test_cancel: Option<CancellationToken>,
    test_status: String,
    receiver: Option<mpsc::Receiver<Event>>,
    cancel: Option<CancellationToken>,
    busy: bool,
    dictionary: Option<peek_dict::Dict>,
    dictionary_missing: bool,
    dict_entry: Option<peek_dict::Entry>,
    desktop: Option<desktop::Desktop>,
    quit: bool,
    paused: bool,
    visible: bool,
    focus_grace_until: std::time::Instant,
    initial_hide: bool,
    screenshot_image: Option<peek_network::ImageInput>,
    send_image: bool,
    decide_image: bool,
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
                "/System/Library/Fonts/Hiragino Sans GB.ttc",
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
        apply_appearance(&cc.egui_ctx, &config);
        let desktop = desktop::Desktop::new(&config, cc.egui_ctx.clone());
        let (desktop, status) = match desktop {
            Ok(d) => (Some(d), status),
            Err(e) => (None, format!("Desktop integration error: {e}")),
        };
        let initial_hide = config.onboarding_complete && desktop.is_some() && status.is_empty();
        Self {
            initial_hide,
            draft: config.clone(),
            settings: !config.onboarding_complete,
            config,
            input: String::new(),
            answer: String::new(),
            followup: String::new(),
            messages: Vec::new(),
            task: Task::Translate,
            manual_task: false,
            target_override: String::new(),
            pinned: false,
            status,
            secret_draft: String::new(),
            decision_secret_draft: String::new(),
            route_note: String::new(),
            runtime: tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("async runtime"),
            client: Client::default(),
            test_rx: None,
            test_cancel: None,
            test_status: String::new(),
            receiver: None,
            cancel: None,
            busy: false,
            dictionary: None,
            dictionary_missing: false,
            dict_entry: None,
            desktop,
            quit: false,
            paused: false,
            visible: true,
            focus_grace_until: std::time::Instant::now() + std::time::Duration::from_millis(350),
            screenshot_image: None,
            send_image: false,
            decide_image: false,
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
        // Dropping receivers invalidates late capture/OCR results without reopening the UI.
        self.capture_rx = None;
        self.ocr_rx = None;
        self.snip = None;
        self.snip_texture = None;
        self.drag_start = None;
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel();
        }
        self.receiver = None;
        self.busy = false;
        peek_core::discard_pending_turn(&mut self.messages);
    }
    fn query(&mut self, ctx: &egui::Context, followup: bool) {
        // Consume explicit upload intent at entry, including all early-error paths.
        let send_image = std::mem::take(&mut self.send_image);
        let decide_image = std::mem::take(&mut self.decide_image);
        let text = if followup {
            self.followup.trim()
        } else {
            self.input.trim()
        }
        .to_owned();
        if text.is_empty() {
            return;
        }
        if text.len() > peek_core::MAX_INPUT_BYTES {
            self.status = "输入超过 64 KiB，请缩短选区或分段查询".into();
            return;
        }
        self.stop();
        let text = text.as_str();
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
            if self.config.smart_mode && !self.manual_task {
                self.task = route.task;
            }
            self.messages = vec![Message {
                role: "system".into(),
                content: self.task.instruction(peek_core::effective_target(
                    &route.target,
                    &self.target_override,
                )),
            }];
        }
        if followup && let Some(system) = self.messages.first_mut() {
            let automatic = local_route(
                &self.input,
                &self.config.target_language,
                &self.config.chinese_target,
            )
            .target;
            system.content = self.task.instruction(peek_core::effective_target(
                &automatic,
                &self.target_override,
            ));
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
        let image = if send_image {
            self.screenshot_image.clone()
        } else {
            None
        };
        self.send_image = false;
        let decision_image = if decide_image {
            self.screenshot_image.clone()
        } else {
            None
        };
        self.decide_image = false;
        let mut messages = self.messages.clone();
        let decision = self.config.decision.clone();
        let use_decision = !followup
            && (self.config.smart_mode || decision_image.is_some())
            && !self.manual_task
            && decision.enabled
            && (self.task != Task::Define || decision_image.is_some());
        let decision_key = if use_decision {
            store::secret(&decision.credential_id).ok()
        } else {
            None
        };
        let query_text = text.to_owned();
        let target = local_route(
            text,
            &self.config.target_language,
            &self.config.chinese_target,
        )
        .target;
        let target = if self.target_override.trim().is_empty() {
            target
        } else {
            self.target_override.clone()
        };
        let fallback_task = self.task;
        self.route_note = if use_decision {
            "正在判断任务…".into()
        } else {
            "本地判断 / 手动模式".into()
        };
        let ctx = ctx.clone();
        let client = self.client.clone();
        self.runtime.spawn(async move {
            if use_decision {
                let routed = if let Some(ref decision_key) = decision_key {
                    tokio::time::timeout(
                        std::time::Duration::from_millis(decision.timeout_ms),
                        async {
                            if let Some(image) = &decision_image {
                                client
                                    .decide_image(
                                        &decision.endpoint,
                                        decision_key,
                                        &decision.model,
                                        &query_text,
                                        image,
                                        cancel.clone(),
                                    )
                                    .await
                            } else {
                                client
                                    .decide(
                                        &decision.endpoint,
                                        decision_key,
                                        &decision.model,
                                        &query_text,
                                        cancel.clone(),
                                    )
                                    .await
                            }
                        },
                    )
                    .await
                    .ok()
                } else {
                    None
                };
                let (task, note) = match routed {
                    Some(Ok(result)) if result.confidence >= decision.min_confidence => (
                        result.task,
                        format!("{} · 置信度 {:.2}", decision.model, result.confidence),
                    ),
                    Some(Ok(_)) => (fallback_task, "决策不确定 · 使用本地判断".into()),
                    _ => (fallback_task, "决策不可用 · 使用本地判断".into()),
                };
                if cancel.is_cancelled() {
                    return;
                }
                messages[0].content = task.instruction(&target);
                if tx.send(Event::Route { task, note }).await.is_err() {
                    return;
                }
                ctx.request_repaint();
            }
            let (net_tx, mut net_rx) = mpsc::channel(32);
            let worker = tokio::spawn(async move {
                if let Some(image) = image {
                    client
                        .stream_image(&provider, &key, &messages, &image, net_tx, cancel)
                        .await
                } else {
                    client
                        .stream(&provider, &key, &messages, net_tx, cancel)
                        .await
                }
            });
            while let Some(event) = net_rx.recv().await {
                if tx.send(event).await.is_err() {
                    break;
                }
                ctx.request_repaint();
            }
            let failure = match worker.await {
                Ok(Ok(())) => None,
                Ok(Err(e)) => Some(e.to_string()),
                Err(_) => Some("网络任务异常退出，请重试".into()),
            };
            if let Some(failure) = failure {
                let _ = tx.send(Event::Failed(failure)).await;
                ctx.request_repaint();
            }
        });
    }
    fn test_connection(&mut self, ctx: &egui::Context) {
        if self.draft.provider.model.trim().is_empty() {
            self.test_status = "请填写模型".into();
            return;
        }
        if let Err(e) = self.draft.validate() {
            self.test_status = e.into();
            return;
        }
        let key = if self.secret_draft.is_empty() {
            match store::secret(&self.draft.provider.credential_id) {
                Ok(key) => key,
                Err(e) => {
                    self.test_status = e;
                    return;
                }
            }
        } else {
            self.secret_draft.clone()
        };
        if let Some(cancel) = self.test_cancel.take() {
            cancel.cancel();
        }
        let cancel = CancellationToken::new();
        self.test_cancel = Some(cancel.clone());
        let (tx, rx) = mpsc::channel(16);
        self.test_rx = Some(rx);
        self.test_status = "测试中（会产生少量 API 用量）…".into();
        let client = self.client.clone();
        let provider = self.draft.provider.clone();
        let ctx = ctx.clone();
        self.runtime.spawn(async move {
            let (stream_tx, mut stream_rx) = mpsc::channel(16);
            let worker = tokio::spawn(async move {
                let messages = [Message {
                    role: "user".into(),
                    content: "Reply only OK.".into(),
                }];
                tokio::time::timeout(
                    std::time::Duration::from_secs(20),
                    client.stream(&provider, &key, &messages, stream_tx, cancel),
                )
                .await
            });
            while let Some(event) = stream_rx.recv().await {
                let _ = tx.send(event).await;
                ctx.request_repaint();
            }
            let failure = match worker.await {
                Ok(Ok(Ok(()))) => None,
                Ok(Ok(Err(e))) => Some(e.to_string()),
                Ok(Err(_)) => Some("连接测试超时".into()),
                Err(_) => Some("连接测试任务失败".into()),
            };
            if let Some(e) = failure {
                let _ = tx.send(Event::Failed(e)).await;
                ctx.request_repaint();
            }
        });
    }
    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("设置");
        ui.collapsing("外观", |ui| {
            egui::ComboBox::from_label("主题")
                .selected_text(&self.draft.theme)
                .show_ui(ui, |ui| {
                    for (value, label) in
                        [("system", "跟随系统"), ("light", "浅色"), ("dark", "深色")]
                    {
                        ui.selectable_value(&mut self.draft.theme, value.into(), label);
                    }
                });
            ui.add(egui::Slider::new(&mut self.draft.zoom, 0.8..=1.5).text("界面缩放"));
        });
        if !self.config.onboarding_complete {
            ui.group(|ui| {
                ui.heading("欢迎使用 Crant Peek");
                ui.label("双击 Ctrl：只查询选区；没有选区不弹窗。");
                ui.label(format!("{}：空白输入；{}：截图", self.config.blank_hotkey, self.config.screenshot_hotkey));
                ui.label("Esc 或失焦收起；固定后可对照阅读。托盘菜单重新打开或退出。");
                ui.weak("词典与 OCR 在本地运行。AI 查询文本会发往你配置的回答/决策服务；不自动上传剪贴板或整屏。");
                if ui.button("了解，开始使用").clicked() {
                    let mut config = self.config.clone(); config.onboarding_complete = true;
                    match store::save(&config) { Ok(()) => { self.config = config; self.draft.onboarding_complete = true; }, Err(e) => self.status = e }
                }
            });
        }
        ui.collapsing("快捷键（保存后重启生效）", |ui| {
            ui.label("空白浮窗"); ui.text_edit_singleline(&mut self.draft.blank_hotkey);
            ui.label("截图"); ui.text_edit_singleline(&mut self.draft.screenshot_hotkey);
            ui.add(egui::Slider::new(&mut self.draft.double_ctrl_ms,150..=800).text("双击 Ctrl 间隔 ms"));
            ui.weak("示例：Super+Shift+A（macOS Command）、Alt+Shift+A（Windows）。不要与系统或其他应用快捷键冲突。");
        });
        ui.collapsing("系统权限与使用说明", |ui| {
            for (name, granted) in permissions::status() {
                ui.label(format!("{name}：{}", if granted { "已授权" } else { "未授权" }));
            }
            ui.weak("双击 Ctrl 只读取选区，无选区不弹窗。应用不支持选区读取时，请用空白浮窗主动粘贴或截图。");
            if cfg!(windows) { ui.weak("不能读取高权限应用或安全输入。系统 OCR 需要已安装的语言包。"); }
            ui.weak("macOS 修改权限后可能需要退出并重新启动 Peek；开发构建路径变化也可能需要重新授权。");
            if ui.button("打开系统隐私设置").clicked()
                && let Err(e) = permissions::open_settings() { self.status = e; }
        });
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
        ui.checkbox(
            &mut self.draft.provider.vision,
            "该回答模型支持图片输入（手动图片解读时上传框选区域）",
        );
        ui.label("API key（留空保留已有凭据）");
        ui.add(egui::TextEdit::singleline(&mut self.secret_draft).password(true));
        ui.horizontal(|ui| {
            if ui.button("测试回答连接（少量 API 用量）").clicked() {
                self.test_connection(ui.ctx());
            }
            if self.test_rx.is_some() && ui.button("取消测试").clicked() {
                if let Some(cancel) = self.test_cancel.take() {
                    cancel.cancel();
                }
                self.test_rx = None;
                self.test_status = "已取消连接测试".into();
            }
        });
        ui.label(&self.test_status);
        ui.label("默认目标语言");
        ui.text_edit_singleline(&mut self.draft.target_language);
        ui.label("中文翻译目标");
        ui.text_edit_singleline(&mut self.draft.chinese_target);
        ui.collapsing("决策模型（可选）", |ui| {
            ui.checkbox(&mut self.draft.decision.enabled, "启用专用决策服务");
            ui.label("Endpoint：完整 URL；TypeSafe 或 Cloudflare 模型路由");
            ui.text_edit_singleline(&mut self.draft.decision.endpoint);
            ui.label("决策 Model");
            ui.text_edit_singleline(&mut self.draft.decision.model);
            ui.label("决策 API key / Token（留空保留）");
            ui.add(egui::TextEdit::singleline(&mut self.decision_secret_draft).password(true));
            ui.add(
                egui::Slider::new(&mut self.draft.decision.min_confidence, 0.0..=1.0)
                    .text("采用阈值"),
            );
            ui.add(
                egui::Slider::new(&mut self.draft.decision.timeout_ms, 100..=10000).text("超时 ms"),
            );
            ui.weak("只发送当前查询文本。不可用时回退本地判断；图像决策尚未接入。");
        });
        ui.checkbox(&mut self.draft.smart_mode, "智能判断模式");
        ui.checkbox(
            &mut self.draft.ocr_auto_query,
            "截图识字后自动查询（识别文字发送到模型）",
        );
        ui.weak("关闭自动查询后只本地识字，点击查询/图片按钮才调用外部服务。");
        ui.checkbox(&mut self.draft.hide_on_blur, "失焦隐藏（取消固定后生效）");
        ui.horizontal(|ui| {
            if ui.button("保存").clicked() {
                let result = self.draft.validate().map_err(str::to_owned).and_then(|()| {
                    let blank: global_hotkey::hotkey::HotKey = self
                        .draft
                        .blank_hotkey
                        .parse()
                        .map_err(|e| format!("空白快捷键无效：{e}"))?;
                    let screenshot: global_hotkey::hotkey::HotKey = self
                        .draft
                        .screenshot_hotkey
                        .parse()
                        .map_err(|e| format!("截图快捷键无效：{e}"))?;
                    if blank.id() == screenshot.id() {
                        return Err("两个入口不能使用相同快捷键".into());
                    }
                    if !self.secret_draft.is_empty() {
                        store::save_secret(&self.draft.provider.credential_id, &self.secret_draft)?;
                    }
                    if !self.decision_secret_draft.is_empty() {
                        store::save_secret(
                            &self.draft.decision.credential_id,
                            &self.decision_secret_draft,
                        )?;
                    }
                    store::save(&self.draft)
                });
                match result {
                    Ok(()) => {
                        let shortcuts_changed = self.config.blank_hotkey != self.draft.blank_hotkey
                            || self.config.screenshot_hotkey != self.draft.screenshot_hotkey
                            || self.config.double_ctrl_ms != self.draft.double_ctrl_ms;
                        self.config = self.draft.clone();
                        apply_appearance(ui.ctx(), &self.config);
                        self.secret_draft.clear();
                        self.decision_secret_draft.clear();
                        self.settings = false;
                        self.status = if shortcuts_changed {
                            "已保存，快捷键修改需重启生效"
                        } else {
                            "已保存"
                        }
                        .into();
                    }
                    Err(e) => self.status = e,
                }
            }
            if ui.button("取消").clicked() {
                self.draft = self.config.clone();
                self.secret_draft.clear();
                self.decision_secret_draft.clear();
                self.settings = false;
            }
        });
    }
}
impl Drop for Peek {
    fn drop(&mut self) {
        if let Some(cancel) = self.test_cancel.take() {
            cancel.cancel();
        }
        self.stop();
    }
}
impl eframe::App for Peek {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        if std::mem::take(&mut self.initial_hide) {
            self.visible = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        let test_events: Vec<_> = self
            .test_rx
            .as_mut()
            .map(|rx| {
                let mut events = Vec::new();
                while let Ok(e) = rx.try_recv() {
                    events.push(e);
                }
                events
            })
            .unwrap_or_default();
        for event in test_events {
            match event {
                Event::Done => {
                    self.test_status = "连接成功 · 流式响应正常".into();
                    self.test_rx = None;
                    self.test_cancel = None;
                }
                Event::Failed(e) => {
                    self.test_status = format!("连接失败：{e}");
                    self.test_rx = None;
                    self.test_cancel = None;
                }
                _ => {}
            }
        }
        let actions: Vec<_> = self
            .desktop
            .as_ref()
            .map(|d| d.events.try_iter().collect())
            .unwrap_or_default();
        for action in actions {
            if self.paused
                && matches!(
                    action,
                    desktop::Action::Blank
                        | desktop::Action::Selection(_)
                        | desktop::Action::Screenshot
                )
            {
                continue;
            }
            match action {
                desktop::Action::TogglePause => {
                    self.paused = !self.paused;
                    if self.paused {
                        self.stop();
                        self.visible = false;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                    }
                    self.status = if self.paused {
                        "快捷入口已暂停（托盘可恢复）"
                    } else {
                        "快捷入口已恢复"
                    }
                    .into();
                }
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
                    self.route_note.clear();
                    self.followup.clear();
                    self.messages.clear();
                    self.manual_task = false;
                    self.screenshot_image = None;
                    self.target_override.clear();
                    self.send_image = false;
                    self.settings = false;
                    self.status.clear();
                    self.visible = true;
                    self.focus_grace_until =
                        std::time::Instant::now() + std::time::Duration::from_millis(350);
                    let size = ctx
                        .input(|i| i.viewport().inner_rect.map(|r| r.size()))
                        .unwrap_or(egui::vec2(480.0, 560.0));
                    if let Some(position) = capture::popup_position(size) {
                        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(position));
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                desktop::Action::Selection(text) => {
                    self.stop();
                    self.input = text;
                    self.answer.clear();
                    self.dict_entry = None;
                    self.route_note.clear();
                    self.followup.clear();
                    self.messages.clear();
                    self.manual_task = false;
                    self.screenshot_image = None;
                    self.target_override.clear();
                    self.send_image = false;
                    self.settings = false;
                    self.status.clear();
                    self.visible = true;
                    self.focus_grace_until =
                        std::time::Instant::now() + std::time::Duration::from_millis(350);
                    let size = ctx
                        .input(|i| i.viewport().inner_rect.map(|r| r.size()))
                        .unwrap_or(egui::vec2(480.0, 560.0));
                    if let Some(position) = capture::popup_position(size) {
                        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(position));
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    self.query(&ctx, false);
                }
                desktop::Action::Settings => {
                    self.stop();
                    self.draft = self.config.clone();
                    self.settings = true;
                    self.visible = true;
                    self.focus_grace_until =
                        std::time::Instant::now() + std::time::Duration::from_millis(350);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                desktop::Action::Screenshot => {
                    self.stop();
                    self.screenshot_image = None;
                    self.target_override.clear();
                    self.send_image = false;
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
            self.focus_grace_until =
                std::time::Instant::now() + std::time::Duration::from_millis(350);
            self.settings = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            match result {
                Ok(text) if !text.trim().is_empty() => {
                    self.input = text;
                    if self.config.ocr_auto_query {
                        self.query(&ctx, false);
                    } else {
                        self.status =
                            "本地识字完成 · 可复制原文或手动查询，尚未发送内容到模型".into();
                    }
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
                            let (rect, response) = ui
                                .allocate_exact_size(size / ctx.zoom_factor(), egui::Sense::drag());
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
                                    selection = Some(capture::selection_to_logical(
                                        r.translate(-rect.min.to_vec2()),
                                        rect.size(),
                                        size,
                                    ));
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
                if !cancelled
                    && let Some(image) = selection.and_then(|rect| capture::crop(screen, rect))
                {
                    let image = capture::prepare_region(image);
                    let mut png = std::io::Cursor::new(Vec::new());
                    if image::DynamicImage::ImageRgba8(image.clone())
                        .write_to(&mut png, image::ImageFormat::Png)
                        .is_ok()
                    {
                        use base64::Engine;
                        self.screenshot_image = Some(peek_network::ImageInput {
                            png_base64: base64::engine::general_purpose::STANDARD
                                .encode(png.get_ref()),
                        });
                    }
                    let (tx, rx) = std::sync::mpsc::channel();
                    self.ocr_rx = Some(rx);
                    self.input.clear();
                    self.answer.clear();
                    self.dict_entry = None;
                    self.messages.clear();
                    self.route_note.clear();
                    self.manual_task = false;
                    self.settings = false;
                    self.visible = true;
                    self.focus_grace_until =
                        std::time::Instant::now() + std::time::Duration::from_millis(350);
                    self.status = "本地识字中…可取消".into();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
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
                Event::Route { task, note } => {
                    self.task = task;
                    self.route_note = note;
                    if let Some(system) = self.messages.first_mut() {
                        let target = local_route(
                            &self.input,
                            &self.config.target_language,
                            &self.config.chinese_target,
                        )
                        .target;
                        system.content =
                            task.instruction(if self.target_override.trim().is_empty() {
                                &target
                            } else {
                                &self.target_override
                            });
                    }
                }
                Event::Text(text) => {
                    if self.answer.len().saturating_add(text.len()) > peek_core::MAX_OUTPUT_BYTES {
                        self.stop();
                        self.status = "回答超过 512 KiB，已停止；请缩小问题范围".into();
                        break;
                    }
                    self.answer.push_str(&text);
                }
                Event::Done => {
                    self.busy = false;
                    self.status = "完成".into();
                    self.messages.push(Message {
                        role: "assistant".into(),
                        content: self.answer.clone(),
                    });
                    if peek_core::bound_history(&mut self.messages) {
                        self.status = "完成 · 已释放较早追问，保留初始问题与最近对话".into();
                    }
                }
                Event::Failed(e) => {
                    peek_core::discard_pending_turn(&mut self.messages);
                    self.busy = false;
                    self.status = e;
                }
            }
        }
        if self.desktop.is_some() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.stop();
            self.visible = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        if self.desktop.is_some()
            && self.visible
            && !self.pinned
            && self.config.hide_on_blur
            && !self.settings
            && std::time::Instant::now() >= self.focus_grace_until
            && ctx.input(|i| i.viewport().focused == Some(false))
        {
            self.stop();
            self.visible = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        if self.visible && std::time::Instant::now() < self.focus_grace_until {
            ctx.request_repaint_after(
                self.focus_grace_until
                    .saturating_duration_since(std::time::Instant::now()),
            );
        }
        egui::CentralPanel::default().show(root, |ui| {
            ui.horizontal(|ui| {
                let title = ui.heading("Crant Peek");
                if title.interact(egui::Sense::drag()).drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                ui.checkbox(&mut self.pinned, "固定");
                if self.paused {
                    ui.weak("快捷入口已暂停");
                }
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
                egui::ScrollArea::vertical().show(ui, |ui| self.settings_ui(ui));
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
                                    self.manual_task = true;
                                }
                            }
                        });
                    if self.manual_task && ui.button("恢复智能").clicked() {
                        self.manual_task = false;
                    }
                    if ui.button("查询").clicked()
                        || ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter))
                    {
                        self.query(&ctx, false);
                    }
                    if self.busy && ui.button("停止").clicked() {
                        self.stop();
                        self.status = "已停止".into();
                    }
                    if ui.button("复制").clicked() {
                        let text = if self.answer.is_empty() {
                            self.dict_entry
                                .as_ref()
                                .map(|e| e.translation.clone())
                                .unwrap_or_default()
                        } else {
                            self.answer.clone()
                        };
                        ctx.copy_text(text);
                    }
                });
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.input.is_empty(), egui::Button::new("复制原文"))
                        .clicked()
                    {
                        ctx.copy_text(self.input.clone());
                        self.status = "已复制原文".into();
                    }
                    if ui.button("清空会话").clicked() {
                        self.stop();
                        self.input.clear();
                        self.answer.clear();
                        self.followup.clear();
                        self.messages.clear();
                        self.dict_entry = None;
                        self.screenshot_image = None;
                        self.send_image = false;
                        self.decide_image = false;
                        self.manual_task = false;
                        self.route_note.clear();
                        self.status = "已清空当前会话".into();
                    }
                });
                if self.screenshot_image.is_some() {
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(
                                self.config.provider.vision && !self.busy,
                                egui::Button::new("图片解读（上传选区）"),
                            )
                            .clicked()
                        {
                            if self.input.trim().is_empty() {
                                self.input = "请解释截图中的内容。".into();
                            }
                            self.task = Task::Explain;
                            self.manual_task = true;
                            self.send_image = true;
                            self.query(&ctx, false);
                        }
                        if ui
                            .add_enabled(
                                self.config.decision.enabled
                                    && matches!(
                                        self.config.decision.model.as_str(),
                                        "clef" | "clef-flash"
                                    )
                                    && !self.busy,
                                egui::Button::new("Clef 判断截图（上传选区）"),
                            )
                            .clicked()
                        {
                            if self.input.trim().is_empty() {
                                self.input = "请判断截图需要什么阅读辅助。".into();
                            }
                            self.manual_task = false;
                            self.decide_image = true;
                            self.query(&ctx, false);
                        }
                        if ui.button("丢弃图片").clicked() {
                            self.screenshot_image = None;
                        }
                    });
                    ui.weak(format!(
                        "Clef 按钮仅发送到决策服务：{}",
                        self.config.decision.endpoint
                    ));
                    ui.weak(format!(
                        "图片解读按钮发送到 {}；后续追问默认只发文字",
                        self.config.provider.base_url
                    ));
                }
                ui.horizontal(|ui| {
                    egui::ComboBox::from_label("当前目标语言")
                        .selected_text(if self.target_override.is_empty() {
                            "自动"
                        } else {
                            &self.target_override
                        })
                        .show_ui(ui, |ui| {
                            for (value, label) in [
                                ("", "自动"),
                                ("Chinese", "中文"),
                                ("English", "英文"),
                                ("Japanese", "日文"),
                            ] {
                                ui.selectable_value(&mut self.target_override, value.into(), label);
                            }
                        });
                    ui.add(
                        egui::TextEdit::singleline(&mut self.target_override)
                            .desired_width(100.0)
                            .hint_text("自定义语言"),
                    );
                });
                ui.weak(&self.route_note);
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
                    let input = ui
                        .add(egui::TextEdit::singleline(&mut self.followup).hint_text("继续追问…"));
                    let enter = input.lost_focus()
                        && ctx.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.command);
                    if (ui
                        .add_enabled(
                            !self.messages.is_empty() && !self.busy,
                            egui::Button::new("发送"),
                        )
                        .clicked()
                        || enter)
                        && !self.messages.is_empty()
                        && !self.busy
                    {
                        self.query(&ctx, true);
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.label(&self.status);
                if self.ocr_rx.is_some() && ui.button("取消识字").clicked() {
                    self.stop();
                    self.screenshot_image = None;
                    self.status = "已取消识字".into();
                }
            });
        });
    }
}
fn main() -> eframe::Result {
    let _instance = match store::config_path().and_then(|path| {
        instance::acquire(&path.with_file_name("instance.lock")).map_err(|e| e.to_string())
    }) {
        Ok(Some(lock)) => Some(lock),
        Ok(None) => return Ok(()),
        Err(e) => {
            eprintln!("Cannot acquire application instance lock: {e}");
            return Ok(());
        }
    };
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
