//! Legacy config.toml schema adapted from Valheim_ServerMaintainer.
//! See LICENSE for the retained copyright notice. Parsing never reads or writes game paths.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt};

pub type UnknownFields = BTreeMap<String, toml::Value>;

/// Explicit access is required to expose a password. Debug output is redacted.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct Password(String);
impl Password {
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

/// Windows syntax is checked on every host; no existence/ownership claim is made.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct WindowsPath(String);
impl WindowsPath {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl From<WindowsPath> for String {
    fn from(path: WindowsPath) -> Self {
        path.0
    }
}
impl TryFrom<String> for WindowsPath {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let normalized = value.replace('/', "\\");
        let bytes = normalized.as_bytes();
        let remainder = if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'\\'
        {
            &normalized[3..]
        } else if let Some(unc) = normalized.strip_prefix("\\\\") {
            let mut parts = unc.splitn(3, '\\');
            let host = parts.next().unwrap_or_default();
            let share = parts.next().unwrap_or_default();
            if !valid_component(host) || !valid_component(share) {
                return Err("invalid UNC host or share");
            }
            parts.next().unwrap_or_default()
        } else {
            return Err("expected an absolute Windows drive or UNC path");
        };
        let remainder = remainder.strip_suffix('\\').unwrap_or(remainder);
        if !remainder.is_empty() && !remainder.split('\\').all(valid_component) {
            return Err("invalid Windows path component");
        }
        Ok(Self(value))
    }
}

fn valid_component(value: &str) -> bool {
    if value.trim().is_empty()
        || value.ends_with(['.', ' '])
        || value
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
    {
        return false;
    }
    let stem = value.split('.').next().unwrap_or_default().to_uppercase();
    !matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) && !(stem.starts_with("COM") || stem.starts_with("LPT"))
        .then(|| &stem[3..])
        .is_some_and(|n| {
            matches!(
                n,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
}

/// A single Windows filename, never a path or alternate data stream.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct WorldName(String);
impl WorldName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl From<WorldName> for String {
    fn from(name: WorldName) -> Self {
        name.0
    }
}
impl TryFrom<String> for WorldName {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        // Leave room for the longest legacy suffix, .fwl.old (8 UTF-16 units).
        if !valid_component(&value) || value.encode_utf16().count() > 247 {
            return Err("invalid Windows world filename");
        }
        Ok(Self(value))
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct LegacyConfig {
    pub paths: PathsConfig,
    pub server: ServerConfig,
    #[serde(default)]
    pub manager: ManagerConfig,
    #[serde(flatten)]
    pub extra: UnknownFields,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct PathsConfig {
    pub steamcmd: WindowsPath,
    pub server_dir: WindowsPath,
    pub save_dir: WindowsPath,
    pub backup_dir: WindowsPath,
    pub log_file: WindowsPath,
    #[serde(flatten)]
    pub extra: UnknownFields,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    pub name: String,
    pub world: WorldName,
    pub password: Password,
    pub port: u16,
    pub public: u8,
    pub save_interval: u32,
    pub backups: u32,
    #[serde(default)]
    pub crossplay: bool,
    #[serde(default = "default_modifier")]
    pub mod_combat: String,
    #[serde(default = "default_modifier")]
    pub mod_deathpenalty: String,
    #[serde(default = "default_modifier")]
    pub mod_resources: String,
    #[serde(default = "default_modifier")]
    pub mod_raids: String,
    #[serde(default = "default_modifier")]
    pub mod_portals: String,
    #[serde(default)]
    pub preset: String,
    #[serde(default)]
    pub world_keys: Vec<String>,
    #[serde(flatten)]
    pub extra: UnknownFields,
}
fn default_modifier() -> String {
    "default".into()
}

#[derive(Clone, Deserialize, Serialize)]
pub struct ManagerConfig {
    pub graceful_stop_timeout_secs: u32,
    pub auto_backup_before_update: bool,
    #[serde(default)]
    pub language: Language,
    #[serde(default)]
    pub public_address: String,
    #[serde(default = "default_backup_short_secs")]
    pub backup_short_secs: u32,
    #[serde(default = "default_backup_long_secs")]
    pub backup_long_secs: u32,
    #[serde(flatten)]
    pub extra: UnknownFields,
}
fn default_backup_short_secs() -> u32 {
    7200
}
fn default_backup_long_secs() -> u32 {
    43200
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub enum Language {
    #[default]
    #[serde(rename = "ja")]
    Ja,
    #[serde(rename = "en")]
    En,
}
impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            graceful_stop_timeout_secs: 30,
            auto_backup_before_update: true,
            language: Language::Ja,
            public_address: String::new(),
            backup_short_secs: default_backup_short_secs(),
            backup_long_secs: default_backup_long_secs(),
            extra: UnknownFields::new(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    // Do not retain toml::de::Error: its source excerpt may contain a password.
    #[error("設定の TOML 構文・型・必須項目・Windows パスを確認してください")]
    Parse,
    #[error("設定値が不正です: {0}")]
    Validation(&'static str),
}
impl LegacyConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        let s = &self.server;
        let require = |condition, field| {
            if condition {
                Ok(())
            } else {
                Err(ConfigError::Validation(field))
            }
        };
        require(
            !s.name.trim().is_empty() && !s.name.chars().any(char::is_control),
            "server.name",
        )?;
        let password = s.password.expose();
        require(
            password.chars().count() >= 5 && !password.chars().any(char::is_control),
            "server.password",
        )?;
        require(
            !s.name.contains(password) && !password.contains(&s.name),
            "server.password / server.name",
        )?;
        require(
            (1024..=65534).contains(&s.port),
            "server.port (1024..=65534)",
        )?;
        require(s.public <= 1, "server.public (0 / 1)")?;
        require(s.save_interval >= 60, "server.save_interval (>= 60)")?;
        Ok(())
    }
}

/// The original text is retained byte-for-byte, including comments and unknown fields.
/// Allowlisted edits are implemented separately in `editor`.
pub struct ConfigDocument {
    source: String,
    settings: LegacyConfig,
}
impl ConfigDocument {
    pub fn parse(source: String) -> Result<Self, ConfigError> {
        let settings: LegacyConfig = toml::from_str(&source).map_err(|_| ConfigError::Parse)?;
        settings.validate()?;
        Ok(Self { source, settings })
    }
    pub fn settings(&self) -> &LegacyConfig {
        &self.settings
    }
    /// Contains credentials. Do not log or display this text.
    pub fn original_toml(&self) -> &str {
        &self.source
    }
}
