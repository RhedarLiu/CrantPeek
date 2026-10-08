#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod capture;
mod design;
mod desktop;

// Shared, GUI-toolkit-agnostic runtime. Re-exported at the crate root so the
// existing `crate::i18n::…` / `store::…` call sites keep working unchanged.
use eframe::egui;
use peek_core::{Config, Message, Protocol, Task, local_route};
use peek_network::{Client, Event};
pub(crate) use peek_runtime::{i18n, instance, ocr, permissions, selection, store};
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
    design::apply(ctx);
}

struct Peek {
    preview: Option<String>,
    preview_frames: u32,
    locale: i18n::I18n,
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
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        cc.egui_ctx.set_fonts(fonts);
        let preview = std::env::var("PEEK_UI_PREVIEW").ok();
        let (mut config, status) = match if preview.is_some() {
            Ok(Config::default())
        } else {
            store::load()
        } {
            Ok(c) => (c, String::new()),
            Err(e) => (
                Config::default(),
                i18n::format("status-config-error", &[("error", &i18n::diagnostic(&e))]),
            ),
        };
        if preview.is_some() {
            config.ui_language = std::env::var("PEEK_UI_LANGUAGE").unwrap_or_else(|_| "en".into());
            config.theme = std::env::var("PEEK_UI_THEME").unwrap_or_else(|_| "light".into());
            config.onboarding_complete = true;
        }
        i18n::set_language(&config.ui_language);
        apply_appearance(&cc.egui_ctx, &config);
        let desktop = if preview.is_some() {
            None
        } else {
            Some(desktop::Desktop::new(&config, cc.egui_ctx.clone()))
        };
        let desktop = desktop.unwrap_or_else(|| Err(String::new()));
        let (desktop, status) = match desktop {
            Ok(d) => (Some(d), status),
            Err(_) if preview.is_some() => (None, String::new()),
            Err(e) => (
                None,
                i18n::format("status-desktop-error", &[("error", &i18n::diagnostic(&e))]),
            ),
        };
        let initial_hide = config.onboarding_complete && desktop.is_some() && status.is_empty();
        Self {
            preview: preview.clone(),
            preview_frames: 0,
            locale: i18n::I18n::new(&config.ui_language),
            initial_hide,
            draft: config.clone(),
            settings: !config.onboarding_complete
                || (preview.is_some()
                    && std::env::var("PEEK_UI_PAGE").as_deref() == Ok("settings")),
            config,
            input: if preview.is_some() {
                i18n::tr("preview-source")
            } else {
                String::new()
            },
            answer: if preview.is_some() {
                i18n::tr("preview-answer")
            } else {
                String::new()
            },
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
            self.status = i18n::tr("status-input-too-long");
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
            self.status = i18n::tr("status-model-missing");
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
                content: self.task.styled_instruction(
                    peek_core::effective_target(&route.target, &self.target_override),
                    &self.config.translation_style,
                ),
            }];
        }
        if followup && let Some(system) = self.messages.first_mut() {
            let automatic = local_route(
                &self.input,
                &self.config.target_language,
                &self.config.chinese_target,
            )
            .target;
            system.content = self.task.styled_instruction(
                peek_core::effective_target(&automatic, &self.target_override),
                &self.config.translation_style,
            );
        }
        self.messages.push(Message {
            role: "user".into(),
            content: text.into(),
        });
        self.followup.clear();
        self.answer.clear();
        self.status = i18n::tr("status-generating");
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
        let translation_style = self.config.translation_style.clone();
        self.route_note = if use_decision {
            i18n::tr("route-deciding")
        } else {
            i18n::tr("route-local")
        };
        let ctx = ctx.clone();
        let client = self.client.clone();
        let ui_language = self.config.ui_language.clone();
        self.runtime.spawn(async move {
            let locale = i18n::I18n::new(&ui_language);
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
                        locale.format(
                            "route-decided",
                            &[
                                ("model", &decision.model),
                                ("confidence", &format!("{:.2}", result.confidence)),
                            ],
                        ),
                    ),
                    Some(Ok(_)) => (fallback_task, locale.text("route-uncertain")),
                    _ => (fallback_task, locale.text("route-unavailable")),
                };
                if cancel.is_cancelled() {
                    return;
                }
                messages[0].content = task.styled_instruction(&target, &translation_style);
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
                let event = if let Event::Failed(code) = event {
                    Event::Failed(locale.text(&code))
                } else {
                    event
                };
                if tx.send(event).await.is_err() {
                    break;
                }
                ctx.request_repaint();
            }
            let failure = match worker.await {
                Ok(Ok(())) => None,
                Ok(Err(e)) => Some(locale.network_error(&e)),
                Err(_) => Some(locale.text("status-worker-crashed")),
            };
            if let Some(failure) = failure {
                let _ = tx.send(Event::Failed(failure)).await;
                ctx.request_repaint();
            }
        });
    }
    fn test_connection(&mut self, ctx: &egui::Context) {
        if self.draft.provider.model.trim().is_empty() {
            self.test_status = i18n::tr("status-model-missing");
            return;
        }
        if let Err(e) = self.draft.validate() {
            self.test_status = i18n::tr(e);
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
        self.test_status = i18n::tr("settings-test-running");
        let client = self.client.clone();
        let provider = self.draft.provider.clone();
        let ctx = ctx.clone();
        let ui_language = self.config.ui_language.clone();
        self.runtime.spawn(async move {
            let locale = i18n::I18n::new(&ui_language);
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
                let event = if let Event::Failed(code) = event {
                    Event::Failed(locale.text(&code))
                } else {
                    event
                };
                let _ = tx.send(event).await;
                ctx.request_repaint();
            }
            let failure = match worker.await {
                Ok(Ok(Ok(()))) => None,
                Ok(Ok(Err(e))) => Some(locale.network_error(&e)),
                Ok(Err(_)) => Some(locale.text("settings-test-timeout")),
                Err(_) => Some(locale.text("settings-test-crashed")),
            };
            if let Some(e) = failure {
                let _ = tx.send(Event::Failed(e)).await;
                ctx.request_repaint();
            }
        });
    }
    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading(i18n::tr("settings-title"));
        egui::ComboBox::from_id_salt("ui-language")
            .selected_text(i18n::tr(match self.draft.ui_language.as_str() {
                "zh-CN" => "language-zh",
                "en" => "language-en",
                _ => "settings-language-system",
            }))
            .show_ui(ui, |ui| {
                for (value, key) in [
                    ("system", "settings-language-system"),
                    ("zh-CN", "language-zh"),
                    ("en", "language-en"),
                ] {
                    ui.selectable_value(&mut self.draft.ui_language, value.into(), i18n::tr(key));
                }
            });
        egui::CollapsingHeader::new(i18n::tr("settings-section-appearance"))
            .id_salt("settings-section-appearance")
            .show(ui, |ui| {
                egui::ComboBox::from_id_salt("settings-theme")
                    .selected_text(i18n::tr(match self.draft.theme.as_str() {
                        "light" => "settings-theme-light",
                        "dark" => "settings-theme-dark",
                        _ => "settings-theme-system",
                    }))
                    .show_ui(ui, |ui| {
                        for (value, label) in [
                            ("system", i18n::tr("settings-theme-system")),
                            ("light", i18n::tr("settings-theme-light")),
                            ("dark", i18n::tr("settings-theme-dark")),
                        ] {
                            ui.selectable_value(&mut self.draft.theme, value.into(), label);
                        }
                    });
                ui.add(
                    egui::Slider::new(&mut self.draft.zoom, 0.8..=1.5)
                        .text(i18n::tr("settings-zoom")),
                );
            });
        if !self.config.onboarding_complete {
            ui.group(|ui| {
                ui.heading(i18n::tr("welcome-title"));
                ui.label(i18n::tr("welcome-selection"));
                ui.label(i18n::format(
                    "welcome-shortcuts",
                    &[
                        ("blank", &self.config.blank_hotkey),
                        ("screenshot", &self.config.screenshot_hotkey),
                    ],
                ));
                ui.label(i18n::tr("welcome-dismiss"));
                ui.weak(i18n::tr("welcome-privacy"));
                if ui.button(i18n::tr("welcome-start")).clicked() {
                    let mut config = self.config.clone();
                    config.onboarding_complete = true;
                    match store::save(&config) {
                        Ok(()) => {
                            self.config = config;
                            self.draft.onboarding_complete = true;
                        }
                        Err(e) => self.status = e,
                    }
                }
            });
        }
        egui::CollapsingHeader::new(i18n::tr("settings-section-shortcuts"))
            .id_salt("settings-section-shortcuts")
            .show(ui, |ui| {
                ui.label(i18n::tr("settings-shortcut-blank"));
                ui.text_edit_singleline(&mut self.draft.blank_hotkey);
                ui.label(i18n::tr("settings-shortcut-screenshot"));
                ui.text_edit_singleline(&mut self.draft.screenshot_hotkey);
                ui.add(
                    egui::Slider::new(&mut self.draft.double_ctrl_ms, 150..=800)
                        .text(i18n::tr("settings-double-ctrl")),
                );
                ui.weak(i18n::tr("settings-shortcut-hint"));
            });
        egui::CollapsingHeader::new(i18n::tr("settings-permissions"))
            .id_salt("settings-permissions")
            .show(ui, |ui| {
                for (name, granted) in permissions::status() {
                    ui.label(format!(
                        "{} · {}",
                        i18n::tr(name),
                        if granted {
                            i18n::tr("settings-permission-granted")
                        } else {
                            i18n::tr("settings-permission-denied")
                        }
                    ));
                }
                ui.weak(i18n::tr("settings-permission-hint"));
                if cfg!(windows) {
                    ui.weak(i18n::tr("settings-permission-windows"));
                }
                ui.weak(i18n::tr("settings-permission-restart"));
                if ui.button(i18n::tr("settings-open-privacy")).clicked()
                    && let Err(e) = permissions::open_settings()
                {
                    self.status = e;
                }
            });
        egui::CollapsingHeader::new(i18n::tr("settings-section-answer"))
            .id_salt("settings-answer")
            .default_open(true)
            .show(ui, |ui| {
                egui::ComboBox::from_id_salt("settings-protocol")
                    .selected_text(i18n::protocol(self.draft.provider.protocol))
                    .show_ui(ui, |ui| {
                        for p in [
                            Protocol::ChatCompletions,
                            Protocol::Responses,
                            Protocol::Anthropic,
                        ] {
                            ui.selectable_value(
                                &mut self.draft.provider.protocol,
                                p,
                                i18n::protocol(p),
                            );
                        }
                    });
                ui.label(i18n::tr("settings-base-url"));
                ui.text_edit_singleline(&mut self.draft.provider.base_url);
                ui.label(i18n::tr("settings-model"));
                ui.text_edit_singleline(&mut self.draft.provider.model);
                ui.add(
                    egui::Slider::new(&mut self.draft.provider.max_output_tokens, 128..=16384)
                        .text(i18n::tr("settings-max-tokens")),
                );
                ui.checkbox(&mut self.draft.provider.vision, i18n::tr("settings-vision"));
                ui.label(i18n::tr("settings-api-key"));
                ui.add(egui::TextEdit::singleline(&mut self.secret_draft).password(true));
                ui.horizontal(|ui| {
                    if ui.button(i18n::tr("settings-test")).clicked() {
                        self.test_connection(ui.ctx());
                    }
                    if self.test_rx.is_some()
                        && ui.button(i18n::tr("settings-test-cancel")).clicked()
                    {
                        if let Some(cancel) = self.test_cancel.take() {
                            cancel.cancel();
                        }
                        self.test_rx = None;
                        self.test_status = i18n::tr("settings-test-cancelled");
                    }
                });
                ui.label(&self.test_status);
            });
        egui::CollapsingHeader::new(i18n::tr("settings-section-translation"))
            .id_salt("settings-translation")
            .show(ui, |ui| {
                ui.label(i18n::tr("settings-default-target"));
                ui.text_edit_singleline(&mut self.draft.target_language);
                ui.label(i18n::tr("settings-chinese-target"));
                ui.text_edit_singleline(&mut self.draft.chinese_target);
                egui::ComboBox::from_id_salt("translation-style")
                    .selected_text(i18n::tr(match self.draft.translation_style.as_str() {
                        "literal" => "settings-style-literal",
                        "technical" => "settings-style-technical",
                        _ => "settings-style-natural",
                    }))
                    .show_ui(ui, |ui| {
                        for (value, key) in [
                            ("natural", "settings-style-natural"),
                            ("literal", "settings-style-literal"),
                            ("technical", "settings-style-technical"),
                        ] {
                            ui.selectable_value(
                                &mut self.draft.translation_style,
                                value.into(),
                                i18n::tr(key),
                            );
                        }
                    });
            });
        egui::CollapsingHeader::new(i18n::tr("settings-section-decision"))
            .id_salt("settings-section-decision")
            .show(ui, |ui| {
                ui.checkbox(
                    &mut self.draft.decision.enabled,
                    i18n::tr("settings-decision-enable"),
                );
                ui.label(i18n::tr("settings-decision-endpoint-hint"));
                ui.text_edit_singleline(&mut self.draft.decision.endpoint);
                ui.label(i18n::tr("settings-decision-model"));
                ui.text_edit_singleline(&mut self.draft.decision.model);
                ui.label(i18n::tr("settings-decision-key"));
                ui.add(egui::TextEdit::singleline(&mut self.decision_secret_draft).password(true));
                ui.add(
                    egui::Slider::new(&mut self.draft.decision.min_confidence, 0.0..=1.0)
                        .text(i18n::tr("settings-decision-threshold")),
                );
                ui.add(
                    egui::Slider::new(&mut self.draft.decision.timeout_ms, 100..=10000)
                        .text(i18n::tr("settings-decision-timeout")),
                );
                ui.weak(i18n::tr("settings-decision-note"));
            });
        egui::CollapsingHeader::new(i18n::tr("settings-section-general"))
            .id_salt("settings-general")
            .show(ui, |ui| {
                ui.checkbox(&mut self.draft.smart_mode, i18n::tr("settings-smart-mode"));
                ui.checkbox(
                    &mut self.draft.ocr_auto_query,
                    i18n::tr("settings-ocr-auto"),
                );
                ui.weak(i18n::tr("settings-ocr-auto-hint"));
                ui.checkbox(
                    &mut self.draft.hide_on_blur,
                    i18n::tr("settings-hide-on-blur"),
                );
            });
    }
    fn settings_footer(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if design::primary(ui, i18n::tr("settings-save")).clicked() {
                let result = self.draft.validate().map_err(i18n::tr).and_then(|()| {
                    let blank: global_hotkey::hotkey::HotKey = self
                        .draft
                        .blank_hotkey
                        .parse::<global_hotkey::hotkey::HotKey>()
                        .map_err(|e| {
                            i18n::format("error-hotkey-blank", &[("detail", &e.to_string())])
                        })?;
                    let screenshot: global_hotkey::hotkey::HotKey = self
                        .draft
                        .screenshot_hotkey
                        .parse::<global_hotkey::hotkey::HotKey>()
                        .map_err(|e| {
                            i18n::format("error-hotkey-screenshot", &[("detail", &e.to_string())])
                        })?;
                    if blank.id() == screenshot.id() {
                        return Err(i18n::tr("error-hotkey-duplicate"));
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
                        i18n::set_language(&self.config.ui_language);
                        if let Some(desktop) = &self.desktop {
                            desktop.refresh_language();
                        }
                        self.locale = i18n::I18n::new(&self.config.ui_language);
                        apply_appearance(ui.ctx(), &self.config);
                        self.secret_draft.clear();
                        self.decision_secret_draft.clear();
                        self.settings = false;
                        self.status = if shortcuts_changed {
                            i18n::tr("status-saved-restart")
                        } else {
                            i18n::tr("status-saved")
                        };
                    }
                    Err(e) => self.status = e,
                }
            }
            if ui.button(i18n::tr("settings-cancel")).clicked() {
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
        if let Some(path) = &self.preview {
            self.preview_frames += 1;
            if self.preview_frames == 3 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            let images = ctx.input(|i| {
                i.events
                    .iter()
                    .filter_map(|event| {
                        if let egui::Event::Screenshot { image, .. } = event {
                            Some(image.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            });
            for image in images {
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    path,
                    &bytes,
                    image.size[0] as u32,
                    image.size[1] as u32,
                    image::ColorType::Rgba8,
                )
                .expect("save UI preview");
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint();
        }
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
                    self.test_status = i18n::tr("settings-test-ok");
                    self.test_rx = None;
                    self.test_cancel = None;
                }
                Event::Failed(e) => {
                    self.test_status =
                        i18n::format("settings-test-failed", &[("error", &i18n::diagnostic(&e))]);
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
                        i18n::tr("status-paused")
                    } else {
                        i18n::tr("status-resumed")
                    };
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
                    self.status =
                        i18n::format("status-capture-failed", &[("error", &i18n::diagnostic(&e))]);
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
                        self.status = i18n::tr("status-ocr-local-only");
                    }
                }
                Ok(_) => self.status = i18n::tr("status-ocr-empty"),
                Err(e) => {
                    self.status =
                        i18n::format("status-ocr-failed", &[("error", &i18n::diagnostic(&e))])
                }
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
                                i18n::tr("snip-hint"),
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
                    self.status = i18n::tr("status-ocr-running");
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
                        system.content = task.styled_instruction(
                            peek_core::effective_target(&target, &self.target_override),
                            &self.config.translation_style,
                        );
                    }
                }
                Event::Text(text) => {
                    if self.answer.len().saturating_add(text.len()) > peek_core::MAX_OUTPUT_BYTES {
                        self.stop();
                        self.status = i18n::tr("status-output-too-long");
                        break;
                    }
                    self.answer.push_str(&text);
                }
                Event::Done => {
                    self.busy = false;
                    self.status = i18n::tr("status-done");
                    self.messages.push(Message {
                        role: "assistant".into(),
                        content: self.answer.clone(),
                    });
                    if peek_core::bound_history(&mut self.messages) {
                        self.status = i18n::tr("status-done-trimmed");
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
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(root.visuals().panel_fill)
                    .inner_margin(20),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    let title = ui.heading(self.locale.text("app-name"));
                    if title.interact(egui::Sense::drag()).drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if design::icon(ui, egui_phosphor::regular::X, i18n::tr("header-close"))
                            .clicked()
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if design::icon(
                            ui,
                            if self.settings {
                                egui_phosphor::regular::ARROW_LEFT
                            } else {
                                egui_phosphor::regular::GEAR
                            },
                            i18n::tr(if self.settings {
                                "header-back"
                            } else {
                                "header-settings"
                            }),
                        )
                        .clicked()
                        {
                            if self.settings {
                                self.settings = false;
                            } else {
                                self.draft = self.config.clone();
                                self.settings = true;
                            }
                        }
                        if design::icon(
                            ui,
                            egui_phosphor::regular::PUSH_PIN,
                            i18n::tr(if self.pinned {
                                "header-unpin"
                            } else {
                                "header-pin"
                            }),
                        )
                        .clicked()
                        {
                            self.pinned = !self.pinned;
                        }
                    });
                });
                ui.separator();
                if self.settings {
                    let content_height = (ui.available_height() - 92.0).max(80.0);
                    egui::ScrollArea::vertical()
                        .max_height(content_height)
                        .show(ui, |ui| {
                            design::card(ui).show(ui, |ui| {
                                ui.set_min_width((ui.available_width() - 2.0).max(100.0));
                                ui.spacing_mut().item_spacing.y = 7.0;
                                self.settings_ui(ui);
                            });
                        });
                    ui.separator();
                    self.settings_footer(ui);
                } else {
                    design::card(ui).show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.input)
                                .desired_width(f32::INFINITY)
                                .desired_rows(3)
                                .frame(egui::Frame::NONE)
                                .hint_text(i18n::tr("query-placeholder")),
                        );
                    });
                    ui.horizontal_wrapped(|ui| {
                        egui::ComboBox::from_id_salt("task")
                            .selected_text(self.locale.task(self.task))
                            .show_ui(ui, |ui| {
                                for task in Task::ALL {
                                    if ui
                                        .selectable_value(
                                            &mut self.task,
                                            task,
                                            self.locale.task(task),
                                        )
                                        .changed()
                                    {
                                        self.manual_task = true;
                                    }
                                }
                            });
                        if self.manual_task && ui.button(i18n::tr("query-restore-smart")).clicked()
                        {
                            self.manual_task = false;
                        }
                        if design::primary(ui, i18n::tr("query-run")).clicked()
                            || ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter))
                        {
                            self.query(&ctx, false);
                        }
                        if self.busy && ui.button(i18n::tr("query-stop")).clicked() {
                            self.stop();
                            self.status = i18n::tr("status-stopped");
                        }
                        if design::icon(ui, egui_phosphor::regular::COPY, i18n::tr("query-copy"))
                            .clicked()
                        {
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
                        if design::icon(
                            ui,
                            egui_phosphor::regular::TEXT_T,
                            i18n::tr("query-copy-source"),
                        )
                        .clicked()
                            && !self.input.is_empty()
                        {
                            ctx.copy_text(self.input.clone());
                            self.status = i18n::tr("status-copied-source");
                        }
                        if design::icon(ui, egui_phosphor::regular::TRASH, i18n::tr("query-clear"))
                            .clicked()
                        {
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
                            self.status = i18n::tr("status-cleared");
                        }
                    });
                    if self.screenshot_image.is_some() {
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .add_enabled(
                                    self.config.provider.vision && !self.busy,
                                    egui::Button::new(i18n::tr("snip-explain")),
                                )
                                .clicked()
                            {
                                if self.input.trim().is_empty() {
                                    self.input = i18n::tr("snip-explain-default");
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
                                    egui::Button::new(i18n::tr("snip-decide")),
                                )
                                .clicked()
                            {
                                if self.input.trim().is_empty() {
                                    self.input = i18n::tr("snip-decide-default");
                                }
                                self.manual_task = false;
                                self.decide_image = true;
                                self.query(&ctx, false);
                            }
                            if ui.button(i18n::tr("snip-discard")).clicked() {
                                self.screenshot_image = None;
                            }
                        });
                        ui.weak(i18n::format(
                            "snip-decide-target",
                            &[("endpoint", &self.config.decision.endpoint)],
                        ));
                        ui.weak(i18n::format(
                            "snip-explain-target",
                            &[("endpoint", &self.config.provider.base_url)],
                        ));
                    }
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("query-target")
                            .selected_text(if self.target_override.is_empty() {
                                i18n::tr("query-target-auto")
                            } else {
                                self.target_override.clone()
                            })
                            .show_ui(ui, |ui| {
                                for (value, label) in [
                                    ("", i18n::tr("query-target-auto")),
                                    ("Chinese", i18n::tr("language-zh")),
                                    ("English", i18n::tr("language-en")),
                                    ("Japanese", i18n::tr("language-ja")),
                                ] {
                                    ui.selectable_value(
                                        &mut self.target_override,
                                        value.into(),
                                        label,
                                    );
                                }
                            });
                        ui.add(
                            egui::TextEdit::singleline(&mut self.target_override)
                                .desired_width(100.0)
                                .hint_text(i18n::tr("query-target-custom")),
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
                                    ui.weak(i18n::tr("dict-badge"));
                                });
                                ui.add(
                                    egui::Label::new(&entry.translation).selectable(true).wrap(),
                                );
                                let forms = entry.forms();
                                if !forms.is_empty() {
                                    let line: Vec<String> = forms
                                        .iter()
                                        .map(|(k, v)| format!("{} {v}", i18n::tr(k)))
                                        .collect();
                                    ui.weak(line.join("  ·  "));
                                }
                                if let Some(lemma) = entry.lemma() {
                                    ui.weak(i18n::format("dict-lemma", &[("lemma", lemma)]));
                                }
                                ui.separator();
                            }
                            if !self.answer.is_empty() {
                                design::card(ui).show(ui, |ui| {
                                    ui.weak(i18n::tr("query-section-answer"));
                                    ui.add(egui::Label::new(&self.answer).selectable(true).wrap());
                                });
                            } else if self.dict_entry.is_none() && !self.busy {
                                ui.add_space(24.0);
                                ui.vertical_centered(|ui| {
                                    ui.heading(i18n::tr("query-empty-title"));
                                    ui.label(i18n::format(
                                        "query-empty-hint",
                                        &[(
                                            "submit",
                                            if cfg!(target_os = "macos") {
                                                "⌘ Enter"
                                            } else {
                                                "Ctrl Enter"
                                            },
                                        )],
                                    ));
                                });
                                ui.add_space(24.0);
                            }
                        });
                    ui.separator();
                    ui.horizontal(|ui| {
                        let width = (ui.available_width() - 84.0).max(80.0);
                        let input = ui.add_sized(
                            [width, 36.0],
                            egui::TextEdit::singleline(&mut self.followup)
                                .hint_text(i18n::tr("query-followup-placeholder")),
                        );
                        let enter = input.lost_focus()
                            && ctx
                                .input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.command);
                        if (ui
                            .add_enabled(
                                !self.messages.is_empty() && !self.busy,
                                egui::Button::new(i18n::tr("query-send")),
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
                    if self.ocr_rx.is_some() && ui.button(i18n::tr("snip-cancel-ocr")).clicked() {
                        self.stop();
                        self.screenshot_image = None;
                        self.status = i18n::tr("status-ocr-cancelled");
                    }
                });
            });
    }
}
fn main() -> eframe::Result {
    // An isolated preview renders offscreen and must not contend for the single-instance
    // lock, otherwise it silently exits while the user has the app open.
    let _instance = if std::env::var("PEEK_UI_PREVIEW").is_ok() {
        None
    } else {
        match store::config_path().and_then(|path| {
            instance::acquire(&path.with_file_name("instance.lock")).map_err(|e| e.to_string())
        }) {
            Ok(Some(lock)) => Some(lock),
            Ok(None) => return Ok(()),
            Err(e) => {
                eprintln!("{}", i18n::format("error-instance", &[("detail", &e)]));
                return Ok(());
            }
        }
    };
    eframe::run_native(
        &i18n::tr("app-name"),
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
