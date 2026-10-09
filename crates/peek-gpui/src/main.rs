//! Crant Peek — GPUI shell.
//!
//! Tray-resident, floating-first: no main window, the peek panel is created
//! hidden at startup and summoned by a global hotkey. This crate replaces the
//! egui/eframe UI layer (`peek-app`) while `peek-core`, `peek-network` and
//! `peek-dict` stay unchanged; shared config/keychain/i18n live in
//! `peek-runtime`.
//!
//! Ordering note: the tray icon and the hotkey manager are installed *inside*
//! the `Application::run` callback, not before it. Creating them earlier makes
//! muda build a plain `NSApplication`, after which `MacPlatform::run` panics
//! with "Ivar platform not found on class NSApplication". Measured in
//! `spikes/gpui-kit/lifecycle-probe-evidence.txt`.
//!
//! Modes:
//!   PEEK_SELFTEST=1   show the panel, verify visibility, hide it, quit

mod fonts;
mod native_window;
mod snip;

use std::time::Duration;

use gpui_kit::base::{IndexPath, Root, StyledExt as _};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tray_icon::Icon as TrayIconImage;
use tray_icon::TrayIconBuilder;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};

use peek_core::{Config, Message, Task, effective_target, local_route};
use peek_network::{Client, Event};
use peek_runtime::action::Action;
use peek_runtime::{i18n, store};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Poll interval for the tray/hotkey channels. Both crates deliver events on
/// their own channels rather than through GPUI, so they are drained on a timer.
const POLL: Duration = Duration::from_millis(60);
/// How often streamed answer text is moved from the network channel into the view.
const DRAIN: Duration = Duration::from_millis(30);

/// Grace period after a programmatic show during which focus loss is ignored.
///
/// `activate_window()` is followed by an activation notification; if the app
/// is not yet frontmost (e.g. it was summoned while another app still holds
/// focus) `is_window_active()` reads false at that moment. Without this grace
/// period the panel would hide itself the instant it appeared — measured as a
/// flaky `visible after show: false` in `PEEK_SELFTEST`.
const FOCUS_GRACE: Duration = Duration::from_millis(500);

thread_local! {
    /// When the panel was last shown. Everything here runs on GPUI's main
    /// thread, so a thread-local is enough.
    static SHOWN_AT: std::cell::Cell<Option<std::time::Instant>> =
        const { std::cell::Cell::new(None) };
}

/// Shows the panel and records when, so focus loss is ignored briefly.
fn show_panel(window: &mut Window) {
    native_window::show(window);
    SHOWN_AT.with(|cell| cell.set(Some(std::time::Instant::now())));
}

/// Whether the panel was shown recently enough to ignore a focus loss.
fn shown_recently() -> bool {
    SHOWN_AT.with(|cell| {
        cell.get()
            .is_some_and(|shown| shown.elapsed() < FOCUS_GRACE)
    })
}

/// Task order in the dropdown, paired with their localisation keys.
const TASK_ORDER: &[Task] = &[
    Task::Translate,
    Task::Define,
    Task::ExplainCode,
    Task::ExplainError,
    Task::Explain,
];
const TASK_KEYS: &[&str] = &[
    "task-translate",
    "task-define",
    "task-explain-code",
    "task-explain-error",
    "task-explain",
];

struct Peek {
    set: &'static fonts::FontSet,
    /// Kept alive so the activation observer stays subscribed.
    _activation: Subscription,

    input: Entity<TextareaState>,
    follow_up: Entity<InputState>,
    tasks: Entity<SelectState<Vec<SharedString>>>,

    answer: String,
    /// Selection text to place in the input box on the next frame. The hook
    /// reports off the UI thread, where no `Window` is available to set it.
    pending_input: Option<String>,
    /// The first query's text. A follow-up re-derives its target language from
    /// this rather than from the follow-up sentence.
    original_query: String,
    status: String,
    busy: bool,
    dictionary_note: String,

