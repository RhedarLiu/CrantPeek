//! Read-only diagnostics; requesting authorization is always an explicit user action.
#[cfg(target_os = "macos")]
pub fn status() -> Vec<(&'static str, bool)> {
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn CGPreflightListenEventAccess() -> bool;
        fn CGPreflightScreenCaptureAccess() -> bool;
    }
    unsafe {
        vec![
            ("settings-permission-accessibility", AXIsProcessTrusted()),
            ("settings-permission-input", CGPreflightListenEventAccess()),
            (
                "settings-permission-screen",
                CGPreflightScreenCaptureAccess(),
            ),
        ]
    }
}
/// Permission-specific System Settings links. Unknown keys do not launch a URL.
#[cfg(any(target_os = "macos", test))]
fn settings_url(permission: &str) -> Option<&'static str> {
    match permission {
        "settings-permission-accessibility" => {
            Some("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        }
        "settings-permission-input" => {
            Some("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent")
        }
        "settings-permission-screen" => {
            Some("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
        }
        _ => None,
    }
}

#[cfg(target_os = "macos")]
pub fn open_permission_settings(permission: &str) -> Result<(), String> {
    let url =
        settings_url(permission).ok_or_else(|| crate::i18n::tr("settings-open-privacy-failed"))?;
    std::process::Command::new("/usr/bin/open")
        .arg(url)
        .status()
        .map_err(|e| e.to_string())?
        .success()
        .then_some(())
        .ok_or_else(|| crate::i18n::tr("settings-open-privacy-failed"))
}

#[cfg(windows)]
pub fn open_permission_settings(_permission: &str) -> Result<(), String> {
    open_settings()
}

#[cfg(target_os = "macos")]
pub fn open_settings() -> Result<(), String> {
    std::process::Command::new("/usr/bin/open")
        .arg("x-apple.systempreferences:com.apple.preference.security")
        .status()
        .map_err(|e| e.to_string())?
        .success()
        .then_some(())
        .ok_or_else(|| crate::i18n::tr("settings-open-privacy-failed"))
}
#[cfg(windows)]
pub fn status() -> Vec<(&'static str, bool)> {
    Vec::new()
}
#[cfg(windows)]
pub fn open_settings() -> Result<(), String> {
    std::process::Command::new("explorer.exe")
        .arg("ms-settings:privacy")
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn permission_links_target_the_matching_system_page() {
        for (key, anchor) in [
            ("settings-permission-accessibility", "Privacy_Accessibility"),
            ("settings-permission-input", "Privacy_ListenEvent"),
            ("settings-permission-screen", "Privacy_ScreenCapture"),
        ] {
            assert_eq!(
                super::settings_url(key),
                Some(
                    format!("x-apple.systempreferences:com.apple.preference.security?{anchor}")
                        .as_str()
                )
            );
        }
        assert_eq!(super::settings_url("unknown"), None);
    }
}
