//! UI- and game-independent contracts. No filesystem or process operations.
pub mod config;
pub mod local;
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct GameId(String);
impl TryFrom<String> for GameId {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err("invalid game ID".into());
        }
        Ok(Self(value))
    }
}
impl From<GameId> for String {
    fn from(value: GameId) -> Self {
        value.0
    }
}
impl AsRef<str> for GameId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for GameId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

macro_rules! identifier {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);
        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
        impl std::str::FromStr for $name {
            type Err = uuid::Error;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                value.parse().map(Self)
            }
        }
    };
}
identifier!(InstanceId);
identifier!(OperationId);
identifier!(BackupId);

/// A reviewed configuration reference. It never authorizes process operations.
/// A different source path creates a new identity; stopped edits retain it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    pub id: InstanceId,
    pub game_id: GameId,
    pub source_path: std::path::PathBuf,
    pub source_sha256: String,
}

#[derive(Clone, Debug)]
pub struct GameDescriptor {
    pub id: GameId,
    pub name: &'static str,
    pub description: &'static str,
    pub sample_world: &'static str,
    pub backup_scope: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instance {
    pub id: InstanceId,
    pub game_id: GameId,
    pub name: String,
    pub world: String,
}

pub const CONFIG_VERSION: u32 = 1;
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub schema_version: u32,
    pub enabled_games: Vec<GameId>,
    pub instances: Vec<Instance>,
}
impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_VERSION,
            enabled_games: vec![],
            instances: vec![],
        }
    }
}

/// Implementations must leave the previous configuration intact on failed save.
pub trait SettingsStore: Send + Sync {
    fn load(&self) -> Result<AppConfig, String>;
    fn save(&self, config: &AppConfig) -> Result<(), String>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessState {
    Absent,
    Alive,
    Unknown,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Readiness {
    Unknown,
    Ready,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Observation {
    pub process: ProcessState,
    pub readiness: Readiness,
}
impl Default for Observation {
    fn default() -> Self {
        Self {
            process: ProcessState::Absent,
            readiness: Readiness::Unknown,
        }
    }
}
/// Reviewed text is intentionally redacted from diagnostics and operation history.
#[derive(Clone, Eq, PartialEq)]
pub struct SettingsWrite {
    pub additional: Vec<SettingsFileWrite>,
    pub expected_sha256: String,
    pub contents: String,
}
impl std::fmt::Debug for SettingsWrite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SettingsWrite([redacted])")
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    Start,
    Stop,
    Backup,
    Restore(BackupId),
    Update,
    Recover,
    EditSettings,
    WriteSettings(std::sync::Arc<SettingsWrite>),
}
impl Command {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Start => "起動",
            Self::Stop => "停止",
            Self::Backup => "バックアップ",
            Self::Restore(_) => "復元",
            Self::Update => "更新",
            Self::Recover => "復元処理の回復",
            Self::EditSettings => "設定編集",
            Self::WriteSettings(_) => "設定の保存",
        }
    }
}

#[derive(Clone, Debug)]
pub struct OperationRequest {
    pub id: OperationId,
    pub instance: Instance,
    pub command: Command,
    pub before: Observation,
}
/// Blocking operations run on a worker, never on the UI event loop.
/// P1 ships only a mock implementation; local server control is not implemented.
pub trait GameBackend: Send + Sync {
    fn execute(&self, request: &OperationRequest) -> Result<Observation, String>;
    fn inspect(&self, _instance: &Instance) -> Result<Option<local::BackendState>, String> {
        Ok(None)
    }
    fn full_log(&self, _instance: &Instance) -> Result<String, String> {
        Err("Full log unavailable / ログ全文を取得できません".into())
    }
    fn is_local(&self) -> bool {
        false
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Backup {
    pub id: BackupId,
    pub instance_id: InstanceId,
    pub world: String,
    pub created_at: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JobStatus {
    Running,
    Completed,
    Failed(String),
}
#[derive(Clone, Debug)]
pub struct Job {
    pub id: OperationId,
    pub instance: Instance,
    pub command: Command,
    pub status: JobStatus,
    pub started_at: u64,
}

pub mod settings;

#[derive(Clone, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SettingsFileWrite {
    pub path: std::path::PathBuf,
    pub expected_sha256: String,
    pub contents: String,
}
impl std::fmt::Debug for SettingsFileWrite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SettingsFileWrite([redacted])")
    }
}
