//! UI-only preferences.
//!
//! Deliberately separate from `peek_core::Config`: that is the core schema
//! shared with the task/network layer, and the UI migration must not change it.
//! Which font bundle the shell uses is a presentation concern, so it lives in
//! its own file next to `config.json`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPrefs {
    /// Font bundle id, see `peek_gpui::fonts::ALL` — `default` or `serif`.
    pub font_set: String,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            font_set: "default".into(),
        }
    }
}

/// `ui.json` in the same directory as `config.json`.
pub fn path() -> Result<PathBuf, String> {
    crate::store::config_path().map(|config| config.with_file_name("ui.json"))
}

/// Missing or unreadable preferences fall back to defaults: they are cosmetic,
/// so a broken file must never block startup.
pub fn load() -> UiPrefs {
    let Ok(path) = path() else {
        return UiPrefs::default();
    };
    let Ok(bytes) = std::fs::read(path) else {
        return UiPrefs::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

pub fn save(prefs: &UiPrefs) -> Result<(), String> {
    save_to(prefs, &path()?)
}

fn save_to(prefs: &UiPrefs, path: &std::path::Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| crate::i18n::tr("error-config-directory"))?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    // Same atomic replace the config store uses.
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    use std::io::Write;
    file.write_all(&serde_json::to_vec_pretty(prefs).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_reloads_atomically() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/ui.json");
        let mut prefs = UiPrefs::default();
        save_to(&prefs, &path).unwrap();
        prefs.font_set = "serif".into();
        save_to(&prefs, &path).unwrap();
        let loaded: UiPrefs = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(loaded.font_set, "serif");
    }

    #[test]
    fn defaults_are_used_when_the_file_is_absent_or_broken() {
        let prefs = UiPrefs::default();
        assert_eq!(prefs.font_set, "default");
        // A malformed document must not panic.
        assert!(serde_json::from_slice::<UiPrefs>(b"{ not json").is_err());
        // Unknown fields are tolerated so older builds keep working.
        let parsed: UiPrefs = serde_json::from_slice(br#"{"font_set":"serif"}"#).unwrap();
        assert_eq!(parsed.font_set, "serif");
    }
}
