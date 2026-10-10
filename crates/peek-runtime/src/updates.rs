//! User-initiated update checks; installation stays under the user's control.
pub const RELEASES_URL: &str = "https://github.com/RhedarLiu/CrantPeek/releases/latest";
pub fn open_releases() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open")
        .arg(RELEASES_URL)
        .status();
    #[cfg(windows)]
    let result = std::process::Command::new("explorer.exe")
        .arg(RELEASES_URL)
        .status();
    #[cfg(not(any(target_os = "macos", windows)))]
    let result = std::process::Command::new("xdg-open")
        .arg(RELEASES_URL)
        .status();
    result
        .ok()
        .filter(|status| status.success())
        .map(|_| ())
        .ok_or_else(|| crate::i18n::tr("status-update-open-failed"))
}
