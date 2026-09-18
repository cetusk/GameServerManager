//! Conan local plan and SQLite validation, based on the supplied MIT maintainer.
use gsm_domain::{
    Instance, Registration,
    config::{Toml, arguments, ini},
    local::{LocalServer, SaveTarget, Shutdown},
};
use std::{fs, path::Path};

pub fn resolve(reg: &Registration, text: &str) -> Result<LocalServer, String> {
    let c = Toml::parse(text)?;
    let root = c.path("paths", "server_dir")?;
    let saved = root.join("ConanSandbox/Saved");
    let config = saved.join("Config/WindowsServer");
    let game_port = c.port("server", "game_port", 7777)?;
    let query = c.port("server", "query_port", 27015)?;
    let rcon = c.port("server", "rcon_port", 25575)?;
    let players = c.number("server", "max_players", 10)?;
    if !(1..=100).contains(&players) {
        return Err("max_players は 1〜100 にしてください".into());
    }
    let mut ports = vec![game_port, query];
    let mut argv = vec![
        "-log".into(),
        format!("-Port={game_port}"),
        format!("-QueryPort={query}"),
        format!("-MaxPlayers={players}"),
    ];
    if c.boolean("server", "rcon_enabled", true)? {
        ports.push(rcon);
        argv.push(format!("-RconPort={rcon}"));
    }
    let mut unique = ports.clone();
    unique.sort();
    unique.dedup();
    if unique.len() != ports.len() {
        return Err("Conan のポートが重複しています".into());
    }
    for arg in arguments(&c.optional("server", "extra_args", "")?)? {
        if !["-useallavailablecores", "-nosteam", "-nobattleye"]
            .contains(&arg.to_ascii_lowercase().as_str())
        {
            return Err(
                "追加引数は useallavailablecores / nosteam / NoBattlEye のみ検証済みです".into(),
            );
        }
        argv.push(arg);
    }
    let mut secrets = vec![];
    for (file, section, keys) in [
        (
            "ServerSettings.ini",
            "ServerSettings",
            &["AdminPassword", "ServerPassword"][..],
        ),
        ("Game.ini", "RconPlugin", &["RconPassword"][..]),
    ] {
        let path = config.join(file);
        if path.is_file() {
            let text = fs::read_to_string(path).map_err(|_| "Conan の INI を読み取れません")?;
            for key in keys {
                if let Some(value) = ini(&text, section, key)? {
                    secrets.push(value);
                }
            }
        }
    }
    let mut exe = root.join("ConanSandbox/Binaries/Win64/ConanSandboxServer-Win64-Shipping.exe");
    let test = root.join("ConanSandbox/Binaries/Win64/ConanSandboxServer-Win64-Test.exe");
    if !exe.is_file() && test.is_file() {
        exe = test;
    }
    let mut targets = vec![
        SaveTarget {
            key: "settings".into(),
            path: config.clone(),
        },
        SaveTarget {
            key: "modlist".into(),
            path: root.join("ConanSandbox/Mods/modlist.txt"),
        },
        SaveTarget {
            key: "manager".into(),
            path: reg.source_path.clone(),
        },
    ];
    for (key, file) in [
        ("db", "game.db"),
        ("db-wal", "game.db-wal"),
        ("db-shm", "game.db-shm"),
        ("db-journal", "game.db-journal"),
    ] {
        targets.push(SaveTarget {
            key: key.into(),
            path: saved.join(file),
        });
    }
    let mut edit_files = vec![reg.source_path.clone()];
    for file in ["ServerSettings.ini", "Engine.ini", "Game.ini"] {
        if config.join(file).is_file() {
            edit_files.push(config.join(file));
        }
    }
    let modlist = root.join("ConanSandbox/Mods/modlist.txt");
    if modlist.is_file() {
        edit_files.push(modlist);
    }
    Ok(LocalServer {
        instance: Instance {
            id: reg.id,
            game_id: reg.game_id.clone(),
            name: if config.join("Engine.ini").is_file() {
                let text = fs::read_to_string(config.join("Engine.ini"))
                    .map_err(|_| "Engine.ini を読み取れません")?;
                ini(&text, "OnlineSubsystem", "ServerName")?
                    .unwrap_or_else(|| "Conan Exiles".into())
            } else {
                "Conan Exiles".into()
            },
            world: "game.db".into(),
        },
        executable: exe,
        working_directory: root.clone(),
        cwd: root,
        arguments: argv,
        save_targets: targets,
        backup_dir: c.path("paths", "backup_dir")?,
        log_file: saved.join("Logs/ConanSandbox.log"),
        shutdown: Shutdown::Console,
        stop_timeout_secs: c.number("manager", "graceful_stop_timeout_sec", 60)?,
        ports,
        steamcmd: c.path("paths", "steamcmd")?,
        steam_app_id: 443030,
        secrets,
        edit_files,
        validate_layout: |_| Ok(()),
        validate_start: |s| validate_saves(&s.save_targets),
        validate_saves,
    })
}
pub fn validate_saves(targets: &[SaveTarget]) -> Result<(), String> {
    let db = &targets
        .iter()
        .find(|t| t.key == "db")
        .ok_or("Conan DB 保存対象がありません")?
        .path;
    if !db.exists() {
        if targets
            .iter()
            .any(|t| t.key.starts_with("db-") && t.path.exists())
        {
            return Err("Conan の DB 本体がなく、journal だけが残っています".into());
        }
        return Ok(());
    }
    // SQLite may create/recover SHM or a journal even on inspection. Work on copies.
    let temp = tempfile::tempdir().map_err(|_| "DB 検証用の一時領域を作成できません")?;
    for (key, file) in [
        ("db", "game.db"),
        ("db-wal", "game.db-wal"),
        ("db-shm", "game.db-shm"),
        ("db-journal", "game.db-journal"),
    ] {
        if let Some(target) = targets.iter().find(|t| t.key == key)
            && target.path.exists()
        {
            fs::copy(&target.path, temp.path().join(file))
                .map_err(|_| "DB 検証用のコピーに失敗しました")?;
        }
    }
    check_database(&temp.path().join("game.db"))
}
fn check_database(path: &Path) -> Result<(), String> {
    let db = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| "Conan の DB を開けません")?;
    let result: String = db
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|_| "Conan の DB 整合性検査に失敗しました")?;
    if result != "ok" {
        return Err("Conan の DB 整合性検査がエラーを返しました".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_copied_wal_without_changing_live_database() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("game.db");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE fixture(value TEXT); INSERT INTO fixture VALUES ('saved-in-wal');").unwrap();
        let targets = [
            SaveTarget {
                key: "db".into(),
                path: path.clone(),
            },
            SaveTarget {
                key: "db-wal".into(),
                path: t.path().join("game.db-wal"),
            },
            SaveTarget {
                key: "db-shm".into(),
                path: t.path().join("game.db-shm"),
            },
        ];
        let before = fs::read(&path).unwrap();
        let wal = fs::read(&targets[1].path).unwrap();
        validate_saves(&targets).unwrap();
        assert_eq!(fs::read(path).unwrap(), before);
        assert_eq!(fs::read(&targets[1].path).unwrap(), wal);
    }
    #[test]
    fn corrupted_database_and_orphan_journal_are_refused() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("game.db");
        let targets = [
            SaveTarget {
                key: "db".into(),
                path: path.clone(),
            },
            SaveTarget {
                key: "db-wal".into(),
                path: t.path().join("game.db-wal"),
            },
        ];
        fs::write(&targets[1].path, b"bad").unwrap();
        assert!(validate_saves(&targets).is_err());
        fs::write(path, b"not-sqlite").unwrap();
        assert!(validate_saves(&targets).is_err());
    }
}
