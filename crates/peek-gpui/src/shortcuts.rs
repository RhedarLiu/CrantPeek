//! Global shortcuts. Each one is registered on its own, so a combination that
//! another application holds disables only that entry, not all three.
use global_hotkey::{GlobalHotKeyManager, hotkey::HotKey};
use std::cell::RefCell;
thread_local! {
    /// The manager and the shortcut registered in each slot, if any.
    static ACTIVE: RefCell<Option<(GlobalHotKeyManager, Vec<Option<HotKey>>)>> = const { RefCell::new(None) };
}

/// Registers one shortcut per slot. Returns the slots that could not be
/// registered; every other slot is active afterwards.
pub fn apply(keys: Vec<HotKey>) -> Result<Vec<usize>, String> {
    ACTIVE.with(|active| {
        let mut active = active.borrow_mut();
        if active.is_none() {
            *active = Some((
                GlobalHotKeyManager::new()
                    .map_err(|_| peek_runtime::i18n::tr("status-hotkey-manager"))?,
                Vec::new(),
            ));
        }
        let (manager, slots) = active.as_mut().unwrap();
        for key in slots.drain(..).flatten() {
            let _ = manager.unregister(key);
        }
        let mut failed = Vec::new();
        for (index, key) in keys.into_iter().enumerate() {
            if manager.register(key).is_ok() {
                slots.push(Some(key));
            } else {
                failed.push(index);
                slots.push(None);
            }
        }
        Ok(failed)
    })
}

/// Releases every global shortcut while one is being recorded.
///
/// A registered combination is consumed by the system before the window sees
/// it, so recording would otherwise fire the old action instead of capturing
/// the keys. `apply` registers them again afterwards.
pub fn suspend() {
    ACTIVE.with(|active| {
        if let Some((manager, slots)) = active.borrow_mut().as_mut() {
            for key in slots.drain(..).flatten() {
                let _ = manager.unregister(key);
            }
        }
    });
}

pub fn role(id: u32) -> Option<usize> {
    ACTIVE.with(|active| {
        active.borrow().as_ref().and_then(|(_, slots)| {
            slots
                .iter()
                .position(|key| key.is_some_and(|key| key.id() == id))
        })
    })
}

/// Why a recorded keystroke cannot become a global shortcut.
#[derive(Debug, PartialEq, Eq)]
pub enum RecordError {
    /// A bare key would fire on every press of it in every application.
    NeedsModifier,
    Unsupported,
}

/// Converts a keystroke from the window into the stored shortcut format,
/// for example `Super+Shift+A`, which `global-hotkey` parses.
pub fn from_keystroke(
    control: bool,
    alt: bool,
    shift: bool,
    platform: bool,
    key: &str,
) -> Result<String, RecordError> {
    let key = key_token(key).ok_or(RecordError::Unsupported)?;
    // Without Ctrl, Alt or Super the key would type text everywhere; Shift
    // alone only changes the character. Function keys type nothing.
    let function_key = key.len() > 1 && key.starts_with('F');
    if !(control || alt || platform || function_key) {
        return Err(RecordError::NeedsModifier);
    }
    let mut parts = Vec::new();
    if control {
        parts.push("Control");
    }
    if alt {
        parts.push("Alt");
    }
    if shift {
        parts.push("Shift");
    }
    if platform {
        parts.push("Super");
    }
    parts.push(&key);
    let value = parts.join("+");
    value
        .parse::<HotKey>()
        .map(|_| value.clone())
        .map_err(|_| RecordError::Unsupported)
}

/// GPUI key names mapped to the names `global-hotkey` accepts.
fn key_token(key: &str) -> Option<String> {
    let key = key.to_ascii_lowercase();
    if key.len() == 1 {
        let c = key.chars().next()?;
        if c.is_ascii_alphanumeric() {
            return Some(c.to_ascii_uppercase().to_string());
        }
        return "`-=[]\\;',./".contains(c).then(|| c.to_string());
    }
    if let Some(number) = key.strip_prefix('f')
        && let Ok(n) = number.parse::<u8>()
        && (1..=24).contains(&n)
    {
        return Some(format!("F{n}"));
    }
    let named = match key.as_str() {
        "space" => "Space",
        "enter" => "Enter",
        "tab" => "Tab",
        "backspace" => "Backspace",
        "delete" => "Delete",
        "insert" => "Insert",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "up" => "Up",
        "down" => "Down",
        "left" => "Left",
        "right" => "Right",
        _ => return None,
    };
    Some(named.to_owned())
}

/// How a stored shortcut is shown: the platform's own modifier names.
pub fn display(value: &str) -> String {
    let parts: Vec<&str> = value.split('+').map(str::trim).collect();
    if cfg!(target_os = "macos") {
        let mut text = String::new();
        let has = |names: &[&str]| {
            parts
                .iter()
                .any(|part| names.iter().any(|name| part.eq_ignore_ascii_case(name)))
        };
        for (names, glyph) in [
            (&["control", "ctrl"][..], "⌃"),
            (&["alt", "option"][..], "⌥"),
            (&["shift"][..], "⇧"),
            (&["super", "command", "cmd"][..], "⌘"),
        ] {
            if has(names) {
                text.push_str(glyph);
            }
        }
        text.push_str(parts.last().copied().unwrap_or_default());
        text
    } else {
        parts
            .iter()
            .map(|part| match part.to_ascii_lowercase().as_str() {
                "control" | "ctrl" => "Ctrl",
                "super" | "command" | "cmd" => "Win",
                "alt" | "option" => "Alt",
                "shift" => "Shift",
                _ => part,
            })
            .collect::<Vec<_>>()
            .join(" + ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_keys_use_the_stored_format() {
        assert_eq!(
            from_keystroke(false, false, true, true, "a").unwrap(),
            "Shift+Super+A"
        );
        assert_eq!(
            from_keystroke(true, true, false, false, "f5").unwrap(),
            "Control+Alt+F5"
        );
        assert_eq!(
            from_keystroke(false, true, false, false, "e").unwrap(),
            "Alt+E"
        );
        assert_eq!(
            from_keystroke(true, false, false, false, "space").unwrap(),
            "Control+Space"
        );
        assert_eq!(
            from_keystroke(false, false, false, true, "/").unwrap(),
            "Super+/"
        );
    }

    #[test]
    fn plain_typing_is_rejected() {
        assert_eq!(
            from_keystroke(false, false, false, false, "a"),
            Err(RecordError::NeedsModifier)
        );
        assert_eq!(
            from_keystroke(false, false, true, false, "a"),
            Err(RecordError::NeedsModifier)
        );
        assert_eq!(
            from_keystroke(true, false, false, false, "escape"),
            Err(RecordError::Unsupported)
        );
    }

    #[test]
    fn function_keys_may_stand_alone() {
        assert_eq!(
            from_keystroke(false, false, false, false, "f8").unwrap(),
            "F8"
        );
    }

    #[test]
    fn every_recorded_value_registers() {
        for value in ["Super+Shift+A", "Alt+E", "Control+Alt+F5"] {
            assert!(value.parse::<HotKey>().is_ok(), "{value}");
        }
    }
}
