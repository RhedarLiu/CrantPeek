//! User-initiated update checks; installation stays under the user's control.
pub const RELEASES_URL: &str = "https://crantpeek.eno.ink/#downloads";
#[derive(Clone, Copy)]
pub enum ProjectLink {
    Website,
    GitHub,
}
impl ProjectLink {
    pub fn url(self) -> &'static str {
        match self {
            Self::Website => "https://crantpeek.eno.ink",
            Self::GitHub => "https://github.com/RhedarLiu/CrantPeek",
        }
    }
}
pub fn open_project_link(target: ProjectLink) -> Result<(), String> {
    open_url(target.url())
}

pub fn open_releases() -> Result<(), String> {
    open_url(RELEASES_URL)
}
fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).status();
    #[cfg(windows)]
    let result = std::process::Command::new("explorer.exe").arg(url).status();
    #[cfg(not(any(target_os = "macos", windows)))]
    let result = std::process::Command::new("xdg-open").arg(url).status();
    result
        .ok()
        .filter(|status| status.success())
        .map(|_| ())
        .ok_or_else(|| crate::i18n::tr("status-update-open-failed"))
}
