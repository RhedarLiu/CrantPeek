//! Short, one-shot transitions. GPUI adopts the end state when reduced motion
//! is enabled, and stops requesting animation frames once the transition ends.
use gpui_kit::Animation;
use std::time::Duration;

pub const MENU_EXIT: Duration = Duration::from_millis(85);

/// Read the OS preference rather than assuming GPUI's default is appropriate.
pub fn sync_system_preference(cx: &mut gpui_kit::App) {
    #[cfg(target_os = "macos")]
    let reduced = unsafe {
        use objc2::{
            class, msg_send,
            runtime::{AnyObject, Bool},
        };
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let reduced: Bool = msg_send![workspace, accessibilityDisplayShouldReduceMotion];
        reduced.as_bool()
    };
    #[cfg(windows)]
    let reduced = unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
        };
        let mut enabled = 1_i32; // Win32 BOOL occupies four bytes.
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&mut enabled as *mut i32).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
            && enabled == 0
    };
    #[cfg(not(any(target_os = "macos", windows)))]
    let reduced = false;
    cx.set_reduce_motion(reduced);
}

pub fn menu_enter() -> Animation {
    Animation::new(Duration::from_millis(145)).with_easing(ease_out)
}

pub fn menu_exit() -> Animation {
    Animation::new(MENU_EXIT).with_easing(|t| t * t)
}

pub fn content_enter() -> Animation {
    Animation::new(Duration::from_millis(130)).with_easing(ease_out)
}

fn ease_out(t: f32) -> f32 {
    1. - (1. - t).powi(3)
}
