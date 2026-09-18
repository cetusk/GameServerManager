//! Game-defined local operation plans. All IO belongs to the host implementation.
use crate::{Backup, Instance, Observation};
use std::path::PathBuf;

#[derive(Clone)]
pub enum Shutdown {
    Console,
    Rcon {
        port: u16,
        password: String,
        commands: Vec<String>,
    },
    Https {
        port: u16,
        token: String,
        password: String,
    },
}
#[derive(Clone, Debug)]
pub struct SaveTarget {
    /// Stable, portable identifier used inside manifests; never a destination path.
    pub key: String,
    pub path: PathBuf,
}
/// Deliberately not Debug/Serialize: argv and shutdown configuration contain secrets.
#[derive(Clone)]
pub struct LocalServer {
    pub instance: Instance,
    pub executable: PathBuf,
    /// Installation root, used by SteamCMD and resource ownership.
    pub cwd: PathBuf,
    pub working_directory: PathBuf,
    pub arguments: Vec<String>,
    pub save_targets: Vec<SaveTarget>,
    pub backup_dir: PathBuf,
    pub log_file: PathBuf,
    pub shutdown: Shutdown,
    pub stop_timeout_secs: u64,
    pub ports: Vec<u16>,
    pub steamcmd: PathBuf,
    pub steam_app_id: u32,
    pub secrets: Vec<String>,
    pub edit_files: Vec<PathBuf>,
    pub validate_layout: fn(&LocalServer) -> Result<(), String>,
    pub validate_start: fn(&LocalServer) -> Result<(), String>,
    pub validate_saves: fn(&[SaveTarget]) -> Result<(), String>,
}
impl LocalServer {
    pub fn redact(&self, text: &str) -> String {
        let mut result = text.to_owned();
        let mut secrets: Vec<_> = self.secrets.iter().filter(|s| !s.is_empty()).collect();
        secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
        for secret in secrets {
            result = result.replace(secret, "[非表示]");
        }
        result
            .lines()
            .map(|line| {
                let lower = line.to_ascii_lowercase();
                if [
                    "password",
                    "authorization",
                    "token=",
                    "token:",
                    "invitecode",
                    "invite code",
                ]
                .iter()
                .any(|key| lower.contains(key))
                {
                    "[認証情報を含む行を非表示]".to_string()
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
/// Read-only installation metadata; Steam build IDs are not game release versions.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum Installation {
    #[default]
    Unknown,
    Unregistered,
    NotInstalled,
    SteamBuild(String),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerAlert {
    pub detail: String,
    pub update_suggested: bool,
}
#[derive(Clone, Debug, Default)]
pub struct ServerInfo {
    pub installation: Installation,
    pub alert: Option<ServerAlert>,
}
pub struct BackendState {
    pub info: ServerInfo,
    pub observation: Observation,
    pub backups: Vec<Backup>,
    pub logs: Vec<String>,
}
