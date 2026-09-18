//! Storage for explicit mock-development roots. Never discovers old game data.
pub mod local;
pub mod registration;
pub mod steamcmd;
use fs2::FileExt;
use gsm_domain::{AppConfig, CONFIG_VERSION, SettingsStore};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub struct FileSettingsStore {
    root: PathBuf,
    filename: &'static str,
    _lock: File,
}
impl FileSettingsStore {
    pub fn open(root: &Path) -> Result<Self, String> {
        Self::open_mode(root, false)
    }
    pub fn open_local(root: &Path) -> Result<Self, String> {
        Self::open_mode(root, true)
    }
    fn open_mode(root: &Path, local: bool) -> Result<Self, String> {
        if !root.is_absolute() {
            return Err("--data-dir には絶対パスを指定してください".into());
        }
        fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join(if local {
                "local-manager.lock"
            } else {
                "mock-manager.lock"
            }))
            .map_err(|e| e.to_string())?;
        lock.try_lock_exclusive()
            .map_err(|e| format!("同じ開発データを使用中か、ロックを取得できません: {e}"))?;
        Ok(Self {
            root,
            filename: if local {
                "local-app.json"
            } else {
                "mock-app.json"
            },
            _lock: lock,
        })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    fn config_path(&self) -> PathBuf {
        self.root.join(self.filename)
    }
}
impl SettingsStore for FileSettingsStore {
    fn load(&self) -> Result<AppConfig, String> {
        let bytes = match fs::read(self.config_path()) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(AppConfig::default()),
            Err(e) => return Err(e.to_string()),
        };
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|e| format!("管理設定が不正です（上書きしません）: {e}"))?;
        if value.get("schema_version").and_then(|v| v.as_u64()) != Some(u64::from(CONFIG_VERSION)) {
            return Err("管理設定の schema_version に対応していません（上書きしません）".into());
        }
        serde_json::from_value(value).map_err(|e| e.to_string())
    }
    fn save(&self, config: &AppConfig) -> Result<(), String> {
        if config.schema_version != CONFIG_VERSION {
            return Err("unsupported schema version".into());
        }
        let mut file = tempfile::NamedTempFile::new_in(&self.root).map_err(|e| e.to_string())?;
        serde_json::to_writer_pretty(file.as_file_mut(), config).map_err(|e| e.to_string())?;
        file.write_all(b"\n").map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.persist(self.config_path())
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn second_writer_is_refused_until_first_store_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let first = FileSettingsStore::open(dir.path()).unwrap();
        assert!(FileSettingsStore::open(dir.path()).is_err());
        drop(first);
        assert!(FileSettingsStore::open(dir.path()).is_ok());
    }
    #[test]
    fn future_and_corrupt_configs_remain_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSettingsStore::open(dir.path()).unwrap();
        for data in ["{\"schema_version\":99}", "not json"] {
            fs::write(store.config_path(), data).unwrap();
            assert!(store.load().is_err());
            assert_eq!(fs::read_to_string(store.config_path()).unwrap(), data);
        }
    }
    #[test]
    fn settings_replace_and_reload_without_leaving_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSettingsStore::open(dir.path()).unwrap();
        let mut config = AppConfig::default();
        store.save(&config).unwrap();
        config
            .enabled_games
            .push("example".to_owned().try_into().unwrap());
        store.save(&config).unwrap();
        assert_eq!(store.load().unwrap(), config);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }
    #[test]
    fn relative_roots_are_never_resolved_against_cwd() {
        assert!(FileSettingsStore::open(Path::new("relative-root")).is_err());
    }
}
