//! Read only an explicit Accessibility selection. Never substitute clipboard text.
use std::sync::atomic::{AtomicU64, Ordering};
static GENERATION: AtomicU64 = AtomicU64::new(0);
pub fn next_generation() -> u64 {
    GENERATION.fetch_add(1, Ordering::SeqCst) + 1
}
pub fn is_current(generation: u64) -> bool {
    GENERATION.load(Ordering::SeqCst) == generation
}
#[cfg(target_os = "macos")]
pub fn read() -> Option<String> {
    macos::read()
}

/// Accessibility offsets use UTF-16, not Rust byte offsets.
#[cfg(any(target_os = "macos", test))]
fn selected_utf16(text: &str, location: isize, length: isize) -> Option<String> {
    let start = usize::try_from(location).ok()?;
    let length = usize::try_from(length).ok()?;
    if length == 0 || length > peek_core::MAX_INPUT_BYTES {
        return None;
    }
    let end = start.checked_add(length)?;
    let units: Vec<_> = text.encode_utf16().collect();
    String::from_utf16(units.get(start..end)?).ok()
}

#[cfg(target_os = "macos")]
mod macos {
    use super::note;
    use core_foundation::{
        base::{CFGetTypeID, CFRelease, CFTypeRef, TCFType},
        string::CFString,
    };
    use std::ffi::c_void;
    type Ax = *const c_void;
    #[repr(C)]
    struct Range {
        location: isize,
        length: isize,
    }
    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {}
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXUIElementCreateSystemWide() -> Ax;
        fn AXUIElementCreateApplication(pid: i32) -> Ax;
        fn AXUIElementCopyAttributeValue(element: Ax, attr: Ax, value: *mut CFTypeRef) -> i32;
        fn AXUIElementCopyParameterizedAttributeValue(
            element: Ax,
            attr: Ax,
            parameter: Ax,
            value: *mut CFTypeRef,
        ) -> i32;
        fn AXUIElementSetMessagingTimeout(element: Ax, timeout: f32) -> i32;
        fn AXUIElementGetPid(element: Ax, pid: *mut i32) -> i32;
        fn AXValueGetTypeID() -> usize;
        fn AXValueGetValue(value: Ax, kind: u32, output: *mut c_void) -> bool;
    }
    struct Owned(Ax);
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe {
                CFRelease(self.0);
            }
        }
    }
    impl Owned {
        fn attr(&self, name: &str) -> Option<Self> {
            let mut value = std::ptr::null();
            let focused = name == "AXFocusedUIElement";
            let name = CFString::new(name);
            let status = unsafe {
                AXUIElementCopyAttributeValue(self.0, name.as_concrete_TypeRef().cast(), &mut value)
            };
            if status == 0 && !value.is_null() {
                Some(Self(value))
            } else {
                if focused {
                    note(&format!(
                        "[selection] focused element lookup failed: status={status}"
                    ));
                }
                if !value.is_null() {
                    unsafe {
                        CFRelease(value);
                    }
                }
                None
            }
        }
        fn parameter(&self, name: &str, parameter: &Self) -> Option<Self> {
            let mut value = std::ptr::null();
            let name = CFString::new(name);
            let status = unsafe {
                AXUIElementCopyParameterizedAttributeValue(
                    self.0,
                    name.as_concrete_TypeRef().cast(),
                    parameter.0,
                    &mut value,
                )
            };
            if status == 0 && !value.is_null() {
                Some(Self(value))
            } else {
                if !value.is_null() {
                    unsafe {
                        CFRelease(value);
                    }
                }
                None
            }
        }
        fn string(&self) -> Option<String> {
            unsafe {
                if CFGetTypeID(self.0) != CFString::type_id() {
                    return None;
                }
                Some(CFString::wrap_under_get_rule(self.0.cast()).to_string())
            }
        }
        fn selection(&self) -> Option<String> {
            let role = self
                .attr("AXRole")
                .and_then(|v| v.string())
                .unwrap_or_default();
            let subrole = self
                .attr("AXSubrole")
                .and_then(|v| v.string())
                .unwrap_or_default();
            if role == "AXSecureTextField" || subrole == "AXSecureTextField" {
                return None;
            }
            if let Some(text) = self.attr("AXSelectedText").and_then(|v| v.string())
                && let Some(text) = peek_core::entry_text(peek_core::Entry::Selection, Some(&text))
            {
                note(&format!("[selection] read via selected text; role={role}"));
                return Some(text);
            }
            if let Some(range) = self.attr("AXSelectedTextRange") {
                let mut offsets = Range {
                    location: 0,
                    length: 0,
                };
                let valid = unsafe {
                    CFGetTypeID(range.0) == AXValueGetTypeID()
                        && AXValueGetValue(range.0, 4, (&mut offsets as *mut Range).cast())
                };
                if valid && offsets.location >= 0 && offsets.length > 0 {
                    let text = self
                        .parameter("AXStringForRange", &range)
                        .and_then(|v| v.string())
                        .or_else(|| {
                            self.attr("AXValue")
                                .and_then(|v| v.string())
                                .and_then(|text| {
                                    super::selected_utf16(&text, offsets.location, offsets.length)
                                })
                        });
                    if let Some(text) = text.and_then(|text| {
                        peek_core::entry_text(peek_core::Entry::Selection, Some(&text))
                    }) {
                        note(&format!("[selection] read via selected range; role={role}"));
                        return Some(text);
                    }
                }
            }
            if let Some(range) = self.attr("AXSelectedTextMarkerRange")
                && let Some(text) = self
                    .parameter("AXStringForTextMarkerRange", &range)
                    .and_then(|v| v.string())
                && let Some(text) = peek_core::entry_text(peek_core::Entry::Selection, Some(&text))
            {
                note(&format!("[selection] read via text markers; role={role}"));
                return Some(text);
            }
            None
        }
    }
    fn foreground_pid() -> Option<i32> {
        objc2::rc::autoreleasepool(|_| unsafe {
            use objc2::{class, msg_send, runtime::AnyObject};
            let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
            let app: *mut AnyObject = msg_send![workspace, frontmostApplication];
            if app.is_null() {
                None
            } else {
                Some(msg_send![app, processIdentifier])
            }
        })
    }
    pub fn read() -> Option<String> {
        let pid = foreground_pid()?;
        let raw = unsafe { AXUIElementCreateApplication(pid) };
        if raw.is_null() {
            return None;
        }
        let app = Owned(raw);
        unsafe {
            AXUIElementSetMessagingTimeout(app.0, 0.5);
        }
        // Do not show our panel until reading completes. Read the same app on
        // every retry, and stop if the user has switched to another app.
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(80));
            }
            if foreground_pid() != Some(pid) {
                return None;
            }
            let focused = app.attr("AXFocusedUIElement").or_else(|| {
                let raw = unsafe { AXUIElementCreateSystemWide() };
                if raw.is_null() {
                    return None;
                }
                let system = Owned(raw);
                unsafe {
                    AXUIElementSetMessagingTimeout(system.0, 0.5);
                }
                let element = system.attr("AXFocusedUIElement")?;
                let mut actual = 0;
                let status = unsafe { AXUIElementGetPid(element.0, &mut actual) };
                (status == 0 && actual == pid).then_some(element)
            });
            if let Some(mut element) = focused {
                // Some web views put their explicit selection on a containing
                // text area. Only walk this focus ancestry, never unrelated UI.
                for _ in 0..6 {
                    unsafe {
                        AXUIElementSetMessagingTimeout(element.0, 0.3);
                    }
                    if element
                        .attr("AXSubrole")
                        .and_then(|v| v.string())
                        .as_deref()
                        == Some("AXSecureTextField")
                    {
                        return None;
                    }
                    if let Some(text) = element.selection() {
                        return Some(text);
                    }
                    let role = element
                        .attr("AXRole")
                        .and_then(|v| v.string())
                        .unwrap_or_default();
                    if matches!(
                        role.as_str(),
                        "AXWindow" | "AXApplication" | "AXSecureTextField"
                    ) {
                        break;
                    }
                    let Some(parent) = element.attr("AXParent") else {
                        break;
                    };
                    element = parent;
                }
            }
            note(&format!(
                "[selection] no readable explicit selection; pid={pid}, attempt={}",
                attempt + 1
            ));
        }
        None
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn read() -> Option<String> {
    None
}

