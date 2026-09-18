//! Reference-only registration storage, separate from synthetic runtime state.
use fs2::FileExt;
use gsm_domain::{GameId, Registration};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

/// Contains credentials; deliberately has no Debug implementation.
pub struct ConfigSource {
    path: PathBuf,
    text: String,
    digest: String,
}
impl ConfigSource {
    pub fn read(path: &Path) -> Result<Self, String> {
        if !path.is_absolute() {
            return Err("設定ファイルには絶対パスを指定してください".into());
        }
        let path = fs::canonicalize(path).map_err(|_| "設定ファイルを開けません")?;
        // Reject a static FIFO/device before open(), which could otherwise block.
        if !fs::metadata(&path)
            .map_err(|_| "設定ファイルを確認できません")?
            .is_file()
        {
            return Err("設定の参照先は通常ファイルにしてください".into());
        }
        let file = File::open(&path).map_err(|_| "設定ファイルを開けません")?;
        if !file
            .metadata()
            .map_err(|_| "設定ファイルを確認できません")?
            .is_file()
        {
            return Err("設定の参照先は通常ファイルにしてください".into());
        }
        let mut bytes = Vec::new();
        file.take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "設定ファイルを読み取れません")?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err("設定ファイルが 2 MiB を超えています".into());
        }
        let digest = format!("{:x}", Sha256::digest(&bytes));
        let text = String::from_utf8(bytes).map_err(|_| "設定ファイルは UTF-8 にしてください")?;
        Ok(Self { path, text, digest })
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn verify_unchanged(&self) -> Result<(), String> {
        let latest = Self::read(&self.path)?;
        if latest.digest != self.digest {
            return Err("検証後に設定が変わりました。再検証してください".into());
        }
        Ok(())
    }
}

/// Called only after the operation gate, stopped check and full backup succeed.
/// Keep the original intact on conflicts or temporary-file failures.
pub fn replace_reviewed(source: &ConfigSource, contents: &str) -> Result<(), String> {
    if contents.len() > 2 * 1024 * 1024 {
        return Err("設定が大きすぎます".into());
    }
    let parent = source.path().parent().ok_or("設定フォルダーが不明です")?;
    let mut file = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| "設定の一時ファイルを作成できません")?;
    file.as_file()
        .set_permissions(
            fs::metadata(source.path())
                .map_err(|_| "設定の権限を確認できません")?
                .permissions(),
        )
        .map_err(|_| "設定の権限を保持できません")?;
    file.write_all(contents.as_bytes())
        .map_err(|_| "設定を書き込めません")?;
    file.as_file()
        .sync_all()
        .map_err(|_| "設定を同期できません")?;
    source.verify_unchanged()?;
    file.persist(source.path())
        .map_err(|_| "設定を保存できません。元ファイルを維持します")?;
    Ok(())
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Records {
    schema_version: u32,
    registrations: Vec<Registration>,
}
pub struct RegistrationStore {
    root: PathBuf,
    state: Mutex<Records>,
    _lock: File,
}
impl RegistrationStore {
    pub fn open(root: &Path) -> Result<Self, String> {
        if !root.is_absolute() {
            return Err("登録データには絶対パスを指定してください".into());
        }
        fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("local-registrations.lock"))
            .map_err(|e| e.to_string())?;
        lock.try_lock_exclusive()
            .map_err(|_| "登録データは別のアプリで使用中です")?;
        let state = match fs::read(root.join("local-registrations.json")) {
            Ok(bytes) => serde_json::from_slice::<Records>(&bytes)
                .map_err(|_| "登録データが不正です（上書きしません）")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Records {
                schema_version: 1,
                registrations: vec![],
            },
            Err(_) => return Err("登録データを読み取れません".into()),
        };
        let mut ids = BTreeSet::new();
        let mut games = BTreeSet::new();
        if state.schema_version != 1
            || state.registrations.iter().any(|r| {
                !r.source_path.is_absolute()
                    || r.source_sha256.len() != 64
                    || !r.source_sha256.bytes().all(|c| c.is_ascii_hexdigit())
                    || !ids.insert(r.id)
                    || !games.insert(r.game_id.clone())
            })
        {
            return Err("登録データのバージョン・内容に対応していません（上書きしません）".into());
        }
        Ok(Self {
            root,
            state: Mutex::new(state),
            _lock: lock,
        })
    }
    pub fn list(&self) -> Vec<Registration> {
        self.state.lock().unwrap().registrations.clone()
    }
    /// Persist first, then update memory. There is one reviewed registration per game.
    pub fn register(&self, game_id: GameId, source: &ConfigSource) -> Result<Registration, String> {
        source.verify_unchanged()?;
        let mut state = self.state.lock().unwrap();
        if let Some(existing) = state.registrations.iter().find(|r| {
            r.game_id == game_id && r.source_path == source.path && r.source_sha256 == source.digest
        }) {
            return Ok(existing.clone());
        }
        let previous = state.registrations.iter().find(|r| r.game_id == game_id);
        if previous.is_some_and(|r| {
            let state = self.root.join("instances").join(r.id.to_string());
            ["process.json", "operation.json", "restore-journal.json"]
                .iter()
                .any(|file| state.join(file).exists())
        }) {
            return Err("プロセスまたは未完了操作の記録があります。停止・回復を完了してから登録してください".into());
        }
        let registration = Registration {
            id: previous
                .filter(|r| r.source_path == source.path)
                .map(|r| r.id)
                .unwrap_or_default(),
            game_id: game_id.clone(),
            source_path: source.path.clone(),
            source_sha256: source.digest.clone(),
        };
        let mut next = Records {
            schema_version: 1,
            registrations: state.registrations.clone(),
        };
        next.registrations.retain(|r| r.game_id != game_id);
        next.registrations.push(registration.clone());
        let mut file = tempfile::NamedTempFile::new_in(&self.root).map_err(|e| e.to_string())?;
        serde_json::to_writer_pretty(file.as_file_mut(), &next)
            .map_err(|_| "登録データを生成できません")?;
        file.write_all(b"\n").map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.persist(self.root.join("local-registrations.json"))
            .map_err(|_| "登録データを保存できません。前の登録を維持します")?;
        *state = next;
        Ok(registration)
    }
}

