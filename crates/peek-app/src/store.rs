use peek_core::Config;
use std::path::PathBuf;

pub fn config_path() -> Result<PathBuf, String> {
    directories::ProjectDirs::from("dev", "Crant", "CrantPeek")
        .map(|d| d.config_dir().join("config.json"))
        .ok_or_else(|| "Cannot locate configuration directory".into())
}
pub fn load() -> Result<Config, String> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(Config::default());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let config: Config = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    config.validate().map_err(str::to_owned)?;
    Ok(config)
}
pub fn save(config: &Config) -> Result<(), String> {
    config.validate().map_err(str::to_owned)?;
    let path = config_path()?;
    let parent = path.parent().ok_or("Invalid config path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    // Config contains no secrets. Sync content before publishing the new file.
    let temp = path.with_extension("json.tmp");
    let mut file = std::fs::File::create(&temp).map_err(|e| e.to_string())?;
    use std::io::Write;
    file.write_all(&serde_json::to_vec_pretty(config).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    std::fs::rename(temp, path).map_err(|e| e.to_string())
}
pub fn secret(id: &str) -> Result<String, String> {
    keyring::Entry::new("CrantPeek", id)
        .map_err(|_| "Cannot open system credential store")?
        .get_password()
        .map_err(|_| "API key not configured; open settings".into())
}
pub fn save_secret(id: &str, value: &str) -> Result<(), String> {
    keyring::Entry::new("CrantPeek", id)
        .map_err(|_| "Cannot open system credential store")?
        .set_password(value)
        .map_err(|_| "Cannot save credential in system store".into())
}
