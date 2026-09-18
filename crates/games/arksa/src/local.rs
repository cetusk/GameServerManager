//! Legacy ARK SA profile adapter. See LICENSE for the supplied maintainer.
use gsm_domain::{
    Instance, Registration,
    config::{absolute, arguments, ini, name},
    local::{LocalServer, SaveTarget, Shutdown},
};
use std::{fs, path::PathBuf};
fn value(text: &str, section: &str, key: &str) -> Result<String, String> {
    ini(text, section, key)?
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("{section}.{key} を設定してください"))
}
fn number(text: &str, section: &str, key: &str) -> Result<u16, String> {
    let n = value(text, section, key)?
        .parse::<u16>()
        .map_err(|_| format!("{section}.{key} はポート番号が必要です"))?;
    if n < 1024 {
        return Err("ポートは 1024 以上が必要です".into());
    }
    Ok(n)
}
fn enabled(text: &str, section: &str, key: &str) -> Result<bool, String> {
    Ok(ini(text, section, key)?.is_some_and(|s| s == "1" || s.eq_ignore_ascii_case("true")))
}

pub fn resolve(reg: &Registration, text: &str) -> Result<LocalServer, String> {
    let raw = value(text, "General", "Edit_Install_Location_Val")?;
    let root = if enabled(text, "General", "Sys_RelativePath")? {
        let p = PathBuf::from(&raw);
        if p.is_absolute()
            || raw.replace('\\', "/").split('/').any(|s| s == "..")
            || raw.contains(':')
        {
            return Err("相対インストール先に絶対パス・親移動は指定できません".into());
        }
        reg.source_path
            .parent()
            .and_then(|p| p.parent())
            .ok_or("旧 Profile フォルダーの親を特定できません")?
            .join(p)
    } else {
        absolute(&raw)?
    };
    let map = name(&value(text, "General", "CB_MapName_Text")?)?;
    let command_key = if enabled(text, "General", "ChB_CMD_override")? {
        "MM_Command_Override"
    } else {
        "MM_Command_Val"
    };
    let mut argv = arguments(&value(text, "General", command_key)?)?;
    if argv.len() < 2 || !argv[0].eq_ignore_ascii_case("ArkAscendedServer.exe") {
        return Err("ARK 起動行は ArkAscendedServer.exe とマップから始めてください".into());
    }
    argv.remove(0);
    let mut segments = argv[0].split('?');
    if segments.next() != Some(&map) {
        return Err("プロファイルのマップと起動行のマップが一致しません".into());
    }
    let mut params = std::collections::BTreeMap::new();
    for part in segments {
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        if params.insert(k.to_ascii_lowercase(), v).is_some() {
            return Err("ARK 起動 URL に重複キーがあります".into());
        }
    }
    for key in [
        "serveradminpassword",
        "rconenabled",
        "rconport",
        "altsavedirectoryname",
        "saveddir",
    ] {
        if params.contains_key(key) {
            return Err("RCON 設定や保存先変更は ARK 起動 URL に含めないでください".into());
        }
    }
    let port = number(text, "Server", "SE_Port")?;
    let query = number(text, "Server", "SE_QueryPort")?;
    let rcon = number(text, "Server", "SE_RCONPort")?;
    if port == query || port == rcon || query == rcon {
        return Err("ARK のポートが重複しています".into());
    }
    for (key, expected) in [("port", port), ("queryport", query)] {
        if params.get(key).and_then(|v| v.parse::<u16>().ok()) != Some(expected) {
            return Err("ARK の起動 URL とプロファイルのポートが一致しません".into());
        }
    }
    if !enabled(text, "Server", "CB_RCONEnabled")? {
        return Err("正常停止のためプロファイルで RCON を有効にしてください".into());
    }
    let admin = value(text, "Server", "Edit_ServerAdminPassword")?;
    let password = ini(text, "Server", "Edit_ServerPassword")?.unwrap_or_default();
    if params.get("serverpassword").copied().unwrap_or("") != password {
        return Err("ARK の起動 URL とプロファイルの接続パスワードが一致しません".into());
    }
    for arg in argv.iter().skip(1) {
        let lower = arg.to_ascii_lowercase();
        if let Some(mods) = lower.strip_prefix("-mods=") {
            if mods
                .split(',')
                .any(|id| id.is_empty() || !id.bytes().all(|c| c.is_ascii_digit()))
            {
                return Err("MOD ID 一覧が不正です".into());
            }
        } else if ![
            "-log",
            "-nobattleye",
            "-servergamelog",
            "-servergamelogincludetribelogs",
            "-serverrconoutputtribelogs",
            "-useallavailablecores",
            "-nosteamclient",
            "-game",
            "-server",
            "-forcerespawndinos",
            "-preventhibernation",
        ]
        .contains(&lower.as_str())
        {
            return Err("ARK の追加起動フラグは未対応です。保存先と設定への影響を確認してから追加してください".into());
        }
    }
    let saved = root.join("ShooterGame/Saved");
    let settings = saved.join("Config/WindowsServer");
    let mut edit_files = vec![reg.source_path.clone()];
    for file in ["GameUserSettings.ini", "Game.ini"] {
        if settings.join(file).is_file() {
            edit_files.push(settings.join(file));
        }
    }
    Ok(LocalServer {
        instance: Instance {
            id: reg.id,
            game_id: reg.game_id.clone(),
            name: ini(text, "General", "Edit_Profile")?.unwrap_or_else(|| "ARK SA".into()),
            world: map.clone(),
        },
        executable: root.join("ShooterGame/Binaries/Win64/ArkAscendedServer.exe"),
        working_directory: root.join("ShooterGame/Binaries/Win64"),
        cwd: root,
        arguments: argv,
        save_targets: vec![
            SaveTarget {
                key: format!("map-{map}"),
                path: saved.join("SavedArks").join(map),
            },
            SaveTarget {
                key: "settings".into(),
                path: settings,
            },
            SaveTarget {
                key: "profile".into(),
                path: reg.source_path.clone(),
            },
        ],
        backup_dir: absolute(&value(text, "Integration", "BackupDir")?)?,
        log_file: saved.join("Logs/ShooterGame.log"),
        shutdown: Shutdown::Rcon {
            port: rcon,
            password: admin.clone(),
            commands: vec!["SaveWorld".into(), "DoExit".into()],
        },
        stop_timeout_secs: 60,
        ports: vec![port, query, rcon],
        steamcmd: absolute(&value(text, "Integration", "SteamCMD")?)?,
        steam_app_id: 2430930,
        secrets: vec![admin, password],
        edit_files,
        validate_layout: |_| Ok(()),
        validate_start,
        validate_saves: |_| Ok(()),
    })
}
fn validate_start(spec: &LocalServer) -> Result<(), String> {
    let path = spec
        .save_targets
        .iter()
        .find(|t| t.key == "settings")
        .ok_or("設定対象がありません")?
        .path
        .join("GameUserSettings.ini");
    let text = fs::read_to_string(path).map_err(|_| "GameUserSettings.ini を読み取れません")?;
    let Shutdown::Rcon { port, password, .. } = &spec.shutdown else {
        return Err("RCON 設定がありません".into());
    };
    if !enabled(&text, "ServerSettings", "RCONEnabled")?
        || number(&text, "ServerSettings", "RCONPort")? != *port
        || value(&text, "ServerSettings", "ServerAdminPassword")? != *password
    {
        return Err("GameUserSettings.ini とプロファイルの RCON 設定が一致しません".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> String {
        r#"[General]
Edit_Profile=Fixture
Edit_Install_Location_Val=C:/ARK
CB_MapName_Text=TheIsland_WP
MM_Command_Val=ArkAscendedServer.exe TheIsland_WP?listen?Port=7777?QueryPort=27015?MaxPlayers=10 -mods=3,1,2 -log
[Server]
SE_Port=7777
SE_QueryPort=27015
SE_RCONPort=27020
CB_RCONEnabled=1
Edit_ServerAdminPassword=artificial-secret
[Integration]
BackupDir=D:/Backups/ARK
SteamCMD=C:/SteamCMD/steamcmd.exe
"#.into()
    }
    fn registration() -> Registration {
        Registration {
            id: Default::default(),
            game_id: "arksa".to_string().try_into().unwrap(),
            source_path: PathBuf::from("C:/Manager/Profile/fixture.ini"),
            source_sha256: "0".repeat(64),
        }
    }
    #[test]
    fn retains_mod_order_and_excludes_admin_secret_from_url() {
        let p = resolve(&registration(), &profile()).unwrap();
        assert!(p.arguments.contains(&"-mods=3,1,2".into()));
        assert_eq!(p.working_directory, p.executable.parent().unwrap());
        assert_ne!(p.cwd, p.working_directory);
        assert!(!p.arguments.join(" ").contains("artificial-secret"));
        assert!(p.save_targets[0].path.ends_with("SavedArks/TheIsland_WP"));
    }
    #[test]
    fn refuses_map_and_port_mismatch_and_rcon_in_url() {
        for text in [
            profile().replace("?Port=7777", "?Port=7778"),
            profile().replace("TheIsland_WP?", "ScorchedEarth_WP?"),
            profile().replace("?listen", "?listen?RCONPort=27020"),
        ] {
            assert!(resolve(&registration(), &text).is_err());
        }
    }
}
