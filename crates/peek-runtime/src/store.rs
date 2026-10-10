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
pub struct LoadedConfig {
    pub config: Config,
    pub warning: Option<String>,
}
trait Credentials {
    fn get(&self, id: &str) -> Result<String, String>;
    fn set(&self, id: &str, value: &str) -> Result<(), String>;
}
struct SystemCredentials;
impl Credentials for SystemCredentials {
    fn get(&self, id: &str) -> Result<String, String> {
        secret(id)
    }
    fn set(&self, id: &str, value: &str) -> Result<(), String> {
        save_secret(id, value)
    }
}
fn decode(path: &std::path::Path) -> Result<Config, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut config: Config = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    config.migrate();
    config.validate().map_err(crate::i18n::tr)?;
    Ok(config)
}
/// Preview rendering never writes config or opens the system credential store.
pub fn load_preview() -> LoadedConfig {
    let mut config = config_path()
        .and_then(|path| decode(&path))
        .unwrap_or_default();
    for channel in &mut config.channels {
        channel.api_key.clear();
    }
    LoadedConfig {
        config,
        warning: None,
    }
}
pub fn load_report() -> LoadedConfig {
    match config_path() {
        Ok(path) => load_from(&path, &SystemCredentials),
        Err(error) => LoadedConfig {
            config: Config::default(),
            warning: Some(error),
        },
    }
}
pub fn load() -> Result<Config, String> {
    Ok(load_report().config)
}
fn load_from(path: &std::path::Path, credentials: &impl Credentials) -> LoadedConfig {
    if !path.exists() {
        return LoadedConfig {
            config: Config::default(),
            warning: None,
        };
    }
    let mut warning = None;
    let mut config = match decode(path) {
        Ok(config) => config,
        Err(_) => {
            // Preserve the original even if no valid backup exists. Never rename
            // over an earlier corrupt copy or print its potentially secret bytes.
            if let Some(parent) = path.parent()
                && let Ok(copy) = tempfile::Builder::new()
                    .prefix("config-damaged-")
                    .suffix(".json")
                    .tempfile_in(parent)
                && std::fs::copy(path, copy.path()).is_ok()
            {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(
                        copy.path(),
                        std::fs::Permissions::from_mode(0o600),
                    );
                }
                let _ = copy.keep();
            }
            warning = Some(crate::i18n::tr("status-config-recovered"));
            decode(&path.with_extension("json.bak")).unwrap_or_default()
        }
    };
    let needs_migration = config
        .channels
        .iter()
        .any(|channel| !channel.api_key.is_empty());
    if needs_migration && save_with_credentials(&config, path, credentials).is_err() {
        warning = Some(crate::i18n::tr("status-key-migration-failed"));
    }
    for channel in &mut config.channels {
        if channel.api_key.is_empty() && !channel.credential_id.is_empty() {
            match credentials.get(&channel.credential_id) {
                Ok(secret) => channel.api_key = secret,
                Err(_) => warning = Some(crate::i18n::tr("status-key-read-failed")),
            }
        }
    }
    LoadedConfig { config, warning }
}
pub fn save(config: &Config) -> Result<(), String> {
    save_with_credentials(config, &config_path()?, &SystemCredentials)
}
fn save_with_credentials(
    config: &Config,
    path: &std::path::Path,
    credentials: &impl Credentials,
) -> Result<(), String> {
    config.validate().map_err(crate::i18n::tr)?;
    let mut public = config.clone();
    for channel in &mut public.channels {
        if !channel.api_key.is_empty() {
            if channel.credential_id.is_empty() {
                channel.credential_id = format!("channel-{}", channel.id);
            }
            if credentials.get(&channel.credential_id).ok().as_deref() != Some(&channel.api_key) {
                credentials.set(&channel.credential_id, &channel.api_key)?;
            }
            channel.api_key.clear();
        }
    }
    // A failed credential write must leave the original file untouched.
    // Both copies are sanitised; write the primary last.
    save_to(&public, &path.with_extension("json.bak"))?;
    save_to(&public, path)
}
fn save_to(config: &Config, path: &std::path::Path) -> Result<(), String> {
    config.validate().map_err(crate::i18n::tr)?;
    let parent = path
        .parent()
        .ok_or_else(|| crate::i18n::tr("error-config-directory"))?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
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
    #[derive(Default)]
    struct FakeCredentials {
        values: std::cell::RefCell<std::collections::HashMap<String, String>>,
        fail: bool,
    }
    impl Credentials for FakeCredentials {
        fn get(&self, id: &str) -> Result<String, String> {
            self.values
                .borrow()
                .get(id)
                .cloned()
                .ok_or("missing".into())
        }
        fn set(&self, id: &str, value: &str) -> Result<(), String> {
            if self.fail {
                return Err("locked".into());
            }
            self.values.borrow_mut().insert(id.into(), value.into());
            Ok(())
        }
    }
    fn keyed_config() -> Config {
        Config {
            channels: vec![peek_core::Channel {
                id: "sample".into(),
                model: "sample-model".into(),
                api_key: "sample-test-secret".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }
    #[test]
    fn legacy_credentials_migrate_without_plaintext_in_either_copy() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        save_to(&keyed_config(), &path).unwrap();
        let credentials = FakeCredentials::default();
        let loaded = load_from(&path, &credentials);
        assert!(loaded.warning.is_none());
        assert_eq!(loaded.config.channels[0].api_key, "sample-test-secret");
        for file in [&path, &path.with_extension("json.bak")] {
            assert!(
                !std::fs::read_to_string(file)
                    .unwrap()
                    .contains("sample-test-secret")
            );
        }
        assert_eq!(
            load_from(&path, &credentials).config.channels[0].api_key,
            "sample-test-secret"
        );
    }
    #[test]
    fn locked_credential_store_preserves_legacy_file_and_saved_settings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        save_to(&keyed_config(), &path).unwrap();
        let before = std::fs::read(&path).unwrap();
        let loaded = load_from(
            &path,
            &FakeCredentials {
                fail: true,
                ..Default::default()
            },
        );
        assert!(loaded.warning.is_some());
        assert_eq!(loaded.config.channels[0].api_key, "sample-test-secret");
        assert_eq!(std::fs::read(path).unwrap(), before);
    }
    #[test]
    fn damaged_config_recovers_channels_and_credentials_from_sanitised_backup() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        let credentials = FakeCredentials::default();
        let mut config = keyed_config();
        config.target_language = "Japanese".into();
        save_with_credentials(&config, &path, &credentials).unwrap();
        std::fs::write(&path, b"{broken configuration").unwrap();
        let loaded = load_from(&path, &credentials);
        assert!(loaded.warning.is_some());
        assert_eq!(loaded.config.target_language, "Japanese");
        assert_eq!(loaded.config.channels[0].api_key, "sample-test-secret");
        assert_eq!(std::fs::read(&path).unwrap(), b"{broken configuration");
        assert!(
            std::fs::read_dir(directory.path())
                .unwrap()
                .filter_map(Result::ok)
                .any(|item| item
                    .file_name()
                    .to_string_lossy()
                    .starts_with("config-damaged-"))
        );
    }
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