/// Create reviewed configuration files without replacing any existing file.
/// The caller orders the registration source last. On an ordinary error, remove
/// only files/directories this call created, and only if their contents still match.
pub fn create_files(files: &[(PathBuf, String)]) -> Result<(), String> {
    let mut seen = std::collections::BTreeSet::new();
    for (path, contents) in files {
        crate::local::paths::validate(path)?;
        if !seen.insert(crate::local::paths::key(path))
            || path
                .try_exists()
                .map_err(|_| "Cannot inspect creation target")?
        {
            return Err(format!(
                "作成先は既に存在するか重複しています / Target exists or is duplicated: {}",
                path.display()
            ));
        }
        if contents.len() > 2 * 1024 * 1024 {
            return Err("Configuration exceeds 2 MiB".into());
        }
    }
    let mut created: Vec<(PathBuf, String)> = vec![];
    let mut directories: Vec<PathBuf> = vec![];
    let result = (|| {
        for (path, contents) in files {
            let parent = path.parent().ok_or("Missing configuration parent")?;
            let mut missing = vec![];
            for p in parent.ancestors() {
                if p.exists() {
                    break;
                }
                missing.push(p.to_path_buf());
            }
            for p in missing.into_iter().rev() {
                crate::local::paths::validate(&p)?;
                fs_create_directory(&p, &mut directories)?;
            }
            crate::local::paths::validate(path)?;
            let mut tmp = tempfile::NamedTempFile::new_in(parent)
                .map_err(|_| "Cannot create temporary configuration")?;
            tmp.write_all(contents.as_bytes())
                .map_err(|_| "Cannot write new configuration")?;
            tmp.as_file()
                .sync_all()
                .map_err(|_| "Cannot sync new configuration")?;
            tmp.persist_noclobber(path).map_err(|_|"作成先に保存できません。既存ファイルは上書きしません / Cannot create configuration; existing files are never overwritten")?;
            created.push((path.clone(), contents.clone()));
        }
        Ok(())
    })();
    if result.is_err() {
        for (p, text) in created.iter().rev() {
            if ConfigSource::read(p).is_ok_and(|s| s.text() == text) {
                let _ = std::fs::remove_file(p);
            }
        }
        for p in directories.iter().rev() {
            let _ = std::fs::remove_dir(p);
        }
    }
    result
}
fn fs_create_directory(path: &Path, created: &mut Vec<PathBuf>) -> Result<(), String> {
    match std::fs::create_dir(path) {
        Ok(()) => {
            created.push(path.into());
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            crate::local::paths::validate(path)?;
            if path.is_dir() {
                Ok(())
            } else {
                Err("Creation parent is not a directory".into())
            }
        }
        Err(_) => Err("Cannot create configuration directory".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn game() -> GameId {
        "example".to_owned().try_into().unwrap()
    }
    #[test]
    fn references_keep_identity_across_stopped_same_source_edits() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("source.toml");
        fs::write(&config, "password = 'artificial-secret'").unwrap();
        let source = ConfigSource::read(&config).unwrap();
        let store = RegistrationStore::open(dir.path()).unwrap();
        let first = store.register(game(), &source).unwrap();
        assert_eq!(store.register(game(), &source).unwrap().id, first.id);
        assert!(
            !fs::read_to_string(dir.path().join("local-registrations.json"))
                .unwrap()
                .contains("artificial-secret")
        );
        drop(store);
        let store = RegistrationStore::open(dir.path()).unwrap();
        assert_eq!(store.list().as_slice(), std::slice::from_ref(&first));
        fs::write(&config, "password = 'different-artificial-secret'").unwrap();
        assert!(store.register(game(), &source).is_err());
        assert_eq!(store.list().as_slice(), std::slice::from_ref(&first));
        let changed = store
            .register(game(), &ConfigSource::read(&config).unwrap())
            .unwrap();
        assert_eq!(changed.id, first.id);
        assert_eq!(store.list().len(), 1);
    }
    #[test]
    fn changing_a_record_with_process_identity_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("source.toml");
        fs::write(&path, "first").unwrap();
        let store = RegistrationStore::open(temp.path()).unwrap();
        let first = store
            .register(game(), &ConfigSource::read(&path).unwrap())
            .unwrap();
        let state = temp.path().join("instances").join(first.id.to_string());
        fs::create_dir_all(&state).unwrap();
        fs::write(state.join("process.json"), "fixture identity").unwrap();
        fs::write(&path, "changed").unwrap();
        assert!(
            store
                .register(game(), &ConfigSource::read(&path).unwrap())
                .is_err()
        );
        assert_eq!(store.list()[0], first);
    }
    #[test]
    fn locked_corrupt_and_failed_storage_never_silently_reset_registration() {
        let dir = tempfile::tempdir().unwrap();
        assert!(ConfigSource::read(dir.path()).is_err());
        let store = RegistrationStore::open(dir.path()).unwrap();
        assert!(RegistrationStore::open(dir.path()).is_err());
        let source_path = dir.path().join("source.toml");
        fs::write(&source_path, "fixture").unwrap();
        let source = ConfigSource::read(&source_path).unwrap();
        fs::create_dir(dir.path().join("local-registrations.json")).unwrap();
        assert!(store.register(game(), &source).is_err());
        assert!(store.list().is_empty());
        fs::remove_dir(dir.path().join("local-registrations.json")).unwrap();
        drop(store);
        for invalid in ["invalid", r#"{"schema_version":99,"registrations":[]}"#] {
            fs::write(dir.path().join("local-registrations.json"), invalid).unwrap();
            assert!(RegistrationStore::open(dir.path()).is_err());
            assert_eq!(
                fs::read_to_string(dir.path().join("local-registrations.json")).unwrap(),
                invalid
            );
        }
    }
}

