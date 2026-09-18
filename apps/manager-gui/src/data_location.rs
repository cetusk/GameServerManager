//! A per-user locator, separate from the chosen management directory.
//! Explicit CLI roots and mock sessions never read or replace this locator.
use anyhow::{Context, Result, bail};
use gsm_domain::SettingsStore;
use gsm_infra::{FileSettingsStore, registration::RegistrationStore};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
struct Location {
    schema_version: u32,
    data_dir: PathBuf,
}

pub fn locator_path() -> Result<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")));
    let base = base.filter(|p| p.is_absolute()).context(
        "ユーザー設定フォルダーが不明です / User configuration directory is unavailable",
    )?;
    Ok(base.join("GameServerManager").join("data-location.json"))
}

pub fn read(path: &Path) -> Result<Option<PathBuf>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let value: Location = serde_json::from_slice(&bytes)
        .context("保存先の記録を読み込めません / Cannot read the saved data location")?;
    if value.schema_version != 1 || !value.data_dir.is_absolute() {
        bail!("保存先の記録が不正です / Invalid saved data location");
    }
    // Never silently recreate a missing disk/folder and open an empty manager.
    if !value.data_dir.try_exists()? || !value.data_dir.is_dir() {
        bail!(
            "保存先が見つかりません。ドライブの接続を確認するか、保存先を選び直してください / Saved directory is unavailable. Check the drive or choose a directory: {}",
            value.data_dir.display()
        );
    }
    Ok(Some(value.data_dir))
}

fn write(path: &Path, root: &Path) -> Result<()> {
    let parent = path.parent().context("Invalid locator path")?;
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(
        &mut file,
        &Location {
            schema_version: 1,
            data_dir: root.into(),
        },
    )?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}

fn ensure_no_pending_records(root: &Path) -> Result<()> {
    let entries = match fs::read_dir(root.join("instances")) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    for entry in entries {
        let entry = entry?;
        for name in [
            "process.json",
            "operation.json",
            "restore-journal.json",
            "settings-undo.json",
        ] {
            if entry.path().join(name).try_exists()? {
                bail!(
                    "現在の保存先にプロセス／未完了操作の記録があります。元の画面で停止・回復してから変更してください / Stop or recover servers in the current directory before switching: {}",
                    entry.path().join(name).display()
                );
            }
        }
    }
    Ok(())
}

/// Called on a worker. Probe write access without replacing any user data.
pub fn prepare(
    root: &Path,
    old: Option<&Path>,
    local: bool,
    locator: Option<&Path>,
) -> Result<PathBuf> {
    if !root.is_absolute() {
        bail!("保存先は絶対パスで指定してください / Choose an absolute directory path");
    }
    // Hold the old root lock while checking records and saving the new locator.
    let old_store = if local
        && let Some(old) = old
        && old.try_exists()?
    {
        Some(FileSettingsStore::open_local(old).map_err(anyhow::Error::msg)?)
    } else {
        None
    };
    let same = old_store
        .as_ref()
        .is_some_and(|store| fs::canonicalize(root).ok().as_deref() == Some(store.root()));
    if !same && let Some(store) = &old_store {
        ensure_no_pending_records(store.root())?;
    }
    let store = if same {
        old_store.as_ref().unwrap()
    } else {
        // Keep the new lock alive through validation and locator persistence.
        return prepare_new(root, local, locator);
    };
    validate_and_remember(store, local, locator)
}

fn prepare_new(root: &Path, local: bool, locator: Option<&Path>) -> Result<PathBuf> {
    let store = if local {
        FileSettingsStore::open_local(root)
    } else {
        FileSettingsStore::open(root)
    }
    .map_err(anyhow::Error::msg)?;
    validate_and_remember(&store, local, locator)
}

fn validate_and_remember(
    store: &FileSettingsStore,
    local: bool,
    locator: Option<&Path>,
) -> Result<PathBuf> {
    store.load().map_err(anyhow::Error::msg)?;
    if local {
        RegistrationStore::open(store.root()).map_err(anyhow::Error::msg)?;
    }
    let prefs = store.root().join(if local {
        "local-preferences.json"
    } else {
        "mock-preferences.json"
    });
    if prefs.try_exists()? {
        crate::preferences::Preferences::read(&prefs).map_err(anyhow::Error::msg)?;
    }
    gsm_infra::steamcmd::read_setting(&gsm_infra::steamcmd::settings_path(store.root(), local))
        .map_err(anyhow::Error::msg)?;
    let mut probe = tempfile::NamedTempFile::new_in(store.root())?;
    probe.write_all(b"GameServerManager write test\n")?;
    probe.as_file().sync_all()?;
    if local && let Some(locator) = locator {
        write(locator, store.root())?;
    }
    Ok(store.root().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_run_remembers_only_explicitly_confirmed_local_paths() {
        let temp = tempfile::tempdir().unwrap();
        let locator = temp.path().join("user/location.json");
        assert_eq!(read(&locator).unwrap(), None);
        let root = temp.path().join("data");
        let chosen = prepare(&root, None, true, Some(&locator)).unwrap();
        assert_eq!(read(&locator).unwrap(), Some(chosen.clone()));
        prepare(&temp.path().join("mock"), None, false, Some(&locator)).unwrap();
        assert_eq!(read(&locator).unwrap(), Some(chosen));
        fs::remove_dir_all(root).unwrap();
        assert!(read(&locator).is_err());
    }
    #[test]
    fn invalid_or_locked_destination_preserves_existing_data_and_locator() {
        let temp = tempfile::tempdir().unwrap();
        let locator = temp.path().join("location.json");
        let root = prepare(&temp.path().join("old"), None, true, Some(&locator)).unwrap();
        let target = temp.path().join("new");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("local-app.json"), "invalid").unwrap();
        assert!(prepare(&target, Some(&root), true, Some(&locator)).is_err());
        assert_eq!(
            fs::read_to_string(target.join("local-app.json")).unwrap(),
            "invalid"
        );
        assert_eq!(read(&locator).unwrap(), Some(root.clone()));
        let lock = FileSettingsStore::open_local(&root).unwrap();
        assert!(
            prepare(
                &temp.path().join("other"),
                Some(&root),
                true,
                Some(&locator)
            )
            .is_err()
        );
        drop(lock);
        assert!(!temp.path().join("other").exists());
    }
    #[test]
    fn switching_cannot_abandon_even_unregistered_recovery_records() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("old");
        let state = root.join("instances/unregistered");
        fs::create_dir_all(&state).unwrap();
        for name in [
            "process.json",
            "operation.json",
            "restore-journal.json",
            "settings-undo.json",
        ] {
            fs::write(state.join(name), "record").unwrap();
            assert!(prepare(&temp.path().join("new"), Some(&root), true, None).is_err());
            assert!(!temp.path().join("new").exists());
            assert!(prepare(&root, Some(&root), true, None).is_ok());
            fs::remove_file(state.join(name)).unwrap();
        }
        assert!(prepare(&temp.path().join("new"), Some(&root), true, None).is_ok());
        assert!(state.is_dir());
    }
    #[test]
    fn bad_locator_is_preserved_and_relative_paths_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let locator = temp.path().join("location.json");
        for bytes in ["broken", r#"{"schema_version":99,"data_dir":"relative"}"#] {
            fs::write(&locator, bytes).unwrap();
            assert!(read(&locator).is_err());
            assert!(prepare(Path::new("relative"), None, true, Some(&locator)).is_err());
            assert_eq!(fs::read_to_string(&locator).unwrap(), bytes);
        }
    }
}
