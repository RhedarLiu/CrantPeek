//! Short, one-shot transitions. GPUI adopts the end state when reduced motion
//! is enabled, and stops requesting animation frames once the transition ends.
use gpui_kit::Animation;
use std::time::{Duration, Instant};

pub const MENU_EXIT: Duration = Duration::from_millis(110);

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
    Animation::new(Duration::from_millis(210)).with_easing(ease_out)
}

pub fn menu_exit() -> Animation {
    Animation::new(MENU_EXIT).with_easing(|t| t * t)
}

pub fn content_enter() -> Animation {
    Animation::new(Duration::from_millis(180)).with_easing(ease_out)
}

fn ease_out(t: f32) -> f32 {
    1. - (1. - t).powi(3)
}

/// A reversible reveal for native-window chrome. Sampling the previous curve
/// before redirecting it keeps hover/typing reversals continuous.
#[derive(Default)]
pub struct Reveal {
    from: f32,
    target: f32,
    started: Option<Instant>,
}

impl Reveal {
    fn value(&self, now: Instant) -> f32 {
        let Some(started) = self.started else {
            return self.target;
        };
        let duration = if self.target > self.from { 0.18 } else { 0.14 };
        let delta = (now.duration_since(started).as_secs_f32() / duration).min(1.);
        self.from + (self.target - self.from) * ease_out(delta)
    }

    /// Returns the revealed fraction and whether another frame is needed.
    pub fn update(&mut self, visible: bool, reduced: bool) -> (f32, bool) {
        let now = Instant::now();
        let target = if visible { 1. } else { 0. };
        if reduced {
            self.from = target;
            self.target = target;
            self.started = None;
            return (target, false);
        }
        if self.target != target {
            self.from = self.value(now);
            self.target = target;
            self.started = Some(now);
        }
        let value = self.value(now);
        let moving = (value - target).abs() > 0.0001;
        if !moving {
            self.started = None;
        }
        (if moving { value } else { target }, moving)
    }
}