    /// Permission name (already an i18n key) and whether it is granted. The
    /// double-tap Ctrl hook and selection reading need Accessibility and Input
    /// Monitoring; without them they fail silently, so the panel says so
    /// instead of appearing broken.
    permissions: Vec<(&'static str, bool)>,
    /// Whether the settings page is showing.
    settings: bool,
    config: Config,
    client: Client,
    runtime: tokio::runtime::Runtime,
    messages: Vec<Message>,
    receiver: Option<mpsc::Receiver<Event>>,
    cancel: Option<CancellationToken>,
}

impl Peek {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Hiding on focus loss. GPUI's callback carries no activation state, so
        // the state is read back from the window itself.
        let activation = cx.observe_window_activation(window, |_this, window, _cx| {
            if !window.is_window_active() && !shown_recently() {
                native_window::hide(window);
            }
        });

        let input = cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 5));
        let follow_up = cx.new(|cx| {
            InputState::new(window, cx).placeholder(i18n::tr("query-followup-placeholder"))
        });

        let config = store::load().unwrap_or_default();
        i18n::set_language(&config.ui_language);
        let double_ctrl_ms = config.double_ctrl_ms;

        let items: Vec<SharedString> = TASK_KEYS.iter().map(|key| i18n::tr(key).into()).collect();
        let tasks = cx.new(|cx| SelectState::new(items, Some(IndexPath::new(0)), window, cx));

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");

        let this = Self {
            set: fonts::initial(),
            _activation: activation,
            input,
            follow_up,
            tasks,
            answer: String::new(),
            pending_input: None,
            original_query: String::new(),
            status: String::new(),
            busy: false,
            dictionary_note: String::new(),
            permissions: peek_runtime::permissions::status(),
            settings: peek_runtime::prefs::load().settings_open,
            config,
            client: Client::default(),
            runtime,
            messages: Vec::new(),
            receiver: None,
            cancel: None,
        };

        // Double-tap Ctrl: the hook and selection reader live in
        // `peek-runtime` and are toolkit agnostic. They report through a
        // channel this view drains on a timer, so no UI context crosses the
        // platform thread boundary.
        let (action_tx, action_rx) = std::sync::mpsc::channel::<Action>();
        let selftest_wake = action_tx.clone();
        let snip_test_wake = action_tx.clone();
        // Kept for the screenshot overlay, which reports recognised text back
        // through the same channel.
        let snip_tx = action_tx.clone();
        peek_runtime::selection::listen(
            double_ctrl_ms,
            action_tx,
            // This view polls the channel, so the wake callback is a no-op.
            std::sync::Arc::new(|| {}),
        );
        let panel_handle = window.window_handle();
        let snip_selftest = std::env::var("PEEK_SNIP_SELFTEST").is_ok();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                while let Ok(action) = action_rx.try_recv() {
                    match action {
                        Action::Selection(text) => {
                            println!("[hook] selection action: {} chars", text.chars().count());
                            // Show first so the panel is on screen while the
                            // answer streams in.
                            let _ =
                                cx.update_window(panel_handle, |_, window, _| show_panel(window));
                            this.update(cx, |peek, cx| {
                                peek.pending_input = Some(text.clone());
                                peek.begin_turn(text, false, cx);
                            })
                            .ok();
                        }
                        Action::Blank => {
                            let _ =
                                cx.update_window(panel_handle, |_, window, _| show_panel(window));
                        }
                        Action::Quit => {
                            cx.update(quit_now);
                            return;
                        }
                        Action::Screenshot => {
                            // The overlay captures on release, so this only
                            // opens the selection window.
                            let tx = snip_tx.clone();
                            let opened = cx.update(|app| snip::open(app, tx));
                            match opened {
                                Some((handle, bounds)) => {
                                    println!(
                                        "[snip] overlay open: origin=({:.0},{:.0}) size={:.0}x{:.0}",
                                        bounds.min[0],
                                        bounds.min[1],
                                        bounds.width(),
                                        bounds.height()
                                    );
                                    if snip_selftest {
                                        // No mouse in a self test, so close it
                                        // on a timer instead of leaving a
                                        // full-screen overlay up.
                                        cx.update(|app| {
                                            app.spawn(async move |cx| {
                                                cx.background_executor()
                                                    .timer(Duration::from_millis(1200))
                                                    .await;
                                                let closed = cx
                                                    .update_window(handle, |_, window, _| {
                                                        window.remove_window()
                                                    })
                                                    .is_ok();
                                                println!("[snip] overlay closed: {closed}");
                                                cx.update(quit_now);
                                            })
                                        })
                                        .detach();
                                    }
                                }
                                None => println!("[snip] overlay NOT opened (no monitor bounds)"),
                            }
                        }
                        Action::Recognized(text) => {
                            let _ = cx
                                .update_window(panel_handle, |_, window, _| show_panel(window));
                            this.update(cx, |peek, cx| {
                                peek.pending_input = Some(text.clone());
                                peek.begin_turn(text, false, cx);
                            })
                            .ok();
                        }
                        // Settings and pause are not ported yet.
                        Action::Settings | Action::TogglePause => {}
                    }
                }
            }
        })
        .detach();

        // `PEEK_SELFTEST` also drives one query turn, so the whole pipeline
        // (config load → credential lookup → localised status → view notify)
        // is exercised without a click. It stops at the network call when no
        // credential is configured, which is reported as-is.
        // `PEEK_SNIP_SELFTEST` drives the overlay through the same action
        // channel the hotkey uses. It takes no screenshot, so it needs no
        // permission, and a watchdog guarantees the process can never leave a
        // full-screen window behind.
        if std::env::var("PEEK_SNIP_SELFTEST").is_ok() {
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_secs(10));
                eprintln!("[snip] watchdog fired — exiting");
                std::process::exit(0);
            });
            for line in peek_runtime::capture::monitor_report() {
                eprintln!("[capture] {line}");
            }
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(500));
                let _ = snip_test_wake.send(Action::Screenshot);
            });
        }

        if std::env::var("PEEK_SELFTEST").is_ok() {
            // Feed the hook's own channel, so the selection path (show + query)
            // is exercised without synthesising a real double-tap Ctrl.
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(700));
                let _ = selftest_wake.send(Action::Selection("ephemeral".into()));
            });
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(900))
                    .await;
                this.update(cx, |peek, cx| {
                    peek.begin_turn("ephemeral".into(), false, cx);
                })
                .ok();
                cx.background_executor()
                    .timer(Duration::from_millis(1600))
                    .await;
                this.update(cx, |peek, cx| {
                    println!("[selftest] query status : {:?}", peek.status);
                    println!("[selftest] font set     : {}", peek.set.id);
                    println!("[selftest] answer bytes : {}", peek.answer.len());
                    println!("[selftest] busy         : {}", peek.busy);
                    // The dictionary path needs no credentials, so it is checked
                    // for real: `ephemeral` is an ordinary ECDICT headword.
                    println!(
                        "[selftest] dict note    : {} bytes",
                        peek.dictionary_note.len()
                    );
                    println!(
                        "[selftest] denied perms : {:?}",
                        peek.permissions
                            .iter()
                            .filter(|(_, granted)| !granted)
                            .map(|(name, _)| *name)
                            .collect::<Vec<_>>()
                    );
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }

        this
    }

    /// The task chosen in the dropdown.
    fn selected_task(&self, cx: &App) -> Task {
        self.tasks
            .read(cx)
            .selected_index(cx)
            .and_then(|index| TASK_ORDER.get(index.row))
            .copied()
            .unwrap_or(Task::Translate)
    }

    /// Offline dictionary first: instant, and independent of any API key.
    fn lookup_word(&mut self, text: &str) -> Option<peek_dict::Entry> {
        if text.chars().count() > 40 || text.split_whitespace().count() != 1 {
            return None;
        }
        store::dictionary_candidates()
            .into_iter()
            .find_map(|path| peek_dict::Dict::open(&path).ok())
            .and_then(|dict| dict.lookup(text))
    }

    /// Cancels any in-flight turn and drops the unanswered tail.
    fn stop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel();
        }
        self.receiver = None;
        self.busy = false;
        peek_core::discard_pending_turn(&mut self.messages);
    }

    /// Switches the bundled font set, applies it to the theme and persists it.
    fn choose_font_set(&mut self, id: &'static str, cx: &mut Context<Self>) {
        let set = fonts::by_id(id);
        self.set = set;
        fonts::apply(cx, set);
        self.persist_prefs();
        cx.notify();
    }

    /// Writes the UI-only preferences. Failures are cosmetic, so they are
    /// reported but never block the interaction.
    fn persist_prefs(&self) {
        let prefs = peek_runtime::prefs::UiPrefs {
            font_set: self.set.id.to_string(),
            settings_open: self.settings,
        };
        if let Err(err) = peek_runtime::prefs::save(&prefs) {
            eprintln!("saving ui prefs failed: {err}");
        }
    }

    /// Reads the input box and starts a turn, clearing it only if one began.
    fn start_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim().to_owned();
        if self.begin_turn(text, false, cx) {
            self.input
                .update(cx, |state, cx| state.set_value("", window, cx));
        }
    }

    /// Sends the follow-up box, continuing the current conversation.
    fn send_follow_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.follow_up.read(cx).value().trim().to_owned();
        if self.begin_turn(text, true, cx) {
            self.follow_up
                .update(cx, |state, cx| state.set_value("", window, cx));
        }
    }

    /// Builds the prompt and starts the network turn. Returns whether a turn
    /// actually started. Split out of `start_query` so `PEEK_SELFTEST` can
    /// drive the whole pipeline without a window or a click.
    fn begin_turn(&mut self, text: String, followup: bool, cx: &mut Context<Self>) -> bool {
        if text.is_empty() {
            return false;
        }
        if text.len() > peek_core::MAX_INPUT_BYTES {
            self.status = i18n::tr("status-input-too-long");
            cx.notify();
            return false;
        }
        // A follow-up needs an existing conversation to continue.
        if followup && self.messages.is_empty() {
            return false;
        }

        self.stop();
        self.answer.clear();
        if !followup {
            self.dictionary_note.clear();
            if let Some(entry) = self.lookup_word(&text) {
                self.dictionary_note = dictionary_text(&entry);
            }
        }

        let key = match store::secret(&self.config.provider.credential_id) {
            Ok(key) => key,
            Err(err) => {
                self.status = err;
                cx.notify();
                return false;
            }
        };
        if self.config.provider.model.trim().is_empty() {
            self.status = i18n::tr("status-model-missing");
            cx.notify();
            return false;
        }

        // A follow-up is routed from the original query, not from its own
        // sentence, so the target language does not drift mid-conversation.
        let routed_text = if followup {
            self.original_query.as_str()
        } else {
            text.as_str()
        };
        let route = local_route(
            routed_text,
            &self.config.target_language,
            &self.config.chinese_target,
        );
        let task = if self.config.smart_mode {
            route.task
        } else {
            self.selected_task(cx)
        };
        let target = effective_target(&route.target, "").to_owned();
        if !followup {
            self.original_query = text.clone();
            self.messages = vec![Message {
                role: "system".into(),
                content: task.styled_instruction(&target, &self.config.translation_style),
            }];
        } else if let Some(system) = self.messages.first_mut() {
            // Re-issue the instruction: the follow-up may be another language.
            system.content = task.styled_instruction(&target, &self.config.translation_style);
        }
        self.messages.push(Message {
            role: "user".into(),
            content: text.clone(),
        });
        peek_core::bound_history(&mut self.messages);

        self.status = i18n::tr("status-generating");
        self.busy = true;
        let (tx, rx) = mpsc::channel(128);
        self.receiver = Some(rx);
        let cancel = CancellationToken::new();
        self.cancel = Some(cancel.clone());

        let provider = self.config.provider.clone();
        let messages = self.messages.clone();
        let client = self.client.clone();
        let ui_language = self.config.ui_language.clone();
        self.runtime.spawn(async move {
            let locale = i18n::I18n::new(&ui_language);
            let (net_tx, mut net_rx) = mpsc::channel(32);
            let worker = tokio::spawn(async move {
                client
                    .stream(&provider, &key, &messages, net_tx, cancel)
                    .await
            });
            while let Some(event) = net_rx.recv().await {
                // Error payloads travel as stable codes; localise them here,
                // off the UI thread.
                let event = match event {
                    Event::Failed(code) => Event::Failed(locale.text(&code)),
                    other => other,
                };
                if tx.send(event).await.is_err() {
                    break;
                }
            }
            let failure = match worker.await {
                Ok(Ok(())) => None,
                Ok(Err(err)) => Some(locale.network_error(&err)),
                Err(_) => Some(locale.text("status-worker-crashed")),
            };
            if let Some(failure) = failure {
                let _ = tx.send(Event::Failed(failure)).await;
            }
        });

        // Move streamed events into the view on the GPUI side: tokio owns the
        // network task, and an `Entity` cannot cross into it.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(DRAIN).await;
                let finished = this
                    .update(cx, |peek, cx| {
                        let mut events = Vec::new();
                        let mut finished = false;
                        match peek.receiver.as_mut() {
                            Some(receiver) => loop {
                                match receiver.try_recv() {
                                    Ok(event) => events.push(event),
                                    Err(mpsc::error::TryRecvError::Empty) => break,
                                    Err(mpsc::error::TryRecvError::Disconnected) => {
                                        finished = true;
                                        break;
                                    }
                                }
                            },
                            None => finished = true,
                        }
                        if !events.is_empty() {
                            peek.apply_events(events);
                            cx.notify();
                        }
                        if finished {
                            peek.receiver = None;
                            peek.busy = false;
                        }
                        finished
                    })
                    .unwrap_or(true);
                if finished {
                    return;
                }
            }
        })
        .detach();
        true
    }

    fn apply_events(&mut self, events: Vec<Event>) {
        for event in events {
            match event {
                Event::Route { note, .. } => self.status = note,
                Event::Text(delta) => {
                    if self.answer.len() + delta.len() > peek_core::MAX_OUTPUT_BYTES {
                        self.status = i18n::tr("status-output-too-long");
                        self.stop();
                        break;
                    }
                    self.answer.push_str(&delta);
                }
                Event::Done => {
                    self.status = i18n::tr("status-done");
                    self.busy = false;
                }
                Event::Failed(message) => {
                    self.status = message;
                    self.busy = false;
                }
            }
        }
    }
}

