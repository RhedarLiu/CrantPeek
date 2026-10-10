//! Crant Peek — GPUI shell.
//!
//! Tray-resident, floating-first: no main window, the peek panel is created
//! hidden at startup and summoned by a global hotkey. `peek-core`,
//! `peek-network` and `peek-dict` hold the domain logic; shared
//! config/keychain/i18n live in `peek-runtime`.
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

// Needed for `on_hover` on an element that carries an id.
use gpui_kit::InteractiveElement as _;
use gpui_kit::assets::IconName;
use gpui_kit::base::{IndexPath, Root, StyledExt as _};
use gpui_kit::component::button::{Button, ButtonVariants as _, DropdownButton};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tray_icon::Icon as TrayIconImage;
use tray_icon::TrayIconBuilder;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};

use peek_core::{Channel, ChannelKind, Config, Message, Task, effective_target, local_route};
use peek_network::{Client, Event};
use peek_runtime::action::{Action, OverlayEvent};
use peek_runtime::{i18n, store};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Poll interval for the tray/hotkey channels. Both crates deliver events on
/// their own channels rather than through GPUI, so they are drained on a timer.
/// Which part of the decision service an input edits.
#[derive(Clone, Copy)]
enum DecisionField {
    Endpoint,
    Model,
    Key,
}

/// Panel geometry: compact until there is something to show, which keeps the
/// default state a bare input box instead of a mostly empty card stack.
const PANEL_WIDTH: f32 = 480.;
/// Panel height with the input box at its two-row minimum and the title row
/// hidden: the padding and the input card, which carries its own controls.
const PANEL_COMPACT_BASE: f32 = 72.;
/// The hover target above the input box while the row is hidden.
const PANEL_CHROME_STRIP: f32 = 16.;
/// The revealed row's own height, which reaches past its visible content into
/// the top of the input box.
///
/// That overlap is the point: revealing the row moves the window up, so the
/// pointer that revealed it would otherwise end up just below the row, un-hover
/// it, and start the two flashing at each other.
const PANEL_CHROME_ROW: f32 = 50.;
/// What the revealed title row adds: the extra top padding, the row and the gap
/// that follows it, less the padding the collapsed state already has.
const PANEL_CHROME_HEIGHT: f32 = 40.;
/// Added per input row, so a longer draft grows the panel instead of spilling
/// outside the input card.
const PANEL_INPUT_ROW: f32 = 22.;
/// A result card taller than this would push the panel past a comfortable
/// reading window; the card scrolls beyond it.
const PANEL_RESULT_MAX_HEIGHT: f32 = 620.;
/// Chrome the settings page always shows: header, tab row, padding and gaps.
const SETTINGS_CHROME: f32 = 130.;
/// One row of the channel list, the add button, or a picker row.
const SETTINGS_ROW: f32 = 46.;

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

