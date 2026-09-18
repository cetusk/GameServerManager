//! Argument generation adapted from the supplied Valheim implementation.
//! A plan is data only: it does not spawn, bind ports, or change game files.
use crate::config::LegacyConfig;
use std::fmt;

pub const SERVER_EXE: &str = "valheim_server.exe";
pub const STEAM_APP_ID: u32 = 896_660;

#[derive(Debug, thiserror::Error)]
#[error("起動設定を確認してください: {0}")]
pub struct LaunchError(pub &'static str);

/// Never format raw arguments into a shell command. A future launcher must use args().
pub struct LaunchPlan {
    executable: String,
    working_directory: String,
    arguments: Vec<String>,
    password: String,
}
impl LaunchPlan {
    pub fn executable(&self) -> &str {
        &self.executable
    }
    pub fn working_directory(&self) -> &str {
        &self.working_directory
    }
    /// Contains the password. For a future direct process API only, never logs/UI.
    pub fn expose_arguments(&self) -> &[String] {
        &self.arguments
    }
    pub fn redacted_preview(&self) -> String {
        let scrub = |text: &str| text.replace(&self.password, "[非表示]");
        let mut lines = vec![
            format!("実行ファイル: {}", scrub(&self.executable)),
            format!("作業フォルダー: {}", scrub(&self.working_directory)),
            "起動引数（1 行につき 1 引数）:".into(),
        ];
        let mut secret = false;
        for arg in &self.arguments {
            lines.push(if secret {
                "  [非表示]".into()
            } else {
                format!("  {:?}", scrub(arg))
            });
            secret = arg == "-password";
        }
        lines.join("\n")
    }
}
impl fmt::Debug for LaunchPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.redacted_preview())
    }
}

fn normalized_path(path: &str) -> String {
    path.replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}
fn contains_path(parent: &str, child: &str) -> bool {
    child == parent || child.starts_with(&format!("{parent}\\"))
}
fn join_windows(path: &str, name: &str) -> String {
    format!("{}\\{name}", path.trim_end_matches(['/', '\\']))
}

pub fn build_launch_plan(config: &LegacyConfig) -> Result<LaunchPlan, LaunchError> {
    config
        .validate()
        .map_err(|_| LaunchError("サーバー名・パスワード・ポート・保存間隔"))?;
    let s = &config.server;
    // The game itself also parses switches; shell-free argv alone does not protect it
    // from a value that masquerades as another switch.
    if [s.name.as_str(), s.world.as_str(), s.password.expose()]
        .iter()
        .any(|v| v.starts_with('-') || v.chars().any(char::is_control))
    {
        return Err(LaunchError(
            "名前・ワールド・パスワードの先頭 '-' または制御文字",
        ));
    }
    let save = normalized_path(config.paths.save_dir.as_str());
    let backup = normalized_path(config.paths.backup_dir.as_str());
    if contains_path(&save, &backup) || contains_path(&backup, &save) {
        return Err(LaunchError(
            "保存先とバックアップ先が同一、または一方が他方の内部です",
        ));
    }
    let executable = join_windows(config.paths.server_dir.as_str(), SERVER_EXE);
    if normalized_path(&executable) == normalized_path(config.paths.log_file.as_str()) {
        return Err(LaunchError("ログ出力先が実行ファイルと一致しています"));
    }
    let mut arguments = vec!["-nographics".into(), "-batchmode".into()];
    for (flag, value) in [
        ("-name", s.name.clone()),
        ("-port", s.port.to_string()),
        ("-world", s.world.as_str().into()),
        ("-password", s.password.expose().into()),
        ("-public", s.public.to_string()),
        ("-savedir", config.paths.save_dir.as_str().into()),
        ("-saveinterval", s.save_interval.to_string()),
        ("-backups", s.backups.to_string()),
        ("-logFile", config.paths.log_file.as_str().into()),
    ] {
        arguments.extend([flag.into(), value]);
    }
    // Values from the supplied GUI's translations.rs, kept game-local.
    for (name, value, allowed) in [
        (
            "combat",
            &s.mod_combat,
            &["veryeasy", "easy", "hard", "veryhard"][..],
        ),
        (
            "deathpenalty",
            &s.mod_deathpenalty,
            &["casual", "veryeasy", "hard", "hardcore"][..],
        ),
        (
            "resources",
            &s.mod_resources,
            &["muchless", "less", "more", "muchmore"][..],
        ),
        (
            "raids",
            &s.mod_raids,
            &["none", "muchless", "less", "more", "muchmore"][..],
        ),
        (
            "portals",
            &s.mod_portals,
            &["casual", "hard", "veryhard"][..],
        ),
    ] {
        if value.is_empty() || value == "default" {
            continue;
        }
        if !allowed.contains(&value.as_str()) {
            return Err(LaunchError("未対応の world modifier"));
        }
        arguments.extend(["-modifier".into(), name.into(), value.clone()]);
    }
    if !s.preset.is_empty() && s.preset != "default" {
        if !["casual", "hard", "hardcore", "immersive", "hammer"].contains(&s.preset.as_str()) {
            return Err(LaunchError("未対応の preset"));
        }
        arguments.extend(["-preset".into(), s.preset.clone()]);
    }
    for key in &s.world_keys {
        if key.is_empty() {
            continue;
        }
        if ![
            "nobuildcost",
            "passivemobs",
            "nomap",
            "noportals",
            "playerevents",
            "showenemyhud",
            "devcommands",
        ]
        .contains(&key.as_str())
        {
            return Err(LaunchError("未対応の world key"));
        }
        arguments.extend(["-setkey".into(), key.clone()]);
    }
    for (flag, value, default) in [
        ("-backupshort", config.manager.backup_short_secs, 7200),
        ("-backuplong", config.manager.backup_long_secs, 43200),
    ] {
        if value == 0 {
            return Err(LaunchError(
                "内部バックアップ間隔は 0 より大きい値が必要です",
            ));
        }
        if value != default {
            arguments.extend([flag.into(), value.to_string()]);
        }
    }
    if s.crossplay {
        arguments.push("-crossplay".into());
    }
    Ok(LaunchPlan {
        executable,
        working_directory: config.paths.server_dir.as_str().into(),
        arguments,
        password: s.password.expose().into(),
    })
}
