//! Read only an explicit Accessibility selection. Never substitute clipboard text.
#[cfg(target_os = "macos")]
pub fn read() -> Option<String> {
    use core_foundation::{
        base::{CFRelease, CFTypeRef, TCFType},
        string::CFString,
    };
    use std::ffi::c_void;
    type Ax = *const c_void;
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXUIElementCreateSystemWide() -> Ax;
        fn AXUIElementCopyAttributeValue(
            element: Ax,
            attr: *const c_void,
            value: *mut CFTypeRef,
        ) -> i32;
        fn AXUIElementSetMessagingTimeout(element: Ax, timeout: f32) -> i32;
    }
    // Each successful Copy returns a retained object, released on every branch.
    unsafe {
        let system = AXUIElementCreateSystemWide();
        if system.is_null() {
            return None;
        }
        AXUIElementSetMessagingTimeout(system, 0.2);
        let focused = CFString::new("AXFocusedUIElement");
        let mut element: CFTypeRef = std::ptr::null();
        let status = AXUIElementCopyAttributeValue(
            system,
            focused.as_concrete_TypeRef().cast(),
            &mut element,
        );
        CFRelease(system);
        if status != 0 || element.is_null() {
            return None;
        }
        AXUIElementSetMessagingTimeout(element, 0.2);
        let selected = CFString::new("AXSelectedText");
        let mut value: CFTypeRef = std::ptr::null();
        let status = AXUIElementCopyAttributeValue(
            element,
            selected.as_concrete_TypeRef().cast(),
            &mut value,
        );
        CFRelease(element);
        if status != 0 || value.is_null() {
            return None;
        }
        if core_foundation::base::CFGetTypeID(value) != CFString::type_id() {
            CFRelease(value);
            return None;
        }
        let text = CFString::wrap_under_create_rule(value.cast()).to_string();
        peek_core::entry_text(peek_core::Entry::Selection, Some(&text))
    }
}
#[cfg(not(any(target_os = "macos", windows)))]
pub fn read() -> Option<String> {
    None
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
        let tap = CGEventTap::new(
            CGEventTapLocation::Session,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::ListenOnly,
            vec![CGEventType::FlagsChanged, CGEventType::KeyDown],
            move |_, kind, event| {
                let mut state = state.borrow_mut();
                if matches!(kind, CGEventType::KeyDown) {
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
                if state.transition(
                    flags.contains(CGEventFlags::CGEventFlagControl),
                    std::time::Instant::now(),
                    std::time::Duration::from_millis(interval),
                ) {
                    // Never block the event-tap callback on Accessibility IPC.
                    let tx = tx.clone();
                    let wake = wake.clone();
                    std::thread::spawn(move || {
                        if let Some(text) = read() {
                            let _ = tx.send(crate::action::Action::Selection(text));
                            wake();
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
            CFRunLoop::run_current();
        }
    });
}
#[cfg(windows)]
mod win {
    use super::DoubleCtrl;
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
