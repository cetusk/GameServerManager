//! Local paths and consistency checks adapted from the supplied MIT maintainer.
use gsm_domain::{
    Instance, Registration,
    config::{Toml, name},
    local::{LocalServer, SaveTarget, Shutdown},
};
use serde_json::Value;
use std::{fs, io::Read, path::Path};
fn json(path: &Path) -> Result<Value, String> {
    let f = fs::File::open(path).map_err(|_| "Windrose の JSON を開けません")?;
    let mut data = vec![];
    f.take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut data)
        .map_err(|_| "JSON を読み取れません")?;
    if data.len() > 2 * 1024 * 1024 {
        return Err("JSON が大きすぎます".into());
    }
    serde_json::from_slice(&data).map_err(|_| "Windrose の JSON 構文を確認してください".into())
}
fn field(value: &Value, key: &str) -> Result<Option<String>, String> {
    let mut values = vec![];
    fn visit(v: &Value, key: &str, values: &mut Vec<String>) {
        match v {
            Value::Object(map) => {
                for (k, v) in map {
                    if k == key {
                        if let Some(s) = v.as_str() {
                            values.push(s.into())
                        }
                    } else {
                        visit(v, key, values)
                    }
                }
            }
            Value::Array(a) => {
                for v in a {
                    visit(v, key, values)
                }
            }
            _ => (),
        }
    }
    visit(value, key, &mut values);
    values.sort();
    values.dedup();
    if values.len() > 1 {
        return Err("Windrose の JSON 内で同名の設定値が競合しています".into());
    }
    Ok(values.pop())
}
pub fn resolve(reg: &Registration, text: &str) -> Result<LocalServer, String> {
    let c = Toml::parse(text)?;
    let root = c.path("paths", "server_dir")?;
    let profile = name(&c.optional("manager", "save_profile", "Default")?)?;
    if profile != "Default" {
        return Err("現在の起動方式で確認できる save_profile は Default のみです".into());
    }
    let candidates = [
        root.join("R5/ServerDescription.json"),
        root.join("ServerDescription.json"),
        root.join("R5/Saved/Config/ServerDescription.json"),
    ];
    let found = candidates
        .iter()
        .filter(|p| p.is_file())
        .collect::<Vec<_>>();
    if found.is_empty() && c.boolean("integration", "initial_setup", false)? {
        return initial_setup(reg, &c, root);
    }
    let description = match found.as_slice() {
        [p] => (*p).clone(),
        _ => return Err(
            "ServerDescription.json が未作成、または複数あります。使用する設定を確認してください"
                .into(),
        ),
    };
    let doc = json(&description)?;
    let world = name(&field(&doc, "WorldIslandId")?.ok_or("WorldIslandId がありません")?)?;
    if !world
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err("WorldIslandId に使用できない文字があります".into());
    }
    let base = root.join("R5/Saved/SaveProfiles").join(profile);
    let v2 = base.join("RocksDB_v2");
    let legacy = base.join("RocksDB");
    let rocks = if v2.is_dir() {
        v2
    } else if legacy.is_dir() {
        legacy
    } else {
        v2
    };
    let version = c.optional("integration", "save_version", "")?;
    let live = if !version.is_empty() {
        rocks.join(name(&version)?).join("Worlds").join(&world)
    } else {
        let mut found = vec![];
        if rocks.is_dir() {
            for entry in fs::read_dir(&rocks).map_err(|e| e.to_string())? {
                let p = entry.map_err(|e| e.to_string())?.path();
                if p.join("Worlds").join(&world).is_dir() {
                    found.push(p.join("Worlds").join(&world));
                }
            }
        }
        match found.as_slice(){[p]=>p.clone(),_=>return Err("ワールドのバージョンを一意に特定できません。integration.save_version を明示してください".into())}
    };
    let password = field(&doc, "Password")?.unwrap_or_default();
    let invite = field(&doc, "InviteCode")?.unwrap_or_default();
    let mut edit_files = vec![reg.source_path.clone(), description.clone()];
    if live.join("WorldDescription.json").is_file() {
        edit_files.push(live.join("WorldDescription.json"));
    }
    let spec = LocalServer {
        instance: Instance {
            id: reg.id,
            game_id: reg.game_id.clone(),
            name: field(&doc, "ServerName")?.unwrap_or_else(|| "Windrose".into()),
            world: world.clone(),
        },
        executable: root.join("R5/Binaries/Win64/WindroseServer-Win64-Shipping.exe"),
        working_directory: root.clone(),
        cwd: root.clone(),
        arguments: vec!["-log".into()],
        save_targets: vec![
            SaveTarget {
                key: format!("world-{world}"),
                path: live,
            },
            SaveTarget {
                key: "game-zips".into(),
                path: base.join("RocksDB_v2_Backups/Worlds").join(&world),
            },
            SaveTarget {
                key: "server-description".into(),
                path: description,
            },
            SaveTarget {
                key: "manager".into(),
                path: reg.source_path.clone(),
            },
        ],
        backup_dir: c.path("paths", "backup_dir")?,
        log_file: root.join("R5/Saved/Logs/R5.log"),
        shutdown: Shutdown::Console,
        stop_timeout_secs: c.number("manager", "graceful_stop_timeout_secs", 30)?,
        ports: vec![],
        steamcmd: c.path("paths", "steamcmd")?,
        steam_app_id: 4129620,
        secrets: vec![password, invite],
        edit_files,
        validate_layout,
        validate_start: |s| validate_saves(&s.save_targets),
        validate_saves,
    };
    validate_saves(&spec.save_targets)?;
    Ok(spec)
}
// New installations let the game generate its own IDs and descriptions. The
// initial scope includes every possible description and the entire new profile tree.
fn initial_setup(
    reg: &Registration,
    c: &Toml,
    root: std::path::PathBuf,
) -> Result<LocalServer, String> {
    let profiles = root.join("R5/Saved/SaveProfiles");
    let mut targets = vec![
        SaveTarget {
            key: "profiles".into(),
            path: profiles,
        },
        SaveTarget {
            key: "manager".into(),
            path: reg.source_path.clone(),
        },
    ];
    for (i, name) in [
        "R5/ServerDescription.json",
        "ServerDescription.json",
        "R5/Saved/Config/ServerDescription.json",
    ]
    .iter()
    .enumerate()
    {
        targets.push(SaveTarget {
            key: format!("description-{i}"),
            path: root.join(name),
        });
    }
    Ok(LocalServer {
        instance: Instance {
            id: reg.id,
            game_id: reg.game_id.clone(),
            name: "Windrose".into(),
            world: "初回起動で作成 / Created on first start".into(),
        },
        executable: root.join("R5/Binaries/Win64/WindroseServer-Win64-Shipping.exe"),
        working_directory: root.clone(),
        cwd: root.clone(),
        arguments: vec!["-log".into()],
        save_targets: targets,
        backup_dir: c.path("paths", "backup_dir")?,
        log_file: root.join("R5/Saved/Logs/R5.log"),
        shutdown: Shutdown::Console,
        stop_timeout_secs: 60,
        ports: vec![],
        steamcmd: c.path("paths", "steamcmd")?,
        steam_app_id: 4129620,
        secrets: vec![],
        edit_files: vec![reg.source_path.clone()],
        validate_layout: |_| Ok(()),
        validate_start: |spec| {
            for t in &spec.save_targets {
                if t.key.starts_with("description-") && t.path.exists() {
                    return Err("初期設定が生成されました。管理画面を再読み込みしてください / Initial settings exist; reload the manager".into());
                }
                if t.key == "profiles"
                    && t.path.exists()
                    && std::fs::read_dir(&t.path)
                        .map_err(|_| "Cannot inspect save profiles")?
                        .next()
                        .is_some()
                {
                    return Err("既存の保存データがあります。初回起動を中止します。設定を確認して登録してください / Existing saves found; review the generated configuration".into());
                }
            }
            Ok(())
        },
        validate_saves: |_| Ok(()),
    })
}
/// Do not snapshot an old version after the game has migrated to another store.
fn validate_layout(spec: &LocalServer) -> Result<(), String> {
    let selected = &spec
        .save_targets
        .iter()
        .find(|t| t.key.starts_with("world-"))
        .ok_or("ワールド対象がありません")?
        .path;
    let profile = selected
        .ancestors()
        .nth(4)
        .ok_or("保存プロファイルを特定できません")?;
    let selected = if selected.try_exists().map_err(|e| e.to_string())? {
        Some(fs::canonicalize(selected).map_err(|e| e.to_string())?)
    } else {
        None
    };
    for store in ["RocksDB_v2", "RocksDB"] {
        let root = profile.join(store);
        if !root.try_exists().map_err(|e| e.to_string())? {
            continue;
        }
        for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
            let world = entry
                .map_err(|e| e.to_string())?
                .path()
                .join("Worlds")
                .join(&spec.instance.world);
            if world.try_exists().map_err(|e| e.to_string())?
                && Some(fs::canonicalize(&world).map_err(|e| e.to_string())?) != selected
            {
                return Err("別の保存バージョンにも同じワールドがあります。使用版を確認し、旧版を別の場所へ退避してから登録を更新してください".into());
            }
        }
    }
    Ok(())
}
pub fn validate_saves(targets: &[SaveTarget]) -> Result<(), String> {
    let world = targets
        .iter()
        .find(|t| t.key.starts_with("world-"))
        .ok_or("ワールド保存対象がありません")?;
    let expected = world.key.strip_prefix("world-").unwrap();
    let description = &targets
        .iter()
        .find(|t| t.key == "server-description")
        .ok_or("ServerDescription 保存対象がありません")?
        .path;
    if field(&json(description)?, "WorldIslandId")?.as_deref() != Some(expected) {
        return Err("ServerDescription と対象ワールドの ID が一致しません".into());
    }
    if world.path.exists() {
        let value = json(&world.path.join("WorldDescription.json"))?;
        let id = value
            .get("WorldDescription")
            .and_then(|v| v.get("islandId"))
            .and_then(Value::as_str);
        if id != Some(expected) {
            return Err("WorldDescription とワールドフォルダーの ID が一致しません".into());
        }
    } else if targets
        .iter()
        .find(|t| t.key == "game-zips")
        .is_some_and(|t| t.path.exists())
    {
        return Err(
            "ゲーム ZIP だけが残っています。旧ツールでワールドを検証してから登録してください"
                .into(),
        );
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_a_newly_created_save_version_before_file_operations() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("server");
        let live = root.join("R5/Saved/SaveProfiles/Default/RocksDB_v2/0.10.0/Worlds/ABC");
        fs::create_dir_all(&live).unwrap();
        fs::write(
            root.join("R5/ServerDescription.json"),
            r#"{"WorldIslandId":"ABC"}"#,
        )
        .unwrap();
        fs::write(
            live.join("WorldDescription.json"),
            r#"{"WorldDescription":{"islandId":"ABC"}}"#,
        )
        .unwrap();
        let reg = Registration {
            id: Default::default(),
            game_id: "windrose".to_string().try_into().unwrap(),
            source_path: t.path().join("config.toml"),
            source_sha256: "0".repeat(64),
        };
        let text = format!(
            "[paths]\nserver_dir='{}'\nsteamcmd='{}'\nbackup_dir='{}'\n",
            root.display(),
            t.path().join("steamcmd.exe").display(),
            t.path().join("backup").display()
        );
        let spec = resolve(&reg, &text).unwrap();
        validate_layout(&spec).unwrap();
        fs::create_dir_all(root.join("R5/Saved/SaveProfiles/Default/RocksDB_v2/0.11.0/Worlds/ABC"))
            .unwrap();
        assert!(validate_layout(&spec).is_err());
    }
    #[test]
    fn snapshots_validate_embedded_ids_even_when_payload_folder_is_renamed() {
        let d = tempfile::tempdir().unwrap();
        let live = d.path().join("payload");
        fs::create_dir(&live).unwrap();
        let desc = d.path().join("description");
        let source = r#"{"WorldIslandId":"ABC","Unknown":{"x":42}}"#;
        fs::write(&desc, source).unwrap();
        fs::write(live.join("WorldDescription.json"),r#"{"WorldDescription":{"islandId":"ABC","WorldSettings":{"{\"TagName\": \"WDS.X\"}":1}}}"#).unwrap();
        let targets = vec![
            SaveTarget {
                key: "world-ABC".into(),
                path: live.clone(),
            },
            SaveTarget {
                key: "server-description".into(),
                path: desc.clone(),
            },
        ];
        assert!(validate_saves(&targets).is_ok());
        assert_eq!(fs::read_to_string(desc).unwrap(), source);
        fs::write(
            live.join("WorldDescription.json"),
            r#"{"WorldDescription":{"islandId":"OTHER"}}"#,
        )
        .unwrap();
        assert!(validate_saves(&targets).is_err());
    }
    #[test]
    fn first_start_uses_no_invented_world_and_reloads_generated_identity() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("fresh");
        let text = format!(
            "[paths]\nserver_dir='{}'\nsteamcmd='{}'\nbackup_dir='{}'\n[integration]\ninitial_setup=true\n",
            root.display(),
            temp.path().join("steamcmd.exe").display(),
            temp.path().join("backup").display()
        );
        let reg = Registration {
            id: Default::default(),
            game_id: "windrose".to_string().try_into().unwrap(),
            source_path: temp.path().join("config.toml"),
            source_sha256: String::new(),
        };
        let initial = resolve(&reg, &text).unwrap();
        (initial.validate_start)(&initial).unwrap();
        assert!(!root.exists());
        assert!(initial.save_targets.iter().any(|t| t.key == "profiles"));
        let live = root.join("R5/Saved/SaveProfiles/Default/RocksDB_v2/0.10/Worlds/ACTUAL-ID");
        fs::create_dir_all(&live).unwrap();
        fs::write(
            root.join("R5/ServerDescription.json"),
            r#"{"ServerName":"Generated","WorldIslandId":"ACTUAL-ID"}"#,
        )
        .unwrap();
        fs::write(
            live.join("WorldDescription.json"),
            r#"{"WorldDescription":{"islandId":"ACTUAL-ID"}}"#,
        )
        .unwrap();
        assert!((initial.validate_start)(&initial).is_err());
        let configured = resolve(&reg, &text).unwrap();
        assert_eq!(configured.instance.world, "ACTUAL-ID");
        assert!(configured.save_targets.iter().any(|t| t.path == live));
        assert!(!configured.save_targets.iter().any(|t| t.key == "profiles"));
    }
}
