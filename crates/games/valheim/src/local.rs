//! Local runtime plan, adapted from the supplied MIT Valheim maintainer.
use crate::{
    config::{ConfigDocument, WorldName},
    launch, world,
};
use gsm_domain::{
    Instance, Registration,
    local::{LocalServer, SaveTarget, Shutdown},
};
use std::path::PathBuf;

pub fn resolve(registration: &Registration, text: &str) -> Result<LocalServer, String> {
    let document = ConfigDocument::parse(text.into()).map_err(|e| e.to_string())?;
    let config = document.settings();
    let plan = launch::build_launch_plan(config).map_err(|e| e.to_string())?;
    let world = config.server.world.as_str();
    let root = PathBuf::from(config.paths.save_dir.as_str());
    let worlds = root.join(world::WORLDS_SUBDIR);
    let mut targets = vec![
        SaveTarget {
            key: "folder".into(),
            path: worlds.join(world),
        },
        SaveTarget {
            key: "manager".into(),
            path: registration.source_path.clone(),
        },
    ];
    for (key, suffix) in [
        ("db", ".db"),
        ("fwl", ".fwl"),
        ("db-old", ".db.old"),
        ("fwl-old", ".fwl.old"),
    ] {
        targets.push(SaveTarget {
            key: key.into(),
            path: worlds.join(format!("{world}{suffix}")),
        });
    }
    for name in ["adminlist", "bannedlist", "permittedlist"] {
        targets.push(SaveTarget {
            key: name.into(),
            path: root.join(format!("{name}.txt")),
        });
    }
    Ok(LocalServer {
        instance: Instance {
            id: registration.id,
            game_id: registration.game_id.clone(),
            name: config.server.name.clone(),
            world: world.into(),
        },
        executable: PathBuf::from(plan.executable()),
        working_directory: PathBuf::from(plan.working_directory()),
        cwd: PathBuf::from(plan.working_directory()),
        arguments: plan.expose_arguments().to_vec(),
        save_targets: targets,
        backup_dir: PathBuf::from(config.paths.backup_dir.as_str()),
        log_file: PathBuf::from(config.paths.log_file.as_str()),
        shutdown: Shutdown::Console,
        stop_timeout_secs: u64::from(config.manager.graceful_stop_timeout_secs),
        ports: vec![config.server.port, config.server.port + 1],
        steamcmd: PathBuf::from(config.paths.steamcmd.as_str()),
        steam_app_id: launch::STEAM_APP_ID,
        secrets: vec![config.server.password.expose().into()],
        edit_files: vec![registration.source_path.clone()],
        validate_layout: |_| Ok(()),
        validate_start: |s| validate_saves(&s.save_targets),
        validate_saves,
    })
}
pub fn validate_saves(targets: &[SaveTarget]) -> Result<(), String> {
    let get = |key: &str| {
        targets
            .iter()
            .find(|t| t.key == key)
            .map(|t| &t.path)
            .ok_or_else(|| "Valheim 保存対象が不足しています".to_string())
    };
    let db = get("db")?;
    let fwl = get("fwl")?;
    let folder = get("folder")?;
    if db.exists() != fwl.exists() {
        return Err("Valheim の .db / .fwl が片方しかありません".into());
    }
    for path in [db, fwl] {
        if path.exists()
            && (!path.is_file() || std::fs::metadata(path).map_err(|e| e.to_string())?.len() == 0)
        {
            return Err(
                "Valheim の旧ワールドファイルが空、または通常ファイルではありません".into(),
            );
        }
    }
    if folder.exists() {
        let name = WorldName::try_from(
            folder
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or("ワールド名が不正")?
                .to_string(),
        )
        .map_err(str::to_string)?;
        let report = world::inspect_world(folder.parent().ok_or("ワールド親パスが不明")?, &name)
            .map_err(|e| e.to_string())?;
        if report.folder_has_entries && !report.generations.iter().any(|g| g.has_required_files()) {
            return Err(
                "Valheim の新形式に同世代の db2 / fwl2 / chunks / ok が揃っていません".into(),
            );
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_incomplete_pairs_and_accepts_complete_new_generation() {
        let temp = tempfile::tempdir().unwrap();
        let targets = vec![
            SaveTarget {
                key: "db".into(),
                path: temp.path().join("W.db"),
            },
            SaveTarget {
                key: "fwl".into(),
                path: temp.path().join("W.fwl"),
            },
            SaveTarget {
                key: "folder".into(),
                path: temp.path().join("W"),
            },
        ];
        assert!(validate_saves(&targets).is_ok());
        std::fs::write(&targets[0].path, b"db").unwrap();
        assert!(validate_saves(&targets).is_err());
        std::fs::write(&targets[1].path, b"fwl").unwrap();
        assert!(validate_saves(&targets).is_ok());
        std::fs::create_dir(&targets[2].path).unwrap();
        std::fs::write(targets[2].path.join("_main.1.db2"), b"db2").unwrap();
        assert!(validate_saves(&targets).is_err());
        for suffix in ["fwl2", "chunks", "ok"] {
            std::fs::write(
                targets[2].path.join(format!("_main.1.{suffix}")),
                b"fixture",
            )
            .unwrap();
        }
        assert!(validate_saves(&targets).is_ok());
    }
    #[test]
    fn plan_keeps_world_scope_and_redacts_password() {
        let text = include_str!("../tests/fixtures/legacy-config.toml");
        let registration = Registration {
            id: Default::default(),
            game_id: "valheim".to_string().try_into().unwrap(),
            source_path: PathBuf::from("C:/Manager/config.toml"),
            source_sha256: "0".repeat(64),
        };
        let plan = resolve(&registration, text).unwrap();
        assert_eq!(plan.steam_app_id, 896660);
        assert_eq!(plan.instance.id, registration.id);
        assert!(plan.save_targets.iter().any(|t| t.key == "folder"));
        assert_eq!(plan.redact(&plan.secrets[0]), "[非表示]");
    }
}
