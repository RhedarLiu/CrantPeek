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

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod fonts;
mod native_window;
mod snip;

use std::time::Duration;

// Needed for `on_hover` on an element that carries an id.
use gpui_kit::InteractiveElement as _;
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::base::{IndexPath, Root, StyledExt as _};
use gpui_kit::component::button::{Button, ButtonVariants as _, DropdownButton};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tray_icon::Icon as TrayIconImage;
use tray_icon::TrayIconBuilder;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};

use peek_core::{
    Channel, ChannelKind, Config, DeepSeekEffort, Message, Task, effective_target, local_route,
};
use peek_network::{Client, Event};
use peek_runtime::action::{Action, OverlayEvent};
use peek_runtime::{i18n, store};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Poll interval for the tray/hotkey channels. Both crates deliver events on
/// their own channels rather than through GPUI, so they are drained on a timer.
/// Which global hotkey an input edits.
#[derive(Clone, Copy)]
enum HotkeyField {
    Blank,
    Screenshot,
    Selection,
}

/// Which assignment a channel dropdown writes.
#[derive(Clone, Copy)]
enum ChannelSlot {
    QuickFirst,
    QuickAdd,
    Llm,
    Decision,
}

/// Panel geometry: compact until there is something to show, which keeps the
/// default state a bare input box instead of a mostly empty card stack.
const PANEL_WIDTH: f32 = 480.;
/// Shared right-hand control column for settings rows.
const SETTINGS_CONTROL_WIDTH: f32 = 260.;
/// Panel height with the input box at its two-row minimum and the title row
/// hidden: the padding and the input card, which carries its own controls.
const PANEL_COMPACT_BASE: f32 = 78.;
/// What the revealed title row adds: the extra top padding, the row and the gap
/// that follows it, less the padding the collapsed state already has.
const PANEL_CHROME_HEIGHT: f32 = 40.;
/// Top of the input box that opens the title row. A fixed band, so a taller
/// result does not make the trigger taller.
const INPUT_CORNER_HEIGHT: f32 = 46.;
/// How far past that corner the pointer must travel before the row hides.
/// Wider than the jitter at the boundary, so the edge does not flap.
const INPUT_CORNER_SLOP: f32 = 24.;
/// Added per input row, so a longer draft grows the panel instead of spilling
/// outside the input card.
const PANEL_INPUT_ROW: f32 = 22.;
/// A result card taller than this would push the panel past a comfortable
/// reading window; the card scrolls beyond it.
const PANEL_RESULT_MAX_HEIGHT: f32 = 620.;
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
    /// Whether the frame currently on screen already includes the title row.
    ///
    /// The pointer's window coordinates only shift once that frame is up, so the
    /// trigger is measured against this rather than against the row we want.
    static CHROME_IN_FRAME: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// When the row last opened or closed. The frame change reports a pointer
    /// leave; samples in this window must not flip the row back.
    static CHROME_CHANGED_AT: std::cell::Cell<Option<std::time::Instant>> =
        const { std::cell::Cell::new(None) };
    /// Pinned windows keep the title row up. A hide request is ignored.
    static CHROME_LOCKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// How long a show or hide is allowed to settle before the pointer may flip it.
const CHROME_SETTLE: Duration = Duration::from_millis(180);

/// Whether the title row is showing.
fn chrome_visible() -> bool {
    CHROME_VISIBLE.with(std::cell::Cell::get)
}

/// Shows or hides the title row. A pinned window refuses to hide it.
fn set_chrome_visible(visible: bool) {
    if !visible && CHROME_LOCKED.with(std::cell::Cell::get) {
        return;
    }
    let changed = CHROME_VISIBLE.with(|cell| {
        if cell.get() == visible {
            return false;
        }
        cell.set(visible);
        true
    });
    if changed {
        CHROME_CHANGED_AT.with(|cell| cell.set(Some(std::time::Instant::now())));
    }
}

fn set_chrome_locked(locked: bool) {
    CHROME_LOCKED.with(|cell| cell.set(locked));
}

/// The frame is still moving after a show or hide. Pointer samples in this
/// span are the move itself, not the user crossing the corner.
fn chrome_settling() -> bool {
    CHROME_CHANGED_AT.with(|cell| cell.get().is_some_and(|at| at.elapsed() < CHROME_SETTLE))
}

/// Shows the panel and records when, so focus loss is ignored briefly.
///
/// The title row is hidden on the way in: a panel reopened after the pointer
/// found it once must look the same as one opened for the first time.
fn show_panel(window: &mut Window) {
    // A pinned window keeps its title row. An unpinned one opens bare.
    if !CHROME_LOCKED.with(std::cell::Cell::get) {
        set_chrome_visible(false);
        CHROME_IN_FRAME.with(|cell| cell.set(false));
    }
    native_window::show(window);
    SHOWN_AT.with(|cell| cell.set(Some(std::time::Instant::now())));
}

fn set_chrome_in_frame(included: bool) {
    CHROME_IN_FRAME.with(|cell| cell.set(included));
}

/// Whether the panel was shown recently enough to ignore a focus loss.
fn shown_recently() -> bool {
    SHOWN_AT.with(|cell| {
        cell.get()
            .is_some_and(|shown| shown.elapsed() < FOCUS_GRACE)
    })
}

/// Right half of the top of the input box, in window coordinates.
///
/// `input_top` is where that box starts in the frame that is actually on
/// screen. `open` widens the box so a pointer sitting on the boundary does not
/// hide the row and immediately show it again. The panel's height is not an
/// input: a long result must not grow the corner.
fn chrome_zone_contains(x: f32, y: f32, width: f32, input_top: f32, open: bool) -> bool {
    if width <= 1. {
        return false;
    }
    // The title row sits above the input once the frame includes it. The whole
    // row stays live, including the buttons on the left of the corner.
    if open && input_top > 0. && y <= input_top {
        return true;
    }
    let mut left = width * 0.5;
    let mut top = input_top;
    let mut bottom = input_top + INPUT_CORNER_HEIGHT;
    if open {
        left -= INPUT_CORNER_SLOP;
        bottom += INPUT_CORNER_SLOP;
        top = (input_top - 6.).max(0.);
    }
    x >= left && y >= top && y < bottom
}

