use peek_core::Config;
use std::path::PathBuf;

pub fn config_path() -> Result<PathBuf, String> {
    directories::ProjectDirs::from("dev", "Crant", "CrantPeek")
        .map(|d| d.config_dir().join("config.json"))
        .ok_or_else(|| crate::i18n::tr("error-config-directory"))
}
/// Where the offline dictionary may live: next to the config, then the dev build output.
pub fn dictionary_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(config) = config_path()
        && let Some(dir) = config.parent()
    {
        paths.push(dir.join("ecdict.pkd"));
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        paths.push(directory.join("ecdict.pkd"));
        paths.push(directory.join("../Resources/ecdict.pkd"));
    }
    paths.push(PathBuf::from("local-assets/ecdict.pkd"));
    paths
}
pub fn load() -> Result<Config, String> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(Config::default());
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let mut config: Config = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let changed = config.migrate();
    config.validate().map_err(crate::i18n::tr)?;
    if changed {
        save_to(&config, &path)?;
    }
    Ok(config)
}
pub fn save(config: &Config) -> Result<(), String> {
    save_to(config, &config_path()?)
}
fn save_to(config: &Config, path: &std::path::Path) -> Result<(), String> {
    config.validate().map_err(crate::i18n::tr)?;
    let parent = path
        .parent()
        .ok_or_else(|| crate::i18n::tr("error-config-directory"))?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    // Unique file in the same directory: atomic cross-platform replace, auto-cleaned on failure.
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    use std::io::Write;
    file.write_all(&serde_json::to_vec_pretty(config).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
pub fn secret(id: &str) -> Result<String, String> {
    keyring::Entry::new("CrantPeek", id)
        .map_err(|_| crate::i18n::tr("error-keychain-open"))?
        .get_password()
        .map_err(|_| crate::i18n::tr("error-key-missing"))
}
pub fn save_secret(id: &str, value: &str) -> Result<(), String> {
    keyring::Entry::new("CrantPeek", id)
        .map_err(|_| crate::i18n::tr("error-keychain-open"))?
        .set_password(value)
        .map_err(|_| crate::i18n::tr("error-keychain-save"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saves_and_replaces_config_without_secrets() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/config.json");
        let mut config = Config::default();
        save_to(&config, &path).unwrap();
        config.target_language = "Japanese".into();
        save_to(&config, &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let loaded: Config = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(loaded.target_language, "Japanese");
        assert!(
            !String::from_utf8(bytes.clone())
                .unwrap()
                .contains("api_key")
        );
        config.double_ctrl_ms = 0;
        assert!(save_to(&config, &path).is_err());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}
