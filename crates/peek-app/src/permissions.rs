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
            ("辅助功能（读取选区）", AXIsProcessTrusted()),
            ("输入监控（双击 Ctrl）", CGPreflightListenEventAccess()),
            ("屏幕录制（截图）", CGPreflightScreenCaptureAccess()),
        ]
    }
}
#[cfg(target_os = "macos")]
pub fn open_settings() -> Result<(), String> {
    std::process::Command::new("/usr/bin/open")
        .arg("x-apple.systempreferences:com.apple.preference.security")
        .status()
        .map_err(|e| e.to_string())?
        .success()
        .then_some(())
        .ok_or_else(|| "无法打开系统设置".into())
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