impl Render for Peek {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A selection arrives off the UI thread, so it is applied here where a
        // `Window` is available.
        if let Some(text) = self.pending_input.take() {
            self.input
                .update(cx, |state, cx| state.set_value(text, window, cx));
        }
        if self.settings {
            self.settings_page(cx).into_any_element()
        } else {
            self.query_page(cx).into_any_element()
        }
    }
}

impl Peek {
    fn query_page(&self, cx: &Context<Self>) -> AnyElement {
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
                        Button::new("open-settings")
                            .secondary()
                            .rounded(px(10.))
                            .h(px(28.))
                            .px(px(8.))
                            .child(
                                Icon::new(gpui_kit::assets::IconName::Settings)
                                    .size(px(16.))
                                    .text_color(muted),
                            )
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                this.settings = true;
                                this.persist_prefs();
                                cx.notify();
                            })),
                    )
                    .child(
                        Icon::new(gpui_kit::assets::IconName::Close)
                            .size(px(16.))
                            .text_color(muted),
                    ),
            )
            // Source input.
            .child(
                div()
                    .w_full()
                    .border_1()
                    .border_color(border)
                    .rounded(px(16.))
                    .p(px(14.))
                    .child(
                        Textarea::new(&self.input)
                            .w_full()
                            .h(px(78.))
                            .appearance(false)
                            .bordered(false),
                    ),
            )
            // Task dropdown plus the primary action.
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        Select::new(&self.tasks)
                            .id("task")
                            .w(px(170.))
                            .rounded(px(10.)),
                    )
                    .child(
                        Button::new("look-up")
                            .primary()
                            .rounded(px(999.))
                            .h(px(34.))
                            .px(px(16.))
                            .label(i18n::tr("query-run"))
                            .loading(self.busy)
                            .on_click(cx.listener(|this, _event, window, cx| {
                                this.start_query(window, cx);
                            })),
                    )
                    .when(self.busy, |this| {
                        this.child(
                            Button::new("stop")
                                .secondary()
                                .rounded(px(999.))
                                .h(px(34.))
                                .px(px(16.))
                                .label(i18n::tr("query-stop"))
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.stop();
                                    this.status = i18n::tr("status-stopped");
                                    cx.notify();
                                })),
                        )
                    }),
            )
            // Offline dictionary result, when the input was a single word.
            .when(!self.dictionary_note.is_empty(), |this| {
                this.child(
                    div()
                        .w_full()
                        .border_1()
                        .border_color(border)
                        .rounded(px(14.))
                        .p(px(14.))
                        .font_family(set.sc)
                        .text_size(px(13.))
                        .child(self.dictionary_note.clone()),
                )
            })
            // Markdown answer.
            .child(
                div()
                    .id("answer")
                    .w_full()
                    .flex_1()
                    .border_1()
                    .border_color(border)
                    .rounded(px(16.))
                    .p(px(16.))
                    .overflow_y_scroll()
                    .when(self.answer.is_empty(), |this| {
                        this.child(
                            div()
                                .font_family(set.latin)
                                .text_size(px(13.))
                                .text_color(muted)
                                .child(i18n::tr("query-empty-title")),
                        )
                    })
                    .when(!self.answer.is_empty(), |this| {
                        this.child(TextView::markdown("answer", self.answer.clone()))
                    }),
            )
            .when(
                self.permissions.iter().any(|(_, granted)| !granted),
                |this| {
                    this.child(
                        div()
                            .w_full()
                            .font_family(set.latin)
                            .text_size(px(11.))
                            .text_color(muted)
                            .child(format!(
                                "{}: {}",
                                i18n::tr("settings-permission-denied"),
                                // `permissions::status()` already yields localisation keys.
                                self.permissions
                                    .iter()
                                    .filter(|(_, granted)| !granted)
                                    .map(|(key, _)| i18n::tr(key))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )),
                    )
                },
            )
            // Follow-up turn, continuing the same conversation.
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        Input::new(&self.follow_up)
                            .id("follow-up")
                            .flex_1()
                            .rounded(px(10.)),
                    )
                    .child(
                        Button::new("send")
                            .rounded(px(999.))
                            .h(px(34.))
                            .px(px(16.))
                            .label(i18n::tr("query-send"))
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _event, window, cx| {
                                this.send_follow_up(window, cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap(px(8.))
                    .font_family(set.latin)
                    .text_size(px(11.))
                    .text_color(muted)
                    .child(self.status.clone())
                    .child(div().flex_1())
                    // Also the label the settings panel will offer; shown here so
                    // the active set is visible while the panel is still to come.
                    .child(set.name),
            )
            .into_any_element()
    }

    /// Settings page. Only the font set is live so far: it is the one item the
    /// migration is required to expose as a choice, and it is verifiable from
    /// an offscreen render.
    fn settings_page(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let set = self.set;
        let fg = theme.foreground;
        let bg = theme.background;
        let muted = theme.muted_foreground;

        v_flex()
            .size_full()
            .bg(bg)
            .text_color(fg)
            .p(px(20.))
            .gap(px(14.))
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
                        Button::new("settings-back")
                            .secondary()
                            .rounded(px(10.))
                            .h(px(28.))
                            .px(px(10.))
                            .label(i18n::tr("header-back"))
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                this.settings = false;
                                this.persist_prefs();
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .font_family(set.latin)
                            .text_size(px(15.))
                            .font_semibold()
                            .child(i18n::tr("settings-title")),
                    ),
            )
            .child(
                v_flex()
                    .w_full()
                    .gap(px(6.))
                    .child(
                        div()
                            .font_family(set.latin)
                            .text_size(px(11.))
                            .text_color(muted)
                            .child(i18n::tr("settings-font-set")),
                    )
                    .children(fonts::ALL.iter().map(|candidate| {
                        let id = candidate.id;
                        let active = id == set.id;
                        Button::new(id)
                            .rounded(px(10.))
                            .h(px(34.))
                            .px(px(12.))
                            .when(active, |button| button.primary())
                            .when(!active, |button| button.secondary())
                            .label(candidate.name)
                            .on_click(cx.listener(move |this, _event, _window, cx| {
                                this.choose_font_set(id, cx);
                            }))
                    })),
            )
            .child(
                v_flex()
                    .w_full()
                    .gap(px(4.))
                    .child(
                        div()
                            .font_family(set.latin)
                            .text_size(px(11.))
                            .text_color(muted)
                            .child(i18n::tr("settings-permissions")),
                    )
                    .children(self.permissions.iter().map(|(key, granted)| {
                        div()
                            .font_family(set.latin)
                            .text_size(px(12.))
                            .child(format!(
                                "{} — {}",
                                i18n::tr(key),
                                i18n::tr(if *granted {
                                    "settings-permission-granted"
                                } else {
                                    "settings-permission-denied"
                                })
                            ))
                    })),
            )
            .into_any_element()
    }
}