/// Hook diagnostics.
///
/// `eprintln!` alone is not enough when a bundle is launched with `open`: it
/// has no terminal, and its stderr pipe can hold output back. Setting
/// `PEEK_SELECTION_LOG` to a path also appends every note to that file.
#[cfg(target_os = "macos")]
pub(crate) fn note(message: &str) {
    eprintln!("{message}");
    if let Ok(path) = std::env::var("PEEK_SELECTION_LOG") {
        use std::io::Write as _;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "{message}");
        }
    }
}

/// Called after an action is queued so the host UI can wake itself. Pass a
/// no-op when the host polls the channel on a timer instead.
pub type Wake = std::sync::Arc<dyn Fn() + Send + Sync + 'static>;

/// Fires only after two standalone Ctrl press/release cycles; shortcuts interrupt it.
#[derive(Default)]
pub struct DoubleCtrl {
    down: bool,
    interrupted: bool,
    press_started: Option<std::time::Instant>,
    last_release: Option<std::time::Instant>,
}
impl DoubleCtrl {
    /// Whether Ctrl is currently held, for the trace log.
    pub fn is_down(&self) -> bool {
        self.down
    }
    pub fn interrupt(&mut self) {
        self.interrupted = true;
        self.last_release = None;
    }
    pub fn transition(
        &mut self,
        down: bool,
        now: std::time::Instant,
        interval: std::time::Duration,
    ) -> bool {
        if down == self.down {
            return false;
        }
        self.down = down;
        if down {
            self.press_started = Some(now);
            self.interrupted = false;
            return false;
        }
        let held_too_long = self
            .press_started
            .take()
            .is_none_or(|start| now.saturating_duration_since(start) > interval);
        if self.interrupted || held_too_long {
            self.last_release = None;
            return false;
        }
        let fire = self
            .last_release
            .is_some_and(|last| now.duration_since(last) <= interval);
        self.last_release = if fire { None } else { Some(now) };
        fire
    }
}