#[cfg(test)]
mod editor_tests {
    use super::*;
    #[test]
    fn reviewed_write_rejects_conflicts_and_preserves_original_on_failure() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.toml");
        fs::write(&path, "original").unwrap();
        let source = ConfigSource::read(&path).unwrap();
        fs::write(&path, "external change").unwrap();
        assert!(replace_reviewed(&source, "approved edit").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "external change");
        let source = ConfigSource::read(&path).unwrap();
        assert!(replace_reviewed(&source, &"x".repeat(2 * 1024 * 1024 + 1)).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "external change");
        replace_reviewed(&source, "approved edit").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "approved edit");
    }
}

#[cfg(test)]
mod creation_tests {
    use super::*;
    #[test]
    fn new_configuration_creation_never_overwrites_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.ini");
        std::fs::write(&old, "existing").unwrap();
        let new = dir.path().join("new.ini");
        assert!(
            create_files(&[
                (new.clone(), "new".into()),
                (old.clone(), "overwrite".into())
            ])
            .is_err()
        );
        assert!(!new.exists());
        assert_eq!(std::fs::read_to_string(old).unwrap(), "existing");
        let engine = dir.path().join("server/Config/Engine.ini");
        create_files(&[
            (engine.clone(), "engine-settings".into()),
            (new.clone(), "manager-settings".into()),
        ])
        .unwrap();
        assert_eq!(std::fs::read_to_string(engine).unwrap(), "engine-settings");
        assert_eq!(std::fs::read_to_string(new).unwrap(), "manager-settings");
    }
}