/// Renders a dictionary hit the way the egui shell did: headword, phonetic,
/// translation, word forms and lemma. `peek_dict::Entry` and `peek_core::Entry`
/// are distinct types, so the text is assembled here.
fn dictionary_text(entry: &peek_dict::Entry) -> String {
    let mut out = String::new();
    out.push_str(&entry.word);
    if !entry.phonetic.is_empty() {
        out.push_str(&format!("   /{}/", entry.phonetic));
    }
    if !entry.translation.is_empty() {
        out.push('\n');
        out.push_str(&entry.translation);
    }
    let forms = entry.forms();
    if !forms.is_empty() {
        let line: Vec<String> = forms
            .iter()
            .map(|(key, value)| format!("{} {value}", i18n::tr(key)))
            .collect();
        out.push('\n');
        out.push_str(&line.join("  ·  "));
    }
    if let Some(lemma) = entry.lemma() {
        out.push('\n');
        out.push_str(&i18n::format("dict-lemma", &[("lemma", lemma)]));
    }
    out
}

/// Sample answer for the offscreen preview. Exercises the Markdown renderer
/// with the scripts this app actually handles.
const SAMPLE_ANSWER: &str = r#"## 需要时出现，看完即走

**Crant Peek** 是一个*浮窗式*阅读助手：选中文字、双击 Ctrl 即可查词或翻译。

