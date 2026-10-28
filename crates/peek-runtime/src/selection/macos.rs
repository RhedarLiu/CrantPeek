use super::note;
use core_foundation::{
    array::{CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex},
    base::{CFEqual, CFGetTypeID, CFRelease, CFRetain, CFTypeRef, TCFType},
    boolean::CFBoolean,
    number::CFNumber,
    string::CFString,
};
use std::ffi::c_void;
use std::time::{Duration, Instant};
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
    fn AXUIElementGetTypeID() -> usize;
    fn AXUIElementPerformAction(element: Ax, action: Ax) -> i32;
    fn CGEventSourceFlagsState(state: i32) -> u64;
    fn CGEventSourceCounterForEventType(state: i32, event: u32) -> u32;
    fn AXIsProcessTrusted() -> bool;
    fn AXValueGetTypeID() -> usize;
    fn AXValueGetValue(value: Ax, kind: u32, output: *mut c_void) -> bool;
    #[cfg(test)]
    fn AXValueCreate(kind: u32, value: *const c_void) -> Ax;
}
#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn IsSecureEventInputEnabled() -> bool;
}
static READ_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
// Every descendant shares its traversal's deadline. A per-message timeout
// alone still allows a hung app to stall dozens of consecutive AX requests.
struct Owned(Ax, Instant);
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.0);
        }
    }
}
impl Owned {
    fn prepare(&self) -> Option<()> {
        let left = self.1.checked_duration_since(Instant::now())?;
        if unsafe { CFGetTypeID(self.0) } != unsafe { AXUIElementGetTypeID() } {
            return None;
        }
        unsafe {
            AXUIElementSetMessagingTimeout(self.0, left.as_secs_f32().min(0.12));
        }
        Some(())
    }
    fn attr(&self, name: &str) -> Option<Self> {
        self.prepare()?;
        let mut value = std::ptr::null();
        let focused = name == "AXFocusedUIElement";
        let name = CFString::new(name);
        let status = unsafe {
            AXUIElementCopyAttributeValue(self.0, name.as_concrete_TypeRef().cast(), &mut value)
        };
        if status == 0 && !value.is_null() {
            Some(Self(value, self.1))
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
        self.prepare()?;
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
            Some(Self(value, self.1))
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
    fn boolean(&self) -> Option<bool> {
        (unsafe { CFGetTypeID(self.0) } == CFBoolean::type_id())
            .then(|| unsafe { CFBoolean::wrap_under_get_rule(self.0.cast()).into() })
    }
    fn number(&self) -> Option<i64> {
        if unsafe { CFGetTypeID(self.0) } != CFNumber::type_id() {
            return None;
        }
        unsafe { CFNumber::wrap_under_get_rule(self.0.cast()).to_i64() }
    }
    fn array(&self, limit: usize) -> Vec<Self> {
        unsafe {
            if CFGetTypeID(self.0) != CFArrayGetTypeID() {
                return vec![];
            }
            let count = CFArrayGetCount(self.0.cast()).max(0) as usize;
            (0..count.min(limit))
                .filter_map(|i| {
                    let value = CFArrayGetValueAtIndex(self.0.cast(), i as isize);
                    (!value.is_null()).then(|| Self(CFRetain(value), self.1))
                })
                .collect()
        }
    }
    fn range(&self) -> Option<Range> {
        let mut range = Range {
            location: 0,
            length: 0,
        };
        let valid = unsafe {
            CFGetTypeID(self.0) == AXValueGetTypeID()
                && AXValueGetValue(self.0, 4, (&mut range as *mut Range).cast())
        };
        // Cocoa uses NSNotFound (NSIntegerMax) for unsupported ranges; that
        // is not an explicit empty selection and must not disable copying.
        (valid && range.location >= 0 && range.location != isize::MAX && range.length >= 0)
            .then_some(range)
    }
    fn secure(&self) -> bool {
        ["AXRole", "AXSubrole"].iter().any(|key| {
            self.attr(key).and_then(|v| v.string()).as_deref() == Some("AXSecureTextField")
        })
    }
    fn copy_action(&self) -> bool {
        if self.prepare().is_none() {
            return false;
        }
        let action = CFString::new("AXPress");
        unsafe { AXUIElementPerformAction(self.0, action.as_concrete_TypeRef().cast()) == 0 }
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
        // Native editors can expose disjoint selections without AXSelectedText.
        if let Some(ranges) = self.attr("AXSelectedTextRanges") {
            let ranges = ranges.array(33);
            // Do not silently return only the first 32 selections.
            if (2..=32).contains(&ranges.len()) {
                let text: Option<Vec<String>> = ranges
                    .iter()
                    .map(|range| self.text_for_range(range))
                    .collect();
                if let Some(text) = text.and_then(|parts| selection_text(&parts.join("\n"))) {
                    note("[selection] read via multiple selected ranges");
                    return Some(text);
                }
            }
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
    fn text_for_range(&self, range: &Owned) -> Option<String> {
        let offsets = range.range()?;
        if offsets.length == 0 || offsets.length as usize > peek_core::MAX_INPUT_BYTES {
            return None;
        }
        self.parameter("AXStringForRange", range)
            .and_then(|v| v.string())
            .or_else(|| {
                self.attr("AXValue")
                    .and_then(|v| v.string())
                    .and_then(|text| super::selected_utf16(&text, offsets.location, offsets.length))
            })
    }
}
fn selection_text(text: &str) -> Option<String> {
    (text.len() <= peek_core::MAX_INPUT_BYTES)
        .then(|| peek_core::entry_text(peek_core::Entry::Selection, Some(text)))
        .flatten()
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
    let system = Owned(raw, Instant::now() + Duration::from_millis(300));
    // Prefer Apple's documented two-step path; some apps/macOS versions do
    // not answer AXFocusedUIElement directly on the system-wide object.
    system
        .attr("AXFocusedApplication")
        .and_then(|app| app.attr("AXFocusedUIElement"))
        .or_else(|| system.attr("AXFocusedUIElement"))
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
        .or_else(|| {
            let raw = unsafe { AXUIElementCreateSystemWide() };
            if raw.is_null() {
                return None;
            }
            let system = Owned(raw, Instant::now() + Duration::from_millis(120));
            element_pid(&system.attr("AXFocusedApplication")?)
        })
        .or_else(foreground_pid)
}
fn source_is_current(pid: i32) -> bool {
    current_source() == Some(pid)
}
fn enable_chromium_accessibility(app: &Owned, pid: i32) {
    // Discover the engine, not just five hard-coded application IDs. This also
    // covers newer Chromium browsers and Electron editors/chat clients.
    let chromium = objc2::rc::autoreleasepool(|_| unsafe {
        let running: *mut AnyObject =
            msg_send![class!(NSRunningApplication), runningApplicationWithProcessIdentifier: pid];
        if running.is_null() {
            return false;
        }
        let url: *mut AnyObject = msg_send![running, bundleURL];
        if url.is_null() {
            return false;
        }
        let path: *mut AnyObject = msg_send![url, path];
        if path.is_null() {
            return false;
        }
        let path = CFString::wrap_under_get_rule(path.cast()).to_string();
        std::fs::read_dir(std::path::Path::new(&path).join("Contents/Frameworks"))
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .take(64)
            .any(|entry| {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                name.ends_with(" helper (renderer).app")
                    || name.ends_with(".framework")
                        && (name.contains("electron")
                            || name.contains("chromium")
                            || name.contains("chrome")
                            || name == "qqnt.framework")
            })
    });
    let manual = app.attr("AXManualAccessibility").and_then(|v| v.boolean());
    if !chromium && manual.is_none() {
        return;
    }
    for attribute in ["AXManualAccessibility", "AXEnhancedUserInterface"] {
        if app.prepare().is_none() {
            break;
        }
        if app.attr(attribute).and_then(|v| v.boolean()) == Some(true) {
            continue;
        }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InputStamp([u32; 4]);
impl InputStamp {
    fn now() -> Self {
        // HID state excludes our private synthetic events. Modifier release is
        // allowed; a new key/click means the user's selection may have changed.
        Self([10, 1, 3, 25].map(|event| unsafe { CGEventSourceCounterForEventType(1, event) }))
    }
}

struct Source {
    pid: i32,
    generation: u64,
    input: InputStamp,
    fixture: bool,
}
impl Source {
    fn current(&self) -> bool {
        super::is_current(self.generation)
            && (self.fixture || (self.input == InputStamp::now() && source_is_current(self.pid)))
    }
}

fn focused(app: &Owned, pid: i32) -> Option<Owned> {
    let mut focus = app.attr("AXFocusedUIElement").or_else(system_focus)?;
    if element_pid(&focus) != Some(pid) {
        return None;
    }
    focus.1 = app.1;
    Some(focus)
}

#[derive(Default)]
struct SelectionEvidence {
    // Only the final focused text control can prove an empty selection.
    // Empty ranges on ancestors and earlier attempts are not authoritative.
    empty: bool,
    positive: bool,
    secure: bool,
}
impl SelectionEvidence {
    fn copy_allowed(&self) -> bool {
        !self.secure && (!self.empty || self.positive)
    }
}

fn read_hierarchy(app: &Owned, pid: i32) -> (Option<String>, SelectionEvidence) {
    let mut evidence = SelectionEvidence::default();
    let Some(mut element) = focused(app, pid) else {
        return (None, evidence);
    };
    for depth in 0..10 {
        if element.secure() {
            evidence.secure = true;
            break;
        }
        let role = element
            .attr("AXRole")
            .and_then(|v| v.string())
            .unwrap_or_default();
        if let Some(range) = element.attr("AXSelectedTextRange").and_then(|v| v.range()) {
            evidence.positive |= range.length > 0;
            if depth == 0 && matches!(role.as_str(), "AXTextField" | "AXTextArea" | "AXComboBox") {
                evidence.empty = range.length == 0;
            }
        }
        if let Some(text) = element.selection() {
            return (Some(text), evidence);
        }
        // Some native/WebKit views focus a container rather than its text
        // child. Limit this to the immediate focused subtree (no window scan).
        if depth == 0 && matches!(role.as_str(), "AXGroup" | "AXScrollArea" | "AXWebArea") {
            for child in element.attr("AXChildren").map_or(vec![], |v| v.array(12)) {
                if element_pid(&child) != Some(pid) || child.secure() {
                    continue;
                }
                if let Some(text) = child.selection() {
                    return (Some(text), evidence);
                }
            }
        }
        if matches!(role.as_str(), "AXWindow" | "AXApplication") {
            break;
        }
        let Some(parent) = element.attr("AXParent") else {
            break;
        };
        if element_pid(&parent) != Some(pid) || unsafe { CFEqual(parent.0, element.0) != 0 } {
            break;
        }
        element = parent;
    }
    (None, evidence)
}

fn is_copy_shortcut(character: &str, modifiers: Option<i64>) -> bool {
    // In AX menu metadata Command is implicit; 0 means Command alone.
    character.eq_ignore_ascii_case("c") && modifiers == Some(0)
}

fn copy_menu(app: &Owned) -> Option<Owned> {
    let bar = app.attr("AXMenuBar")?;
    // Copy is a direct item in a top-level menu. Do not search contextual
    // submenus such as Copy Link, Copy Image or developer tools commands.
    for top in bar.attr("AXChildren")?.array(16) {
        for menu in top.attr("AXChildren").map_or(vec![], |v| v.array(1)) {
            for item in menu.attr("AXChildren").map_or(vec![], |v| v.array(64)) {
                let command = item
                    .attr("AXMenuItemCmdChar")
                    .and_then(|v| v.string())
                    .unwrap_or_default();
                if !command.eq_ignore_ascii_case("c") {
                    continue;
                }
                let modifiers = item.attr("AXMenuItemCmdModifiers").and_then(|v| v.number());
                if is_copy_shortcut(&command, modifiers)
                    && item.attr("AXEnabled").and_then(|v| v.boolean()) == Some(true)
                {
                    return Some(item);
                }
            }
        }
    }
    None
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
                    note("[selection] clipboard snapshot exceeds safe memory bound; copy skipped");
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

/// A pasteboard can publish its types and fulfill text at different times.
/// Wait for a stable generation AND readable text, not exactly baseline + 1.
struct CopyPoll {
    baseline: isize,
    last: isize,
    stable_since: Instant,
}
impl CopyPoll {
    fn new(baseline: isize) -> Self {
        Self {
            baseline,
            last: baseline,
            stable_since: Instant::now(),
        }
    }
    fn ready(&mut self, count: isize, text_ready: bool, now: Instant) -> bool {
        if count != self.last {
            self.last = count;
            self.stable_since = now;
        }
        count != self.baseline
            && text_ready
            && now.duration_since(self.stable_since) >= Duration::from_millis(40)
    }
}

fn clipboard_text(board: *mut AnyObject) -> Option<String> {
    // NSPasteboard converts the standard string types for us (including legacy
    // NSStringPboardType); don't assume every app publishes UTF-8 directly.
    for name in ["public.utf8-plain-text", "NSStringPboardType"] {
        let kind = CFString::new(name);
        let value: *mut AnyObject = unsafe {
            msg_send![board, stringForType: kind.as_concrete_TypeRef().cast::<AnyObject>()]
        };
        if !value.is_null() {
            let text = unsafe { CFString::wrap_under_get_rule(value.cast()).to_string() };
            if let Some(text) = selection_text(&text) {
                return Some(text);
            }
        }
    }
    None
}

fn post_copy(source: &Source, clipboard_unchanged: impl Fn() -> bool) -> bool {
    use core_graphics::{
        event::{CGEvent, CGEventFlags},
        event_source::{CGEventSource, CGEventSourceStateID},
    };
    let modifiers = CGEventFlags::CGEventFlagCommand
        | CGEventFlags::CGEventFlagShift
        | CGEventFlags::CGEventFlagControl
        | CGEventFlags::CGEventFlagAlternate;
    let deadline = Instant::now() + Duration::from_secs(2);
    while unsafe { CGEventSourceFlagsState(1) } & modifiers.bits() != 0 {
        if Instant::now() >= deadline || !source.current() {
            return false;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    if !source.current() || unsafe { IsSecureEventInputEnabled() } || !clipboard_unchanged() {
        return false;
    }
    // A complete Command-down/C-down/C-up/Command-up sequence supports custom
    // event handlers too. A private source never releases the user's real keys.
    let Ok(event_source) = CGEventSource::new(CGEventSourceStateID::Private) else {
        return false;
    };
    let events: Option<Vec<_>> = [(55, true), (8, true), (8, false), (55, false)]
        .iter()
        .map(|(key, down)| CGEvent::new_keyboard_event(event_source.clone(), *key, *down).ok())
        .collect();
    let Some(events) = events else {
        return false;
    };
    for (i, event) in events.iter().enumerate() {
        event.set_flags(if i == 3 {
            CGEventFlags::empty()
        } else {
            CGEventFlags::CGEventFlagCommand
        });
        event.post_to_pid(source.pid);
    }
    true
}

fn copy_selection(source: &Source, menu: Option<Owned>) -> Option<String> {
    if !source.current() || unsafe { IsSecureEventInputEnabled() } {
        return None;
    }
    objc2::rc::autoreleasepool(|_| unsafe {
        let board: *mut AnyObject = if source.fixture {
            let name = CFString::new("dev.crant.selection-fixture");
            msg_send![class!(NSPasteboard), pasteboardWithName: name.as_concrete_TypeRef().cast::<AnyObject>()]
        } else {
            msg_send![class!(NSPasteboard), generalPasteboard]
        };
        if board.is_null() {
            return None;
        }
        let baseline: isize = msg_send![board, changeCount];
        let saved = snapshot_clipboard(board)?;
        let now: isize = msg_send![board, changeCount];
        if now != baseline || !source.current() {
            return None;
        }
        // Do not clear the user's pasteboard to request a copy. If the app does
        // nothing, its old content is untouched and can never become a result.
        let via_menu = menu.is_some_and(|mut item| {
            item.1 = Instant::now() + Duration::from_millis(200);
            source.current() && item.copy_action()
        });
        if !via_menu
            && !post_copy(source, || {
                let count: isize = msg_send![board, changeCount];
                count == baseline
            })
        {
            return None;
        }
        note(if via_menu {
            "[selection] copy requested via application menu"
        } else {
            "[selection] copy requested via targeted keyboard events"
        });
        let deadline = Instant::now() + Duration::from_millis(1400);
        let mut poll = CopyPoll::new(baseline);
        let mut restore: Option<Restore> = None;
        let mut interrupted = false;
        loop {
            if !source.fixture && InputStamp::now() != source.input {
                // A real key/click invalidates ownership; do not restore over
                // the user's concurrent copy, even if it has only one write.
                if let Some(restore) = restore.as_mut() {
                    restore.expected = isize::MIN;
                }
                // Keep the serialization lock until the already-posted copy
                // has had its settling window. Otherwise a quick second
                // hotkey can mistake the first app's late write for its own.
                interrupted = true;
            }
            let count: isize = msg_send![board, changeCount];
            if !interrupted
                && count != baseline
                && restore.is_none()
                && count == baseline.saturating_add(1)
            {
                // Only a single write is unambiguous enough to restore. If a
                // clipboard manager/user writes again, Restore leaves it alone.
                restore = Some(Restore {
                    board,
                    items: saved.clone(),
                    expected: count,
                });
            }
            let text = (count != baseline).then(|| clipboard_text(board)).flatten();
            let stable: isize = msg_send![board, changeCount];
            if !interrupted && poll.ready(stable, stable == count && text.is_some(), Instant::now())
            {
                if source.current() {
                    return text;
                }
                return None;
            }
            if Instant::now() >= deadline {
                note("[selection] copy timed out without readable text");
                return None;
            }
            // Even a cancelled generation waits for a dispatched copy to settle
            // before releasing READ_LOCK, so it cannot land in the next request.
            std::thread::sleep(Duration::from_millis(15));
        }
    })
}

pub fn read(generation: u64) -> Option<String> {
    if !super::is_current(generation) || !unsafe { AXIsProcessTrusted() } {
        return None;
    }
    // Capture the source before waiting for any previous copy to restore.
    let input = InputStamp::now();
    let pid = current_source()?;
    if pid == std::process::id() as i32 {
        return None;
    }
    let source = Source {
        pid,
        generation,
        input,
        fixture: false,
    };
    read_source(source)
}

pub fn read_test_fixture(pid: i32, generation: u64) -> Option<String> {
    let fixture = objc2::rc::autoreleasepool(|_| unsafe {
        let running: *mut AnyObject =
            msg_send![class!(NSRunningApplication), runningApplicationWithProcessIdentifier: pid];
        if running.is_null() {
            return false;
        }
        let bundle: *mut AnyObject = msg_send![running, bundleIdentifier];
        !bundle.is_null()
            && CFString::wrap_under_get_rule(bundle.cast()) == "dev.crant.selection-fixture"
    });
    if !fixture || !unsafe { AXIsProcessTrusted() } {
        return None;
    }
    read_source(Source {
        pid,
        generation,
        input: InputStamp::now(),
        fixture: true,
    })
}

fn read_source(source: Source) -> Option<String> {
    let started = Instant::now();
    let Source {
        pid, generation, ..
    } = source;
    let _read_guard = READ_LOCK.lock().ok()?;
    if !source.current() {
        return None;
    }
    note(&format!(
        "[selection] source pid={pid}, generation={generation}"
    ));
    let raw = unsafe { AXUIElementCreateApplication(pid) };
    if raw.is_null() {
        return None;
    }
    let mut app = Owned(raw, Instant::now() + Duration::from_millis(1000));
    let mut final_evidence = SelectionEvidence::default();
    for attempt in 0..3 {
        if !source.current() {
            return None;
        }
        if attempt == 1 {
            enable_chromium_accessibility(&app, pid);
        }
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(80 * attempt));
        }
        let (text, evidence) = read_hierarchy(&app, pid);
        final_evidence = evidence;
        if final_evidence.secure {
            return None;
        }
        if let Some(text) = text {
            if !source.current() {
                return None;
            }
            note(&format!(
                "[selection] completed via AX in {}ms",
                started.elapsed().as_millis()
            ));
            return Some(text);
        }
        if Instant::now() >= app.1 {
            break;
        }
    }
    if !final_evidence.copy_allowed() {
        note("[selection] focused text control has no selection; copy skipped");
        return None;
    }
    app.1 = Instant::now() + Duration::from_millis(500);
    let menu = copy_menu(&app);
    let text = copy_selection(&source, menu);
    note(&format!(
        "[selection] completed via copy in {}ms, success={}",
        started.elapsed().as_millis(),
        text.is_some()
    ));
    text
}

#[cfg(test)]
mod clipboard_tests {
    use super::*;
    use objc2_foundation::NSData;

    #[test]
    fn copy_poll_waits_for_delayed_text_and_stable_multi_write_generations() {
        let start = Instant::now();
        let mut poll = CopyPoll::new(10);
        assert!(!poll.ready(10, true, start)); // pre-existing text
        assert!(!poll.ready(11, false, start)); // types announced, text not ready
        assert!(!poll.ready(12, true, start + Duration::from_millis(800)));
        assert!(!poll.ready(12, true, start + Duration::from_millis(820)));
        assert!(poll.ready(12, true, start + Duration::from_millis(850)));
        assert!(!poll.ready(13, true, start + Duration::from_millis(860)));
    }

    #[test]
    fn focused_empty_range_does_not_override_positive_ancestor_selection() {
        let mut evidence = SelectionEvidence {
            empty: true,
            ..Default::default()
        };
        assert!(!evidence.copy_allowed()); // don't copy the editor's whole line
        evidence.positive = true;
        assert!(evidence.copy_allowed()); // focused node was only a caret proxy
        evidence.secure = true;
        assert!(!evidence.copy_allowed());
        assert!(SelectionEvidence::default().copy_allowed()); // unsupported AX != empty
    }

    #[test]
    fn copy_menu_ignores_copy_path_and_missing_modifier_metadata() {
        assert!(is_copy_shortcut("c", Some(0)));
        assert!(is_copy_shortcut("C", Some(0)));
        for modifiers in [None, Some(1), Some(2), Some(4), Some(8)] {
            assert!(!is_copy_shortcut("c", modifiers));
        }
        assert!(!is_copy_shortcut("x", Some(0)));
    }

    #[test]
    fn selection_size_and_whitespace_are_bounded() {
        assert_eq!(selection_text(" 中文😀 \n").as_deref(), Some("中文😀"));
        assert!(selection_text(" \n").is_none());
        assert!(selection_text(&"x".repeat(peek_core::MAX_INPUT_BYTES + 1)).is_none());
    }

    #[test]
    fn nsnotfound_range_is_unavailable_not_an_empty_selection() {
        for (location, length, valid) in [
            (isize::MAX, 0, false),
            (-1, 0, false),
            (0, 0, true),
            (2, 3, true),
        ] {
            let range = Range { location, length };
            let raw = unsafe { AXValueCreate(4, (&range as *const Range).cast()) };
            assert!(!raw.is_null());
            let value = Owned(raw, Instant::now() + Duration::from_secs(1));
            assert_eq!(value.range().is_some(), valid);
        }
    }

    fn with_board(test: impl FnOnce(*mut AnyObject)) {
        // Mirror production serialization: AppKit pasteboard objects are not
        // safe to initialize/use concurrently from arbitrary test workers.
        let _guard = READ_LOCK.lock().unwrap();
        objc2::rc::autoreleasepool(|_| unsafe {
            // Never touch the general / user clipboard in regression tests.
            let board: *mut AnyObject = msg_send![class!(NSPasteboard), pasteboardWithUniqueName];
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