#[cfg(target_os = "macos")]
pub fn listen(interval: u64, tx: std::sync::mpsc::Sender<crate::action::Action>, wake: Wake) {
    std::thread::spawn(move || {
        use core_foundation::runloop::{CFRunLoop, kCFRunLoopCommonModes};
        use core_graphics::event::{
            CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
            CGEventType,
        };
        let state = std::cell::RefCell::new(DoubleCtrl::default());
        // PEEK_HOOK_DEBUG logs every Ctrl transition, which tells "the tap
        // delivers nothing" apart from "the double-tap rule did not match".
        let trace = std::env::var("PEEK_HOOK_DEBUG").is_ok();
        let tap = CGEventTap::new(
            CGEventTapLocation::Session,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::ListenOnly,
            vec![CGEventType::FlagsChanged, CGEventType::KeyDown],
            move |_, kind, event| {
                let mut state = state.borrow_mut();
                if matches!(kind, CGEventType::KeyDown) {
                    if trace {
                        note("[selection] other key down (sequence reset)");
                    }
                    state.interrupt();
                    return None;
                }
                let flags = event.get_flags();
                if flags.intersects(
                    CGEventFlags::CGEventFlagCommand
                        | CGEventFlags::CGEventFlagAlternate
                        | CGEventFlags::CGEventFlagShift,
                ) {
                    state.interrupt();
                    return None;
                }
                let ctrl_down = flags.contains(CGEventFlags::CGEventFlagControl);
                if trace && ctrl_down != state.is_down() {
                    note(&format!("[selection] ctrl down={ctrl_down}"));
                }
                if state.transition(
                    ctrl_down,
                    std::time::Instant::now(),
                    std::time::Duration::from_millis(interval),
                ) {
                    // Never block the event-tap callback on Accessibility IPC.
                    let tx = tx.clone();
                    let wake = wake.clone();
                    std::thread::spawn(move || match read() {
                        Some(text) => {
                            note(&format!(
                                "[selection] double tap read {} chars",
                                text.chars().count()
                            ));
                            let _ = tx.send(crate::action::Action::Selection(text));
                            wake();
                        }
                        // The tap fires as soon as Input Monitoring is granted,
                        // but reading the selection additionally needs
                        // Accessibility, and it fails silently without it.
                        None => {
                            note("[selection] double tap detected, but no selection is readable")
                        }
                    });
                }
                None
            },
        );
        if let Ok(tap) = tap
            && let Ok(source) = tap.mach_port.create_runloop_source(0)
        {
            let runloop = CFRunLoop::get_current();
            unsafe {
                runloop.add_source(&source, kCFRunLoopCommonModes);
            }
            tap.enable();
            // Visible confirmation that the hook is live: installing the tap is
            // what Input Monitoring gates, and it fails quietly otherwise.
            // Creating the tap succeeds even without Input Monitoring; such a
            // tap simply never receives an event, so this line alone does not
            // mean the hook works. Only a logged Ctrl transition does.
            note("[selection] double-tap Ctrl event tap created");
            CFRunLoop::run_current();
        } else {
            note("[selection] hook NOT installed - grant Input Monitoring to this app");
        }
    });
}
#[cfg(windows)]
mod win {
    use super::{DoubleCtrl, Wake};
    use std::sync::{Mutex, OnceLock, mpsc::Sender};
    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationTextPattern, UIA_TextPatternId,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, HHOOK, KBDLLHOOKSTRUCT, MSG,
        SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN,
        WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    struct Shared {
        state: DoubleCtrl,
        interval: std::time::Duration,
        tx: Sender<crate::action::Action>,
        wake: Wake,
    }
    static SHARED: OnceLock<Mutex<Shared>> = OnceLock::new();