- 离线词典优先，毫秒级返回
- AI 回答按 Markdown 渲染
- `Esc` 或失焦即隐藏

```rust
fn main() {
    let x: u32 = 42;
}
```

> 日文示例：エラーを解析します。直線と骨格、今日の海。
"#;

/// Renders the panel offscreen to a PNG, the GPUI counterpart of the egui
/// shell's `PEEK_UI_PREVIEW`. No window is shown, so this is safe to run
/// unattended and gives the Markdown answer a visual check without credentials.
fn render_preview(path: &str) -> anyhow::Result<()> {
    let mut cx = gpui_kit::HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        std::sync::Arc::new(gpui_kit::assets::Assets),
        gpui_kit::platform::current_headless_renderer,
    );

    let window = cx.open_window(size(px(480.), px(560.)), |window, cx| {
        gpui_kit::init(cx);
        if let Err(err) = fonts::register(cx) {
            eprintln!("font registration failed: {err}");
        }
        let missing = fonts::missing(cx);
        if !missing.is_empty() {
            eprintln!("bundled fonts MISSING: {}", missing.join(", "));
        }
        fonts::apply(cx, fonts::initial());
        let view = cx.new(|cx| Peek::new(window, cx));
        let settings_page = std::env::var("PEEK_PAGE").as_deref() == Ok("settings");
        view.update(cx, |peek, cx| {
            if settings_page {
                // Not persisted: a preview must not overwrite the real choice.
                peek.settings = true;
            } else {
                peek.answer = SAMPLE_ANSWER.into();
                peek.status = i18n::tr("status-done");
            }
            cx.notify();
        });
        cx.new(|cx| Root::new(view, window, cx))
    })?;

    cx.run_until_parked();
    let image = cx.capture_screenshot(window.into())?;
    let out = std::path::PathBuf::from(path);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    image.save(&out)?;
    println!(
        "wrote {} ({}x{})",
        out.display(),
        image.width(),
        image.height()
    );
    Ok(())
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

