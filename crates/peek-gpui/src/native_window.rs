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

use gpui_kit::Window;

/// Shows and focuses the peek window.
pub fn show(window: &mut Window) {
    #[cfg(windows)]
    windows_show(window);
    window.activate_window();
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
            RawWindowHandle::Win32(win32) => Some(win32.hwnd.as_ptr()),
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