thread_local! {
    /// Whether the title row is showing.
    ///
    /// It lives outside the view because showing the window is a free
    /// function: every summon has to start minimal, and this is the piece of
    /// state that reset needs to reach.
    static CHROME_VISIBLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

thread_local! {
    /// Screen y of the panel's top edge while the title row is hidden.
    ///
    /// Revealing the row grows the window, and which way a platform grows one is
    /// its own choice, so the position is measured and corrected rather than
    /// assumed. Cleared on every show, since the panel is placed again then.
    static ANCHOR_TOP: std::cell::Cell<Option<f32>> = const { std::cell::Cell::new(None) };
    /// Whether the strip may reveal the title row again.
    ///
    /// At the boundary between the input box and the row, hiding puts the
    /// pointer straight onto the strip, which would reveal the row again the
    /// same instant: the two would flash at each other. The strip only counts
    /// once the pointer has left it since the last hide.
    static STRIP_ARMED: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// Whether the title row is showing.
fn chrome_visible() -> bool {
    CHROME_VISIBLE.with(std::cell::Cell::get)
}

/// Shows or hides the title row.
fn set_chrome_visible(visible: bool) {
    CHROME_VISIBLE.with(|cell| cell.set(visible));
}

/// Shows the panel and records when, so focus loss is ignored briefly.
///
/// The title row is hidden on the way in: a panel reopened after the pointer
/// found it once must look the same as one opened for the first time.
fn show_panel(window: &mut Window) {
    set_chrome_visible(false);
    ANCHOR_TOP.with(|cell| cell.set(None));
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
/// The picker's last entry: let the content decide. Kept apart from `Task`
/// because it is a choice about the choice, not a task.
const TASK_AUTO_INDEX: usize = TASK_ORDER.len();
const TASK_KEYS: &[&str] = &[
    "task-translate",
    "task-define",
    "task-explain-code",
    "task-explain-error",
    "task-explain",
    "task-auto",
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

    /// Set while a turn belongs to the screenshot overlay: its progress is
    /// forwarded there instead of being shown in the panel.
    overlay: Option<std::sync::mpsc::Sender<OverlayEvent>>,
    /// Permission name (already an i18n key) and whether it is granted. The
    /// double-tap Ctrl hook and selection reading need Accessibility and Input
    /// Monitoring; without them they fail silently, so the panel says so
    /// instead of appearing broken.
    permissions: Vec<(&'static str, bool)>,
    /// Whether the settings page is showing.
    settings: bool,
    /// A message waiting for a `Window`, with whether it reports a failure. A
    /// turn's outcome arrives through the poll loop, where no notification can
    /// be raised.
    pending_toast: Option<(bool, String)>,
    /// Channel a connection test is running for, so its row can show progress
    /// on the button rather than an extra line of text.
    testing_channel: Option<String>,
    /// Sends test results from the network task to the poll loop: the channel
    /// id, whether it failed, and the message to show.
    test_tx: std::sync::mpsc::Sender<(String, bool, String)>,
    /// Editable decision-service settings, persisted as they are typed.
    decision_endpoint: Entity<InputState>,
    decision_model: Entity<InputState>,
    decision_key: Entity<InputState>,
    /// Editable global hotkeys, persisted as they are typed.
    blank_hotkey: Entity<InputState>,
    screenshot_hotkey: Entity<InputState>,
    /// Channel being edited, or `None` when the dialog is adding one.
    editing_channel: Option<String>,
    /// Draft fields for the channel dialog.
    channel_name: Entity<InputState>,
    channel_endpoint: Entity<InputState>,
    channel_model: Entity<InputState>,
    channel_key: Entity<InputState>,
    channel_kind: Entity<SelectState<Vec<SharedString>>>,
    /// Which settings tab is showing.
    settings_tab: usize,
    /// Pinned to a regular window: the app takes a Dock icon and menu bar and
    /// stops hiding when it loses focus. Unpinned is the quick peek that appears
    /// and leaves, with no Dock presence at all.
    pinned: bool,
    /// Last applied window height, so the window resizes on transitions only
    /// rather than on every frame.
    applied_height: Option<f32>,
    config: Config,
    client: Client,
    runtime: tokio::runtime::Runtime,
    messages: Vec<Message>,
    receiver: Option<mpsc::Receiver<Event>>,
    cancel: Option<CancellationToken>,
}

impl Peek {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        action_tx: std::sync::mpsc::Sender<Action>,
        action_rx: std::sync::mpsc::Receiver<Action>,
    ) -> Self {
        // Hiding on focus loss. GPUI's callback carries no activation state, so
        // the state is read back from the window itself.
        native_window::hide_window_buttons(window);
        let activation = cx.observe_window_activation(window, |this, window, _cx| {
            // A pinned window behaves like a normal window and stays put.
            if !this.pinned && !window.is_window_active() && !shown_recently() {
                native_window::hide(window);
            }
        });

        let input = cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 5));
        let follow_up = cx.new(|cx| {
            InputState::new(window, cx).placeholder(i18n::tr("query-followup-placeholder"))
        });
        let decision_endpoint = cx.new(|cx| InputState::new(window, cx));
        let decision_model = cx.new(|cx| InputState::new(window, cx));
        let decision_key = cx.new(|cx| InputState::new(window, cx).masked(true));
        let blank_hotkey = cx.new(|cx| InputState::new(window, cx));
        let screenshot_hotkey = cx.new(|cx| InputState::new(window, cx));
        // Draft fields for adding a channel. One set is enough: the fields a
        // kind needs are shown or hidden as the type changes.
        let channel_name = cx.new(|cx| InputState::new(window, cx));
        let channel_endpoint = cx.new(|cx| InputState::new(window, cx));
        let channel_model = cx.new(|cx| InputState::new(window, cx));
        let channel_key = cx.new(|cx| InputState::new(window, cx).masked(true));
        let kinds: Vec<SharedString> = ChannelKind::ALL
            .iter()
            .map(|kind| i18n::tr(kind.label_key()).into())
            .collect();
        let channel_kind =
            cx.new(|cx| SelectState::new(kinds, Some(IndexPath::new(0)), window, cx));

        let (test_tx, test_rx) = std::sync::mpsc::channel();
        // Enter is handled inside the input, so it is picked up from its event
        // rather than from a key listener above it, which never sees the key.
        cx.subscribe(&input, |this, _input, event: &InputEvent, cx| {
            if let InputEvent::PressEnter { secondary, shift } = event
                && !*secondary
                && !*shift
            {
                this.submit(cx);
                cx.notify();
            }
        })
        .detach();
        let config = store::load().unwrap_or_default();
        i18n::set_language(&config.ui_language);
        for (input, value) in [
            (&decision_endpoint, &config.decision.endpoint),
            (&decision_model, &config.decision.model),
            (&decision_key, &config.decision.credential_id),
            (&blank_hotkey, &config.blank_hotkey),
            (&screenshot_hotkey, &config.screenshot_hotkey),
        ] {
            let value = value.clone();
            input.update(cx, |state, cx| state.set_value(&value, window, cx));
        }
        // Persisted as they are typed. The running app keeps the hotkeys it
        // registered at startup, which is why the page says a restart applies
        // a change.
        // The decision service is reached with its own endpoint, model and
        // credential, separate from any channel.
        for (input, field) in [
            (&decision_endpoint, DecisionField::Endpoint),
            (&decision_model, DecisionField::Model),
            (&decision_key, DecisionField::Key),
        ] {
            cx.observe(input, move |this, state, cx| {
                let value = state.read(cx).value().trim().to_owned();
                let slot = match field {
                    DecisionField::Endpoint => &mut this.config.decision.endpoint,
                    DecisionField::Model => &mut this.config.decision.model,
                    DecisionField::Key => &mut this.config.decision.credential_id,
                };
                if *slot != value {
                    *slot = value;
                    this.persist_config();
                    cx.notify();
                }
            })
            .detach();
        }
        for (input, screenshot) in [(&blank_hotkey, false), (&screenshot_hotkey, true)] {
            cx.observe(input, move |this, state, cx| {
                let value = state.read(cx).value().trim().to_owned();
                let slot = if screenshot {
                    &mut this.config.screenshot_hotkey
                } else {
                    &mut this.config.blank_hotkey
                };
                if *slot != value {
                    *slot = value;
                    this.persist_config();
                    cx.notify();
                }
            })
            .detach();
        }

        let items: Vec<SharedString> = TASK_KEYS.iter().map(|key| i18n::tr(key).into()).collect();
        // The configuration's `smart_mode` now only decides where the picker
        // starts: automatic, or a fixed task. Either way the user can change it.
        let initial = if config.smart_mode {
            TASK_AUTO_INDEX
        } else {
            0
        };
        let tasks = cx.new(|cx| SelectState::new(items, Some(IndexPath::new(initial)), window, cx));

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");

        let this = Self {
            set: fonts::initial(),
            _activation: activation,
            input,
            follow_up,
            settings_tab: 0,
            pending_toast: None,
            testing_channel: None,
            test_tx,
            decision_endpoint,
            decision_model,
            decision_key,
            blank_hotkey,
            screenshot_hotkey,
            editing_channel: None,
            channel_name,
            channel_endpoint,
            channel_model,
            channel_key,
            channel_kind,
            tasks,
            answer: String::new(),
            applied_height: None,
            pinned: false,
            overlay: None,
            pending_input: None,
            original_query: String::new(),
            status: String::new(),
            busy: false,
            dictionary_note: String::new(),
            permissions: peek_runtime::permissions::status(),
            // The peek always opens on the query page: settings is a view the
            // user steps into, not a place to be summoned back to.
            settings: false,
            config,
            client: Client::default(),
            runtime,
            messages: Vec::new(),
            receiver: None,
            cancel: None,
        };

        // `main` owns the action channel and feeds it from the selection hook,
        // the tray and the global hotkeys; this view drains it on a timer, so no
        // UI context ever crosses a platform thread boundary.
        let selftest_wake = action_tx.clone();
        let snip_test_wake = action_tx.clone();
        // Used by the screenshot overlay to report recognised text back.
        let snip_tx = action_tx;
        let panel_handle = window.window_handle();
        let snip_selftest = std::env::var("PEEK_SNIP_SELFTEST").is_ok();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                while let Ok((id, failed, message)) = test_rx.try_recv() {
                    this.update(cx, |peek, cx| {
                        peek.testing_channel = None;
                        if failed {
                            peek.notify_error(message);
                        } else {
                            peek.notify_success(message);
                        }
                        let _ = id;
                        cx.notify();
                    })
                    .ok();
                }
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
                            // The panel would otherwise sit under the overlay
                            // and compete with the selection.
                            let _ = cx.update_window(panel_handle, |_, window, _| {
                                native_window::hide(window)
                            });
                            // The overlay captures on release, so this only
                            // opens the selection window.
                            let tx = snip_tx.clone();
                            let close_on_copy = this
                                .update(cx, |peek, _| peek.config.snip_close_on_copy)
                                .unwrap_or(true);
                            let opened = cx.update(|app| snip::open(app, tx, close_on_copy));
                            match opened {
                                Ok((handle, bounds)) => {
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
                                Err(err) => println!("[snip] overlay NOT opened: {err}"),
                            }
                        }
                        Action::Translate { text, replies } => {
                            // The overlay owns this turn: progress goes back to
                            // it and the panel stays hidden.
                            let started = this
                                .update(cx, |peek, cx| {
                                    peek.overlay = Some(replies);
                                    let started = peek.begin_turn(text, false, cx);
                                    if !started {
                                        // `begin_turn` left a status explaining
                                        // why it could not run.
                                        let status = peek.status.clone();
                                        peek.forward_overlay(OverlayEvent::Status(status));
                                        peek.forward_overlay(OverlayEvent::Finished);
                                        peek.overlay = None;
                                    }
                                    started
                                })
                                .unwrap_or(false);
                            let _ = started;
                        }
                        Action::Recognized(text) => {
                            let _ = cx
                                .update_window(panel_handle, |_, window, _| show_panel(window));
                            let text = text.trim().to_owned();
                            this.update(cx, |peek, cx| {
                                if text.is_empty() {
                                    // Say so instead of the selection vanishing.
                                    peek.notify_error(i18n::tr("status-ocr-empty"));
                                    cx.notify();
                                } else {
                                    peek.pending_input = Some(text.clone());
                                    peek.begin_turn(text, false, cx);
                                }
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
    /// Whether there is more to show than the input box.
    fn has_result(&self) -> bool {
        // Deliberately not `busy`: a running turn shows its progress on the
        // button, so an empty result card would be a large blank area.
        !self.answer.is_empty() || !self.dictionary_note.is_empty()
    }

    /// Panel height while there is a result, following how much has streamed in.
    ///
    /// Wrapping is estimated from the character count, so the panel grows with
    /// the answer instead of opening at a fixed size and leaving it empty.
    fn result_height(&self) -> f32 {
        let columns = 42.0;
        let rows: f32 = self
            .answer
            .lines()
            .map(|line| (line.chars().count() as f32 / columns).ceil().max(1.0))
            .sum();
        let dictionary = if self.dictionary_note.is_empty() {
            0.0
        } else {
            96.0
        };
        let chrome = PANEL_COMPACT_BASE + PANEL_INPUT_ROW * 2.0 + 96.0;
        (chrome + dictionary + rows * 26.0).clamp(chrome, PANEL_RESULT_MAX_HEIGHT)
    }

    /// The chosen task, or `None` for the automatic choice.
    fn selected_task(&self, cx: &App) -> Option<Task> {
        self.tasks
            .read(cx)
            .selected_index(cx)
            .and_then(|index| TASK_ORDER.get(index.row))
            .copied()
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
        };
        if let Err(err) = peek_runtime::prefs::save(&prefs) {
            eprintln!("saving ui prefs failed: {err}");
        }
    }

    /// Reads the input box and starts a turn, clearing it only if one began.
    fn start_query(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.submit(cx);
    }

    /// Sends whatever is in the box.
    ///
    /// Split from the button handler because the input reports Enter through an
    /// event, and that arrives without a Window.
    fn submit(&mut self, cx: &mut Context<Self>) {
        // The text stays in the box: clearing it before there is a result takes
        // away what the user just wrote, and they may want to edit it.
        let text = self.input.read(cx).value().trim().to_owned();
        self.begin_turn(text, false, cx);
    }

    /// Translates through the configured DeepLX endpoint.
    ///
    /// It reuses the same event channel as an AI turn, so the panel and the
    /// screenshot overlay need no separate path; the endpoint answers in one
    /// piece rather than as a stream.
    fn begin_deeplx(
        &mut self,
        endpoint: String,
        key: String,
        text: String,
        cx: &mut Context<Self>,
    ) -> bool {
        let route = local_route(
            &text,
            &self.config.target_language,
            &self.config.chinese_target,
        );
        let target = effective_target(&route.target, "").to_owned();
        self.original_query = text.clone();
        self.busy = true;
        let (tx, rx) = mpsc::channel(8);
        self.receiver = Some(rx);
        let cancel = CancellationToken::new();
        self.cancel = Some(cancel.clone());
        let client = self.client.clone();
        let ui_language = self.config.ui_language.clone();
        self.runtime.spawn(async move {
            let locale = i18n::I18n::new(&ui_language);
            match client
                .translate_deeplx(&endpoint, &key, &text, &target, cancel)
                .await
            {
                Ok(translated) => {
                    let _ = tx.send(Event::Text(translated)).await;
                    let _ = tx.send(Event::Done).await;
                }
                Err(err) => {
                    let _ = tx.send(Event::Failed(locale.network_error(&err))).await;
                }
            }
        });
        cx.notify();
        true
    }

    /// Height for the settings page's current tab.
    ///
    /// The page is a stack of sections of very different lengths, so a single
    /// fixed height either clipped the long tab or left the short one with a
    /// large empty area. This estimates from what the tab actually renders and
    /// stays within a usable range.
    fn settings_height(&self) -> f32 {
        let channels = self.config.channels.len() as f32;
        let height = match self.settings_tab {
            // Font set: a label and two buttons.
            0 => SETTINGS_CHROME + 92.,
            // Shortcuts: two fields and a hint.
            1 => SETTINGS_CHROME + 130.,
            // Channels: the list, the add button, the copy switch, the
            // used-for pickers and the decision service.
            2 => SETTINGS_CHROME + 320. + SETTINGS_ROW * (channels + 4.),
            // Permissions: one row per permission.
            _ => SETTINGS_CHROME + 24. + SETTINGS_ROW * self.permissions.len() as f32,
        };
        // The translation tab carries the channel list, the pickers and the
        // decision service, so its ceiling is higher than the others'.
        height.clamp(240., 900.)
    }

    /// A label and the dropdown that chooses for it.
    fn picker_row(
        &self,
        label: String,
        picker: impl IntoElement,
        set: &'static crate::fonts::FontSet,
    ) -> AnyElement {
        h_flex()
            .w_full()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .w(px(72.))
                    .font_family(set.latin)
                    .text_size(px(12.))
                    .child(label),
            )
            .child(div().flex_1().child(picker))
            .into_any_element()
    }

    /// A dropdown of channels. Only channels the caller offers appear, so a
    /// translation-only channel is never listed for AI work.
    fn channel_picker(
        &self,
        id: &'static str,
        choices: Vec<(String, String)>,
        active: String,
        ai: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        let current = choices
            .iter()
            .find(|(channel, _)| *channel == active)
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| i18n::tr("channels-none"));
        DropdownButton::new(id)
            .button(
                // The dropdown draws its own chevron beside the button, so the
                // button sizes to the row instead of claiming the full width.
                Button::new(SharedString::from(format!("{id}_button")))
                    .label(current)
                    .flex_1(),
            )
            .dropdown_menu(move |mut menu, _window, _cx| {
                for (channel, name) in &choices {
                    let target = entity.clone();
                    let chosen = channel.clone();
                    let selected = *channel == active;
                    menu = menu.item(PopupMenuItem::new(name.clone()).checked(selected).on_click(
                        move |_event, _window, cx| {
                            target.update(cx, |peek, cx| {
                                peek.assign_channel(ai, chosen.clone(), cx);
                            });
                        },
                    ));
                }
                menu
            })
    }

    /// A small muted section label, used across the settings page.
    fn section_label(
        &self,
        text: String,
        set: &'static crate::fonts::FontSet,
        muted: Hsla,
    ) -> AnyElement {
        div()
            .font_family(set.latin)
            .text_size(px(11.))
            .text_color(muted)
            .child(text)
            .into_any_element()
    }

    /// The kind currently picked in the add form.
    fn draft_kind(&self, cx: &App) -> ChannelKind {
        self.channel_kind
            .read(cx)
            .selected_index(cx)
            .and_then(|index| ChannelKind::ALL.get(index.row))
            .copied()
            .unwrap_or_default()
    }

    /// Opens the channel form. `editing` pre-fills it from an existing channel.
    ///
    /// The form lives in a dialog rather than in the settings page: the page
    /// stays a scannable list, and the same form serves adding and editing.
    fn open_channel_dialog(
        &mut self,
        editing: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let existing = editing
            .as_deref()
            .and_then(|id| self.config.channel(id))
            .cloned();
        let kind = existing.as_ref().map(|c| c.kind).unwrap_or_default();
        let name = existing
            .as_ref()
            .map(|c| c.name.clone())
            .unwrap_or_default();
        let endpoint = existing
            .as_ref()
            .map(|c| c.endpoint.clone())
            .unwrap_or_default();
        let model = existing
            .as_ref()
            .map(|c| c.model.clone())
            .unwrap_or_default();
        self.editing_channel = editing;
        self.channel_name
            .update(cx, |state, cx| state.set_value(&name, window, cx));
        self.channel_endpoint
            .update(cx, |state, cx| state.set_value(&endpoint, window, cx));
        self.channel_model
            .update(cx, |state, cx| state.set_value(&model, window, cx));
        // Never pre-filled: the stored secret stays in the keychain, and an
        // empty field keeps it.
        self.channel_key
            .update(cx, |state, cx| state.set_value("", window, cx));
        let index = ChannelKind::ALL
            .iter()
            .position(|candidate| *candidate == kind)
            .unwrap_or(0);
        self.channel_kind.update(cx, |state, cx| {
            state.set_selected_index(Some(IndexPath::new(index)), window, cx);
        });

        let entity = cx.entity();
        let title = i18n::tr(if self.editing_channel.is_some() {
            "channels-edit-title"
        } else {
            "channels-add-title"
        });
        window.open_dialog(cx, move |dialog, _window, _cx| {
            // The builder runs on every frame, so each closure gets its own
            // handle rather than moving the only one.
            let content_entity = entity.clone();
            let ok_entity = entity.clone();
            dialog
                .title(title.clone())
                .w(px(420.))
                .content(move |content, _window, cx| {
                    let peek = content_entity.read(cx);
                    let kind = peek.draft_kind(cx);
                    let muted = cx.theme().muted_foreground;
                    let field = |label: String, control: AnyElement| {
                        v_flex()
                            .w_full()
                            .gap(px(4.))
                            .child(div().text_size(px(11.)).text_color(muted).child(label))
                            .child(control)
                            .into_any_element()
                    };
                    content.child(
                        v_flex()
                            .w_full()
                            .gap(px(10.))
                            .child(field(
                                i18n::tr("channels-kind"),
                                Select::new(&peek.channel_kind)
                                    .id("channel-kind-dialog")
                                    .into_any_element(),
                            ))
                            .child(field(
                                i18n::tr("channels-name"),
                                Input::new(&peek.channel_name).into_any_element(),
                            ))
                            .child(field(
                                i18n::tr("channels-endpoint"),
                                Input::new(&peek.channel_endpoint).into_any_element(),
                            ))
                            .when(kind.needs_credential(), |this| {
                                this.child(field(
                                    if peek.editing_channel.is_some() {
                                        // The stored secret is never read back.
                                        format!(
                                            "{} · {}",
                                            i18n::tr("channels-key"),
                                            i18n::tr("channels-key-keep")
                                        )
                                    } else {
                                        i18n::tr("channels-key")
                                    },
                                    Input::new(&peek.channel_key).into_any_element(),
                                ))
                            })
                            .when(kind.needs_model(), |this| {
                                this.child(field(
                                    i18n::tr("channels-model"),
                                    Input::new(&peek.channel_model).into_any_element(),
                                ))
                            }),
                    )
                })
                // A plain Dialog renders no buttons of its own: the footer is
                // the caller's, so these are the two actions it needs.
                .footer(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            Button::new("channel-cancel")
                                .label(i18n::tr("channels-cancel"))
                                .on_click(move |_event, window, cx| {
                                    window.close_dialog(cx);
                                }),
                        )
                        .child(
                            Button::new("channel-save")
                                .primary()
                                .label(i18n::tr("channels-save"))
                                .on_click(move |_event, window, cx| {
                                    ok_entity.update(cx, |peek, cx| peek.save_channel(window, cx));
                                    window.close_dialog(cx);
                                }),
                        ),
                )
        });
    }

    /// Saves the dialog's channel, adding it or updating the one being edited.
    ///
    /// A kind declares what it needs: an AI kind takes an endpoint, a model and
    /// a credential, while DeepLX takes an endpoint only. The secret goes to the
    /// keychain and config keeps a reference.
    fn save_channel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let kind = self.draft_kind(cx);
        let name = self.channel_name.read(cx).value().trim().to_owned();
        let endpoint = self.channel_endpoint.read(cx).value().trim().to_owned();
        let model = self.channel_model.read(cx).value().trim().to_owned();
        let key = self.channel_key.read(cx).value().to_string();
        if name.is_empty() || endpoint.is_empty() {
            self.notify_error(i18n::tr("status-channel-incomplete"));
            cx.notify();
            return;
        }
        if kind.needs_model() && model.is_empty() {
            self.notify_error(i18n::tr("error-config-channel-model"));
            cx.notify();
            return;
        }

        let editing = self.editing_channel.clone();
        let id = editing.clone().unwrap_or_else(|| {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_millis())
                .unwrap_or(0);
            format!("ch-{stamp}")
        });
        // An existing key is kept when the field is left empty, so the secret
        // never has to be re-entered to change another field. It is stored with
        // the channel rather than in the keychain: see `Channel::api_key`.
        let api_key = if key.trim().is_empty() {
            editing
                .as_deref()
                .and_then(|id| self.config.channel(id))
                .map(|channel| channel.api_key.clone())
                .unwrap_or_default()
        } else {
            key
        };
        let saved = Channel {
            id: id.clone(),
            name,
            kind,
            endpoint,
            model,
            api_key,
            credential_id: String::new(),
            vision: false,
            max_output_tokens: 2048,
        };
        match editing {
            Some(_) => {
                if let Some(slot) = self
                    .config
                    .channels
                    .iter_mut()
                    .find(|channel| channel.id == id)
                {
                    *slot = saved;
                }
            }
            None => {
                self.config.channels.push(saved);
                // The first channel becomes the default for a place that has
                // nothing chosen yet.
                if self.config.basic_channel.is_empty() {
                    self.config.basic_channel = id.clone();
                }
                if self.config.ai_channel.is_empty() && kind.is_ai() {
                    self.config.ai_channel = id;
                }
            }
        }
        // A channel that changed kind may no longer be able to serve its place.
        if self.config.ai().is_none() {
            // Resolved first: the iterator borrows the config.
            let fallback = self
                .config
                .ai_candidates()
                .next()
                .map(|channel| channel.id.clone())
                .unwrap_or_default();
            self.config.ai_channel = fallback;
        }
        self.editing_channel = None;
        self.persist_config();
        for input in [
            &self.channel_name,
            &self.channel_endpoint,
            &self.channel_model,
            &self.channel_key,
        ] {
            input.update(cx, |state, cx| state.set_value("", window, cx));
        }
        cx.notify();
    }

    /// Checks that a channel actually answers.
    ///
    /// The endpoint a channel is configured with is a base URL, so a request
    /// goes to `<endpoint>/chat/completions` for an OpenAI-style protocol. That
    /// is exactly where a URL missing its `/v1` segment fails, so the result
    /// names the URL that was tried.
    fn test_channel(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(channel) = self.config.channel(&id).cloned() else {
            return;
        };
        self.testing_channel = Some(id.clone());
        cx.notify();
        let client = self.client.clone();
        let tx = self.test_tx.clone();
        let ui_language = self.config.ui_language.clone();
        self.runtime.spawn(async move {
            let locale = i18n::I18n::new(&ui_language);
            // A failure names the URL it tried: a base missing its version
            // segment is the usual cause, and the status alone hides that.
            let told = |url: &str, err: &peek_network::Error| {
                locale.format(
                    "channels-test-failed",
                    &[("detail", &format!("{} · {url}", locale.network_error(err)))],
                )
            };
            let (ok, message) = if channel.kind == ChannelKind::DeepLx {
                let url = channel.endpoint.clone();
                match client
                    .translate_deeplx(
                        &url,
                        &channel.api_key,
                        "hello",
                        "Chinese",
                        CancellationToken::new(),
                    )
                    .await
                {
                    Ok(_) => (false, i18n::tr("channels-test-ok")),
                    Err(err) => (true, told(&url, &err)),
                }
            } else {
                let Some(provider) = channel.provider() else {
                    let _ = tx.send((id, true, i18n::tr("status-channel-missing")));
                    return;
                };
                let url = peek_network::endpoint_url(&provider)
                    .unwrap_or_else(|_| provider.base_url.clone());
                match client.probe(&provider, &channel.api_key).await {
                    Ok(()) => (false, i18n::tr("channels-test-ok")),
                    Err(err) => (true, told(&url, &err)),
                }
            };
            let _ = tx.send((id, ok, message));
        });
    }

    /// Removes a channel and clears any selection that pointed at it.
    fn remove_channel(&mut self, id: &str, cx: &mut Context<Self>) {
        self.config.channels.retain(|channel| channel.id != id);
        if self.config.basic_channel == id {
            self.config.basic_channel = self
                .config
                .channels
                .first()
                .map(|channel| channel.id.clone())
                .unwrap_or_default();
        }
        if self.config.ai_channel == id {
            // Resolved before the assignment: the iterator borrows the config.
            let fallback = self
                .config
                .ai_candidates()
                .next()
                .map(|channel| channel.id.clone())
                .unwrap_or_default();
            self.config.ai_channel = fallback;
        }
        self.persist_config();
        cx.notify();
    }

    /// Points a place in the app at a channel.
    fn assign_channel(&mut self, ai: bool, id: String, cx: &mut Context<Self>) {
        if ai {
            self.config.ai_channel = id;
        } else {
            self.config.basic_channel = id;
        }
        self.persist_config();
        cx.notify();
    }

    fn persist_config(&mut self) {
        if let Err(err) = store::save(&self.config) {
            eprintln!("config save failed: {err}");
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

    /// Queues an error for the next frame, where a Window exists to raise a
    /// notification. Errors are collected here rather than written into the
    /// panel: the panel is for results, not for a log.
    fn notify_error(&mut self, message: String) {
        self.pending_toast = Some((true, message));
    }

    /// A confirmation, for something that succeeded.
    fn notify_success(&mut self, message: String) {
        self.pending_toast = Some((false, message));
    }

    /// Builds the prompt and starts the network turn. Returns whether a turn
    /// actually started. Split out of `start_query` so `PEEK_SELFTEST` can
    /// drive the whole pipeline without a window or a click.
    fn begin_turn(&mut self, text: String, followup: bool, cx: &mut Context<Self>) -> bool {
        if text.is_empty() {
            return false;
        }
        if text.len() > peek_core::MAX_INPUT_BYTES {
            self.notify_error(i18n::tr("status-input-too-long"));
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
        // The picker is authoritative. Automatic is now one of its entries
        // rather than a hidden setting that overrode the choice.
        let task = self.selected_task(cx).unwrap_or(route.task);
        let target = effective_target(&route.target, "").to_owned();

        // Plain translation may go through a basic channel such as DeepLX;
        // everything else needs an AI channel. A channel's kind decides where
        // it can be offered, so this never picks DeepLX for code explanation.
        let chosen = match task {
            Task::Translate => self.config.basic().or_else(|| self.config.ai()).cloned(),
            _ => self.config.ai().cloned(),
        };
        let Some(channel) = chosen else {
            self.notify_error(i18n::tr("status-channel-missing"));
            cx.notify();
            return false;
        };
        if channel.kind == ChannelKind::DeepLx {
            return self.begin_deeplx(channel.endpoint.clone(), channel.api_key.clone(), text, cx);
        }
        let Some(provider) = channel.provider() else {
            self.notify_error(i18n::tr("status-channel-missing"));
            cx.notify();
            return false;
        };
        let key = channel.api_key.clone();
        if channel.kind.needs_credential() && key.trim().is_empty() {
            self.notify_error(i18n::tr("status-key-missing"));
            cx.notify();
            return false;
        }
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
            // Delimited, so the model can tell the text it works on from the
            // instructions it was given.
            content: peek_core::wrap_content(&text),
        });
        peek_core::bound_history(&mut self.messages);

        self.busy = true;
        let (tx, rx) = mpsc::channel(128);
        self.receiver = Some(rx);
        let cancel = CancellationToken::new();
        self.cancel = Some(cancel.clone());

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

    /// An icon button in the component's own style: the default variant's
    /// minimal border, the icon in the button's icon slot, and a tooltip so the
    /// action is still discoverable without a label.
    fn icon_button(
        &self,
        id: &'static str,
        icon: gpui_kit::assets::IconName,
        tooltip: String,
        cx: &Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Button {
        Button::new(id)
            // Ghost: the title row is chrome over the panel, so bordered
            // buttons there read as a toolbar bolted on.
            .ghost()
            .icon(icon)
            .tooltip(tooltip)
            .on_click(cx.listener(move |this, _event, window, cx| on_click(this, window, cx)))
    }

    /// Switches between the quick peek and a regular window.
    fn toggle_pinned(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pinned = !self.pinned;
        cx.set_activation_policy(if self.pinned {
            ActivationPolicy::Regular
        } else {
            ActivationPolicy::Accessory
        });
        if self.pinned {
            // A regular app's menu bar only appears once it is activated.
            cx.activate(true);
        } else {
            // Dropping the Dock icon must not take the window with it.
            native_window::show(window);
        }
        cx.notify();
    }

    /// Reports progress to the overlay, dropping the link when it has gone.
    fn forward_overlay(&mut self, event: OverlayEvent) {
        let broken = match &self.overlay {
            Some(tx) => tx.send(event).is_err(),
            None => false,
        };
        if broken {
            self.overlay = None;
        }
    }

    fn apply_events(&mut self, events: Vec<Event>) {
        for event in events {
            match event {
                Event::Route { note, .. } => {
                    // Routing is internal detail; only the overlay, which has
                    // no other surface while it works, hears about it.
                    self.forward_overlay(OverlayEvent::Status(note));
                }
                Event::Text(delta) => {
                    if self.answer.len() + delta.len() > peek_core::MAX_OUTPUT_BYTES {
                        self.notify_error(i18n::tr("status-output-too-long"));
                        self.stop();
                        break;
                    }
                    self.answer.push_str(&delta);
                    self.forward_overlay(OverlayEvent::Chunk(delta));
                }
                Event::Done => {
                    // A finished turn says so by having an answer; a "done"
                    // line only takes space.
                    self.status.clear();
                    self.busy = false;
                    self.forward_overlay(OverlayEvent::Finished);
                    self.overlay = None;
                }
                Event::Failed(message) => {
                    // The detail is long and often carries a URL, so it goes to
                    // a notification and the panel keeps only a short state.
                    self.notify_error(message.clone());
                    self.busy = false;
                    self.forward_overlay(OverlayEvent::Failed(message));
                    self.forward_overlay(OverlayEvent::Finished);
                    self.overlay = None;
                }
            }
        }
    }
}

impl Render for Peek {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A selection arrives off the UI thread, so it is applied here where a
        // `Window` is available.
        if let Some((failed, message)) = self.pending_toast.take() {
            let toast = if failed {
                Notification::error(message)
            } else {
                Notification::success(message)
            };
            window.push_notification(toast.autohide(true), cx);
        }
        if let Some(text) = self.pending_input.take() {
            self.input
                .update(cx, |state, cx| state.set_value(text, window, cx));
        }
        // Resize before drawing. The query page stays compact until it has
        // something to show, while the settings page is always tall; sizing it
        // only from the query page left settings clipped to a compact window.
        let height = if self.settings {
            self.settings_height()
        } else if self.has_result() {
            self.result_height()
        } else {
            // The textarea auto-grows between two and five rows, so the compact
            // panel follows it rather than guessing a single height.
            let rows = self.input.read(cx).value().lines().count().clamp(2, 5) as f32;
            let chrome = if chrome_visible() {
                PANEL_CHROME_HEIGHT
            } else {
                0.
            };
            PANEL_COMPACT_BASE + PANEL_INPUT_ROW * rows + chrome
        };
        // Where the panel's top edge sits while collapsed, so revealing the
        // title row grows the window upwards and leaves the input box on the
        // same line of the screen.
        let top = window.bounds().origin.y.as_f32();
        let anchor = ANCHOR_TOP.with(|cell| match cell.get() {
            Some(anchor) => anchor,
            None => {
                let anchor = top
                    + if chrome_visible() {
                        PANEL_CHROME_HEIGHT
                    } else {
                        0.
                    };
                cell.set(Some(anchor));
                anchor
            }
        });
        let desired_top = anchor
            - if chrome_visible() {
                PANEL_CHROME_HEIGHT
            } else {
                0.
            };
        let drift = top - desired_top;
        if self.applied_height != Some(height) || drift.abs() > 0.5 {
            self.applied_height = Some(height);
            window.resize(size(px(PANEL_WIDTH), px(height)));
            native_window::move_up(window, drift);
        }
        if self.settings {
            return self.settings_page(cx).into_any_element();
        }
        self.query_page(cx).into_any_element()
    }
}

impl Peek {
    /// The task picker: one icon while it is closed, so the input box stays
    /// nearly empty, and a labelled list when it opens.
    fn task_picker(&self, cx: &Context<Self>) -> impl IntoElement {
        // `None` is the automatic choice, which the content decides.
        let current = self.selected_task(cx);
        let tasks = self.tasks.clone();
        let entity = cx.entity();
        let automatic = current.is_none();
        let (icon, tooltip) = match current {
            Some(task) => (
                Self::task_icon(task),
                i18n::tr(
                    TASK_KEYS[TASK_ORDER
                        .iter()
                        .position(|candidate| *candidate == task)
                        .unwrap_or(0)],
                ),
            ),
            None => (
                gpui_kit::assets::IconName::Sparkles,
                i18n::tr("task-auto-tooltip"),
            ),
        };
        DropdownButton::new("task-picker")
            .button(
                Button::new("task-picker-button")
                    .icon(icon)
                    .tooltip(tooltip),
            )
            .dropdown_menu(move |mut menu, _window, _cx| {
                for (index, task) in TASK_ORDER.iter().enumerate() {
                    let target = entity.clone();
                    let tasks = tasks.clone();
                    let task = *task;
                    menu = menu.item(
                        PopupMenuItem::new(i18n::tr(TASK_KEYS[index]))
                            .icon(Self::task_icon(task))
                            .checked(current == Some(task))
                            .on_click(move |_event, window, cx| {
                                tasks.update(cx, |state, cx| {
                                    state.set_selected_index(
                                        Some(IndexPath::new(index)),
                                        window,
                                        cx,
                                    );
                                });
                                target.update(cx, |_, cx| cx.notify());
                            }),
                    );
                }
                let target = entity.clone();
                let tasks = tasks.clone();
                menu = menu.item(
                    PopupMenuItem::new(i18n::tr("task-auto"))
                        .icon(gpui_kit::assets::IconName::Sparkles)
                        .checked(automatic)
                        .on_click(move |_event, window, cx| {
                            tasks.update(cx, |state, cx| {
                                state.set_selected_index(
                                    Some(IndexPath::new(TASK_AUTO_INDEX)),
                                    window,
                                    cx,
                                );
                            });
                            target.update(cx, |_, cx| cx.notify());
                        }),
                );
                menu
            })
    }

    /// The icon that stands for a task, so the picker can collapse to it.
    fn task_icon(task: Task) -> gpui_kit::assets::IconName {
        use gpui_kit::assets::IconName;
        match task {
            Task::Translate => IconName::Languages,
            Task::Define => IconName::BookA,
            Task::ExplainCode => IconName::Code,
            Task::ExplainError => IconName::Bug,
            Task::Explain => IconName::BookOpen,
        }
    }

    fn query_page(&self, cx: &Context<Self>) -> AnyElement {
        let basic_choices: Vec<(String, String)> = self
            .config
            .basic_candidates()
            .map(|channel| (channel.id.clone(), channel.name.clone()))
            .collect();
        let basic_active = self.config.basic_channel.clone();
        let basic_entity = cx.entity();
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
            // The sides hug the input box whether or not the title row is
            // showing; only the top needs room for it. The gap keeps the input,
            // the answer and the follow-up row from touching.
            .px(px(4.))
            .pt(px(if chrome_visible() { 5. } else { 4. }))
            .pb(px(6.))
            .gap(px(5.))
            // Esc hides the panel — "appear when needed, gone when done". A
            // pinned window is a normal window, so Esc leaves it alone.
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if !this.pinned && event.keystroke.key == "escape" {
                    native_window::hide(window);
                    return;
                }
                // A keystroke means the user is working in the box, so the
                // title row goes away again.
                if chrome_visible() {
                    set_chrome_visible(false);
                    cx.notify();
                }
            }))
            // Hidden, the row is replaced by a thin strip that catches the
            // pointer; the window is sized for whichever of the two is showing.
            .when(!chrome_visible(), |this| {
                this.child(
                    // An overlay, so the hidden panel can hug the input box
                    // while still offering something to hover.
                    div()
                        .id("chrome-strip")
                        .absolute()
                        .top(px(0.))
                        .left(px(0.))
                        .right(px(0.))
                        .h(px(PANEL_CHROME_STRIP))
                        .on_hover(cx.listener(|_this, hovered: &bool, _window, cx| {
                            // An offscreen render parks the pointer at the
                            // origin, which is over this strip, so previews
                            // ignore it.
                            let preview = std::env::var("PEEK_RENDER").is_ok();
                            if !*hovered {
                                // Left the strip: it is free to reveal again.
                                STRIP_ARMED.with(|cell| cell.set(true));
                            } else if !preview
                                && !chrome_visible()
                                && STRIP_ARMED.with(std::cell::Cell::get)
                            {
                                set_chrome_visible(true);
                                cx.notify();
                            }
                        })),
                )
            })
            .when(chrome_visible(), |this| {
                this.child(
                    h_flex()
                        .id("chrome-row")
                        .w_full()
                        .h(px(PANEL_CHROME_ROW))
                        // Content at the top, with the rest of the row left
                        // empty so it still catches the pointer once the window
                        // has moved up.
                        .items_start()
                        .pt(px(2.))
                        .gap(px(10.))
                        // Inset so the title lines up with the text inside the
                        // input box, whose own padding starts at the same place.
                        .px(px(14.))
                        // The row is only there while the pointer is on it.
                        // Leaving downwards lands past the strip, so the two do
                        // not take turns reappearing.
                        .on_hover(cx.listener(|_this, hovered: &bool, _window, cx| {
                            if !*hovered && chrome_visible() {
                                set_chrome_visible(false);
                                STRIP_ARMED.with(|cell| cell.set(false));
                                cx.notify();
                            }
                        }))
                        .child(
                            // A tag rather than plain text, so the title and
                            // the buttons look like one row of controls.
                            Tag::secondary()
                                .rounded(px(8.))
                                .h(px(28.))
                                .px(px(10.))
                                .child(
                                    div()
                                        .font_family(set.latin)
                                        .text_size(px(12.))
                                        .font_semibold()
                                        .child("Crant Peek"),
                                ),
                        )
                        .child(div().flex_1())
                        .child(
                            // Which channel answers a plain translation. Only
                            // channels that can translate are listed, and the same
                            // choice can be made in settings. The icon says
                            // "channel", not "translate": the input box already
                            // uses the language icon for the task picker.
                            DropdownButton::new("basic-channel")
                                .button(
                                    Button::new("basic-channel-button")
                                        .ghost()
                                        .icon(IconName::Cable)
                                        .tooltip(i18n::tr("channels-basic-pick")),
                                )
                                .dropdown_menu(move |mut menu, _window, _cx| {
                                    if basic_choices.is_empty() {
                                        return menu.item(
                                            PopupMenuItem::new(i18n::tr("channels-none"))
                                                .disabled(true),
                                        );
                                    }
                                    for (id, name) in &basic_choices {
                                        let target = basic_entity.clone();
                                        let chosen = id.clone();
                                        let selected = *id == basic_active;
                                        menu = menu.item(
                                            PopupMenuItem::new(name.clone())
                                                .checked(selected)
                                                .on_click(move |_event, _window, cx| {
                                                    target.update(cx, |peek, cx| {
                                                        peek.assign_channel(
                                                            false,
                                                            chosen.clone(),
                                                            cx,
                                                        );
                                                    });
                                                }),
                                        );
                                    }
                                    menu
                                }),
                        )
                        .child(self.icon_button(
                            "pin-window",
                            if self.pinned {
                                gpui_kit::assets::IconName::PinOff
                            } else {
                                gpui_kit::assets::IconName::Pin
                            },
                            if self.pinned {
                                i18n::tr("action-unpin")
                            } else {
                                i18n::tr("action-pin")
                            },
                            cx,
                            |this, window, cx| this.toggle_pinned(window, cx),
                        ))
                        .child(self.icon_button(
                            "open-settings",
                            gpui_kit::assets::IconName::Settings,
                            i18n::tr("settings-title"),
                            cx,
                            |this, _window, cx| {
                                this.settings = true;
                                this.persist_prefs();
                                cx.notify();
                            },
                        ))
                        .child(self.icon_button(
                            "close-panel",
                            gpui_kit::assets::IconName::X,
                            i18n::tr("action-close"),
                            cx,
                            |_this, window, _cx| native_window::hide(window),
                        )),
                )
            })
            // Source input. The controls sit inside its bottom corners, so
            // the panel is one box and nothing else until there is a result.
            .child(
                div()
                    .relative()
                    .w_full()
                    // Slid back under the row, so the overlap that steadies the
                    // hover costs no visible space.
                    .mt(px(if chrome_visible() { -16. } else { 0. }))
                    // The textarea grows with its content up to five rows; a
                    // fixed height here let it paint over the row below, and
                    // clipping keeps that from ever happening again.
                    .overflow_hidden()
                    .border_1()
                    .border_color(border)
                    .rounded(px(16.))
                    .p(px(14.))
                    .child(
                        Textarea::new(&self.input)
                            .w_full()
                            .appearance(false)
                            .bordered(false)
                            // Room for the controls drawn over the corner.
                            .pb(px(34.)),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(10.))
                            .right(px(10.))
                            .bottom(px(8.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(self.task_picker(cx))
                            .child(div().flex_1())
                            .when(self.busy, |this| {
                                this.child(
                                    Button::new("stop")
                                        .icon(gpui_kit::assets::IconName::Square)
                                        .tooltip(i18n::tr("query-stop"))
                                        .on_click(cx.listener(|this, _event, _window, cx| {
                                            this.stop();
                                            cx.notify();
                                        })),
                                )
                            })
                            .child(
                                Button::new("look-up")
                                    .icon(gpui_kit::assets::IconName::Send)
                                    .tooltip(i18n::tr("query-run"))
                                    .loading(self.busy)
                                    .on_click(cx.listener(|this, _event, window, cx| {
                                        this.start_query(window, cx);
                                    })),
                            ),
                    ),
            )
            .when(self.has_result(), |this| {
                this
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
                            // Fades in as it arrives, so the card does not
                            // appear as a hard edge mid-stream.
                            .child(
                                TextView::markdown("answer", self.answer.clone()).with_animation(
                                    "answer-in",
                                    Animation::new(std::time::Duration::from_millis(200)),
                                    |view, delta| view.opacity(delta),
                                ),
                            ),
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
                                    .label(i18n::tr("query-send"))
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _event, window, cx| {
                                        this.send_follow_up(window, cx);
                                    })),
                            ),
                    )
            })
            .into_any_element()
    }

    /// Settings page. Only the font set is live so far: it is the one item the
    /// migration is required to expose as a choice, and it is verifiable from
    /// an offscreen render.
    fn settings_page(&self, cx: &Context<Self>) -> AnyElement {
        // Collected up front so the button closures do not borrow the config
        // while the layout is being built.
        let channels: Vec<(String, String, &'static str, bool)> = self
            .config
            .channels
            .iter()
            .map(|channel| {
                (
                    channel.id.clone(),
                    channel.name.clone(),
                    channel.kind.label_key(),
                    channel.kind.is_ai(),
                )
            })
            .collect();
        let ai_channels: Vec<(String, String)> = channels
            .iter()
            .filter(|(_, _, _, is_ai)| *is_ai)
            .map(|(id, name, _, _)| (id.clone(), name.clone()))
            .collect();
        let basic_active = self.config.basic_channel.clone();
        let ai_active = self.config.ai_channel.clone();
        let theme = cx.theme();
        let set = self.set;
        let fg = theme.foreground;
        let bg = theme.background;
        let muted = theme.muted_foreground;

        v_flex()
            .id("settings_scroll")
            .size_full()
            .bg(bg)
            .text_color(fg)
            .p(px(20.))
            .gap(px(14.))
            // The page is taller than the window: channels, their fields, the
            // used-for list and the permission report all live here.
            .overflow_y_scroll()
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
                            .icon(gpui_kit::assets::IconName::ArrowLeft)
                            .tooltip(i18n::tr("header-back"))
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
                TabBar::new("settings_tabs")
                    // The default filled-tab strip reads as a different design
                    // language; an underline row matches a flat settings page.
                    .underline()
                    .children([
                        Tab::new().label(i18n::tr("settings-tab-appearance")),
                        Tab::new().label(i18n::tr("settings-tab-shortcuts")),
                        Tab::new().label(i18n::tr("settings-tab-translation")),
                        Tab::new().label(i18n::tr("settings-tab-permissions")),
                    ])
                    .selected_index(self.settings_tab)
                    .on_click(cx.listener(|this, index: &usize, _window, cx| {
                        this.settings_tab = *index;
                        cx.notify();
                    })),
            )
            .when(self.settings_tab == 0, |this| {
                this.child(
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
                                .when(active, |button| button.primary())
                                .label(candidate.name)
                                .on_click(cx.listener(move |this, _event, _window, cx| {
                                    this.choose_font_set(id, cx);
                                }))
                        })),
                )
            })
            .when(self.settings_tab == 2, |this| {
                this.child(
                    v_flex()
                        .w_full()
                        .gap(px(6.))
                        .child(self.section_label(i18n::tr("channels-title"), set, muted))
                        .when(self.config.channels.is_empty(), |this| {
                            this.child(
                                div()
                                    .font_family(set.latin)
                                    .text_size(px(11.))
                                    .text_color(muted)
                                    .child(i18n::tr("channels-empty")),
                            )
                        })
                        // Each configured channel, with a way to remove it. A kind
                        // decides what a channel needs, so the label carries it.
                        .children(channels.iter().map(|(id, name, kind_key, _)| {
                            let edit_id = id.clone();
                            let remove_id = id.clone();
                            let test_id = id.clone();
                            let label = format!("{name} · {}", i18n::tr(kind_key));
                            // The test reports through a notification; its
                            // progress belongs on the button, not in a line.
                            let testing = self
                                .testing_channel
                                .as_deref()
                                .is_some_and(|tested| tested == id);
                            v_flex()
                                .w_full()
                                .gap(px(4.))
                                .child(
                                    h_flex()
                                        .w_full()
                                        .items_center()
                                        .gap(px(8.))
                                        .child(
                                            div()
                                                .flex_1()
                                                .font_family(set.latin)
                                                .text_size(px(12.))
                                                .child(label),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!("test-{id}")))
                                                .icon(IconName::PlugZap)
                                                .loading(testing)
                                                .tooltip(i18n::tr("channels-test"))
                                                .on_click(cx.listener(
                                                    move |this, _event, _window, cx| {
                                                        this.test_channel(test_id.clone(), cx);
                                                    },
                                                )),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!("edit-{id}")))
                                                .icon(IconName::Pencil)
                                                .tooltip(i18n::tr("channels-edit"))
                                                .on_click(cx.listener(
                                                    move |this, _event, window, cx| {
                                                        this.open_channel_dialog(
                                                            Some(edit_id.clone()),
                                                            window,
                                                            cx,
                                                        );
                                                    },
                                                )),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!("remove-{id}")))
                                                .icon(IconName::X)
                                                .tooltip(i18n::tr("channels-remove"))
                                                .on_click(cx.listener(
                                                    move |this, _event, _window, cx| {
                                                        this.remove_channel(&remove_id, cx);
                                                    },
                                                )),
                                        ),
                                )
                                .into_any_element()
                        }))
                        .child(
                            Button::new("add-channel")
                                .icon(IconName::Plus)
                                .label(i18n::tr("channels-add"))
                                .on_click(cx.listener(|this, _event, window, cx| {
                                    this.open_channel_dialog(None, window, cx);
                                })),
                        ),
                )
            })
            // Shortcuts: one editable field per global hotkey.
            .when(self.settings_tab == 1, |this| {
                this.child(
                    v_flex()
                        .w_full()
                        .gap(px(10.))
                        .child(
                            div()
                                .font_family(set.latin)
                                .text_size(px(11.))
                                .text_color(muted)
                                .child(i18n::tr("settings-section-shortcuts")),
                        )
                        .child(self.picker_row(
                            i18n::tr("settings-shortcut-blank"),
                            Input::new(&self.blank_hotkey).into_any_element(),
                            set,
                        ))
                        .child(self.picker_row(
                            i18n::tr("settings-shortcut-screenshot"),
                            Input::new(&self.screenshot_hotkey).into_any_element(),
                            set,
                        ))
                        .child(
                            div()
                                .font_family(set.latin)
                                .text_size(px(10.5))
                                .text_color(muted)
                                .child(i18n::tr("settings-shortcuts-hint")),
                        ),
                )
            })
            // Which channel serves which place, plus the copy behaviour.
            .when(self.settings_tab == 2, |this| {
                this.child(
                    v_flex()
                        .w_full()
                        .gap(px(10.))
                        .child(
                            h_flex()
                                .w_full()
                                .items_center()
                                .gap(px(10.))
                                .child(
                                    div()
                                        .flex_1()
                                        .font_family(set.latin)
                                        .text_size(px(12.))
                                        .child(i18n::tr("settings-snip-close-on-copy")),
                                )
                                .child(
                                    Switch::new("snip_close_on_copy")
                                        .checked(self.config.snip_close_on_copy)
                                        .on_click(cx.listener(
                                            |this, checked: &bool, _window, cx| {
                                                this.config.snip_close_on_copy = *checked;
                                                this.persist_config();
                                                cx.notify();
                                            },
                                        )),
                                ),
                        )
                        .child(self.section_label(i18n::tr("channels-usage"), set, muted))
                        .child(
                            self.picker_row(
                                i18n::tr("channels-basic"),
                                self.channel_picker(
                                    "basic_picker",
                                    channels
                                        .iter()
                                        .map(|(id, name, _, _)| (id.clone(), name.clone()))
                                        .collect(),
                                    basic_active.clone(),
                                    false,
                                    cx,
                                ),
                                set,
                            ),
                        )
                        .child(self.picker_row(
                            i18n::tr("channels-ai"),
                            self.channel_picker(
                                "ai_picker",
                                ai_channels,
                                ai_active.clone(),
                                true,
                                cx,
                            ),
                            set,
                        ))
                        .when(channels.is_empty(), |this| {
                            this.child(
                                div()
                                    .font_family(set.latin)
                                    .text_size(px(11.))
                                    .text_color(muted)
                                    .child(i18n::tr("channels-none")),
                            )
                        })
                        // The routing decision service: which model decides
                        // what a piece of content is asking for.
                        .child(self.section_label(i18n::tr("settings-decision"), set, muted))
                        .child(
                            h_flex()
                                .w_full()
                                .items_center()
                                .gap(px(10.))
                                .child(
                                    div()
                                        .flex_1()
                                        .font_family(set.latin)
                                        .text_size(px(12.))
                                        .child(i18n::tr("settings-decision-enabled")),
                                )
                                .child(
                                    Switch::new("decision_enabled")
                                        .checked(self.config.decision.enabled)
                                        .on_click(cx.listener(
                                            |this, checked: &bool, _window, cx| {
                                                this.config.decision.enabled = *checked;
                                                this.persist_config();
                                                cx.notify();
                                            },
                                        )),
                                ),
                        )
                        .child(self.picker_row(
                            i18n::tr("channels-endpoint"),
                            Input::new(&self.decision_endpoint).into_any_element(),
                            set,
                        ))
                        .child(self.picker_row(
                            i18n::tr("channels-model"),
                            Input::new(&self.decision_model).into_any_element(),
                            set,
                        ))
                        .child(self.picker_row(
                            i18n::tr("channels-key"),
                            Input::new(&self.decision_key).into_any_element(),
                            set,
                        )),
                )
            })
            .when(self.settings_tab == 3, |this| {
                this.child(
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
                        // One row per permission with a status chip, rather
                        // than a sentence with a dash in it.
                        .children(self.permissions.iter().map(|(key, granted)| {
                            let (background, foreground) = if *granted {
                                (theme.success, theme.success_foreground)
                            } else {
                                (theme.danger, theme.danger_foreground)
                            };
                            h_flex()
                                .w_full()
                                .items_center()
                                .gap(px(8.))
                                .child(
                                    // Name on top, purpose underneath in the
                                    // muted colour: a parenthetical would make
                                    // one dense line instead of a readable pair.
                                    v_flex()
                                        .flex_1()
                                        .gap(px(1.))
                                        .child(
                                            div()
                                                .font_family(set.latin)
                                                .text_size(px(12.))
                                                .child(i18n::tr(key)),
                                        )
                                        .child(
                                            div()
                                                .font_family(set.latin)
                                                .text_size(px(10.5))
                                                .text_color(muted)
                                                .child(i18n::tr(&format!("{key}-purpose"))),
                                        ),
                                )
                                .child(
                                    div()
                                        .px(px(8.))
                                        .py(px(2.))
                                        .rounded(px(999.))
                                        .bg(background)
                                        .text_color(foreground)
                                        .text_size(px(11.))
                                        .child(i18n::tr(if *granted {
                                            "settings-permission-granted"
                                        } else {
                                            "settings-permission-denied"
                                        })),
                                )
                                .into_any_element()
                        })),
                )
            })
            .into_any_element()
    }
}