fn main() -> anyhow::Result<()> {
    let selftest = std::env::var("PEEK_SELFTEST").is_ok();

    if let Ok(path) = std::env::var("PEEK_RENDER") {
        return render_preview(&path);
    }

    // Selection diagnostic: AXIsProcessTrusted() can report true while real
    // Accessibility calls still fail with kAXErrorAPIDisabled - which is what
    // happens when the app is launched from a terminal rather than by
    // LaunchServices. This exercises the API itself, and shows no window.
    if std::env::var("PEEK_SELECTION_SELFTEST").is_ok() {
        match peek_runtime::selection::read() {
            Some(text) => println!("selection read ok: {} chars", text.chars().count()),
            None => println!("selection read failed (see the [selection] lines above)"),
        }
        return Ok(());
    }

    // Environment diagnostic: creates and shows no window. Run through a
    // packaged .app binary to see the permissions macOS grants that bundle.
    if std::env::var("PEEK_MONITOR_REPORT").is_ok() {
        // Written to a file as well when PEEK_SELECTION_LOG is set, because
        // `open` refuses to redirect stdio into an already-running app and that
        // redirection also muddies which process TCC holds responsible.
        let report = |line: String| {
            println!("{line}");
            if let Ok(path) = std::env::var("PEEK_SELECTION_LOG") {
                use std::io::Write as _;
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let _ = writeln!(file, "{line}");
                }
            }
        };
        for (name, granted) in peek_runtime::permissions::status() {
            report(format!("permission {name}: {granted}"));
        }
        report(format!(
            "cursor monitor bounds: {:?}",
            peek_runtime::capture::monitor_bounds()
        ));
        for line in peek_runtime::capture::monitor_report() {
            report(line);
        }
        return Ok(());
    }

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
        fonts::apply(cx, fonts::initial());

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
        let show_item = MenuItem::new(i18n::tr("tray-open"), true, None);
        let quit_item = MenuItem::new(i18n::tr("tray-quit"), true, None);
        let show_id = show_item.id().clone();
        let quit_id = quit_item.id().clone();
        let _ = menu.append(&show_item);
        let _ = menu.append(&quit_item);
        match TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip(i18n::tr("tray-tooltip"))
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
        // depend on a human pressing the hotkey.
        if selftest {
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                let before = cx
                    .update_window(handle, |_, window, _| native_window::is_visible(window))
                    .unwrap_or(false);
                println!("[selftest] visible at startup : {before}  <- want false");

                cx.update_window(handle, |_, window, _| show_panel(window))
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
                // Long enough for the query-pipeline check spawned in
                // `Peek::new` to finish before the process exits.
                cx.background_executor()
                    .timer(Duration::from_millis(3200))
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
                        let _ = cx.update_window(handle, |_, window, _| show_panel(window));
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
