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
#[cfg(not(target_os = "macos"))]
pub fn read() -> Option<String> {
    None
}

/// Fires only after two standalone Ctrl press/release cycles; shortcuts interrupt it.
#[derive(Default)]
pub struct DoubleCtrl {
    down: bool,
    interrupted: bool,
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
            self.interrupted = false;
            return false;
        }
        if self.interrupted {
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
pub fn listen(
    interval: u64,
    tx: std::sync::mpsc::Sender<crate::desktop::Action>,
    ctx: eframe::egui::Context,
) {
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
                    // Do not steal focus before obtaining the selection.
                    if let Some(text) = read() {
                        let _ = tx.send(crate::desktop::Action::Selection(text));
                        ctx.request_repaint();
                    }
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
#[cfg(not(target_os = "macos"))]
pub fn listen(
    _interval: u64,
    _tx: std::sync::mpsc::Sender<crate::desktop::Action>,
    _ctx: eframe::egui::Context,
) {
}

#[cfg(test)]
mod tests {
    use super::*;
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