    pub fn read() -> Option<String> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok().ok()?;
            struct ComGuard;
            impl Drop for ComGuard {
                fn drop(&mut self) {
                    unsafe {
                        CoUninitialize();
                    }
                }
            }
            let _com = ComGuard;
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
            let element = automation.GetFocusedElement().ok()?;
            let pattern: IUIAutomationTextPattern =
                element.GetCurrentPatternAs(UIA_TextPatternId).ok()?;
            let ranges = pattern.GetSelection().ok()?;
            let mut text = String::new();
            for i in 0..ranges.Length().ok()?.min(8) {
                let part = ranges.GetElement(i).ok()?.GetText(100_000).ok()?;
                text.push_str(&part.to_string());
            }
            peek_core::entry_text(peek_core::Entry::Selection, Some(&text))
        }
    }

    unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0
            && let Some(shared) = SHARED.get()
        {
            let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            let message = wparam.0 as u32;
            let is_ctrl = info.vkCode == 0xA2 || info.vkCode == 0xA3; // VK_LCONTROL / VK_RCONTROL
            let down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
            let up = message == WM_KEYUP || message == WM_SYSKEYUP;
            if let Ok(mut s) = shared.lock() {
                if !is_ctrl && down {
                    s.state.interrupt();
                } else if is_ctrl && (down || up) {
                    let interval = s.interval;
                    if s.state
                        .transition(down, std::time::Instant::now(), interval)
                    {
                        let tx = s.tx.clone();
                        let wake = s.wake.clone();
                        // Read off the hook thread: UI Automation must never block input.
                        std::thread::spawn(move || {
                            if let Some(text) = read() {
                                let _ = tx.send(crate::action::Action::Selection(text));
                                wake();
                            }
                        });
                    }
                }
            }
        }
        unsafe { CallNextHookEx(None::<HHOOK>, code, wparam, lparam) }
    }

    pub fn listen(interval: u64, tx: Sender<crate::action::Action>, wake: Wake) {
        let _ = SHARED.set(Mutex::new(Shared {
            state: DoubleCtrl::default(),
            interval: std::time::Duration::from_millis(interval),
            tx,
            wake,
        }));
        std::thread::spawn(|| unsafe {
            let Ok(handle) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), None, 0) else {
                return;
            };
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = UnhookWindowsHookEx(handle);
        });
    }
}
#[cfg(windows)]
pub use win::listen;

#[cfg(not(any(target_os = "macos", windows)))]
pub fn listen(_interval: u64, _tx: std::sync::mpsc::Sender<crate::action::Action>, _wake: Wake) {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accessibility_selection_offsets_handle_chinese_and_surrogate_pairs() {
        assert_eq!(
            selected_utf16("a中文😀end", 1, 4).as_deref(),
            Some("中文😀")
        );
        assert_eq!(selected_utf16("a中文😀end", 5, 3).as_deref(), Some("end"));
        assert!(selected_utf16("a中文😀end", 3, 1).is_none());
        assert!(selected_utf16("abc", -1, 1).is_none());
        assert!(selected_utf16("abc", 2, 2).is_none());
        assert!(selected_utf16("abc", 0, 0).is_none());
    }
    #[test]
    fn long_hold_and_slow_double_press_do_not_trigger() {
        let mut d = DoubleCtrl::default();
        let t = std::time::Instant::now();
        let w = std::time::Duration::from_millis(350);
        assert!(!d.transition(true, t, w));
        assert!(!d.transition(false, t + w * 2, w));
        assert!(!d.transition(true, t + w * 2 + w / 4, w));
        assert!(!d.transition(false, t + w * 2 + w / 4, w));
        assert!(!d.transition(true, t + w * 4, w));
        assert!(!d.transition(false, t + w * 4, w));
    }
    #[test]
    fn two_complete_taps_only() {
        let mut d = DoubleCtrl::default();
        let t = std::time::Instant::now();
        let window = std::time::Duration::from_millis(350);
        assert!(!d.transition(true, t, window));
        assert!(!d.transition(false, t, window));
        assert!(!d.transition(true, t + window / 2, window));
        assert!(d.transition(false, t + window / 2, window));
        assert!(!d.transition(true, t + window, window));
        assert!(!d.transition(false, t + window, window));
    }
    #[test]
    fn copy_shortcut_does_not_trigger() {
        let mut d = DoubleCtrl::default();
        let t = std::time::Instant::now();
        let w = std::time::Duration::from_millis(350);
        d.transition(true, t, w);
        d.interrupt();
        assert!(!d.transition(false, t, w));
        d.transition(true, t, w);
        assert!(!d.transition(false, t, w));
    }
}

#[cfg(test)]
mod generation_tests {
    #[test]
    fn new_read_or_invalidation_makes_previous_result_stale() {
        let old = super::next_generation();
        let latest = super::next_generation();
        assert!(!super::is_current(old));
        assert!(super::is_current(latest));
        super::next_generation();
        assert!(!super::is_current(latest));
    }
}
