//! Read the focused selection; on macOS, fall back to a fresh copy and restore
//! the complete clipboard. Never substitute pre-existing clipboard text.
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
        boolean::CFBoolean,
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
        fn AXUIElementSetAttributeValue(element: Ax, attr: Ax, value: Ax) -> i32;
        fn CGEventSourceFlagsState(state: i32) -> u64;
        fn AXIsProcessTrusted() -> bool;
        fn AXValueGetTypeID() -> usize;
        fn AXValueGetValue(value: Ax, kind: u32, output: *mut c_void) -> bool;
    }
    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        fn IsSecureEventInputEnabled() -> bool;
    }
    static READ_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
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
    fn system_focus() -> Option<Owned> {
        let raw = unsafe { AXUIElementCreateSystemWide() };
        if raw.is_null() {
            return None;
        }
        let system = Owned(raw);
        unsafe {
            AXUIElementSetMessagingTimeout(system.0, 0.2);
        }
        system.attr("AXFocusedUIElement")
    }
    fn element_pid(element: &Owned) -> Option<i32> {
        let mut pid = 0;
        (unsafe { AXUIElementGetPid(element.0, &mut pid) } == 0 && pid > 0).then_some(pid)
    }
    fn current_source() -> Option<i32> {
        // Spotlight and other floating search panels can own keyboard focus
        // while NSWorkspace still names the application underneath them.
        system_focus()
            .and_then(|element| element_pid(&element))
            .or_else(foreground_pid)
    }
    fn source_is_current(pid: i32) -> bool {
        current_source() == Some(pid)
    }
    fn enable_chromium_accessibility(app: &Owned, pid: i32) {
        let chromium = objc2::rc::autoreleasepool(|_| unsafe {
            use objc2::{class, msg_send, runtime::AnyObject};
            let running: *mut AnyObject = msg_send![class!(NSRunningApplication), runningApplicationWithProcessIdentifier: pid];
            if running.is_null() {
                return false;
            }
            let bundle: *mut AnyObject = msg_send![running, bundleIdentifier];
            if bundle.is_null() {
                return false;
            }
            let bundle = CFString::wrap_under_get_rule(bundle.cast()).to_string();
            [
                "com.google.Chrome",
                "com.microsoft.VSCode",
                "com.microsoft.edgemac",
                "com.brave.Browser",
                "com.vscodium",
            ]
            .iter()
            .any(|prefix| bundle.starts_with(prefix))
        });
        if !chromium {
            return;
        }
        for attribute in ["AXManualAccessibility", "AXEnhancedUserInterface"] {
            let name = CFString::new(attribute);
            let status = unsafe {
                AXUIElementSetAttributeValue(
                    app.0,
                    name.as_concrete_TypeRef().cast(),
                    CFBoolean::true_value().as_concrete_TypeRef().cast(),
                )
            };
            note(&format!(
                "[selection] enable {attribute}: pid={pid}, status={status}"
            ));
        }
    }

    /// Only a copy produced after our request can become a selection. Saved
    /// pasteboard items include every type, not just text or the first item.
    use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};

    fn snapshot_clipboard(board: *mut AnyObject) -> Option<Vec<Retained<AnyObject>>> {
        unsafe {
            let old: *mut AnyObject = msg_send![board, pasteboardItems];
            let mut saved: Vec<Retained<AnyObject>> = Vec::new();
            let count: usize = if old.is_null() {
                0
            } else {
                msg_send![old, count]
            };
            let mut total = 0usize;
            for index in 0..count {
                let item: *mut AnyObject = msg_send![old, objectAtIndex: index];
                let types: *mut AnyObject = msg_send![item, types];
                let types_count: usize = msg_send![types, count];
                let copy: Retained<AnyObject> = msg_send![class!(NSPasteboardItem), new];
                for t in 0..types_count {
                    let kind: *mut AnyObject = msg_send![types, objectAtIndex: t];
                    let data: *mut AnyObject = msg_send![item, dataForType: kind];
                    if data.is_null() {
                        note("[selection] clipboard format cannot be saved; copy skipped");
                        return None;
                    }
                    let length: usize = msg_send![data, length];
                    total = total.checked_add(length)?;
                    if total > 64 * 1024 * 1024 {
                        note(
                            "[selection] clipboard snapshot exceeds safe memory bound; copy skipped",
                        );
                        return None;
                    }
                    let data: Retained<AnyObject> = msg_send![data, copy];
                    let ok: bool = msg_send![&*copy, setData: &*data, forType: kind];
                    if !ok {
                        return None;
                    }
                }
                saved.push(copy);
            }
            Some(saved)
        }
    }

    struct Restore {
        board: *mut AnyObject,
        items: Vec<Retained<AnyObject>>,
        expected: isize,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            unsafe {
                let current: isize = msg_send![self.board, changeCount];
                if current != self.expected {
                    return;
                }
                let _: isize = msg_send![self.board, clearContents];
                if !self.items.is_empty() {
                    let array: Retained<AnyObject> = msg_send![class!(NSMutableArray), new];
                    for item in &self.items {
                        let _: () = msg_send![&*array, addObject: &**item];
                    }
                    let ok: bool = msg_send![self.board, writeObjects: &*array];
                    if !ok {
                        note("[selection] clipboard restoration failed");
                    }
                }
            }
        }
    }

    fn copy_selection(pid: i32, generation: u64) -> Option<String> {
        use core_graphics::{
            event::{CGEvent, CGEventFlags},
            event_source::{CGEventSource, CGEventSourceStateID},
        };
        use objc2::{class, msg_send, runtime::AnyObject};
        note(&format!("[selection] starting fresh copy: pid={pid}"));
        if !unsafe { AXIsProcessTrusted() } || unsafe { IsSecureEventInputEnabled() } {
            note("[selection] copy unavailable: accessibility or secure input");
            return None;
        }
        let modifiers = CGEventFlags::CGEventFlagCommand
            | CGEventFlags::CGEventFlagShift
            | CGEventFlags::CGEventFlagControl
            | CGEventFlags::CGEventFlagAlternate;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(900);
        while unsafe { CGEventSourceFlagsState(1) } & modifiers.bits() != 0 {
            if !super::is_current(generation)
                || !source_is_current(pid)
                || std::time::Instant::now() >= deadline
            {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let down = CGEvent::new_keyboard_event(
            CGEventSource::new(CGEventSourceStateID::Private).ok()?,
            8,
            true,
        )
        .ok()?;
        let up = CGEvent::new_keyboard_event(
            CGEventSource::new(CGEventSourceStateID::Private).ok()?,
            8,
            false,
        )
        .ok()?;
        down.set_flags(CGEventFlags::CGEventFlagCommand);
        up.set_flags(CGEventFlags::CGEventFlagCommand);
        objc2::rc::autoreleasepool(|_| unsafe {
            let board: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
            let original_count: isize = msg_send![board, changeCount];
            let saved = snapshot_clipboard(board)?;
            let now: isize = msg_send![board, changeCount];
            if now != original_count || !super::is_current(generation) || !source_is_current(pid) {
                return None;
            }
            let baseline: isize = msg_send![board, clearContents];
            // Hold a guard even on cancellation / timeout. Do not overwrite a
            // later clipboard change made by the user or another application.
            let mut restore = Restore {
                board,
                items: saved,
                expected: baseline,
            };
            down.post_to_pid(pid);
            up.post_to_pid(pid);
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(600);
            let kind = CFString::new("public.utf8-plain-text");
            while std::time::Instant::now() < deadline {
                let count: isize = msg_send![board, changeCount];
                if count != baseline {
                    // A normal copy declares one new pasteboard generation.
                    // More changes could belong to a concurrent user copy;
                    // leave that clipboard alone instead of restoring over it.
                    if count != baseline.saturating_add(1) {
                        note("[selection] clipboard changed again; leave newer contents intact");
                        return None;
                    }
                    restore.expected = count;
                    let value: *mut AnyObject = msg_send![board, stringForType: kind.as_concrete_TypeRef().cast::<AnyObject>()];
                    if !value.is_null() {
                        let text = CFString::wrap_under_get_rule(value.cast()).to_string();
                        let stable: isize = msg_send![board, changeCount];
                        if stable == count
                            && super::is_current(generation)
                            && source_is_current(pid)
                        {
                            let text =
                                peek_core::entry_text(peek_core::Entry::Selection, Some(&text));
                            note(&format!(
                                "[selection] fresh copy: pid={pid}, chars={}",
                                text.as_ref().map_or(0, |s| s.chars().count())
                            ));
                            return text;
                        }
                        return None;
                    }
                }
                // If the user switches away, still wait for the posted copy to
                // complete so restoration cannot race its delayed clipboard write.
                // The current-source check above rejects the resulting text.
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            note(&format!("[selection] fresh copy timed out: pid={pid}"));
            None
        })
    }

    pub fn read() -> Option<String> {
        let generation = super::GENERATION.load(std::sync::atomic::Ordering::SeqCst);
        let _read_guard = READ_LOCK.lock().ok()?;
        if !super::is_current(generation) {
            return None;
        }
        let pid = current_source()?;
        if pid == std::process::id() as i32 {
            return None;
        }
        note(&format!(
            "[selection] source pid={pid}, foreground={:?}",
            foreground_pid()
        ));
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
        let mut explicit_empty = false;
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(80));
            }
            if !super::is_current(generation) || !source_is_current(pid) {
                return None;
            }
            if attempt == 1 {
                enable_chromium_accessibility(&app, pid);
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
                    if let Some(range) = element.attr("AXSelectedTextRange") {
                        let mut offsets = Range {
                            location: 0,
                            length: 0,
                        };
                        let valid = unsafe {
                            CFGetTypeID(range.0) == AXValueGetTypeID()
                                && AXValueGetValue(range.0, 4, (&mut offsets as *mut Range).cast())
                        };
                        explicit_empty |= valid && offsets.length == 0;
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
        // Editors such as VS Code copy the whole line when no text is selected.
        // An explicit empty range must not turn that into a selection query.
        if explicit_empty {
            note("[selection] explicit empty selection; copy skipped");
            return None;
        }
        copy_selection(pid, generation)
    }
    #[cfg(test)]
    mod clipboard_tests {
        use super::*;
        use objc2_foundation::NSData;

        fn with_board(test: impl FnOnce(*mut AnyObject)) {
            objc2::rc::autoreleasepool(|_| unsafe {
                // Never touch the general / user clipboard in regression tests.
                let board: *mut AnyObject =
                    msg_send![class!(NSPasteboard), pasteboardWithUniqueName];
                assert!(!board.is_null());
                test(board);
                let _: () = msg_send![board, releaseGlobally];
            });
        }
        fn write(board: *mut AnyObject, items: &[Vec<(&str, &[u8])>]) -> isize {
            unsafe {
                let _: isize = msg_send![board, clearContents];
                let array: Retained<AnyObject> = msg_send![class!(NSMutableArray), new];
                for fields in items {
                    let item: Retained<AnyObject> = msg_send![class!(NSPasteboardItem), new];
                    for (kind, bytes) in fields {
                        let kind = CFString::new(kind);
                        let data = NSData::with_bytes(bytes);
                        let ok: bool = msg_send![&*item, setData: &*data, forType: kind.as_concrete_TypeRef().cast::<AnyObject>()];
                        assert!(ok);
                    }
                    let _: () = msg_send![&*array, addObject: &*item];
                }
                let ok: bool = msg_send![board, writeObjects: &*array];
                assert!(ok);
                msg_send![board, changeCount]
            }
        }
        fn contents(board: *mut AnyObject) -> Vec<Vec<(String, Vec<u8>)>> {
            unsafe {
                let items: *mut AnyObject = msg_send![board, pasteboardItems];
                if items.is_null() {
                    return vec![];
                }
                let count: usize = msg_send![items, count];
                (0..count)
                    .map(|i| {
                        let item: *mut AnyObject = msg_send![items, objectAtIndex: i];
                        let types: *mut AnyObject = msg_send![item, types];
                        let count: usize = msg_send![types, count];
                        (0..count)
                            .map(|i| {
                                let kind: *mut AnyObject = msg_send![types, objectAtIndex: i];
                                let data: *mut AnyObject = msg_send![item, dataForType: kind];
                                assert!(!data.is_null());
                                let len: usize = msg_send![data, length];
                                let bytes: *const c_void = msg_send![data, bytes];
                                (
                                    CFString::wrap_under_get_rule(kind.cast()).to_string(),
                                    if len == 0 {
                                        vec![]
                                    } else {
                                        std::slice::from_raw_parts(bytes.cast::<u8>(), len).to_vec()
                                    },
                                )
                            })
                            .collect()
                    })
                    .collect()
            }
        }
        #[test]
        fn restores_all_formats_and_items_after_a_copy() {
            with_board(|board| {
                write(
                    board,
                    &[
                        vec![
                            ("public.utf8-plain-text", "中文😀".as_bytes()),
                            ("public.html", b"<b>rich text</b>"),
                            ("com.crantpeek.test.binary", &[0, 1, 255]),
                        ],
                        vec![("public.utf8-plain-text", b"second item")],
                    ],
                );
                let before = contents(board);
                let saved = snapshot_clipboard(board).unwrap();
                let expected = write(
                    board,
                    &[vec![("public.utf8-plain-text", b"fresh selected text")]],
                );
                drop(Restore {
                    board,
                    items: saved,
                    expected,
                });
                assert_eq!(contents(board), before);
            });
        }
        #[test]
        fn newer_clipboard_change_is_never_overwritten() {
            with_board(|board| {
                write(board, &[vec![("public.utf8-plain-text", b"original")]]);
                let saved = snapshot_clipboard(board).unwrap();
                let expected = write(board, &[vec![("public.utf8-plain-text", b"our copy")]]);
                write(
                    board,
                    &[vec![("public.utf8-plain-text", b"newer user copy")]],
                );
                let newer = contents(board);
                drop(Restore {
                    board,
                    items: saved,
                    expected,
                });
                assert_eq!(contents(board), newer);
            });
        }
        #[test]
        fn cancelled_or_empty_copy_restores_original_data() {
            with_board(|board| unsafe {
                write(
                    board,
                    &[vec![(
                        "public.utf8-plain-text",
                        b"old clipboard must not become selection",
                    )]],
                );
                let before = contents(board);
                let items = snapshot_clipboard(board).unwrap();
                let expected: isize = msg_send![board, clearContents];
                assert!(contents(board).is_empty());
                drop(Restore {
                    board,
                    items,
                    expected,
                });
                assert_eq!(contents(board), before);
            });
        }
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
pub use win::{listen, read};

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
