//! Stopped-server snapshots. Destinations always come from the reviewed spec.
use super::paths::*;
use gsm_domain::{
    Backup, BackupId, InstanceId,
    local::{LocalServer, SaveTarget},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    directory: bool,
    files: BTreeMap<PathBuf, String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    backup: Backup,
    roots: BTreeMap<String, Option<Entry>>,
    paths: BTreeMap<String, String>,
}
fn hash(path: &Path) -> Result<String, String> {
    let mut f = fs::File::open(path).map_err(|e| {
        format!(
            "ハッシュ検証用ファイルを開けません [{}]: {e}",
            path.display()
        )
    })?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf).map_err(|e| {
            format!(
                "ハッシュ検証用ファイルを読み取れません [{}]: {e}",
                path.display()
            )
        })?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
fn inventory(path: &Path) -> Result<Option<Entry>, String> {
    if !exists(path)? {
        return Ok(None);
    }
    let meta = fs::symlink_metadata(path).map_err(err)?;
    if meta.is_file() {
        return Ok(Some(Entry {
            directory: false,
            files: BTreeMap::from([(PathBuf::from("payload"), hash(path)?)]),
        }));
    }
    if !meta.is_dir() {
        return Err("通常ファイル／ディレクトリ以外は保存できません".into());
    }
    let mut files = BTreeMap::new();
    let mut pending = vec![path.to_owned()];
    while let Some(dir) = pending.pop() {
        for item in fs::read_dir(&dir).map_err(err)? {
            let p = item.map_err(err)?.path();
            validate(&p)?;
            let m = fs::symlink_metadata(&p).map_err(err)?;
            if m.is_dir() {
                pending.push(p);
            } else if m.is_file() {
                files.insert(p.strip_prefix(path).map_err(err)?.to_owned(), hash(&p)?);
            } else {
                return Err("特殊ファイルを保存対象から除いてください".into());
            }
        }
    }
    Ok(Some(Entry {
        directory: true,
        files,
    }))
}
fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    validate(from)?;
    validate(to)?;
    if from.is_file() {
        let parent = to.parent().ok_or("親パス不明")?;
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "コピー先フォルダーを作成できません [{}]: {e}",
                parent.display()
            )
        })?;
        fs::copy(from, to).map_err(|e| {
            format!(
                "ファイルをコピーできません [{} → {}]: {e}",
                from.display(),
                to.display()
            )
        })?;
        // Windows FlushFileBuffers requires a handle opened with write access.
        // File::open is read-only and causes ERROR_ACCESS_DENIED even for a writable file.
        fs::OpenOptions::new()
            .write(true)
            .open(to)
            .map_err(|e| format!("コピー先を保存確定用に開けません [{}]: {e}", to.display()))?
            .sync_all()
            .map_err(|e| format!("コピー先の保存を確定できません [{}]: {e}", to.display()))?;
        return Ok(());
    }
    fs::create_dir_all(to)
        .map_err(|e| format!("コピー先フォルダーを作成できません [{}]: {e}", to.display()))?;
    for item in fs::read_dir(from).map_err(|e| {
        format!(
            "コピー元フォルダーを読み取れません [{}]: {e}",
            from.display()
        )
    })? {
        let item =
            item.map_err(|e| format!("コピー元の項目を読み取れません [{}]: {e}", from.display()))?;
        copy_tree(&item.path(), &to.join(item.file_name()))?;
    }
    Ok(())
}
fn root(spec: &LocalServer) -> PathBuf {
    spec.backup_dir
        .join("gsm")
        .join(spec.instance.id.to_string())
}
fn manifest(spec: &LocalServer, id: BackupId) -> Result<(Manifest, PathBuf), String> {
    let dir = root(spec).join(id.to_string());
    validate(&dir)?;
    let bytes = fs::read(dir.join("manifest.json")).map_err(err)?;
    let m: Manifest = serde_json::from_slice(&bytes).map_err(|_| "バックアップ目録が不正です")?;
    if m.version != 1
        || m.backup.id != id
        || m.backup.instance_id != spec.instance.id
        || m.backup.world != spec.instance.world
    {
        return Err("バックアップの対象が一致しません".into());
    }
    if m.paths
        != spec
            .save_targets
            .iter()
            .map(|t| (t.key.clone(), key(&t.path)))
            .collect()
    {
        return Err("バックアップ作成時と保存対象パスが異なります。設定を確認してください".into());
    }
    let keys: Vec<_> = spec.save_targets.iter().map(|t| t.key.clone()).collect();
    if m.roots.len() != keys.len() || keys.iter().any(|k| !m.roots.contains_key(k)) {
        return Err("保存対象の構成が変更されています".into());
    }
    for (key, expected) in &m.roots {
        if !component(key) {
            return Err("不正な保存対象キー".into());
        }
        let actual = inventory(&dir.join(key))?;
        if serde_json::to_value(actual).map_err(err)?
            != serde_json::to_value(expected).map_err(err)?
        {
            return Err("バックアップの内容／ハッシュが一致しません".into());
        }
    }
    (spec.validate_saves)(
        &spec
            .save_targets
            .iter()
            .map(|t| SaveTarget {
                key: t.key.clone(),
                path: dir.join(&t.key),
            })
            .collect::<Vec<_>>(),
    )?;
    Ok((m, dir))
}
pub fn create(spec: &LocalServer) -> Result<Backup, String> {
    create_inner(spec, false)
}
fn create_inner(spec: &LocalServer, allow_empty: bool) -> Result<Backup, String> {
    if !allow_empty {
        (spec.validate_saves)(&spec.save_targets)?;
    }
    let parent = root(spec);
    validate(&parent)?;
    fs::create_dir_all(&parent)
        .map_err(|e| format!("バックアップ先を作成できません [{}]: {e}", parent.display()))?;
    let staging = tempfile::tempdir_in(&parent).map_err(|e| {
        format!(
            "バックアップ一時フォルダーを作成できません [{}]: {e}",
            parent.display()
        )
    })?;
    let backup = Backup {
        id: BackupId::new(),
        instance_id: spec.instance.id,
        world: spec.instance.world.clone(),
        created_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(err)?
            .as_secs(),
    };
    let mut roots = BTreeMap::new();
    for target in &spec.save_targets {
        if !component(&target.key) || roots.contains_key(&target.key) {
            return Err("保存対象キーが不正・重複しています".into());
        }
        let before = inventory(&target.path)?;
        if before.is_some() {
            copy_tree(&target.path, &staging.path().join(&target.key))?;
        }
        let copied = inventory(&staging.path().join(&target.key))?;
        let after = inventory(&target.path)?;
        if serde_json::to_value(&before).map_err(err)?
            != serde_json::to_value(&copied).map_err(err)?
            || serde_json::to_value(&before).map_err(err)?
                != serde_json::to_value(&after).map_err(err)?
        {
            return Err("保存中に対象が変化しました。停止状態を確認してください".into());
        }
        roots.insert(target.key.clone(), before);
    }
    if !allow_empty && roots.values().all(Option::is_none) {
        return Err("バックアップ対象がありません".into());
    }
    write_json(
        &staging.path().join("manifest.json"),
        &Manifest {
            version: 1,
            backup: backup.clone(),
            roots,
            paths: spec
                .save_targets
                .iter()
                .map(|t| (t.key.clone(), key(&t.path)))
                .collect(),
        },
    )?;
    fs::rename(staging.path(), parent.join(backup.id.to_string())).map_err(err)?;
    Ok(backup)
}
pub fn list(spec: &LocalServer) -> Result<Vec<Backup>, String> {
    let parent = root(spec);
    if !exists(&parent)? {
        return Ok(vec![]);
    }
    let mut out = vec![];
    for entry in fs::read_dir(parent).map_err(err)? {
        let path = entry.map_err(err)?.path();
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with('.'))
        {
            continue;
        }
        if !path.join("manifest.json").is_file() {
            continue;
        }
        validate(&path)?;
        let data = fs::read(path.join("manifest.json")).map_err(err)?;
        let m: Manifest =
            serde_json::from_slice(&data).map_err(|_| "バックアップ目録が不正です")?;
        if m.version != 1
            || m.backup.instance_id != spec.instance.id
            || path.file_name().and_then(|s| s.to_str()) != Some(&m.backup.id.to_string())
        {
            return Err("バックアップ目録と対象が一致しません".into());
        }
        if m.backup.world == spec.instance.world {
            out.push(m.backup);
        }
    }
    out.sort_by_key(|b| b.created_at);
    Ok(out)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    instance: InstanceId,
    token: String,
    entries: Vec<Move>,
    committed: bool,
    paths: BTreeMap<String, String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Move {
    key: String,
    existed: bool,
    replacement: bool,
    started: bool,
}
fn journal_path(state: &Path) -> PathBuf {
    state.join("restore-journal.json")
}
fn siblings(target: &Path, token: &str) -> Result<(PathBuf, PathBuf), String> {
    let parent = target.parent().ok_or("復元先の親パスが不明")?;
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("復元先の名前が不正")?;
    Ok((
        parent.join(format!(".gsm-{token}-{name}.old")),
        parent.join(format!(".gsm-{token}-{name}.new")),
    ))
}
pub fn pending(state: &Path) -> bool {
    journal_path(state).exists()
}
pub fn recover(spec: &LocalServer, state: &Path) -> Result<(), String> {
    let path = journal_path(state);
    validate(&path)?;
    if !exists(&path)? {
        return Ok(());
    }
    let j: Journal = serde_json::from_slice(&fs::read(&path).map_err(err)?)
        .map_err(|_| "復元記録が不正です。手動確認が必要です")?;
    if j.version != 1
        || j.instance != spec.instance.id
        || j.token.parse::<InstanceId>().is_err()
        || j.paths
            != spec
                .save_targets
                .iter()
                .map(|t| (t.key.clone(), key(&t.path)))
                .collect()
        || j.entries.len() != spec.save_targets.len()
        || j.entries
            .iter()
            .map(|e| &e.key)
            .collect::<std::collections::BTreeSet<_>>()
            != spec.save_targets.iter().map(|t| &t.key).collect()
    {
        return Err("復元記録の対象が一致しません".into());
    }
    for item in j.entries.iter().rev() {
        let target = spec
            .save_targets
            .iter()
            .find(|t| t.key == item.key)
            .ok_or("復元対象が設定から削除されています")?;
        let (old, new) = siblings(&target.path, &j.token)?;
        if j.committed {
            remove(&old)?;
            remove(&new)?;
            continue;
        }
        if item.started {
            if exists(&old)? {
                remove(&target.path)?;
                fs::rename(&old, &target.path).map_err(err)?;
            } else if !item.existed && !exists(&new)? && item.replacement {
                remove(&target.path)?;
            }
        }
        remove(&new)?;
    }
    fs::remove_file(path).map_err(err)
}
pub fn restore(spec: &LocalServer, state: &Path, id: BackupId) -> Result<(), String> {
    if pending(state) {
        return Err("前回の復元処理が未完了です。回復を実行してください".into());
    }
    let (m, dir) = manifest(spec, id)?;
    // A verified pre-restore snapshot must finish before any live path is renamed.
    create_inner(spec, true)?;
    let token = InstanceId::new().to_string();
    let mut j = Journal {
        version: 1,
        instance: spec.instance.id,
        token,
        entries: vec![],
        committed: false,
        paths: spec
            .save_targets
            .iter()
            .map(|t| (t.key.clone(), key(&t.path)))
            .collect(),
    };
    for target in &spec.save_targets {
        let (old, new) = siblings(&target.path, &j.token)?;
        if exists(&old)? || exists(&new)? {
            return Err("復元用の一時パスが存在します".into());
        }
        j.entries.push(Move {
            key: target.key.clone(),
            existed: exists(&target.path)?,
            replacement: m.roots[&target.key].is_some(),
            started: false,
        });
    }
    write_json(&journal_path(state), &j)?;
    let outcome: Result<(), String> = (|| {
        for (target, item) in spec.save_targets.iter().zip(&j.entries) {
            if item.replacement {
                let (_, new) = siblings(&target.path, &j.token)?;
                copy_tree(&dir.join(&target.key), &new)?;
                let copied = inventory(&new)?;
                if serde_json::to_value(copied).map_err(err)?
                    != serde_json::to_value(&m.roots[&target.key]).map_err(err)?
                {
                    return Err("復元用コピーの検証に失敗しました".into());
                }
            }
        }
        for (index, target) in spec.save_targets.iter().enumerate() {
            j.entries[index].started = true;
            write_json(&journal_path(state), &j)?;
            let (old, new) = siblings(&target.path, &j.token)?;
            if j.entries[index].existed {
                fs::rename(&target.path, old).map_err(err)?;
            }
            if j.entries[index].replacement {
                fs::rename(new, &target.path).map_err(err)?;
            }
        }
        j.committed = true;
        write_json(&journal_path(state), &j)?;
        Ok(())
    })();
    match (outcome, recover(spec, state)) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(e), Ok(())) => Err(format!("復元を中止し、直前の状態へ戻しました: {e}")),
        (_, Err(e)) => Err(format!("復元の回復が必要です: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gsm_domain::{Instance, local::Shutdown};
    fn fixture(dir: &Path) -> LocalServer {
        LocalServer {
            instance: Instance {
                id: InstanceId::new(),
                game_id: "fixture".to_string().try_into().unwrap(),
                name: "fixture".into(),
                world: "world".into(),
            },
            executable: dir.join("server/game.exe"),
            cwd: dir.join("server"),
            working_directory: dir.join("server"),
            arguments: vec![],
            save_targets: vec![
                SaveTarget {
                    key: "world".into(),
                    path: dir.join("live/world"),
                },
                SaveTarget {
                    key: "metadata".into(),
                    path: dir.join("live/meta.json"),
                },
            ],
            backup_dir: dir.join("backups"),
            log_file: dir.join("server/log.txt"),
            shutdown: Shutdown::Console,
            stop_timeout_secs: 30,
            ports: vec![],
            steamcmd: dir.join("steamcmd.exe"),
            steam_app_id: 1,
            secrets: vec![],
            edit_files: vec![],
            validate_layout: |_| Ok(()),
            validate_start: |_| Ok(()),
            validate_saves: |_| Ok(()),
        }
    }
    fn put(path: impl AsRef<Path>, data: &str) {
        let p = path.as_ref();
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, data).unwrap();
    }
    #[test]
    fn copy_failure_reports_both_paths_without_changing_the_source() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.txt");
        let destination = temp.path().join("occupied-directory");
        put(&source, "private configuration contents");
        fs::create_dir(&destination).unwrap();
        let error = copy_tree(&source, &destination).unwrap_err();
        assert!(error.contains("ファイルをコピーできません"));
        assert!(error.contains(source.to_str().unwrap()));
        assert!(error.contains(destination.to_str().unwrap()));
        assert!(!error.contains("private configuration contents"));
        assert_eq!(
            fs::read_to_string(&source).unwrap(),
            "private configuration contents"
        );
        assert!(destination.is_dir());
    }
    #[test]
    fn restore_replaces_tree_and_retains_verified_pre_restore_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let s = fixture(temp.path());
        let state = temp.path().join("state");
        put(s.save_targets[0].path.join("chunk"), "original");
        put(&s.save_targets[1].path, "metadata");
        let b = create(&s).unwrap();
        put(s.save_targets[0].path.join("chunk"), "later");
        put(s.save_targets[0].path.join("stale"), "stale");
        restore(&s, &state, b.id).unwrap();
        assert_eq!(
            fs::read_to_string(s.save_targets[0].path.join("chunk")).unwrap(),
            "original"
        );
        assert!(!s.save_targets[0].path.join("stale").exists());
        assert_eq!(list(&s).unwrap().len(), 2);
        assert!(!pending(&state));
        let pre = list(&s)
            .unwrap()
            .into_iter()
            .find(|x| x.id != b.id)
            .unwrap();
        let (_, dir) = manifest(&s, pre.id).unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("world/chunk")).unwrap(),
            "later"
        );
    }
    #[test]
    fn tampering_and_wrong_world_do_not_mutate_live_data() {
        let temp = tempfile::tempdir().unwrap();
        let s = fixture(temp.path());
        let state = temp.path().join("state");
        put(s.save_targets[0].path.join("chunk"), "good");
        let b = create(&s).unwrap();
        put(
            root(&s).join(b.id.to_string()).join("world/chunk"),
            "tampered",
        );
        assert!(restore(&s, &state, b.id).is_err());
        assert_eq!(
            fs::read_to_string(s.save_targets[0].path.join("chunk")).unwrap(),
            "good"
        );
        assert!(!pending(&state));
        let b = create(&s).unwrap();
        let mut wrong = s.clone();
        wrong.instance.world = "different".into();
        assert!(restore(&wrong, &state, b.id).is_err());
        let mut moved = s.clone();
        moved.save_targets[0].path = temp.path().join("different-server");
        assert!(restore(&moved, &state, b.id).is_err());
        assert!(!moved.save_targets[0].path.exists());
        assert_eq!(list(&s).unwrap().len(), 2);
    }
    #[test]
    fn recovery_rolls_back_interrupted_moves_and_is_repeatable() {
        let temp = tempfile::tempdir().unwrap();
        let s = fixture(temp.path());
        let state = temp.path().join("state");
        put(&s.save_targets[1].path, "old");
        let token = InstanceId::new().to_string();
        let (old, new) = siblings(&s.save_targets[1].path, &token).unwrap();
        fs::rename(&s.save_targets[1].path, &old).unwrap();
        put(&s.save_targets[1].path, "replacement");
        put(s.save_targets[0].path.join("chunk"), "newly created");
        let j = Journal {
            version: 1,
            instance: s.instance.id,
            token,
            committed: false,
            paths: s
                .save_targets
                .iter()
                .map(|t| (t.key.clone(), key(&t.path)))
                .collect(),
            entries: vec![
                Move {
                    key: "world".into(),
                    existed: false,
                    replacement: true,
                    started: true,
                },
                Move {
                    key: "metadata".into(),
                    existed: true,
                    replacement: true,
                    started: true,
                },
            ],
        };
        write_json(&journal_path(&state), &j).unwrap();
        let mut changed = s.clone();
        changed.save_targets[0].path = temp.path().join("unrelated");
        assert!(recover(&changed, &state).is_err());
        assert!(pending(&state));
        recover(&s, &state).unwrap();
        recover(&s, &state).unwrap();
        assert!(!s.save_targets[0].path.exists());
        assert_eq!(fs::read_to_string(&s.save_targets[1].path).unwrap(), "old");
        assert!(!old.exists() && !new.exists());
    }
    #[test]
    fn missing_live_data_can_be_restored_and_duplicate_journal_keys_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let s = fixture(temp.path());
        let state = temp.path().join("state");
        put(&s.save_targets[1].path, "old");
        let b = create(&s).unwrap();
        remove(&s.save_targets[1].path).unwrap();
        restore(&s, &state, b.id).unwrap();
        assert_eq!(fs::read_to_string(&s.save_targets[1].path).unwrap(), "old");
        let j = Journal {
            version: 1,
            instance: s.instance.id,
            token: InstanceId::new().to_string(),
            committed: false,
            paths: s
                .save_targets
                .iter()
                .map(|t| (t.key.clone(), key(&t.path)))
                .collect(),
            entries: (0..2)
                .map(|_| Move {
                    key: "metadata".into(),
                    existed: false,
                    replacement: true,
                    started: true,
                })
                .collect(),
        };
        write_json(&journal_path(&state), &j).unwrap();
        assert!(recover(&s, &state).is_err());
        assert_eq!(fs::read_to_string(&s.save_targets[1].path).unwrap(), "old");
    }
    #[cfg(unix)]
    #[test]
    fn linked_payload_is_rejected_before_publish() {
        let temp = tempfile::tempdir().unwrap();
        let s = fixture(temp.path());
        put(temp.path().join("outside"), "keep");
        fs::create_dir_all(&s.save_targets[0].path).unwrap();
        std::os::unix::fs::symlink(
            temp.path().join("outside"),
            s.save_targets[0].path.join("link"),
        )
        .unwrap();
        assert!(create(&s).is_err());
        assert!(list(&s).unwrap().is_empty());
    }
}
