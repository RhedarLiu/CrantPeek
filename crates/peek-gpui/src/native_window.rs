//! Showing and hiding the peek window.
//!
//! GPUI 0.3.x exposes no `hide()`/`show()` for a single window: `PlatformWindow`
//! only has `visibility()` (read-only), `minimize()`, and
//! `on_visibility_change()`. `minimize_window()` drops the window into the Dock,
//! which is the wrong feel for a peek that is meant to appear and leave, and
//! `remove_window()` destroys it.
//!
//! Showing therefore uses `Window::activate_window()` (macOS
//! `makeKeyAndOrderFront:`), and hiding goes through the native window reached
//! via `raw_window_handle::HasWindowHandle`. This is measured working in the
//! spike: `is_visible()` tracked `orderOut:` correctly in both directions, and
//! creating the window hidden plus activating it costs ~22 ms.
//!
//! This project already carries per-platform code for the hotkey hook, selection
//! reading and OCR, so a window-visibility shim is consistent with its shape.

use gpui_kit::{AppContext as _, Window};

/// Shows and focuses the peek window.
pub fn show(window: &mut Window) {
    #[cfg(windows)]
    windows_show(window);
    window.activate_window();
}

/// Refresh AppKit's input context after GPUI has focused a text editor.
/// Run outside the App borrow: querying the context can synchronously ask
/// GPUI's input handler for its current selection.
pub fn activate_text_input(window: &mut Window, cx: &mut gpui_kit::App) {
    #[cfg(target_os = "macos")]
    {
        let handle = window.window_handle();
        cx.spawn(async move |cx| {
            let view = cx
                .update_window(handle, |_, window, _| macos_view(window))
                .ok()
                .flatten();
            if let Some(view) = view {
                unsafe {
                    use objc2::{msg_send, runtime::AnyObject};
                    let view = view as *mut AnyObject;
                    let context: *mut AnyObject = msg_send![view, inputContext];
                    if !context.is_null() {
                        let _: () = msg_send![context, activate];
                        let _: () = msg_send![context, invalidateCharacterCoordinates];
                    }
                }
            }
        })
        .detach();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (window, cx);
}

/// Hides the peek window, keeping it alive for the next summon.
pub fn hide(window: &mut Window) {
    #[cfg(target_os = "macos")]
    macos_hide(window);
    #[cfg(windows)]
    windows_hide(window);
}

/// Whether the window is currently on screen, as the platform reports it.
pub fn is_visible(window: &Window) -> bool {
    window.is_visible()
}

/// Applies a panel size and toolbar offset before publishing the new layout.
/// The native operation runs outside an App borrow, so synchronous AppKit
/// resize callbacks can update GPUI's viewport and Metal drawable normally.
pub fn resize_panel(
    window: &mut Window,
    size: gpui_kit::Size<gpui_kit::Pixels>,
    upward: f32,
    cx: &mut gpui_kit::App,
    on_resized: impl FnOnce(&mut Window, &mut gpui_kit::App) + 'static,
) {
    let handle = window.window_handle();
    cx.spawn(async move |cx| {
        #[cfg(target_os = "macos")]
        let native_resized = {
            let view = cx
                .update_window(handle, |_, window, _| macos_view(window))
                .ok()
                .flatten();
            if let Some(view) = view {
                // No await between retrieving the view and changing its frame:
                // the window cannot be removed midway through this main-thread operation.
                unsafe {
                    use objc2::{msg_send, runtime::AnyObject};
                    use objc2_foundation::{NSRect, NSSize};
                    let view = view as *mut AnyObject;
                    let native: *mut AnyObject = msg_send![view, window];
                    if native.is_null() {
                        false
                    } else {
                        let _: () = msg_send![native, disableScreenUpdatesUntilFlush];
                        let mut frame: NSRect = msg_send![native, frame];
                        let content = NSRect {
                            origin: Default::default(),
                            size: NSSize::new(
                                f64::from(size.width.as_f32()),
                                f64::from(size.height.as_f32()),
                            ),
                        };
                        let new_frame: NSRect = msg_send![native, frameRectForContentRect: content];
                        // AppKit's y-axis points upward. Preserve the old top
                        // edge, then raise it by exactly the added toolbar height.
                        frame.origin.y +=
                            f64::from(upward) + frame.size.height - new_frame.size.height;
                        frame.size = new_frame.size;
                        let screen: *mut AnyObject = msg_send![native, screen];
                        if !screen.is_null() {
                            let visible: NSRect = msg_send![screen, visibleFrame];
                            frame.origin.y = frame.origin.y.clamp(
                                visible.origin.y,
                                (visible.origin.y + visible.size.height - frame.size.height)
                                    .max(visible.origin.y),
                            );
                            frame.origin.x = frame.origin.x.clamp(
                                visible.origin.x,
                                (visible.origin.x + visible.size.width - frame.size.width)
                                    .max(visible.origin.x),
                            );
                        }
                        // One frame change, rather than move now / resize on
                        // the next executor turn. GPUI's view callback updates
                        // the drawable size as part of this operation.
                        let _: () = msg_send![native, setFrame: frame, display: false];
                        true
                    }
                }
            } else {
                false
            }
        };
        #[cfg(not(target_os = "macos"))]
        let native_resized = false;
        let _ = cx.update_window(handle, |_, window, cx| {
            if !native_resized {
                move_up(window, upward);
                window.resize(size);
            }
            // Also covers the headless platform, which emits no resize callback.
            window.bounds_changed(cx);
            on_resized(window, cx);
        });
    })
    .detach();
}

/// Moves the window upward without changing the renderer's content size.
/// GPUI's resize preserves the macOS top edge; moving that edge by the toolbar
/// height keeps the input at the same screen position when the row appears.
pub fn move_up(window: &Window, distance: f32) {
    #[cfg(target_os = "macos")]
    {
        let Some(view) = macos_view(window) else {
            return;
        };
        unsafe {
            use objc2::{msg_send, runtime::AnyObject};
            use objc2_foundation::NSRect;
            let view = view as *mut AnyObject;
            let native: *mut AnyObject = msg_send![view, window];
            if native.is_null() {
                return;
            }
            // Keep AppKit from displaying the intermediate moved frame before
            // GPUI has resized and painted the newly revealed toolbar.
            let _: () = msg_send![native, disableScreenUpdatesUntilFlush];
            let frame: NSRect = msg_send![native, frame];
            let mut origin = frame.origin;
            origin.y += f64::from(distance);
            let _: () = msg_send![native, setFrameOrigin: origin];
        }
    }
    #[cfg(windows)]
    if let Some(handle) = hwnd(window) {
        use windows::Win32::{
            Foundation::{HWND, RECT},
            UI::WindowsAndMessaging::{
                GetWindowRect, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
            },
        };
        unsafe {
            let handle = HWND(handle);
            let mut rect = RECT::default();
            if GetWindowRect(handle, &mut rect).is_ok() {
                let _ = SetWindowPos(
                    handle,
                    None,
                    rect.left,
                    rect.top - (distance * window.scale_factor()).round() as i32,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = (window, distance);
}

#[cfg(target_os = "macos")]
fn macos_view(window: &Window) -> Option<*mut std::ffi::c_void> {
    use raw_window_handle::RawWindowHandle;

    // The inherent `Window::window_handle()` returns an `AnyWindowHandle` and
    // shadows the trait method, so the raw handle needs qualified syntax.
    match <Window as raw_window_handle::HasWindowHandle>::window_handle(window) {
        Ok(raw) => match raw.as_raw() {
            RawWindowHandle::AppKit(appkit) => Some(appkit.ns_view.as_ptr()),
            _ => None,
        },
        Err(_) => None,
    }
}

/// Turns a GPUI window into a full-screen capture overlay.
///
/// GPUI always gives a macOS window a titled style mask
/// (`NSTitledWindowMask | NSFullSizeContentViewWindowMask`), and macOS keeps a
/// titled window inside the visible frame at the normal window level. That is
/// why the first overlay run left the real menu bar uncovered and drew the
/// captured menu bar next to it. Borderless plus `NSScreenSaverWindowLevel`
/// covers the menu bar, which is what Snipaste and similar tools do.
pub fn make_capture_overlay(window: &mut Window) {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::NSRect;

        let Some(ns_view) = macos_view(window) else {
            return;
        };
        unsafe {
            use objc2::msg_send;
            use objc2::runtime::AnyObject;
            let view = ns_view as *mut AnyObject;
            let ns_window: *mut AnyObject = msg_send![view, window];
            if ns_window.is_null() {
                return;
            }
            // NSBorderlessWindowMask: no titlebar, so the window may extend
            // over the menu bar.
            let _: () = msg_send![ns_window, setStyleMask: 0usize];
            // NSScreenSaverWindowLevel, above the menu bar (NSMainMenuWindowLevel
            // is 24) and above other applications' full-screen windows.
            let _: () = msg_send![ns_window, setLevel: 1000isize];
            // canJoinAllSpaces | stationary | ignoresCycle | fullScreenAuxiliary
            let behavior: usize = (1 << 0) | (1 << 4) | (1 << 6) | (1 << 8);
            let _: () = msg_send![ns_window, setCollectionBehavior: behavior];
            let _: () = msg_send![ns_window, setHasShadow: false];
            // The frame GPUI asked for was clamped to the visible frame while
            // the window still had a titlebar, so take the screen's full frame.
            let screen: *mut AnyObject = msg_send![ns_window, screen];
            if !screen.is_null() {
                let frame: NSRect = msg_send![screen, frame];
                let _: () = msg_send![ns_window, setFrame: frame, display: true];
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}

/// Hides the close/minimise/zoom buttons.
///
/// The panel is a titled window, so macOS draws traffic lights in its corner.
/// The panel has its own close button and is summoned by hotkey, so the system
/// ones are noise.
pub fn hide_window_buttons(window: &mut Window) {
    #[cfg(target_os = "macos")]
    {
        let Some(ns_view) = macos_view(window) else {
            return;
        };
        unsafe {
            use objc2::msg_send;
            use objc2::runtime::AnyObject;
            let view = ns_view as *mut AnyObject;
            let ns_window: *mut AnyObject = msg_send![view, window];
            if ns_window.is_null() {
                return;
            }
            // NSWindowCloseButton, NSWindowMiniaturizeButton, NSWindowZoomButton
            for kind in [0usize, 1, 2] {
                let button: *mut AnyObject = msg_send![ns_window, standardWindowButton: kind];
                if !button.is_null() {
                    let _: () = msg_send![button, setHidden: true];
                }
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}

#[cfg(target_os = "macos")]
fn macos_hide(window: &mut Window) {
    let Some(ns_view) = macos_view(window) else {
        return;
    };
    unsafe {
        use objc2::msg_send;
        use objc2::runtime::AnyObject;
        let view = ns_view as *mut AnyObject;
        let ns_window: *mut AnyObject = msg_send![view, window];
        let _: () = msg_send![ns_window, orderOut: std::ptr::null_mut::<AnyObject>()];
    }
}

#[cfg(windows)]
fn hwnd(window: &Window) -> Option<*mut std::ffi::c_void> {
    use raw_window_handle::RawWindowHandle;

    match <Window as raw_window_handle::HasWindowHandle>::window_handle(window) {
        Ok(raw) => match raw.as_raw() {
            // raw-window-handle 0.6 stores the Win32 handle as a `NonZeroIsize`,
            // not a pointer, so it is converted rather than dereferenced.
            RawWindowHandle::Win32(win32) => Some(win32.hwnd.get() as *mut std::ffi::c_void),
            _ => None,
        },
        Err(_) => None,
    }
}

#[cfg(windows)]
fn windows_show(window: &mut Window) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SW_SHOW, ShowWindow};
    if let Some(hwnd) = hwnd(window) {
        unsafe {
            let _ = ShowWindow(HWND(hwnd), SW_SHOW);
        }
    }
}

#[cfg(windows)]
fn windows_hide(window: &mut Window) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindow};
    if let Some(hwnd) = hwnd(window) {
        unsafe {
            let _ = ShowWindow(HWND(hwnd), SW_HIDE);
        }
    }
}
