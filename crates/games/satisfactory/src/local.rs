//! Game-specific path and HTTPS shutdown plan. Imported TOML is never rewritten.
use gsm_domain::{
    Instance, Registration,
    config::Toml,
    local::{LocalServer, SaveTarget, Shutdown},
};
use std::{fs, path::PathBuf};

pub fn resolve(reg: &Registration, text: &str) -> Result<LocalServer, String> {
    let c = Toml::parse(text)?;
    let root = c.path("paths", "server_dir")?;
    let saves = c.path("paths", "save_dir")?;
    let port = c.port("server", "port", 7777)?;
    let messaging = c.port("server", "messaging_port", 8888)?;
    if port == messaging {
        return Err("ゲームとメッセージングのポートが重複しています".into());
    }
    let explicit = c.optional("integration", "executable", "")?;
    let candidates = [
        "FactoryGame/Binaries/Win64/FactoryServer-Win64-Shipping-Cmd.exe",
        "FactoryGame/Binaries/Win64/FactoryServer-Win64-Shipping.exe",
        "FactoryServer-Win64-Shipping-Cmd.exe",
        "FactoryServer-Win64-Shipping.exe",
    ];
    let found = candidates
        .iter()
        .map(|p| root.join(p))
        .filter(|p| p.is_file())
        .collect::<Vec<_>>();
    let executable = if !explicit.is_empty() {
        gsm_domain::config::absolute(&explicit)?
    } else {
        match found.as_slice(){[p]=>p.clone(),[]=>root.join(candidates[0]),_=>return Err("実行ファイル候補が複数あります。integration.executable で実プロセスを指定してください".into())}
    };
    if ![
        "FactoryServer-Win64-Shipping-Cmd.exe",
        "FactoryServer-Win64-Shipping.exe",
    ]
    .iter()
    .any(|n| {
        executable
            .file_name()
            .is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(n))
    }) {
        return Err(
            "FactoryServer.exe ランチャーではなく Shipping 実行ファイルを指定してください".into(),
        );
    }
    let mut arguments = vec![
        "-log".into(),
        "-unattended".into(),
        format!("-Port={port}"),
        format!("-ReliablePort={messaging}"),
    ];
    if c.value("server", "external_reliable_port").is_some() {
        let ext = c.number("server", "external_reliable_port", 0)?;
        if ext > 65535 {
            return Err("外部メッセージングポートが不正です".into());
        }
        if ext > 0 {
            arguments.push(format!("-ExternalReliablePort={ext}"));
        }
    }
    let token = c.optional("manager", "api_token", "")?;
    let password = c.optional("server", "admin_password", "")?;
    let spec = LocalServer {
        instance: Instance {
            id: reg.id,
            game_id: reg.game_id.clone(),
            name: c.text("server", "name")?,
            world: "Dedicated SaveGames".into(),
        },
        executable,
        working_directory: root.clone(),
        cwd: root,
        arguments,
        save_targets: vec![
            SaveTarget {
                key: "saves".into(),
                path: saves,
            },
            SaveTarget {
                key: "manager".into(),
                path: reg.source_path.clone(),
            },
        ],
        backup_dir: c.path("paths", "backup_dir")?,
        log_file: c.path("paths", "log_file")?,
        shutdown: if c.boolean("integration", "initial_setup", false)?
            && token.is_empty()
            && password.is_empty()
        {
            Shutdown::Console
        } else {
            Shutdown::Https {
                port,
                token: token.clone(),
                password: password.clone(),
            }
        },
        stop_timeout_secs: c.number("manager", "graceful_stop_timeout_secs", 60)?,
        ports: vec![port, messaging],
        steamcmd: c.path("paths", "steamcmd")?,
        steam_app_id: 1690800,
        secrets: vec![
            token,
            password,
            c.optional("server", "client_password", "")?,
        ],
        edit_files: vec![reg.source_path.clone()],
        validate_layout,
        validate_start,
        validate_saves,
    };
    if c.number("manager", "app_id", 1690800)? != 1690800 {
        return Err("Satisfactory 専用サーバー以外の App ID は使用できません".into());
    }
    Ok(spec)
}
fn validate_layout(spec: &LocalServer) -> Result<(), String> {
    validate_saves(&spec.save_targets)?;
    let saves = &spec
        .save_targets
        .iter()
        .find(|t| t.key == "saves")
        .ok_or("保存先がありません")?
        .path;
    let engine =
        PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA がありません")?)
            .join("FactoryGame/Saved/SaveGames");
    // Installing binaries before the first save exists is allowed. Start still requires binding.
    if !engine.try_exists().map_err(|e| e.to_string())?
        && !saves.try_exists().map_err(|e| e.to_string())?
    {
        return Ok(());
    }
    verify_binding(&engine, saves)
}
fn validate_start(spec: &LocalServer) -> Result<(), String> {
    validate_saves(&spec.save_targets)?;
    let saves = &spec
        .save_targets
        .iter()
        .find(|t| t.key == "saves")
        .ok_or("保存先がありません")?
        .path;
    let engine =
        PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA がありません")?)
            .join("FactoryGame/Saved/SaveGames");
    if matches!(spec.shutdown, Shutdown::Console) {
        return verify_initial_binding(&engine, saves);
    }
    verify_binding(&engine, saves)?;
    match &spec.shutdown {
        Shutdown::Https {
            token, password, ..
        } if !token.is_empty() || !password.is_empty() => Ok(()),
        _ => Err("正常停止に必要な API トークンまたは管理者パスワードを設定してください".into()),
    }
}
fn verify_initial_binding(engine: &std::path::Path, saves: &std::path::Path) -> Result<(), String> {
    if !engine.exists() && !saves.exists() {
        let normalize = |p: &std::path::Path| {
            p.to_string_lossy()
                .replace('\\', "/")
                .trim_start_matches("//?/")
                .trim_end_matches('/')
                .to_lowercase()
        };
        if normalize(engine) != normalize(saves) {
            return Err("初回の保存先は実際の SaveGames を指定してください / Use the game's actual SaveGames folder for first start".into());
        }
        Ok(())
    } else {
        verify_binding(engine, saves)
    }
}
pub fn verify_binding(engine: &std::path::Path, saves: &std::path::Path) -> Result<(), String> {
    let a = fs::canonicalize(engine).map_err(
        |_| "ゲームの SaveGames パスを確認できません。既存の保存先設定を確認してください",
    )?;
    let b = fs::canonicalize(saves).map_err(|_| "指定した save_dir を確認できません")?;
    if a != b {
        return Err(
            "ゲームの実際の SaveGames と save_dir が一致しません。保存先は自動変更しません".into(),
        );
    }
    Ok(())
}
pub fn validate_saves(targets: &[SaveTarget]) -> Result<(), String> {
    let root = &targets
        .iter()
        .find(|t| t.key == "saves")
        .ok_or("保存対象がありません")?
        .path;
    if !root.exists() {
        return Ok(());
    }
    if !root.is_dir() {
        return Err("SaveGames はディレクトリが必要です".into());
    }
    for item in fs::read_dir(root).map_err(|e| e.to_string())? {
        let item = item.map_err(|e| e.to_string())?;
        let name = item.file_name().to_string_lossy().to_string();
        if !matches!(name.as_str(), "server" | "blueprints")
            && !(name.starts_with("ServerSettings") && name.ends_with(".sav"))
        {
            return Err("SaveGames に専用サーバー以外のデータがあります。専用サーバーの保存領域を分離してから登録してください".into());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn binding_mismatch_does_not_move_or_create_paths() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("engine");
        let b = t.path().join("selected");
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        fs::write(a.join("personal.sav"), "original").unwrap();
        assert!(verify_binding(&a, &b).is_err());
        assert_eq!(
            fs::read_to_string(a.join("personal.sav")).unwrap(),
            "original"
        );
        assert!(verify_binding(&a, &a).is_ok());
        assert!(
            validate_saves(&[SaveTarget {
                key: "saves".into(),
                path: a
            }])
            .is_err()
        );
    }
    #[cfg(unix)]
    #[test]
    fn existing_link_is_verified_without_retargeting() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("engine");
        let b = t.path().join("selected");
        fs::create_dir(&b).unwrap();
        std::os::unix::fs::symlink(&b, &a).unwrap();
        verify_binding(&a, &b).unwrap();
        assert_eq!(fs::read_link(a).unwrap(), b);
    }
    #[test]
    fn initial_binding_allows_only_the_actual_game_path_without_creating_it() {
        let temp = tempfile::tempdir().unwrap();
        let engine = temp.path().join("actual-savegames");
        let wrong = temp.path().join("wrong");
        assert!(verify_initial_binding(&engine, &wrong).is_err());
        verify_initial_binding(&engine, &engine).unwrap();
        assert!(!engine.exists());
        assert!(!wrong.exists());
    }
}
