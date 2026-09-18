//! Reviewed multi-file writes. A durable undo journal blocks operations until recovered.
use super::paths::{key, validate, write_json};
use crate::registration::{ConfigSource, replace_reviewed};
use gsm_domain::{SettingsFileWrite, SettingsWrite, local::LocalServer};
use std::{collections::BTreeSet, fs, path::Path};
#[derive(serde::Serialize, serde::Deserialize)]
struct Undo {
    path: std::path::PathBuf,
    before: String,
    after: String,
}
pub fn pending(state: &Path) -> bool {
    state.join("settings-undo.json").exists()
}
/// Resolve a pending operation against its original registration after a restart.
/// The original source digest must match the registered digest; never guess a config.
pub fn recovery_source(
    reg: &gsm_domain::Registration,
    state: &Path,
) -> Result<Option<String>, String> {
    use sha2::{Digest, Sha256};
    if !pending(state) {
        return Ok(None);
    }
    let path = state.join("settings-undo.json");
    validate(&path)?;
    let file = fs::File::open(path).map_err(|_| "Cannot read settings recovery record")?;
    let undo: Vec<Undo> = serde_json::from_reader(std::io::Read::take(file, 32 * 1024 * 1024))
        .map_err(|_| "Invalid settings recovery record")?;
    let matches: Vec<_> = undo
        .iter()
        .filter(|u| key(&u.path) == key(&reg.source_path))
        .collect();
    if matches.len() != 1 {
        return Err("Recovery record does not identify the registered source".into());
    }
    let source = matches[0];
    if format!("{:x}", Sha256::digest(source.before.as_bytes())) != reg.source_sha256 {
        return Err("Recovery source does not match the registered digest".into());
    }
    Ok(Some(source.before.clone()))
}
fn existing_key(path: &Path) -> Result<String, String> {
    validate(path)?;
    // ConfigSource canonicalizes paths. On Windows this also expands 8.3 names
    // (e.g. RUNNER~1), so stripping only the verbatim prefix is not sufficient.
    let resolved = fs::canonicalize(path).map_err(|_| "Cannot resolve settings scope path")?;
    validate(&resolved)?;
    Ok(key(&resolved))
}
fn allowed(spec: &LocalServer, path: &Path) -> Result<String, String> {
    let target = existing_key(path)?;
    if !spec
        .edit_files
        .iter()
        .any(|p| existing_key(p).is_ok_and(|k| k == target))
        || !spec.save_targets.iter().any(|t| {
            existing_key(&t.path).is_ok_and(|k| target == k || target.starts_with(&(k + "\\")))
        })
    {
        return Err(
            "設定の編集・バックアップ対象外です / File is outside the editable backup scope".into(),
        );
    }
    Ok(target)
}
pub fn prepare(
    spec: &LocalServer,
    source: &Path,
    change: &SettingsWrite,
) -> Result<Vec<SettingsFileWrite>, String> {
    let mut writes = change.additional.clone();
    writes.push(SettingsFileWrite {
        path: source.into(),
        expected_sha256: change.expected_sha256.clone(),
        contents: change.contents.clone(),
    });
    let mut seen = BTreeSet::new();
    for write in &writes {
        let target = allowed(spec, &write.path)?;
        if !seen.insert(target) {
            return Err("Duplicate settings target".into());
        }
        let file = ConfigSource::read(&write.path)?;
        if file.digest() != write.expected_sha256 {
            return Err("設定が変更されています。読み直して確認してください / Settings changed; reload before saving".into());
        }
        if write.contents.len() > 2 * 1024 * 1024 {
            return Err("Settings file too large".into());
        }
    }
    Ok(writes)
}
pub fn apply(spec: &LocalServer, state: &Path, writes: &[SettingsFileWrite]) -> Result<(), String> {
    if pending(state) {
        return Err("設定の回復が必要です / Recover settings first".into());
    }
    let mut undo = vec![];
    let mut seen = BTreeSet::new();
    for write in writes {
        if !seen.insert(allowed(spec, &write.path)?) {
            return Err("Duplicate settings target".into());
        }
        let file = ConfigSource::read(&write.path)?;
        if file.digest() != write.expected_sha256 {
            return Err("Settings changed before saving / 保存前に設定が変更されました".into());
        }
        undo.push(Undo {
            path: write.path.clone(),
            before: file.text().into(),
            after: write.contents.clone(),
        });
    }
    write_json(&state.join("settings-undo.json"), &undo)?;
    let result = (|| {
        for write in writes {
            let file = ConfigSource::read(&write.path)?;
            if file.digest() != write.expected_sha256 {
                return Err("Settings changed during saving / 保存中に設定が変更されました".into());
            }
            if file.text() != write.contents {
                replace_reviewed(&file, &write.contents)?;
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        return match recover(spec, state) {
            Ok(()) => Err(error),
            Err(_) => Err(format!(
                "{error}\n設定の一部が未確定です。「回復」が必要です / Settings recovery required"
            )),
        };
    }
    fs::remove_file(state.join("settings-undo.json")).map_err(|_|"Settings saved; journal cleanup failed. Recover before continuing / 設定保存後の回復記録を削除できません".into())
}
pub fn recover(spec: &LocalServer, state: &Path) -> Result<(), String> {
    if !pending(state) {
        return Ok(());
    }
    let path = state.join("settings-undo.json");
    validate(&path)?;
    let file = fs::File::open(&path).map_err(|_| "Cannot open settings recovery record")?;
    let undo: Vec<Undo> = serde_json::from_reader(std::io::Read::take(file, 32 * 1024 * 1024))
        .map_err(|_| "Invalid settings recovery record")?;
    let mut seen = BTreeSet::new();
    // Preflight every file before undoing any; never overwrite an external edit.
    for u in &undo {
        if !seen.insert(allowed(spec, &u.path)?) {
            return Err("Duplicate recovery target".into());
        }
        let current = ConfigSource::read(&u.path)?;
        if current.text() != u.before && current.text() != u.after {
            return Err("設定が外部で変更されています。バックアップと照合してください / External edit prevents automatic recovery".into());
        }
    }
    for u in undo.iter().rev() {
        let current = ConfigSource::read(&u.path)?;
        if current.text() == u.after && u.before != u.after {
            replace_reviewed(&current, &u.before)?;
        }
    }
    fs::remove_file(path).map_err(|_| "Cannot remove settings recovery record".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    use gsm_domain::{
        Instance,
        local::{SaveTarget, Shutdown},
    };
    fn fixture(dir: &Path) -> LocalServer {
        LocalServer {
            instance: Instance {
                id: Default::default(),
                game_id: "fixture".to_string().try_into().unwrap(),
                name: "Fixture".into(),
                world: "world".into(),
            },
            executable: dir.join("server.exe"),
            cwd: dir.into(),
            working_directory: dir.into(),
            arguments: vec![],
            save_targets: vec![
                SaveTarget {
                    key: "manager".into(),
                    path: dir.join("config.toml"),
                },
                SaveTarget {
                    key: "settings".into(),
                    path: dir.join("settings"),
                },
            ],
            backup_dir: dir.join("backup"),
            log_file: dir.join("server.log"),
            shutdown: Shutdown::Console,
            stop_timeout_secs: 60,
            ports: vec![],
            steamcmd: dir.join("steamcmd.exe"),
            steam_app_id: 1,
            secrets: vec![],
            edit_files: vec![dir.join("config.toml"), dir.join("settings/Game.ini")],
            validate_layout: |_| Ok(()),
            validate_start: |_| Ok(()),
            validate_saves: |_| Ok(()),
        }
    }
    fn setup() -> (tempfile::TempDir, LocalServer, SettingsWrite) {
        let dir = tempfile::tempdir().unwrap();
        let spec = fixture(dir.path());
        fs::create_dir(dir.path().join("settings")).unwrap();
        fs::create_dir(dir.path().join("state")).unwrap();
        for path in &spec.edit_files {
            fs::write(path, "old-secret").unwrap();
        }
        let a = ConfigSource::read(&spec.edit_files[0]).unwrap();
        let b = ConfigSource::read(&spec.edit_files[1]).unwrap();
        let change = SettingsWrite {
            expected_sha256: a.digest().into(),
            contents: "new-manager".into(),
            additional: vec![SettingsFileWrite {
                path: b.path().into(),
                expected_sha256: b.digest().into(),
                contents: "new-engine".into(),
            }],
        };
        (dir, spec, change)
    }
    #[test]
    fn writes_authorized_files_source_last_and_refuses_conflicts_before_changes() {
        let (dir, spec, mut change) = setup();
        let state = dir.path().join("state");
        let writes = prepare(&spec, &spec.edit_files[0], &change).unwrap();
        assert_eq!(writes.last().unwrap().path, spec.edit_files[0]);
        let outside = dir.path().join("unrelated.ini");
        fs::write(&outside, "private").unwrap();
        change.additional[0].path = outside.clone();
        assert!(prepare(&spec, &spec.edit_files[0], &change).is_err());
        assert_eq!(fs::read_to_string(outside).unwrap(), "private");
        fs::write(&spec.edit_files[1], "external-edit").unwrap();
        assert!(apply(&spec, &state, &writes).is_err());
        assert_eq!(
            fs::read_to_string(&spec.edit_files[0]).unwrap(),
            "old-secret"
        );
        assert!(!pending(&state));
        fs::write(&spec.edit_files[1], "old-secret").unwrap();
        apply(&spec, &state, &writes).unwrap();
        assert_eq!(
            fs::read_to_string(&spec.edit_files[0]).unwrap(),
            "new-manager"
        );
        assert_eq!(
            fs::read_to_string(&spec.edit_files[1]).unwrap(),
            "new-engine"
        );
        assert!(!pending(&state));
    }
    #[test]
    fn resolved_paths_still_require_both_scopes_and_reject_duplicate_targets() {
        let (dir, spec, mut change) = setup();
        let state = dir.path().join("state");
        let source = ConfigSource::read(&spec.edit_files[0]).unwrap();
        let mut restricted = spec.clone();
        restricted.edit_files.clear();
        assert!(prepare(&restricted, source.path(), &change).is_err());
        restricted = spec.clone();
        restricted.save_targets.clear();
        assert!(prepare(&restricted, source.path(), &change).is_err());

        // Sharing a string prefix with a backed-up directory is insufficient.
        let sibling = dir.path().join("settings-other/Game.ini");
        fs::create_dir(sibling.parent().unwrap()).unwrap();
        fs::write(&sibling, "old-secret").unwrap();
        restricted = spec.clone();
        restricted.edit_files.push(sibling.clone());
        change.additional[0].path = sibling;
        assert!(prepare(&restricted, source.path(), &change).is_err());

        // Raw and canonical spellings of the same source cannot be written twice.
        change.additional[0].path = source.path().into();
        assert!(prepare(&spec, &spec.edit_files[0], &change).is_err());
        let writes = vec![
            SettingsFileWrite {
                path: spec.edit_files[0].clone(),
                expected_sha256: source.digest().into(),
                contents: "new".into(),
            },
            SettingsFileWrite {
                path: source.path().into(),
                expected_sha256: source.digest().into(),
                contents: "other".into(),
            },
        ];
        assert!(apply(&spec, &state, &writes).is_err());
        assert!(!pending(&state));
        for path in &spec.edit_files {
            assert_eq!(fs::read_to_string(path).unwrap(), "old-secret");
        }
    }
    #[cfg(unix)]
    #[test]
    fn canonicalization_does_not_allow_symlinks_into_the_editable_scope() {
        let (dir, spec, change) = setup();
        let link = dir.path().join("linked-config.toml");
        std::os::unix::fs::symlink(&spec.edit_files[0], &link).unwrap();
        assert!(prepare(&spec, &link, &change).is_err());
        let mut linked_scope = spec.clone();
        linked_scope.edit_files[0] = link;
        assert!(prepare(&linked_scope, &spec.edit_files[0], &change).is_err());
    }
    #[test]
    fn interrupted_batch_recovers_without_overwriting_external_edits() {
        let (dir, spec, change) = setup();
        let state = dir.path().join("state");
        let writes = prepare(&spec, &spec.edit_files[0], &change).unwrap();
        let undo: Vec<_> = writes
            .iter()
            .map(|w| Undo {
                path: w.path.clone(),
                before: "old-secret".into(),
                after: w.contents.clone(),
            })
            .collect();
        write_json(&state.join("settings-undo.json"), &undo).unwrap();
        let reg = gsm_domain::Registration {
            id: spec.instance.id,
            game_id: spec.instance.game_id.clone(),
            source_path: spec.edit_files[0].clone(),
            source_sha256: change.expected_sha256.clone(),
        };
        assert_eq!(
            recovery_source(&reg, &state).unwrap().as_deref(),
            Some("old-secret")
        );
        let mut wrong = reg.clone();
        wrong.source_sha256 = "0".repeat(64);
        assert!(recovery_source(&wrong, &state).is_err());

        fs::write(&writes[0].path, &writes[0].contents).unwrap();
        fs::write(&writes[1].path, "external-edit").unwrap();
        assert!(recover(&spec, &state).is_err());
        assert!(pending(&state));
        assert_eq!(fs::read_to_string(&writes[0].path).unwrap(), "new-engine");
        fs::write(&writes[1].path, "old-secret").unwrap();
        recover(&spec, &state).unwrap();
        for w in &writes {
            assert_eq!(fs::read_to_string(&w.path).unwrap(), "old-secret");
        }
        assert!(!pending(&state));
        recover(&spec, &state).unwrap();
    }
    #[test]
    fn changing_paths_cannot_target_another_registered_servers_data() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let first = fixture(a.path());
        let mut second = fixture(b.path());
        super::super::LocalBackend::validate_pair(&first, &second).unwrap();
        second.backup_dir = first.save_targets[1].path.join("backup");
        assert!(super::super::LocalBackend::validate_pair(&first, &second).is_err());
        second = fixture(b.path());
        second.cwd = first.cwd.join("nested");
        assert!(super::super::LocalBackend::validate_pair(&first, &second).is_err());
    }
}