/// Whether the pointer is in the corner that keeps the title row open.
fn pointer_in_chrome_zone(position: Point<Pixels>, window: &Window) -> bool {
    let width = window.bounds().size.width.as_f32();
    let input_top = if CHROME_IN_FRAME.with(std::cell::Cell::get) {
        PANEL_CHROME_HEIGHT
    } else {
        0.
    };
    chrome_zone_contains(
        position.x.as_f32(),
        position.y.as_f32(),
        width,
        input_top,
        chrome_visible(),
    )
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
    follow_up: Entity<TextareaState>,
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
    /// The in-flight turn is a follow-up, so its spinner belongs on that button.
    follow_up_turn: bool,
    /// The task list is open. The compact panel grows so the list is not cut off.
    task_menu_open: bool,
    /// Cleared on the next frame. See `send_follow_up`.
    clear_follow_up: bool,
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
    /// The conversation as bubbles: `true` for the user's own turn.
    ///
    /// Kept apart from `messages`, which is what the API is sent: that history
    /// carries the system instruction and wraps the first user turn as
    /// untrusted content, neither of which belongs on screen.
    bubbles: Vec<(bool, String)>,
    /// Channel a connection test is running for, so its row can show progress
    /// on the button rather than an extra line of text.
    testing_channel: Option<String>,

    /// Sends test results from the network task to the poll loop: the channel
    /// id, whether it failed, and the message to show.
    test_tx: std::sync::mpsc::Sender<(String, bool, String)>,
    /// Sends a decided task back from the decision service: the epoch it belongs
    /// to, the text it was decided for, and the task.
    route_tx: std::sync::mpsc::Sender<(u64, String, Task)>,
    /// Bumped whenever a newer query or a stop should discard an in-flight decision.
    decision_epoch: u64,
    decision_cancel: Option<CancellationToken>,
    /// Identifies the drain loop that owns `receiver`. Older loops exit.
    turn_id: u64,

    /// Editable global hotkeys, persisted as they are typed.
    blank_hotkey: Entity<InputState>,
    screenshot_hotkey: Entity<InputState>,
    selection_hotkey: Entity<InputState>,
    /// Channel being edited, or `None` when the dialog is adding one.
    editing_channel: Option<String>,
    /// Draft fields for the channel dialog.
    channel_name: Entity<InputState>,
    channel_endpoint: Entity<InputState>,
    channel_model: Entity<InputState>,
    channel_tokens: Entity<InputState>,
    channel_key: Entity<InputState>,
    channel_kind: Entity<SelectState<Vec<SharedString>>>,
    channel_effort: Entity<SelectState<Vec<SharedString>>>,
    /// Which settings tab is showing.
    settings_tab: usize,
    /// Laid-out height of the settings page, measured from its children.
    settings_measured: f32,
    /// Actual input card height, including wrapped text and corner controls.
    input_measured: f32,
    answer_measured: f32,
    query_measured: f32,
    follow_height: f32,
    dialog_measured: f32,
    /// Vertical space currently added above the input by the toolbar.
    chrome_offset: f32,
    geometry_pending: bool,
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

        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 5)
                .submit_on_enter(true)
        });
        // Same editor as the query box, including IME composition and submit.
        let follow_up = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 4)
                .submit_on_enter(true)
                .placeholder(i18n::tr("query-followup-placeholder"))
        });
        // The quick panel is nonactivating. ASCII key events still reach it,
        // but macOS can leave the input-source switcher attached to the prior
        // app. Start a native text session when the user focuses follow-up.
        cx.on_focus(
            &follow_up.read(cx).focus_handle(cx),
            window,
            |this, window, cx| {
                cx.activate(true);
                SHOWN_AT.with(|cell| cell.set(Some(std::time::Instant::now())));
                native_window::show(window);
                let focus = this.follow_up.read(cx).focus_handle(cx);
                window.on_next_frame(move |window, cx| {
                    if focus.is_focused(window) {
                        native_window::activate_text_input(window, cx);
                    }
                });
            },
        )
        .detach();
        let blank_hotkey = cx.new(|cx| InputState::new(window, cx));
        let screenshot_hotkey = cx.new(|cx| InputState::new(window, cx));
        let selection_hotkey = cx.new(|cx| InputState::new(window, cx));
        // Draft fields for adding a channel. One set is enough: the fields a
        // kind needs are shown or hidden as the type changes.
        let channel_name = cx.new(|cx| InputState::new(window, cx));
        let channel_endpoint = cx.new(|cx| InputState::new(window, cx));
        let channel_model = cx.new(|cx| InputState::new(window, cx));
        let channel_tokens = cx.new(|cx| InputState::new(window, cx));
        let channel_key = cx.new(|cx| InputState::new(window, cx).masked(true));
        let kinds: Vec<SharedString> = ChannelKind::ALL
            .iter()
            .map(|kind| i18n::tr(kind.label_key()).into())
            .collect();
        let channel_kind =
            cx.new(|cx| SelectState::new(kinds, Some(IndexPath::new(0)), window, cx));

        let efforts: Vec<SharedString> = DeepSeekEffort::ALL
            .iter()
            .map(|effort| i18n::tr(effort.label_key()).into())
            .collect();
        let channel_effort =
            cx.new(|cx| SelectState::new(efforts, Some(IndexPath::new(0)), window, cx));
        cx.subscribe_in(
            &channel_kind,
            window,
            |this, _, _: &SelectEvent<Vec<SharedString>>, window, cx| {
                if this.draft_kind(cx) == ChannelKind::DeepSeek {
                    for (field, value) in [
                        (&this.channel_name, "DeepSeek"),
                        (&this.channel_endpoint, "https://api.deepseek.com"),
                        (&this.channel_model, "deepseek-flash"),
                    ] {
                        if field.read(cx).value().trim().is_empty() {
                            field.update(cx, |state, cx| state.set_value(value, window, cx));
                        }
                    }
                }
                cx.notify();
            },
        )
        .detach();

        let (test_tx, test_rx) = std::sync::mpsc::channel();
        let (route_tx, route_rx) = std::sync::mpsc::channel();
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
        // Plain Enter sends. Shift+Enter is a newline, and a composing IME
        // consumes Enter before this event exists.
        cx.subscribe(&follow_up, |this, _input, event: &InputEvent, cx| {
            if let InputEvent::PressEnter { secondary, shift } = event
                && !*secondary
                && !*shift
            {
                this.send_follow_up(cx);
                cx.notify();
            }
        })
        .detach();
        let config = store::load().unwrap_or_default();
        i18n::set_language(&config.ui_language);
        for (input, value) in [
            (&blank_hotkey, &config.blank_hotkey),
            (&screenshot_hotkey, &config.screenshot_hotkey),
            (&selection_hotkey, &config.selection_hotkey),
        ] {
            let value = value.clone();
            input.update(cx, |state, cx| state.set_value(&value, window, cx));
        }
        // Persisted as they are typed. The running app keeps the hotkeys it
        // registered at startup, which is why the page says a restart applies
        // a change.
        for (input, which) in [
            (&blank_hotkey, HotkeyField::Blank),
            (&screenshot_hotkey, HotkeyField::Screenshot),
            (&selection_hotkey, HotkeyField::Selection),
        ] {
            cx.observe(input, move |this, state, cx| {
                let value = state.read(cx).value().trim().to_owned();
                let slot = match which {
                    HotkeyField::Blank => &mut this.config.blank_hotkey,
                    HotkeyField::Screenshot => &mut this.config.screenshot_hotkey,
                    HotkeyField::Selection => &mut this.config.selection_hotkey,
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
            settings_measured: 0.,
            pending_toast: None,
            bubbles: Vec::new(),
            testing_channel: None,
            test_tx,
            route_tx,
            decision_epoch: 0,
            decision_cancel: None,
            turn_id: 0,
            blank_hotkey,
            screenshot_hotkey,
            selection_hotkey,
            editing_channel: None,
            channel_name,
            channel_endpoint,
            channel_model,
            channel_tokens,
            channel_key,
            channel_kind,
            channel_effort,
            tasks,
            answer: String::new(),
            applied_height: None,
            input_measured: 0.,
            answer_measured: 0.,
            query_measured: 0.,
            follow_height: 38.,
            dialog_measured: 0.,
            chrome_offset: 0.,
            geometry_pending: false,
            pinned: false,
            overlay: None,
            pending_input: None,
            original_query: String::new(),
            status: String::new(),
            busy: false,
            follow_up_turn: false,
            task_menu_open: false,
            clear_follow_up: false,
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
                while let Ok((epoch, text, task)) = route_rx.try_recv() {
                    this.update(cx, |peek, cx| {
                        // A newer query, a stop, or a timeout already moved on.
                        if epoch != peek.decision_epoch {
                            return;
                        }
                        peek.decision_cancel = None;
                        peek.begin_turn_as(text, false, Some(task), cx);
                    })
                    .ok();
                }
                while let Ok((_id, failed, message)) = test_rx.try_recv() {
                    this.update(cx, |peek, cx| {
                        peek.testing_channel = None;
                        if failed {
                            peek.notify_error(message);
                        } else {
                            peek.notify_success(message);
                        }
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
                                Err(err) => {
                                    this.update(cx, |peek, cx| {
                                        peek.notify_error(i18n::diagnostic(&err));
                                        cx.notify();
                                    }).ok();
                                    let _ = cx.update_window(panel_handle, |_, window, _| show_panel(window));
                                }
                            }
                        }
                        Action::Translate { text, replies } => {
                            // The overlay owns this turn. Finish the panel turn
                            // first, then run the overlay without touching the
                            // panel transcript.
                            let started = this
                                .update(cx, |peek, cx| {
                                    peek.stop();
                                    peek.overlay = Some(replies);
                                    let started = peek.begin_isolated(text, cx);
                                    if !started {
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

        // Exercise native toolbar transitions without submitting any query.
        if std::env::var("PEEK_CHROME_SELFTEST").is_ok() {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(500)).await;
                cx.update_window(panel_handle, |_, window, _| show_panel(window)).ok();
                cx.background_executor().timer(Duration::from_millis(250)).await;
                let mut anchor: Option<f32> = None;
                let mut passed = true;
                for open in [false, true, false, true, false, true, false] {
                    this.update(cx, |_, cx| {
                        set_chrome_visible(open);
                        cx.notify();
                    }).ok();
                    cx.background_executor().timer(Duration::from_millis(250)).await;
                    let offset = this.update(cx, |peek, _| peek.chrome_offset).unwrap_or_default();
                    if let Ok((y, frame_height, viewport_height)) = cx.update_window(panel_handle, |_, window, _| {
                        (window.bounds().origin.y.as_f32() + offset,
                         window.bounds().size.height.as_f32(),
                         window.viewport_size().height.as_f32())
                    }) {
                        let baseline = *anchor.get_or_insert(y);
                        let stable = (y - baseline).abs() < 0.5
                            && (frame_height - viewport_height).abs() < 0.5
                            && (offset - if open { PANEL_CHROME_HEIGHT } else { 0. }).abs() < 0.5;
                        passed &= stable;
                        println!("[chrome-test] open={open} input_y={y:.1} frame={frame_height:.1} viewport={viewport_height:.1} stable={stable}");
                    } else {
                        passed = false;
                    }
                }
                println!("[chrome-test] passed={passed}");
                cx.update(quit_now);
            }).detach();
        }

        this
    }

    /// A follow-up has been sent, so the result is a conversation rather than one answer.
    fn conversation(&self) -> bool {
        self.bubbles.iter().filter(|(user, _)| *user).count() > 1
    }

    /// The single answer to show before any follow-up. Streaming text wins over
    /// the finished bubble.
    fn plain_answer(&self) -> Option<String> {
        if self.conversation() {
            return None;
        }
        if !self.answer.is_empty() {
            return Some(self.answer.clone());
        }
        self.bubbles
            .iter()
            .rev()
            .find(|(user, _)| !*user)
            .map(|(_, text)| text.clone())
    }

    /// Whether there is more to show than the input box.
    fn has_result(&self) -> bool {
        // Deliberately not `busy`: a running turn shows its progress on the
        // button, so an empty result card would be a large blank area.
        self.plain_answer().is_some() || self.conversation() || !self.dictionary_note.is_empty()
    }

    /// Panel height while there is a result, following how much has streamed in.
    ///
    /// Uses the laid-out Markdown or conversation, including the input and
    /// follow-up controls. Long answers scroll at the panel's reading limit.
    fn result_height(&self) -> f32 {
        self.query_measured
            .max(self.input_measured + 80.)
            .clamp(180., PANEL_RESULT_MAX_HEIGHT)
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
    ///
    /// The file is mapped once. Opening it on every lookup reread the index
    /// on the UI thread.
    fn lookup_word(&mut self, text: &str) -> Option<peek_dict::Entry> {
        if text.chars().count() > 40 || text.split_whitespace().count() != 1 {
            return None;
        }
        fn shared_dict() -> Option<&'static peek_dict::Dict> {
            static DICT: std::sync::OnceLock<Option<peek_dict::Dict>> = std::sync::OnceLock::new();
            DICT.get_or_init(|| {
                store::dictionary_candidates()
                    .into_iter()
                    .find_map(|path| peek_dict::Dict::open(&path).ok())
            })
            .as_ref()
        }
        shared_dict().and_then(|dict| dict.lookup(text))
    }

    /// Drops the network task and any decision that has not come back yet.
    fn stop_transport(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel();
        }
        self.receiver = None;
        self.busy = false;
        self.follow_up_turn = false;
        self.decision_epoch = self.decision_epoch.wrapping_add(1);
        if let Some(cancel) = self.decision_cancel.take() {
            cancel.cancel();
        }
    }

    /// A stopped or failed turn keeps a partial answer and drops an empty one,
    /// so the next request does not inherit an unanswered user message.
    fn settle_stopped_turn(&mut self) {
        if !self.answer.is_empty() {
            self.messages.push(Message {
                role: "assistant".into(),
                content: self.answer.clone(),
            });
            self.bubbles.push((false, std::mem::take(&mut self.answer)));
            peek_core::bound_history(&mut self.messages);
            return;
        }
        peek_core::discard_pending_turn(&mut self.messages);
        if self.bubbles.last().is_some_and(|(user, _)| *user) {
            self.bubbles.pop();
        }
    }

    /// Cancels the in-flight turn. An overlay turn does not rewrite the panel.
    fn stop(&mut self) {
        let overlay = self.overlay.take();
        self.stop_transport();
        if let Some(tx) = overlay {
            let _ = tx.send(OverlayEvent::Finished);
            return;
        }
        self.settle_stopped_turn();
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
        if text.is_empty() {
            return;
        }
        if text.len() > peek_core::MAX_INPUT_BYTES {
            self.notify_error(i18n::tr("status-input-too-long"));
            cx.notify();
            return;
        }
        // Automatic, with a decision service configured, asks it what this is;
        // a failed call, a low confidence or no service falls back to the local
        // rules, which need no network at all.
        let automatic = self.selected_task(cx).is_none();
        let Some(channel) = self.config.decision_service().cloned() else {
            self.begin_turn(text, false, cx);
            return;
        };
        if !automatic {
            self.begin_turn(text, false, cx);
            return;
        }
        let min_confidence = self.config.decision.min_confidence;
        let client = self.client.clone();
        let tx = self.route_tx.clone();
        let fallback = local_route(
            &text,
            &self.config.target_language,
            &self.config.chinese_target,
        )
        .task;
        // The previous turn is finished first, and this decision gets the epoch
        // that `stop_transport` just published. A later query bumps it again.
        self.stop();
        let epoch = self.decision_epoch;
        let cancel = CancellationToken::new();
        self.decision_cancel = Some(cancel.clone());
        self.busy = true;
        let timeout = std::time::Duration::from_millis(self.config.decision.timeout_ms.max(4000));
        let endpoint = channel.endpoint.clone();
        let key = channel.api_key.clone();
        let model = channel.model.clone();
        cx.notify();
        self.runtime.spawn(async move {
            let chosen = match client
                .decide(&endpoint, &key, &model, &text, timeout, cancel)
                .await
            {
                Ok(answer) if answer.confidence >= min_confidence => answer.task,
                _ => fallback,
            };
            let _ = tx.send((epoch, text, chosen));
        });
    }

    /// Height for the settings page's current tab.
    ///
    /// The number comes from the laid-out children, so a short tab does not
    /// keep a tall empty area and a longer one is not clipped. The first frame
    /// uses a stand-in until that measurement arrives.
    fn settings_height(&self) -> f32 {
        let height = if self.settings_measured > 1. {
            self.settings_measured
        } else {
            420.
        };
        height.clamp(220., 900.)
    }

    /// Settings labels share the left column; controls have a fixed width
    /// aligned with the right edge, independent of the translated label.
    fn picker_row(
        &self,
        label: String,
        picker: impl IntoElement,
        set: &'static crate::fonts::FontSet,
    ) -> AnyElement {
        h_flex()
            .w_full()
            .min_h(px(36.))
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .font_family(set.latin)
                    .text_size(px(12.))
                    .child(label),
            )
            .child(
                div()
                    .w(px(SETTINGS_CONTROL_WIDTH))
                    .flex_none()
                    .child(picker),
            )
            .into_any_element()
    }

    /// A dropdown of channels. Only channels the caller offers appear, so a
    /// translation-only channel is never listed for AI work.
    fn channel_picker(
        &self,
        id: &'static str,
        choices: Vec<(String, String)>,
        active: String,
        slot: ChannelSlot,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        let current = choices
            .iter()
            .find(|(channel, _)| *channel == active)
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| i18n::tr("channels-none"));
        DropdownButton::new(id)
            .w_full()
            .button(
                // The dropdown draws its own chevron beside the button, so the
                // button sizes to the row instead of claiming the full width.
                Button::new(SharedString::from(format!("{id}_button")))
                    .label(current)
                    .flex_1(),
            )
            .dropdown_menu(move |mut menu, _window, _cx| {
                let target = entity.clone();
                let none = active.is_empty();
                menu = menu.item(
                    PopupMenuItem::new(i18n::tr("channels-none"))
                        .checked(none)
                        .on_click(move |_event, _window, cx| {
                            target.update(cx, |peek, cx| {
                                peek.assign_channel(slot, String::new(), cx);
                            });
                        }),
                );
                for (channel, name) in &choices {
                    let target = entity.clone();
                    let chosen = channel.clone();
                    let selected = *channel == active;
                    menu = menu.item(PopupMenuItem::new(name.clone()).checked(selected).on_click(
                        move |_event, _window, cx| {
                            target.update(cx, |peek, cx| {
                                peek.assign_channel(slot, chosen.clone(), cx);
                            });
                        },
                    ));
                }
                menu
            })
    }

    fn move_quick_channel(&mut self, id: &str, direction: isize, cx: &mut Context<Self>) {
        if let Some(ids) = &mut self.config.quick_channels
            && let Some(index) = ids.iter().position(|old| old == id)
            && let Some(next) = index.checked_add_signed(direction)
            && next < ids.len()
        {
            ids.swap(index, next);
            self.persist_config();
            cx.notify();
        }
    }

    fn quick_channel_list(&self, cx: &Context<Self>) -> AnyElement {
        let ids = self.config.quick_channels.as_deref().unwrap_or_default();
        let choices = self
            .config
            .quick_candidates()
            .filter(|c| !ids.contains(&c.id))
            .map(|c| (c.id.clone(), c.name.clone()))
            .collect();
        v_flex()
            .w_full()
            .gap(px(8.))
            .child(self.section_label(
                i18n::tr("channels-quick"),
                self.set,
                cx.theme().muted_foreground,
            ))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(cx.theme().muted_foreground)
                    .child(i18n::tr("channels-quick-hint")),
            )
            .child(
                v_flex()
                    .id("quick-priority-list")
                    .w_full()
                    .max_h(px(240.))
                    .overflow_y_scroll()
                    .gap(px(4.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(px(10.))
                    .p(px(8.))
                    .when(ids.is_empty(), |this| {
                        this.child(div().text_size(px(12.)).child(i18n::tr("channels-none")))
                    })
                    .children(ids.iter().enumerate().filter_map(|(index, id)| {
                        let channel = self.config.channel(id)?;
                        let up = id.clone();
                        let down = id.clone();
                        let remove = id.clone();
                        Some(
                            h_flex()
                                .w_full()
                                .flex_none()
                                .items_center()
                                .gap(px(6.))
                                .child(div().flex_1().text_size(px(12.)).child(format!(
                                    "{}. {}",
                                    index + 1,
                                    channel.name
                                )))
                                .child(
                                    Button::new(SharedString::from(format!("quick-up-{id}")))
                                        .ghost()
                                        .icon(IconName::ArrowUp)
                                        .disabled(index == 0)
                                        .tooltip(i18n::tr("channels-up"))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.move_quick_channel(&up, -1, cx)
                                        })),
                                )
                                .child(
                                    Button::new(SharedString::from(format!("quick-down-{id}")))
                                        .ghost()
                                        .icon(IconName::ArrowDown)
                                        .disabled(index + 1 == ids.len())
                                        .tooltip(i18n::tr("channels-down"))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.move_quick_channel(&down, 1, cx)
                                        })),
                                )
                                .child(
                                    Button::new(SharedString::from(format!("quick-remove-{id}")))
                                        .ghost()
                                        .icon(IconName::X)
                                        .tooltip(i18n::tr("channels-remove-quick"))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(ids) = &mut this.config.quick_channels {
                                                ids.retain(|id| id != &remove);
                                            }
                                            this.persist_config();
                                            cx.notify();
                                        })),
                                ),
                        )
                    })),
            )
            .child(self.picker_row(
                i18n::tr("channels-add-quick"),
                self.channel_picker(
                    "quick-add",
                    choices,
                    String::new(),
                    ChannelSlot::QuickAdd,
                    cx,
                ),
                self.set,
            ))
            .into_any_element()
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

        let tokens = existing
            .as_ref()
            .map(|c| c.max_output_tokens)
            .unwrap_or(peek_core::DEFAULT_OUTPUT_TOKENS)
            .to_string();
        self.channel_tokens
            .update(cx, |state, cx| state.set_value(tokens, window, cx));
        self.dialog_measured = 0.;
        let effort = existing
            .as_ref()
            .map(|c| c.reasoning_effort)
            .unwrap_or_default();
        let effort_index = DeepSeekEffort::ALL
            .iter()
            .position(|candidate| *candidate == effort)
            .unwrap_or(0);
        self.channel_effort.update(cx, |state, cx| {
            state.set_selected_index(Some(IndexPath::new(effort_index)), window, cx);
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
                .margin_top(px(16.))
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
                            .on_children_prepainted({
                                let entity = content_entity.clone();
                                move |bounds, _window, cx| {
                                    let height =
                                        bounds.iter().map(|b| b.size.height.as_f32()).sum::<f32>()
                                            + 10. * bounds.len().saturating_sub(1) as f32
                                            + 112.;
                                    let entity = entity.clone();
                                    cx.defer(move |cx| {
                                        entity.update(cx, |this, cx| {
                                            if (this.dialog_measured - height).abs() > 0.5 {
                                                this.dialog_measured = height;
                                                cx.notify();
                                            }
                                        })
                                    });
                                }
                            })
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
                            .when(kind != ChannelKind::GoogleFree, |this| {
                                this.child(field(
                                    i18n::tr("channels-endpoint"),
                                    Input::new(&peek.channel_endpoint).into_any_element(),
                                ))
                            })
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
                            })
                            .when(kind.is_ai(), |this| {
                                this.child(field(
                                    i18n::tr("settings-max-tokens"),
                                    Input::new(&peek.channel_tokens).into_any_element(),
                                ))
                            })
                            .when(kind == ChannelKind::DeepSeek, |this| {
                                this.child(field(
                                    i18n::tr("channels-reasoning-effort"),
                                    Select::new(&peek.channel_effort)
                                        .id("channel-effort-dialog")
                                        .into_any_element(),
                                ))
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(muted)
                                        .child(i18n::tr("channels-reasoning-hint")),
                                )
                            })
                            .when(kind.supports_decision(), |this| {
                                this.child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(muted)
                                        .child(i18n::tr("channels-decision-hint")),
                                )
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
                                    let saved = ok_entity
                                        .update(cx, |peek, cx| peek.save_channel(window, cx));
                                    if saved {
                                        window.close_dialog(cx);
                                    }
                                }),
                        ),
                )
        });
    }

    /// Saves the dialog's channel, adding it or updating the one being edited.
    ///
    /// A kind declares what it needs: an AI kind takes an endpoint, a model and
    /// a credential, while DeepLX takes an endpoint only. Returns whether the
    /// dialog can close. A rejected draft stays open.
    fn save_channel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let kind = self.draft_kind(cx);
        let name = self.channel_name.read(cx).value().trim().to_owned();
        let endpoint = if kind == ChannelKind::GoogleFree {
            "https://translate.googleapis.com/translate_a/single".to_owned()
        } else {
            self.channel_endpoint.read(cx).value().trim().to_owned()
        };
        let model = self.channel_model.read(cx).value().trim().to_owned();
        let key = self.channel_key.read(cx).value().to_string();
        if name.is_empty() || endpoint.is_empty() {
            self.notify_error(i18n::tr("status-channel-incomplete"));
            cx.notify();
            return false;
        }
        if kind.needs_model() && model.is_empty() {
            self.notify_error(i18n::tr("error-config-channel-model"));
            cx.notify();
            return false;
        }
        if peek_core::validate_endpoint(&endpoint, true).is_err() {
            self.notify_error(i18n::tr("error-config-endpoint"));
            cx.notify();
            return false;
        }

        let max_output_tokens = if kind.is_ai() {
            match self.channel_tokens.read(cx).value().trim().parse::<u32>() {
                Ok(value) if (128..=peek_core::MAX_OUTPUT_TOKENS).contains(&value) => value,
                _ => {
                    self.notify_error(i18n::tr("error-config-tokens"));
                    cx.notify();
                    return false;
                }
            }
        } else {
            peek_core::DEFAULT_OUTPUT_TOKENS
        };
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
        let old_config = self.config.clone();
        let existing = editing.as_deref().and_then(|id| self.config.channel(id));
        let saved = Channel {
            id: id.clone(),
            name,
            kind,
            endpoint,
            model,
            api_key,
            credential_id: existing
                .map(|c| c.credential_id.clone())
                .unwrap_or_default(),
            vision: kind.is_ai() && existing.is_some_and(|c| c.vision),
            max_output_tokens,
            reasoning_effort: if kind == ChannelKind::DeepSeek {
                self.channel_effort
                    .read(cx)
                    .selected_index(cx)
                    .and_then(|index| DeepSeekEffort::ALL.get(index.row))
                    .copied()
                    .unwrap_or_default()
            } else {
                DeepSeekEffort::Off
            },
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
                // The first usable channel becomes the default for a place
                // that has nothing chosen yet. A decision model is not a
                // translator and not an LLM.
                if kind.is_ai() || kind.supports_basic() {
                    self.config
                        .quick_channels
                        .get_or_insert_with(Vec::new)
                        .push(id.clone());
                }
                if self.config.ai_channel.is_empty() && kind.is_ai() {
                    self.config.ai_channel = id.clone();
                }
                if self.config.decision_channel.is_empty() && kind.supports_decision() {
                    self.config.decision_channel = id;
                }
            }
        }
        self.config.repair_channel_assignments();
        if !self.persist_config() {
            self.config = old_config;
            cx.notify();
            return false;
        }
        self.editing_channel = None;
        for input in [
            &self.channel_name,
            &self.channel_endpoint,
            &self.channel_model,
            &self.channel_key,
        ] {
            input.update(cx, |state, cx| state.set_value("", window, cx));
        }
        cx.notify();
        true
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
            let (ok, message) = if channel.kind == ChannelKind::Decision {
                let url = channel.endpoint.clone();
                match client
                    .decide(
                        &url,
                        &channel.api_key,
                        &channel.model,
                        "Hello",
                        std::time::Duration::from_millis(4_000),
                        CancellationToken::new(),
                    )
                    .await
                {
                    Ok(decision) => (
                        false,
                        locale.format(
                            "channels-decision-result",
                            &[("task", &locale.task(decision.task))],
                        ),
                    ),
                    Err(err) => (true, told(&url, &err)),
                }
            } else if channel.kind == ChannelKind::GoogleFree {
                match client
                    .translate_google("hello", "Chinese", CancellationToken::new())
                    .await
                {
                    Ok(_) => (false, locale.text("channels-test-ok")),
                    Err(err) => (true, told(&channel.endpoint, &err)),
                }
            } else if channel.kind == ChannelKind::DeepLx {
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
                    Ok(_) => (false, locale.text("channels-test-ok")),
                    Err(err) => (true, told(&url, &err)),
                }
            } else {
                let Some(provider) = channel.provider() else {
                    let _ = tx.send((id, true, locale.text("status-channel-missing")));
                    return;
                };
                let url = peek_network::endpoint_url(&provider)
                    .unwrap_or_else(|_| provider.base_url.clone());
                match client.probe(&provider, &channel.api_key).await {
                    Ok(()) => (false, locale.text("channels-test-ok")),
                    Err(err) => (true, told(&url, &err)),
                }
            };
            let _ = tx.send((id, ok, message));
        });
    }

    /// Removes a channel and clears any selection that pointed at it.
    fn remove_channel(&mut self, id: &str, cx: &mut Context<Self>) {
        self.config.channels.retain(|channel| channel.id != id);
        if let Some(ids) = &mut self.config.quick_channels {
            ids.retain(|old| old != id);
        }
        self.config.repair_channel_assignments();
        self.persist_config();
        cx.notify();
    }

    /// Points a place in the app at a channel.
    fn assign_channel(&mut self, slot: ChannelSlot, id: String, cx: &mut Context<Self>) {
        match slot {
            ChannelSlot::QuickFirst => {
                let ids = self.config.quick_channels.get_or_insert_with(Vec::new);
                ids.retain(|old| old != &id);
                if !id.is_empty() {
                    ids.insert(0, id);
                }
            }
            ChannelSlot::QuickAdd => {
                let ids = self.config.quick_channels.get_or_insert_with(Vec::new);
                if !id.is_empty() && !ids.contains(&id) {
                    ids.push(id);
                }
            }
            ChannelSlot::Llm => self.config.ai_channel = id,
            ChannelSlot::Decision => self.config.decision_channel = id,
        }
        self.persist_config();
        cx.notify();
    }

    fn persist_config(&mut self) -> bool {
        if let Err(err) = store::save(&self.config) {
            self.notify_error(i18n::format("status-config-save-error", &[("error", &err)]));
            return false;
        }
        true
    }

    /// Sends the follow-up box, continuing the current conversation.
    ///
    /// The box is cleared on the next frame, where a `Window` exists. Enter
    /// arrives from the editor's event, which does not carry one.
    fn send_follow_up(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let text = self.follow_up.read(cx).value().trim().to_owned();
        if self.begin_turn(text, true, cx) {
            self.clear_follow_up = true;
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
        self.begin_turn_as(text, followup, None, cx)
    }

    /// Starts a turn, optionally with the task already decided.
    fn begin_turn_as(
        &mut self,
        text: String,
        followup: bool,
        decided: Option<Task>,
        cx: &mut Context<Self>,
    ) -> bool {
        self.start_turn(text, followup, decided, false, cx)
    }

    /// A screenshot turn. The caller has already settled the panel and stored
    /// the overlay channel. This turn must not rewrite that transcript.
    fn begin_isolated(&mut self, text: String, cx: &mut Context<Self>) -> bool {
        self.start_turn(text, false, None, true, cx)
    }

    fn start_turn(
        &mut self,
        text: String,
        followup: bool,
        decided: Option<Task>,
        isolated: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if text.is_empty() {
            return false;
        }
        if text.len() > peek_core::MAX_INPUT_BYTES {
            self.fail_turn(isolated, i18n::tr("status-input-too-long"), cx);
            return false;
        }
        if followup && !isolated && self.messages.is_empty() {
            return false;
        }

        if isolated {
            // The panel turn was settled before the overlay channel was stored.
            self.stop_transport();
        } else {
            self.stop();
        }
        self.answer.clear();

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
        let task = decided
            .or_else(|| self.selected_task(cx))
            .unwrap_or(route.task);
        let target = effective_target(&route.target, "").to_owned();
        // A follow-up continues the conversation, which a translation-only
        // endpoint cannot do. The first translation may still use DeepLX.
        if !isolated && !followup {
            self.dictionary_note.clear();
            if let Some(entry) = self.lookup_word(&text) {
                self.dictionary_note = dictionary_text(&entry);
            }
        }

        let channels = self.config.answer_channels(task, followup);
        if channels.is_empty() {
            if !isolated && !followup && !self.dictionary_note.is_empty() {
                // Offline lookup must work before any service is configured.
                self.bubbles.clear();
                self.messages.clear();
                self.original_query = text;
                self.status.clear();
                cx.notify();
                return true;
            }
            self.fail_turn(isolated, i18n::tr("status-channel-missing"), cx);
            return false;
        }

        let instruction = task.styled_instruction(&target, &self.config.translation_style);
        if !isolated {
            if !followup {
                self.bubbles.clear();
                self.original_query = text.clone();
                self.messages = vec![Message {
                    role: "system".into(),
                    content: instruction.clone(),
                }];
            } else if let Some(system) = self.messages.first_mut() {
                system.content = instruction.clone();
            }
            let content = if followup {
                text.clone()
            } else {
                peek_core::wrap_content(&text)
            };
            self.messages.push(Message {
                role: "user".into(),
                content,
            });
            self.bubbles.push((true, text.clone()));
            peek_core::bound_history(&mut self.messages);
        }

        self.follow_up_turn = followup && !isolated;
        let messages = if isolated {
            vec![
                Message {
                    role: "system".into(),
                    content: instruction,
                },
                Message {
                    role: "user".into(),
                    content: peek_core::wrap_content(&text),
                },
            ]
        } else {
            self.messages.clone()
        };
        self.launch_stream(channels, text, target, messages, !isolated, cx);
        true
    }

    fn fail_turn(&mut self, isolated: bool, message: String, cx: &mut Context<Self>) {
        if isolated {
            self.status = message.clone();
            self.forward_overlay(OverlayEvent::Status(message));
            self.forward_overlay(OverlayEvent::Finished);
            self.overlay = None;
        } else {
            self.notify_error(message);
        }
        cx.notify();
    }

    fn launch_stream(
        &mut self,
        channels: Vec<Channel>,
        text: String,
        target: String,
        messages: Vec<Message>,
        panel: bool,
        cx: &mut Context<Self>,
    ) {
        if panel {
            self.busy = true;
        }
        let (tx, rx) = mpsc::channel(128);
        self.receiver = Some(rx);
        let cancel = CancellationToken::new();
        self.cancel = Some(cancel.clone());
        let client = self.client.clone();
        let ui_language = self.config.ui_language.clone();
        self.runtime.spawn(async move {
            let locale = i18n::I18n::new(&ui_language);
            let (net_tx, mut net_rx) = mpsc::channel(32);
            let worker = tokio::spawn(async move {
                client
                    .stream_channels(&channels, &text, &target, &messages, net_tx, cancel)
                    .await
            });
            while let Some(event) = net_rx.recv().await {
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
        self.spawn_drain(cx);
        cx.notify();
    }

    /// Moves streamed events into the view. A newer turn makes this loop exit
    /// without reading that turn's receiver.
    fn spawn_drain(&mut self, cx: &mut Context<Self>) {
        self.turn_id = self.turn_id.wrapping_add(1);
        let turn = self.turn_id;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(DRAIN).await;
                let finished = this
                    .update(cx, |peek, cx| {
                        if peek.turn_id != turn {
                            return true;
                        }
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
            // A regular window keeps the title row. Hover is only for a peek.
            set_chrome_locked(true);
            set_chrome_visible(true);
            // A regular app's menu bar only appears once it is activated.
            cx.activate(true);
        } else {
            set_chrome_locked(false);
            set_chrome_visible(false);
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
            let overlay = self.overlay.is_some();
            match event {
                Event::Route { note, .. } => {
                    if overlay {
                        self.forward_overlay(OverlayEvent::Status(note));
                    }
                }
                Event::Text(delta) => {
                    if overlay {
                        self.forward_overlay(OverlayEvent::Chunk(delta));
                        continue;
                    }
                    if self.answer.len() + delta.len() > peek_core::MAX_OUTPUT_BYTES {
                        self.notify_error(i18n::tr("status-output-too-long"));
                        self.stop();
                        break;
                    }
                    self.answer.push_str(&delta);
                }
                Event::Done => {
                    if overlay {
                        self.forward_overlay(OverlayEvent::Finished);
                        self.overlay = None;
                        self.busy = false;
                        self.status.clear();
                        continue;
                    }
                    // The answer joins the conversation. Without this the next
                    // question saw the system prompt and the user's own words
                    // and nothing else, so a follow-up had nothing to follow.
                    if !self.answer.is_empty() {
                        self.messages.push(Message {
                            role: "assistant".into(),
                            content: self.answer.clone(),
                        });
                        self.bubbles.push((false, std::mem::take(&mut self.answer)));
                        peek_core::bound_history(&mut self.messages);
                    }
                    self.status.clear();
                    self.busy = false;
                }
                Event::Failed(message) => {
                    if overlay {
                        self.forward_overlay(OverlayEvent::Failed(message));
                        self.forward_overlay(OverlayEvent::Finished);
                        self.overlay = None;
                        self.busy = false;
                        continue;
                    }
                    self.notify_error(message);
                    self.settle_stopped_turn();
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
        if self.clear_follow_up {
            self.clear_follow_up = false;
            self.follow_up
                .update(cx, |state, cx| state.set_value("", window, cx));
        }
        // Resize before drawing. The query page stays compact until it has
        // something to show, while the settings page is always tall; sizing it
        // only from the query page left settings clipped to a compact window.
        let mut height = if self.settings {
            self.settings_height()
        } else if self.has_result() {
            // Same as the compact panel: the row grows the window upward
            // instead of covering the result.
            self.result_height()
                + if chrome_visible() {
                    PANEL_CHROME_HEIGHT
                } else {
                    0.
                }
        } else {
            // The textarea auto-grows between two and five rows, so the compact
            // panel follows it rather than guessing a single height.
            let rows = self.input.read(cx).value().lines().count().clamp(2, 5) as f32;
            let chrome = if chrome_visible() {
                PANEL_CHROME_HEIGHT
            } else {
                0.
            };
            (self.input_measured + 8.).max(PANEL_COMPACT_BASE + PANEL_INPUT_ROW * rows) + chrome
        };
        // The task list hangs down from the button. A compact panel ends at
        // that button, so the window has to grow while the list is open.
        if self.task_menu_open && !self.settings {
            let chrome = if chrome_visible() {
                PANEL_CHROME_HEIGHT
            } else {
                0.
            };
            let needed = (self.input_measured + 8.).max(PANEL_COMPACT_BASE + PANEL_INPUT_ROW * 2.)
                + chrome
                + 240.;
            if height < needed {
                height = needed;
            }
        }
        // Dialogs need enough viewport space for their fields and popup menus.
        // Closing the dialog restores the height of the underlying settings tab.
        if window.has_active_dialog(cx) {
            let fallback = if self.draft_kind(cx) == ChannelKind::GoogleFree {
                240.
            } else {
                450.
            };
            height = height.max(if self.dialog_measured > 0. {
                self.dialog_measured + 32.
            } else {
                fallback
            });
        }
        let chrome_offset = if !self.settings && chrome_visible() {
            PANEL_CHROME_HEIGHT
        } else {
            0.
        };
        // Keep the currently painted layout until the native frame has moved
        // and resized. Rendering the new toolbar into the old viewport produces
        // a visible intermediate frame, even with AppKit updates suppressed.
        if !self.geometry_pending
            && (self.applied_height != Some(height) || self.chrome_offset != chrome_offset)
        {
            self.geometry_pending = true;
            let entity = cx.entity();
            native_window::resize_panel(
                window,
                size(px(PANEL_WIDTH), px(height)),
                chrome_offset - self.chrome_offset,
                cx,
                move |window, cx| {
                    entity.update(cx, |this, cx| {
                        this.applied_height = Some(height);
                        this.chrome_offset = chrome_offset;
                        this.geometry_pending = false;
                        cx.notify();
                    });
                    window.refresh();
                },
            );
        }
        // The pointer zone follows the frame actually painted, not the request.
        set_chrome_in_frame(!self.settings && self.chrome_offset > 0.);
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
        // Opens down and to the right. The split control's default anchor
        // hangs the list off the left edge of this narrow panel.
        Button::new("task-picker")
            .icon(icon)
            .tooltip(tooltip)
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
            .on_open_change({
                let entity = cx.entity();
                move |open, _window, cx| {
                    let open = *open;
                    entity.update(cx, |this, cx| {
                        if this.task_menu_open != open {
                            this.task_menu_open = open;
                            cx.notify();
                        }
                    });
                }
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

    fn result_body(&self, children: Vec<AnyElement>, cx: &Context<Self>) -> AnyElement {
        v_flex()
            .on_children_prepainted({
                let entity = cx.entity();
                move |bounds, _window, cx| {
                    let height = bounds.iter().map(|b| b.size.height.as_f32()).sum::<f32>()
                        + 8. * bounds.len().saturating_sub(1) as f32;
                    let entity = entity.clone();
                    cx.defer(move |cx| {
                        entity.update(cx, |this, cx| {
                            if (this.answer_measured - height).abs() > 0.5 {
                                this.answer_measured = height;
                                cx.notify();
                            }
                        })
                    });
                }
            })
            .w_full()
            .flex_none()
            .gap(px(8.))
            .children(children)
            .into_any_element()
    }

    fn query_page(&self, cx: &Context<Self>) -> AnyElement {
        let basic_choices: Vec<(String, String)> = self
            .config
            .quick_candidates()
            .map(|channel| (channel.id.clone(), channel.name.clone()))
            .collect();
        let basic_active = self
            .config
            .quick_channels
            .as_ref()
            .and_then(|ids| ids.first())
            .cloned()
            .unwrap_or_default();
        let basic_entity = cx.entity();
        let theme = cx.theme();
        let set = self.set;
        let fg = theme.foreground;
        let bg = theme.background;
        let border = theme.border;

        // One turn of the conversation. The user's own words sit on the right
        // and the assistant's on the left, which is what makes the two readable
        // apart in a panel this narrow.
        let primary = theme.primary;
        let primary_foreground = theme.primary_foreground;
        let secondary = theme.secondary;
        let secondary_foreground = theme.secondary_foreground;
        let bubble = move |index: usize, from_user: bool, text: String| -> AnyElement {
            let (bubble_bg, bubble_fg) = if from_user {
                (primary, primary_foreground)
            } else {
                (secondary, secondary_foreground)
            };
            let body: AnyElement = if from_user {
                div().text_size(px(13.)).child(text).into_any_element()
            } else {
                TextView::markdown(SharedString::from(format!("bubble-{index}")), text)
                    .text_size(px(13.))
                    .into_any_element()
            };
            h_flex()
                .w_full()
                .flex_none()
                .when(from_user, |this| this.justify_end())
                .when(!from_user, |this| this.justify_start())
                .child(
                    div()
                        .max_w(px(340.))
                        .bg(bubble_bg)
                        .text_color(bubble_fg)
                        .rounded(px(14.))
                        .px(px(12.))
                        .py(px(9.))
                        .child(body),
                )
                .into_any_element()
        };

        v_flex()
            .on_children_prepainted({
                let entity = cx.entity();
                let has_result = self.has_result();
                let has_answer = self.conversation() || self.plain_answer().is_some();
                let answer_index = (if self.dictionary_note.is_empty() {
                    1
                } else {
                    2
                }) + usize::from(self.chrome_offset > 0.);
                let answer_height = self.answer_measured + 30.;
                move |bounds, _window, cx| {
                    if !has_result {
                        return;
                    }
                    let height = bounds
                        .iter()
                        .enumerate()
                        .map(|(index, b)| {
                            if has_answer && index == answer_index {
                                answer_height
                            } else {
                                b.size.height.as_f32()
                            }
                        })
                        .sum::<f32>()
                        + 8.
                        + 5. * bounds.len().saturating_sub(1) as f32;
                    let entity = entity.clone();
                    cx.defer(move |cx| {
                        entity.update(cx, |this, cx| {
                            // The chrome is tracked separately by the render path.
                            let height = height - this.chrome_offset;
                            if (this.query_measured - height).abs() > 0.5 {
                                this.query_measured = height;
                                cx.notify();
                            }
                        })
                    });
                }
            })
            .id("panel")
            .size_full()
            .bg(bg)
            .text_color(fg)
            // The corner is the right half of the top of the input box, not of
            // the whole panel. Mouse moves carry a position, so nothing is
            // laid over the text. Leaving the window closes the row; a layout
            // change under a still pointer does not.
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if this.pinned || std::env::var("PEEK_RENDER").is_ok() || chrome_settling() {
                    return;
                }
                let want = pointer_in_chrome_zone(event.position, window);
                if want != chrome_visible() {
                    set_chrome_visible(want);
                    cx.notify();
                }
            }))
            .on_mouse_exit(cx.listener(|this, _event: &MouseExitEvent, _window, cx| {
                if this.pinned || std::env::var("PEEK_RENDER").is_ok() || chrome_settling() {
                    return;
                }
                if chrome_visible() {
                    set_chrome_visible(false);
                    cx.notify();
                }
            }))
            // The sides hug the input box whether or not the title row is
            // showing; only the top needs room for it. The gap keeps the input,
            // the answer and the follow-up row from touching.
            .px(px(4.))
            .pt(px(4.))
            .pb(px(4.))
            .gap(px(5.))
            // Esc hides the panel — "appear when needed, gone when done". A
            // pinned window is a normal window, so Esc leaves it alone.
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if !this.pinned && event.keystroke.key == "escape" {
                    native_window::hide(window);
                    return;
                }
                // A keystroke in the query box dismisses the title row. Doing
                // that while the follow-up is focused resizes the window under
                // the caret and macOS drops the Chinese input session.
                let follow_up_focused = this.follow_up.read(cx).focus_handle(cx).is_focused(window);
                if !this.pinned && chrome_visible() && !follow_up_focused {
                    set_chrome_visible(false);
                    cx.notify();
                }
            }))
            .when(self.chrome_offset > 0., |this| {
                this.child(
                    h_flex()
                        .id("chrome-row")
                        .h(px(PANEL_CHROME_HEIGHT - 5.))
                        .flex_none()
                        .w_full()
                        .items_center()
                        .gap(px(10.))
                        // Inset so the title lines up with the text inside the
                        // input box, whose own padding starts at the same place.
                        .px(px(14.))
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
                                                            ChannelSlot::QuickFirst,
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
                    .on_children_prepainted({
                        let entity = cx.entity();
                        move |bounds, _window, cx| {
                            let Some(text) = bounds.first() else {
                                return;
                            };
                            // Card padding (14 per edge) and its border (1 per edge).
                            let needed = text.size.height.as_f32() + 30.;
                            let entity = entity.clone();
                            cx.defer(move |cx| {
                                entity.update(cx, |this, cx| {
                                    if (this.input_measured - needed).abs() > 0.5 {
                                        this.input_measured = needed;
                                        cx.notify();
                                    }
                                });
                            });
                        }
                    })
                    .relative()
                    .flex_none()
                    .w_full()
                    .border_1()
                    .border_color(border)
                    .rounded(px(16.))
                    .p(px(14.))
                    .child(
                        // Clip the text, not the task list. The list is a
                        // sibling of this box and has to paint past it.
                        div().w_full().overflow_hidden().child(
                            Textarea::new(&self.input)
                                .w_full()
                                .appearance(false)
                                .bordered(false)
                                // Room for the controls drawn over the corner.
                                .pb(px(34.)),
                        ),
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
                            .when(self.busy && !self.follow_up_turn, |this| {
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
                                    .loading(self.busy && !self.follow_up_turn)
                                    .on_click(cx.listener(|this, _event, window, cx| {
                                        this.start_query(window, cx);
                                    })),
                            ),
                    ),
            )
            .when(self.has_result(), |this| {
                let conversation = self.conversation();
                let plain = self.plain_answer();
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
                                .flex_none()
                                .font_family(set.sc)
                                .text_size(px(13.))
                                .child(self.dictionary_note.clone()),
                        )
                    })
                    // One answer is a markdown card. A follow-up turns that
                    // same history into a conversation.
                    .when(conversation, |this| {
                        this.child(
                            v_flex()
                                .id("answer")
                                .w_full()
                                .flex_1()
                                .border_1()
                                .border_color(border)
                                .rounded(px(14.))
                                .p(px(14.))
                                .overflow_y_scroll()
                                .child(self.result_body(
                                    {
                                        let mut children: Vec<AnyElement> = self
                                            .bubbles
                                            .iter()
                                            .enumerate()
                                            .map(|(index, (from_user, text))| {
                                                bubble(index, *from_user, text.clone())
                                            })
                                            .collect();
                                        if !self.answer.is_empty() {
                                            children.push(bubble(
                                                self.bubbles.len(),
                                                false,
                                                self.answer.clone(),
                                            ));
                                        }
                                        children
                                    },
                                    cx,
                                )),
                        )
                    })
                    .when(!conversation && plain.is_some(), |this| {
                        let text = plain.clone().unwrap_or_default();
                        this.child(
                            div()
                                .id("answer")
                                .w_full()
                                .flex_1()
                                .overflow_y_scroll()
                                .border_1()
                                .border_color(border)
                                .rounded(px(14.))
                                .p(px(14.))
                                .child(self.result_body(
                                    vec![
                                            TextView::markdown("plain-answer", text)
                                                .text_size(px(13.))
                                                .into_any_element(),
                                        ],
                                    cx,
                                )),
                        )
                    })
                    // Follow-up turn, continuing the same conversation.
                    .child(
                        h_flex()
                            .on_children_prepainted({
                                let entity = cx.entity();
                                move |bounds, _window, cx| {
                                    let Some(input) = bounds.first() else {
                                        return;
                                    };
                                    let height = input.size.height.as_f32();
                                    let entity = entity.clone();
                                    cx.defer(move |cx| {
                                        entity.update(cx, |this, cx| {
                                            if (this.follow_height - height).abs() > 0.5 {
                                                this.follow_height = height;
                                                cx.notify();
                                            }
                                        })
                                    });
                                }
                            })
                            .w_full()
                            .flex_none()
                            .items_end()
                            .gap(px(8.))
                            .child(Textarea::new(&self.follow_up).flex_1().rounded(px(10.)))
                            .when(self.busy && self.follow_up_turn, |this| {
                                this.child(
                                    Button::new("follow-stop")
                                        .h(px(self.follow_height))
                                        .icon(gpui_kit::assets::IconName::Square)
                                        .tooltip(i18n::tr("query-stop"))
                                        .on_click(cx.listener(|this, _event, _window, cx| {
                                            this.stop();
                                            cx.notify();
                                        })),
                                )
                            })
                            .child(
                                Button::new("follow-send")
                                    .h(px(self.follow_height))
                                    .icon(gpui_kit::assets::IconName::Send)
                                    .tooltip(i18n::tr("query-send"))
                                    .loading(self.busy && self.follow_up_turn)
                                    .on_click(cx.listener(|this, _event, _window, cx| {
                                        this.send_follow_up(cx);
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
        let ai_channels: Vec<(String, String)> = self
            .config
            .ai_candidates()
            .map(|channel| (channel.id.clone(), channel.name.clone()))
            .collect();
        let decision_channels: Vec<(String, String)> = self
            .config
            .decision_candidates()
            .map(|channel| (channel.id.clone(), channel.name.clone()))
            .collect();
        let ai_active = self.config.ai_channel.clone();
        let decision_active = self.config.decision_channel.clone();
        let theme = cx.theme();
        let set = self.set;
        let fg = theme.foreground;
        let bg = theme.background;
        let muted = theme.muted_foreground;

        v_flex()
            // Children keep their own height. The window then adopts the sum,
            // so the page is exactly as tall as what this tab shows. This has
            // to be set before `id`, which wraps the div.
            .on_children_prepainted({
                let measure = cx.entity();
                move |bounds, _window, cx| {
                    if bounds.is_empty() {
                        return;
                    }
                    let stacked = bounds
                        .iter()
                        .map(|bound| bound.size.height.as_f32())
                        .sum::<f32>();
                    let gaps = 14. * (bounds.len() - 1) as f32;
                    // Vertical padding on this page, top and bottom.
                    // The page padding is 20 on each edge. Inputs paint a little
                    // past the layout box, so the last field needs a few more
                    // points or the window clips it.
                    let needed = (stacked + gaps + 56.).clamp(220., 900.);
                    let measure = measure.clone();
                    cx.defer(move |cx| {
                        measure.update(cx, |this, cx| {
                            if (this.settings_measured - needed).abs() > 1.5 {
                                this.settings_measured = needed;
                                cx.notify();
                            }
                        });
                    });
                }
            })
            .id("settings_scroll")
            .size_full()
            .items_start()
            .bg(bg)
            .text_color(fg)
            .p(px(20.))
            .gap(px(14.))
            .overflow_y_scroll()
            .on_key_down(|event, window, _cx| {
                if event.keystroke.key == "escape" {
                    native_window::hide(window);
                }
            })
            .child(
                h_flex()
                    .w_full()
                    .flex_none()
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
                div().w_full().flex_none().child(
                    TabBar::new("settings_tabs")
                        // The default filled-tab strip reads as a different design
                        // language; an underline row matches a flat settings page.
                        .underline()
                        .children([
                            Tab::new().label(i18n::tr("settings-tab-appearance")),
                            Tab::new().label(i18n::tr("settings-tab-shortcuts")),
                            Tab::new().label(i18n::tr("settings-tab-channels")),
                            Tab::new().label(i18n::tr("settings-tab-translation")),
                            Tab::new().label(i18n::tr("settings-tab-permissions")),
                        ])
                        .selected_index(self.settings_tab)
                        .on_click(cx.listener(|this, index: &usize, _window, cx| {
                            this.settings_tab = *index;
                            if *index == 4 {
                                this.permissions = peek_runtime::permissions::status();
                            }
                            cx.notify();
                        })),
                ),
            )
            .when(self.settings_tab == 0, |this| {
                this.child(
                    v_flex()
                        .w_full()
                        .flex_none()
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
                        .flex_none()
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
                        .flex_none()
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
                        .child(self.picker_row(
                            i18n::tr("settings-shortcut-selection"),
                            Input::new(&self.selection_hotkey).into_any_element(),
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
            .when(self.settings_tab == 3, |this| {
                this.child(
                    v_flex()
                        .w_full()
                        .flex_none()
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
                        .child(self.quick_channel_list(cx))
                        .child(self.picker_row(
                            i18n::tr("channels-followup"),
                            self.channel_picker(
                                "ai_picker",
                                ai_channels,
                                ai_active.clone(),
                                ChannelSlot::Llm,
                                cx,
                            ),
                            set,
                        ))
                        .child(self.picker_row(
                            i18n::tr("channels-decision"),
                            self.channel_picker(
                                "decision_picker",
                                decision_channels,
                                decision_active.clone(),
                                ChannelSlot::Decision,
                                cx,
                            ),
                            set,
                        )),
                )
            })
            .when(self.settings_tab == 4, |this| {
                this.child(
                    v_flex()
                        .w_full()
                        .flex_none()
                        .gap(px(4.))
                        .child(
                            div()
                                .font_family(set.latin)
                                .text_size(px(11.))
                                .text_color(muted)
                                .child(i18n::tr("settings-permissions")),
                        )
                        .when(cfg!(windows), |this| {
                            this.child(div().text_size(px(11.)).text_color(muted)
                                .child(i18n::tr("settings-permission-windows")))
                        })
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
                                .child(
                                    Button::new(SharedString::from(format!("open-{key}")))
                                        .icon(IconName::ExternalLink)
                                        .tooltip(i18n::tr("settings-permission-open"))
                                        .accessibility_label(i18n::tr("settings-permission-open"))
                                        .on_click(cx.listener({
                                            let key = *key;
                                            move |this, _event, _window, cx| {
                                                if let Err(err) = peek_runtime::permissions::open_permission_settings(key) {
                                                    this.notify_error(err);
                                                }
                                                cx.notify();
                                            }
                                        })),
                                )
                                .into_any_element()
                        }))
                        .child(
                            h_flex()
                                .w_full()
                                .gap(px(8.))
                                .pt(px(8.))
                                .child(
                                    Button::new("refresh-permissions")
                                        .label(i18n::tr("settings-permission-refresh"))
                                        .on_click(cx.listener(|this, _event, _window, cx| {
                                            this.permissions = peek_runtime::permissions::status();
                                            cx.notify();
                                        })),
                                ),
                        ),
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

            if std::env::var("PEEK_OFFLINE_SELFTEST").is_ok() {
                peek.config = Config::default();
                peek.pending_input = Some("ephemeral".into());
                assert!(peek.begin_turn("ephemeral".into(), false, cx));
                assert!(
                    !peek.dictionary_note.is_empty(),
                    "offline dictionary must answer without channels"
                );
                assert!(
                    peek.receiver.is_none(),
                    "offline query must not start a network request"
                );
                assert!(
                    peek.pending_toast.is_none(),
                    "offline lookup must not report a missing channel"
                );
                println!("[selftest] offline lookup without channels passed");
                return;
            }

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
                // A single answer, rendered as markdown rather than bubbles.
                "plain" => {
                    peek.pending_input = Some("这段话帮我翻译一下".to_string());
                    peek.bubbles = vec![
                        (true, "这段话帮我翻译一下".to_string()),
                        (false, SAMPLE_ANSWER.to_string()),
                    ];
                    peek.dictionary_note = "stream   /striːm/\n流；溪流".to_string();
                    if let Ok(answer) = std::env::var("PEEK_ANSWER") {
                        peek.bubbles = vec![(true, "asdsad".into()), (false, answer)];
                        peek.dictionary_note.clear();
                    }
                    if std::env::var("PEEK_CHROME").is_ok() {
                        set_chrome_visible(true);
                    }
                }
                _ => {
                    if std::env::var("PEEK_CHROME").is_ok() {
                        set_chrome_visible(true);
                    }
                    // A real conversation, so both sides of the layout show.
                    peek.bubbles = vec![
                        (true, "这段话帮我翻译一下".to_string()),
                        (false, SAMPLE_ANSWER.to_string()),
                        (true, "第二点再展开说说".to_string()),
                        (
                            false,
                            "第二点是 **Markdown 渲染**：标题、列表、代码块和引用都按各自的\
                             样式排布，长文在卡片内滚动。"
                                .to_string(),
                        ),
                    ];
                    peek.answer = String::new();
                    // No status: a finished turn shows its answer and nothing
                    // else, which is what the preview should mirror.
                    peek.status.clear();
                }
            }
            cx.notify();
        });
        cx.new(|cx| Root::new(view, window, cx))
    })?;

    // A few turns: settings measures its children after the first paint and
    // resizes, and the next paint has to fill that new window.
    for _ in 0..8 {
        cx.run_until_parked();
    }
    // Exercise editor focus transfer without contacting a translation service.
    if std::env::var("PEEK_INPUT_SELFTEST").is_ok()
        && let Some(view) = published.borrow().clone()
    {
        use gpui_kit::test::TestWindowExt as _;
        for follow in [false, true] {
            cx.update_window(window.into(), |_, window, cx| {
                let state = if follow {
                    view.read(cx).follow_up.clone()
                } else {
                    view.read(cx).input.clone()
                };
                let id = ElementId::from(("input", state.entity_id()));
                window.click_at(id, point(px(12.), px(12.)), cx);
            })?;
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                let state = if follow {
                    view.read(cx).follow_up.clone()
                } else {
                    view.read(cx).input.clone()
                };
                window.render_frame(cx);
                assert!(
                    state.read(cx).focus_handle(cx).is_focused(window),
                    "editor focus missing (follow={follow})"
                );
                window.input("中文", cx);
                assert!(state.read(cx).value().contains("中文"));
            })?;
        }
        println!("[input-selftest] main and follow-up accept Chinese text after clicking");
    }
    // The channel dialog is opened by a click, so a preview opens it directly.
    // It has to wait for the first frame: the component layer registers the
    // per-window state dialogs look up while painting.
    if std::env::var("PEEK_DIALOG").is_ok()
        && let Some(view) = published.borrow().clone()
    {
        cx.update_window(window.into(), |_, window, cx| {
            view.update(cx, |peek, cx| {
                peek.open_channel_dialog(None, window, cx);
                if let Ok(kind) = std::env::var("PEEK_DIALOG_KIND")
                    && let Some(index) = ChannelKind::ALL.iter().position(|k| k.label_key() == kind)
                {
                    peek.channel_kind.update(cx, |state, cx| {
                        state.set_selected_index(Some(IndexPath::new(index)), window, cx);
                    });
                    cx.notify();
                }
            });
        })?;
        for _ in 0..4 {
            cx.run_until_parked();
        }
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

/// Keeps the single-instance lock for the process lifetime.
///
/// A second launch exits. If the lock cannot be created, the app still starts.
fn keep_single_instance() -> Option<std::fs::File> {
    let Ok(config) = peek_runtime::store::config_path() else {
        return None;
    };
    let directory = config.parent()?;
    match peek_runtime::instance::acquire(&directory.join("instance.lock")) {
        Ok(Some(file)) => Some(file),
        Ok(None) => {
            eprintln!("Crant Peek is already running");
            std::process::exit(0);
        }
        Err(err) => {
            eprintln!("instance lock failed: {err}");
            None
        }
    }
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

    // Held until the process exits. A second launch finds the lock and leaves.
    let _instance = keep_single_instance();

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

        // One action channel, owned here: the tray menu and the global hotkeys
        // feed it, and the panel view drains it. Selection uses the same hotkey
        // path. The old double-tap Ctrl hook stayed installed beside it and
        // treated an empty selection differently, so it is not started.
        let (action_tx, action_rx) = std::sync::mpsc::channel::<Action>();
        let tray_tx = action_tx.clone();
        let view_tx = action_tx.clone();
        // The hotkey loop reads the selection on a thread of its own.
        let selection_tx = action_tx.clone();

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
            ("selection", hotkey_config.selection_hotkey.as_str()),
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
        let selection_hotkey_id = parsed
            .iter()
            .find(|(name, _)| *name == "selection")
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
                let mut ask_selection = false;

                while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
                    if event.state != HotKeyState::Pressed {
                        continue;
                    }
                    if Some(event.id) == toggle_id {
                        toggle = true;
                    } else if Some(event.id) == snip_hotkey_id {
                        screenshot = true;
                    } else if Some(event.id) == selection_hotkey_id {
                        ask_selection = true;
                    }
                }

                if ask_selection {
                    // Reading a selection goes through the accessibility API,
                    // which can take a moment, so it happens off this thread.
                    // An empty or unreadable selection opens the panel ready for
                    // input rather than doing nothing at all.
                    let tx = selection_tx.clone();
                    std::thread::spawn(move || {
                        let text = peek_runtime::selection::read().unwrap_or_default();
                        let text = text.trim().to_owned();
                        let action = if text.is_empty() {
                            Action::Blank
                        } else {
                            Action::Selection(text)
                        };
                        let _ = tx.send(action);
                    });
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

#[cfg(test)]
mod chrome_zone_tests {
    use super::{
        INPUT_CORNER_HEIGHT, INPUT_CORNER_SLOP, PANEL_CHROME_HEIGHT, chrome_zone_contains,
    };

    #[test]
    fn the_corner_does_not_grow_when_the_panel_does() {
        let width = 480.;
        assert!(chrome_zone_contains(400., 10., width, 0., false));
        assert!(!chrome_zone_contains(100., 10., width, 0., false));
        // Lower half of a tall panel, still on the right. Not the input corner.
        assert!(!chrome_zone_contains(400., 400., width, 0., false));
        assert!(!chrome_zone_contains(
            400.,
            INPUT_CORNER_HEIGHT + 1.,
            width,
            0.,
            false
        ));
    }

    #[test]
    fn an_open_corner_keeps_slack_and_the_title_row() {
        let width = 480.;
        let top = PANEL_CHROME_HEIGHT;
        assert!(chrome_zone_contains(20., 10., width, top, true));
        let inside_slack = top + INPUT_CORNER_HEIGHT + INPUT_CORNER_SLOP - 1.;
        assert!(chrome_zone_contains(400., inside_slack, width, top, true));
        assert!(!chrome_zone_contains(
            400.,
            inside_slack + 2.,
            width,
            top,
            true
        ));
        // Hiding moves the window back down by the row. That same pointer must
        // land outside the strict corner, or the edge opens and closes itself.
        assert!(!chrome_zone_contains(
            400.,
            inside_slack - PANEL_CHROME_HEIGHT,
            width,
            0.,
            false
        ));
    }
}