/// Renders a dictionary hit: headword, phonetic,
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

/// Renders the panel offscreen to a PNG. No window is shown, so this is safe to run
/// unattended and gives the Markdown answer a visual check without credentials.
fn render_preview(path: &str) -> anyhow::Result<()> {
    let mut cx = gpui_kit::HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        // `AllAssets` is the full Lucide catalog; the default `Assets` only
        // embeds the icons the component library itself uses, so an app icon
        // like `Pin` renders as nothing with it.
        std::sync::Arc::new(gpui_kit::assets::AllAssets),
        gpui_kit::platform::current_headless_renderer,
    );

    let page = std::env::var("PEEK_PAGE").unwrap_or_default();
    // The overlay preview needs a desktop-sized canvas; the panel is a popup.
    let preview_size = if page == "snip" {
        size(px(1400.), px(900.))
    } else {
        size(px(480.), px(560.))
    };
    // The component layer must be initialised before a window opens: it
    // registers per-window state that notifications and dialogs look up. The
    // running app already does this; the preview did it too late, which only
    // showed up once a preview pushed a notification.
    {
        let mut app = cx.app.borrow_mut();
        gpui_kit::init(&mut app);
    }
    // The panel view is created inside the closure but a preview may need to
    // drive it (the channel dialog) once the window has painted.
    let published: std::rc::Rc<std::cell::RefCell<Option<Entity<Peek>>>> = Default::default();
    let publish_target = published.clone();
    let window = cx.open_window(preview_size, |window, cx| {
        if let Err(err) = fonts::register(cx) {
            eprintln!("font registration failed: {err}");
        }
        let missing = fonts::missing(cx);
        if !missing.is_empty() {
            eprintln!("bundled fonts MISSING: {}", missing.join(", "));
        }
        fonts::apply(cx, fonts::initial());
        // A preview needs no external actions; the sender is kept alive so the
        // receiver does not disconnect.
        let (_preview_tx, preview_rx) = std::sync::mpsc::channel::<Action>();
        if page == "snip" {
            let view = cx.new(snip::Snip::preview);
            return cx.new(|cx| gpui_kit::base::Root::new(view, window, cx));
        }
        let view = cx.new(|cx| Peek::new(window, cx, _preview_tx, preview_rx));
        *publish_target.borrow_mut() = Some(view.clone());
        view.update(cx, |peek, cx| {
            // In-memory only: a preview must never overwrite the real choice.

            match page.as_str() {
                // Not persisted: a preview must not overwrite the real choice.
                "settings" => {
                    peek.settings = true;
                    if let Ok(tab) = std::env::var("PEEK_TAB")
                        && let Ok(index) = tab.parse()
                    {
                        peek.settings_tab = index;
                    }
                }
                // The default state: only the input box, nothing else.
                "compact" => {
                    if std::env::var("PEEK_CHROME").is_ok() {
                        set_chrome_visible(true);
                    }
                }
                _ => {
                    peek.answer = SAMPLE_ANSWER.into();
                    // No status: a finished turn shows its answer and nothing
                    // else, which is what the preview should mirror.
                    peek.status.clear();
                }
            }
            cx.notify();
        });
        cx.new(|cx| Root::new(view, window, cx))
    })?;

    cx.run_until_parked();
    // The channel dialog is opened by a click, so a preview opens it directly.
    // It has to wait for the first frame: the component layer registers the
    // per-window state dialogs look up while painting.
    if std::env::var("PEEK_DIALOG").is_ok()
        && let Some(view) = published.borrow().clone()
    {
        cx.update_window(window.into(), |_, window, cx| {
            view.update(cx, |peek, cx| peek.open_channel_dialog(None, window, cx));
        })?;
        cx.run_until_parked();
    }
    // A notification needs the window's root, which the component layer
    // registers on the first frame, so the push has to wait for one. This is
    // only a preview aid: real pushes come from clicks, long after first paint.
    if std::env::var("PEEK_NOTIFY").is_ok() {
        cx.update_window(window.into(), |_, window, cx| {
            window.push_notification(
                // The same shape a failed turn raises, so a preview checks the
                // path the user actually sees.
                Notification::error(format!(
                    "{} {}",
                    i18n::tr("status-failed"),
                    "HTTP 404 · https://router.bloret.net/chat/completions"
                )),
                cx,
            );
        })?;
        // The toast animates in, so the clock has to move for it to be opaque.
        for _ in 0..30 {
            cx.advance_clock(std::time::Duration::from_millis(80));
            cx.run_until_parked();
        }
    }
    // With PEEK_FRAME set, the window frame is printed so a layout change can be
    // checked against where the input box actually sits on screen: a capture
    // shows the contents, not their place on the display.
    if std::env::var("PEEK_FRAME").is_ok() {
        cx.update_window(window.into(), |_, window, _| {
            let bounds = window.bounds();
            println!(
                "[frame] origin=({:.0},{:.0}) size=({:.0}x{:.0})",
                bounds.origin.x.as_f32(),
                bounds.origin.y.as_f32(),
                bounds.size.width.as_f32(),
                bounds.size.height.as_f32()
            );
        })?;
    }
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
            size: size(
                px(PANEL_WIDTH),
                px(PANEL_COMPACT_BASE + PANEL_INPUT_ROW * 2.),
            ),
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

    // An asset source must be attached explicitly: `application()` only builds
    // the platform, so without this the icon font never loads and every `Icon`
    // renders as nothing. The offscreen preview passes the same assets, which is
    // why icons appeared there and not in the running app.
    // Filled in once the panel exists: `on_reopen` is an `Application` hook and
    // so cannot capture the window handle directly.
    let reopen_target: std::rc::Rc<std::cell::Cell<Option<AnyWindowHandle>>> =
        std::rc::Rc::new(std::cell::Cell::new(None));
    let reopen_for_hook = reopen_target.clone();

    let application = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);
    // Double-clicking the Dock icon of a pinned window brings the panel back;
    // without this the icon appears to do nothing. `on_reopen` borrows the
    // application, so it is a statement rather than part of the chain.
    application.on_reopen(move |app| {
        if let Some(handle) = reopen_for_hook.get() {
            app.update_window(handle, |_, window, _| show_panel(window))
                .ok();
        }
    });
    application.run(move |cx| {
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

        // One action channel, owned here: the selection hook, the tray menu and
        // the global hotkeys all feed it, and the panel view drains it.
        let (action_tx, action_rx) = std::sync::mpsc::channel::<Action>();
        peek_runtime::selection::listen(
            store::load().unwrap_or_default().double_ctrl_ms,
            action_tx.clone(),
            // The view polls the channel, so the wake callback is a no-op.
            std::sync::Arc::new(|| {}),
        );
        let tray_tx = action_tx.clone();
        let view_tx = action_tx.clone();

        let handle = match gpui_kit::open_window(window_options(), cx, move |window, cx| {
            let view = cx.new(|cx| Peek::new(window, cx, view_tx, action_rx));
            cx.new(|cx| Root::new(view, window, cx))
        }) {
            Ok((handle, _)) => handle,
            Err(err) => {
                eprintln!("open_window failed: {err}");
                cx.quit();
                return;
            }
        };
        reopen_target.set(Some(handle));
        // Peek mode is an accessory app: no Dock icon and no menu bar. GPUI
        // switches to Regular whenever a window opens, so this has to run
        // after the panel exists; the pin button switches back for a
        // regular window, and unpinning restores this.
        cx.set_activation_policy(ActivationPolicy::Accessory);

        // Tray icon — installed here, see the ordering note at the top.
        let menu = Menu::new();
        let show_item = MenuItem::new(i18n::tr("tray-open"), true, None);
        let snip_item = MenuItem::new(i18n::tr("tray-screenshot"), true, None);
        let quit_item = MenuItem::new(i18n::tr("tray-quit"), true, None);
        let show_id = show_item.id().clone();
        let snip_id = snip_item.id().clone();
        let quit_id = quit_item.id().clone();
        let _ = menu.append(&show_item);
        let _ = menu.append(&snip_item);
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

        // Both hotkeys come from the configuration, so they can be changed in
        // settings without rebuilding. A string the platform cannot parse is
        // reported and skipped rather than stopping the app.
        let hotkey_config = store::load().unwrap_or_default();
        let parsed: Vec<(&str, Option<HotKey>)> = [
            ("blank", hotkey_config.blank_hotkey.as_str()),
            ("screenshot", hotkey_config.screenshot_hotkey.as_str()),
        ]
        .into_iter()
        .map(|(name, text)| {
            let parsed = text.parse::<HotKey>().map_err(|err| {
                eprintln!("hotkey \"{text}\" for {name} is invalid: {err}");
                err
            });
            (name, parsed.ok())
        })
        .collect();
        let toggle_id = parsed
            .iter()
            .find(|(name, _)| *name == "blank")
            .and_then(|(_, hotkey)| hotkey.as_ref())
            .map(HotKey::id);
        let snip_hotkey_id = parsed
            .iter()
            .find(|(name, _)| *name == "screenshot")
            .and_then(|(_, hotkey)| hotkey.as_ref())
            .map(HotKey::id);
        match GlobalHotKeyManager::new() {
            Ok(manager) => {
                let mut registered = 0;
                for (name, hotkey) in parsed.into_iter().filter_map(|(n, h)| h.map(|h| (n, h))) {
                    match manager.register(hotkey) {
                        Ok(()) => registered += 1,
                        Err(err) => eprintln!("hotkey register failed for {name}: {err}"),
                    }
                }
                println!("{registered} global hotkey(s) registered");
                std::mem::forget(manager);
            }
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
                let mut screenshot = false;

                while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
                    if event.state != HotKeyState::Pressed {
                        continue;
                    }
                    if Some(event.id) == toggle_id {
                        toggle = true;
                    } else if Some(event.id) == snip_hotkey_id {
                        screenshot = true;
                    }
                }
                while let Ok(event) = MenuEvent::receiver().try_recv() {
                    if event.id == show_id {
                        toggle = true;
                    } else if event.id == snip_id {
                        screenshot = true;
                    } else if event.id == quit_id {
                        quit = true;
                    }
                }

                // The overlay is opened by the view, which owns that state, so
                // the request goes through the same channel as the hook's.
                if screenshot {
                    let _ = tray_tx.send(Action::Screenshot);
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
