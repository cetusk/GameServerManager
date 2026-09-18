use gsm_domain::{
    Registration, SettingsFileWrite, SettingsWrite,
    settings::{self, Documents, Field, Kind},
};
use gsm_infra::registration::ConfigSource;
use std::{collections::BTreeMap, path::PathBuf};
#[derive(Clone)]
pub struct Draft {
    original: Documents,
    sources: BTreeMap<String, (PathBuf, String)>,
    registration: Option<Registration>,
    pub validation_root: Option<PathBuf>,
    pub other_registrations: Vec<Registration>,
    game: String,
    world: bool,
    schema: Vec<Field>,
    initial_values: Vec<(String, String)>,
    pub values: Vec<(String, String)>,
    pub notes: String,
}
impl Draft {
    pub fn load(
        game: &str,
        registration: Option<Registration>,
        local: bool,
        world: bool,
        mock_source: Option<Documents>,
    ) -> Result<Self, String> {
        let mut original = Documents::new();
        let mut sources = BTreeMap::new();
        let mut notes = String::new();
        if local {
            let reg = registration
                .as_ref()
                .ok_or("先に設定ファイルを登録してください / Register a configuration first")?;
            let source = ConfigSource::read(&reg.source_path)?;
            if source.digest() != reg.source_sha256 {
                return Err("設定が変更されています。登録し直してください / Settings changed; register again".into());
            }
            let spec = crate::catalog::resolve(reg, source.text())?;
            for path in &spec.edit_files {
                let file = ConfigSource::read(path)?;
                let key = if gsm_infra::local::paths::key(path)
                    == gsm_infra::local::paths::key(source.path())
                {
                    "manager".to_owned()
                } else {
                    path.file_name()
                        .ok_or("Invalid file name")?
                        .to_string_lossy()
                        .into_owned()
                };
                original.insert(key.clone(), file.text().into());
                sources.insert(key, (file.path().into(), file.digest().into()));
            }
            notes.push_str("保存範囲 / Save locations:\n");
            for t in spec.save_targets.iter().filter(|t| {
                !["manager", "profile", "settings", "server-description"].contains(&t.key.as_str())
            }) {
                notes.push_str(&format!("{}: {}\n", t.key, t.path.display()));
            }
            if game == "arksa" {
                // Resolve legacy relative installations for display without altering the file until edited.
                notes.push_str(&format!("設置先 / Installation: {}\n", spec.cwd.display()));
            }
        } else {
            original = mock_source.unwrap_or_else(|| mock_documents(game));
        }
        let mut schema = crate::catalog::settings_schema(game);
        schema.sort_by_key(|f| f.kind == Kind::Path);
        let mut values = vec![];
        if world {
            #[cfg(feature = "valheim")]
            if game == "valheim" {
                values = game_valheim::editor::fields(&original["manager"], true)?;
            }
            if values.is_empty() {
                return Err(
                    "World editor unavailable / ワールド項目は元ファイルで編集してください".into(),
                );
            }
        } else {
            for f in &schema {
                if let Some(text) = original.get(f.file) {
                    match settings::read(text,f) {
                        Ok(v)=>values.push((f.id.into(),if f.kind==Kind::Secret{String::new()}else{v})),
                        Err(_) if f.section=="@json"=>notes.push_str(&format!("{}: 実ファイルで一意に確認できないため編集不可 / unavailable in this file\n",label(f.id,false))),
                        Err(e)=>return Err(e),
                    }
                } else {
                    notes.push_str(&format!(
                        "{}: {} が未作成のため編集不可 / file not generated yet\n",
                        label(f.id, false),
                        f.file
                    ));
                }
            }
            if let Some(name) = crate::catalog::settings_name(game, &original["manager"])?
                && let Some((_, v)) = values.iter_mut().find(|(k, _)| k == "name")
            {
                *v = name;
            }
        }
        Ok(Self {
            original,
            sources,
            registration: if local { registration } else { None },
            validation_root: None,
            other_registrations: vec![],
            game: game.into(),
            world,
            schema,
            initial_values: values.clone(),
            values,
            notes,
        })
    }
    pub fn field(&self, key: &str) -> Option<&Field> {
        self.schema.iter().find(|f| f.id == key)
    }
    pub fn change(&mut self, key: &str, value: String) {
        if let Some((_, v)) = self.values.iter_mut().find(|(k, _)| k == key) {
            *v = value;
        }
    }
    pub fn reviewed(&self, english: bool) -> Result<(SettingsWrite, String, Documents), String> {
        let mut updated = self.original.clone();
        let mut changed = vec![];
        let mut secrets = vec![];
        if self.world {
            #[cfg(feature = "valheim")]
            {
                let text =
                    game_valheim::editor::apply(&self.original["manager"], &self.values, true)?;
                updated.insert("manager".into(), text);
            }
        } else {
            for (key, value) in &self.values {
                let f = self.field(key).ok_or("Unsupported field")?;
                let old = settings::read(&self.original[f.file], f)?;
                if f.kind == Kind::Secret {
                    secrets.push(old.clone());
                    secrets.push(value.clone());
                    if value.is_empty() {
                        continue;
                    }
                }
                let displayed = if self.game == "arksa" && key == "name" {
                    crate::catalog::settings_name(&self.game, &self.original["manager"])?
                        .unwrap_or_default()
                } else {
                    old
                };
                if value == &displayed {
                    continue;
                }
                if key == "name" && value.trim().is_empty() {
                    return Err("サーバー名を入力してください / Enter a server name".into());
                }
                updated.insert(f.file.into(), settings::write(&updated[f.file], f, value)?);
                changed.push(key.clone());
            }
            if !changed.is_empty() {
                crate::catalog::synchronize_settings(&self.game, &mut updated, &changed)?;
            }
        }
        if updated == self.original {
            return Err("No changes / 変更はありません".into());
        }
        // Installation switching and editing the old installation must be separate saves.
        if changed.iter().any(|k| k == "server_dir")
            && updated
                .iter()
                .any(|(k, v)| k != "manager" && self.original.get(k) != Some(v))
        {
            return Err("設置先の変更を先に保存してから、移動先の設定を開き直してください / Save the installation path first, then reopen settings at that location".into());
        }
        if let Some(reg) = &self.registration {
            let next = crate::catalog::resolve(reg, &updated["manager"])?;
            if let Some(data) = &self.validation_root {
                gsm_infra::local::LocalBackend::validate_spec(&next, data)?;
            }
            if changed
                .iter()
                .any(|k| self.field(k).is_some_and(|f| f.kind == Kind::Path))
            {
                for other in self.other_registrations.iter().filter(|r| r.id != reg.id) {
                    let source = ConfigSource::read(&other.source_path)?;
                    if source.digest() != other.source_sha256 {
                        return Err("他の登録設定が変更されています。先に再登録してください / Another registration changed; register it again first".into());
                    }
                    let other_spec = crate::catalog::resolve(other, source.text())?;
                    gsm_infra::local::LocalBackend::validate_pair(&next, &other_spec)?;
                }
            }

            if self.game == "satisfactory" && changed.iter().any(|k| k == "save_dir") {
                (next.validate_layout)(&next)?;
            }
        }
        let mut summary = vec![];
        for (key, value) in &self.values {
            let old = self
                .initial_values
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
                .unwrap_or_default();
            if (self.world && old != value) || changed.contains(key) {
                if self.field(key).is_some_and(|f| f.kind == Kind::Secret) {
                    summary.push(format!(
                        "{}: {}",
                        label(key, english),
                        if english {
                            "change (hidden)"
                        } else {
                            "変更（非表示）"
                        }
                    ));
                } else {
                    summary.push(format!(
                        "{}: {} → {}",
                        label(key, english),
                        option_label(key, old, english),
                        option_label(key, value, english)
                    ));
                }
            }
        }
        // Also remove credentials repeated in any non-secret field.
        for f in &self.schema {
            if f.kind == Kind::Secret
                && let Some(t) = self.original.get(f.file)
            {
                secrets.push(settings::read(t, f)?);
            }
        }
        let mut summary = summary.join("\n");
        secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
        for secret in secrets {
            if !secret.is_empty() {
                summary = summary.replace(&secret, "[非表示]");
            }
        }
        let additional = updated
            .iter()
            .filter(|(k, v)| k.as_str() != "manager" && self.original.get(*k) != Some(*v))
            .map(|(k, v)| {
                let (path, digest) = self
                    .sources
                    .get(k)
                    .cloned()
                    .unwrap_or_else(|| (PathBuf::from(k), String::new()));
                SettingsFileWrite {
                    path,
                    expected_sha256: digest,
                    contents: v.clone(),
                }
            })
            .collect();
        let write = SettingsWrite {
            expected_sha256: self
                .sources
                .get("manager")
                .map(|s| s.1.clone())
                .unwrap_or_default(),
            contents: updated["manager"].clone(),
            additional,
        };
        Ok((write, summary, updated))
    }
}
fn mock_documents(game: &str) -> Documents {
    let mut docs = Documents::new();
    for f in crate::catalog::settings_schema(game) {
        if f.section == "@json" {
            docs.insert(f.file.into(),r#"{"ServerName":"Fixture Windrose","Password":"fixture-secret","IsPasswordProtected":true,"MaxPlayerCount":4,"WorldIslandId":"ABC"}"#.into());
            continue;
        }
        let text = docs.entry(f.file.into()).or_default();
        let value = if f.kind == Kind::Path {
            format!("C:/GsmFixture/{}", f.key)
        } else if f.kind == Kind::Secret {
            "fixture-secret".into()
        } else if f.default.is_empty() {
            "Fixture Server".into()
        } else {
            f.default.into()
        };
        if let Ok(next) = settings::write(text, &f, &value) {
            *text = next;
        }
    }
    if game == "valheim" {
        docs.insert(
            "manager".into(),
            include_str!("../../../crates/games/valheim/tests/fixtures/legacy-config.toml").into(),
        );
    }
    if game == "arksa" {
        let p = docs.get_mut("manager").unwrap();
        for (section, key, value) in [
            ("General", "CB_MapName_Text", "TheIsland_WP"),
            (
                "General",
                "MM_Command_Val",
                "ArkAscendedServer.exe TheIsland_WP?SessionName=Fixture?Port=7777?QueryPort=27015?ServerPassword=fixture-secret -log",
            ),
            ("Server", "CB_RCONEnabled", "1"),
        ] {
            *p = settings::set_ini(p, section, key, value).unwrap();
        }
        docs.insert("GameUserSettings.ini".into(),"[ServerSettings]\nRCONEnabled=True\nRCONPort=27020\nServerAdminPassword=fixture-secret\n".into());
    }
    docs
}
pub type Drafts = BTreeMap<(String, bool), Draft>;
pub fn label(key: &str, en: bool) -> &str {
    let (ja, english) = match key {
        "name" => ("サーバー名", "Server name"),
        "new_config" => ("設定ファイルの保存先", "New configuration file"),
        "new_world" => ("新しいワールド名", "New world name"),
        "new_map" => ("マップ名", "Map name"),
        "profile_name" => ("プロファイル名", "Profile name"),
        "display_name" => ("管理画面での表示名", "Name in this manager"),
        "steamcmd" => ("SteamCMD 実行ファイル", "SteamCMD executable"),
        "server_dir" => ("サーバー設置先", "Server installation folder"),
        "backup_dir" => ("バックアップ保存先", "Backup folder"),
        "save_dir" => ("ワールド保存先", "World save folder"),
        "log_file" => ("ログファイル", "Log file"),
        "query_port" => ("検索ポート", "Query port"),
        "rcon_port" => ("RCON ポート", "RCON port"),
        "rcon_enabled" => ("RCON を有効にする", "Enable RCON"),
        "max_players" => ("最大参加人数", "Maximum players"),
        "admin_password" => ("管理者パスワード（変更用）", "New administrator password"),
        "rcon_password" => ("RCON パスワード（変更用）", "New RCON password"),
        "password_protected" => ("参加パスワードを有効にする", "Require join password"),
        "api_password" => ("管理APIログイン用パスワード", "API login password"),
        "api_token" => ("管理APIトークン", "API token"),
        "messaging_port" => ("メッセージングポート", "Reliable messaging port"),
        "external_reliable_port" => (
            "外部メッセージングポート（0＝既定）",
            "External reliable port (0 = default)",
        ),
        "port" => ("接続ポート", "Game port"),
        "password" => ("参加パスワード（変更用）", "New join password"),
        "save_interval" => ("自動保存間隔（秒）", "Save interval (seconds)"),
        "backups" => ("ゲーム内バックアップ保存件数", "In-game backup count"),
        "public" => ("公開範囲", "Visibility"),
        "crossplay" => ("クロスプレイ", "Crossplay"),
        "mod_combat" => ("戦闘", "Combat"),
        "mod_deathpenalty" => ("死亡ペナルティー", "Death penalty"),
        "mod_resources" => ("資源量", "Resources"),
        "mod_raids" => ("襲撃", "Raids"),
        "mod_portals" => ("ポータル", "Portals"),
        _ => (key, key),
    };
    if en { english } else { ja }
}
pub fn choices(key: &str) -> Vec<slint::SharedString> {
    if ["crossplay", "rcon_enabled", "password_protected"].contains(&key) {
        return vec!["false".into(), "true".into()];
    }
    #[cfg(feature = "valheim")]
    {
        game_valheim::editor::choices(key)
            .iter()
            .map(|s| (*s).into())
            .collect()
    }
    #[cfg(not(feature = "valheim"))]
    {
        let _ = key;
        vec![]
    }
}

pub fn option_label<'a>(key: &str, value: &'a str, en: bool) -> &'a str {
    if !key.starts_with("mod_")
        && !["public", "crossplay", "rcon_enabled", "password_protected"].contains(&key)
    {
        return value;
    }
    let (ja, english) = match (key, value) {
        ("public", "0") => ("非公開", "Private"),
        ("public", "1") => ("公開", "Public"),
        ("crossplay" | "rcon_enabled" | "password_protected", "false") => ("無効", "Disabled"),
        ("crossplay" | "rcon_enabled" | "password_protected", "true") => ("有効", "Enabled"),
        (_, "default") => ("標準", "Default"),
        (_, "veryeasy") => ("とても易しい", "Very easy"),
        (_, "easy") => ("易しい", "Easy"),
        ("mod_portals", "hard") => ("より厳しい制限", "Stricter restrictions"),
        ("mod_portals", "veryhard") => ("使用不可", "No portals"),
        ("mod_portals", "casual") => ("制限なし", "Unrestricted"),
        (_, "hard") => ("難しい", "Hard"),
        (_, "veryhard") => ("とても難しい", "Very hard"),
        (_, "hardcore") => ("ハードコア", "Hardcore"),
        (_, "casual") => ("カジュアル", "Casual"),
        (_, "muchless") => ("とても少ない", "Much less"),
        (_, "less") => ("少ない", "Less"),
        (_, "more") => ("多い", "More"),
        (_, "muchmore") => ("とても多い", "Much more"),
        (_, "none") => ("なし", "None"),
        _ => return value,
    };
    if en { english } else { ja }
}
pub fn display_choices(key: &str, en: bool) -> Vec<slint::SharedString> {
    choices(key)
        .iter()
        .map(|v| option_label(key, v, en).into())
        .collect()
}
pub fn choice_value(key: &str, display: &str, en: bool) -> String {
    choices(key)
        .iter()
        .find(|v| option_label(key, v, en) == display)
        .map(|v| v.to_string())
        .unwrap_or_else(|| display.into())
}

pub type ReviewReply = (
    (String, bool),
    Result<(SettingsWrite, String, Documents), String>,
);
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_enabled_game_exposes_common_paths_and_preserves_secrets() {
        for game in crate::catalog::games() {
            let id = game.id.to_string();
            let mut draft = Draft::load(&id, None, false, false, None).unwrap();
            for key in ["steamcmd", "server_dir", "backup_dir"] {
                assert!(draft.values.iter().any(|(k, _)| k == key), "{id}: {key}");
            }
            for f in draft.schema.iter().filter(|f| f.kind == Kind::Secret) {
                assert!(
                    draft
                        .values
                        .iter()
                        .find(|(k, _)| k == f.id)
                        .is_none_or(|(_, v)| v.is_empty())
                );
            }
            assert!(draft.reviewed(false).is_err(), "{id}: unchanged");
            draft.change("steamcmd", "C:/New Tools/steamcmd.exe".into());
            draft.change("backup_dir", "D:/New Backups".into());
            let (write, summary, updated) = draft.reviewed(true).unwrap();
            assert!(summary.contains("D:/New Backups"));
            assert!(!summary.contains("fixture-secret"));
            assert!(write.additional.is_empty());
            for f in draft.schema.iter().filter(|f| f.kind == Kind::Secret) {
                if let Some(text) = updated.get(f.file) {
                    assert_eq!(
                        settings::read(text, f).unwrap(),
                        settings::read(&draft.original[f.file], f).unwrap()
                    );
                }
            }
            let reopened = Draft::load(&id, None, false, false, Some(updated)).unwrap();
            assert!(
                reopened
                    .values
                    .iter()
                    .any(|(k, v)| k == "steamcmd" && v == "C:/New Tools/steamcmd.exe")
            );
            draft.change("backup_dir", "relative/path".into());
            assert!(draft.reviewed(false).is_err());
        }
    }
    #[cfg(feature = "valheim")]
    #[test]
    fn valheim_connections_and_paths_reach_launch_plan_and_secrets_stay_hidden() {
        let mut draft = Draft::load("valheim", None, false, false, None).unwrap();
        draft.change("name", "fixture-only-password renamed".into());
        draft.change("password", "new-fixture-secret".into());
        draft.change("port", "2466".into());
        draft.change("save_dir", "D:/Valheim Saves".into());
        let (write, summary, _) = draft.reviewed(false).unwrap();
        assert!(!summary.contains("fixture-only-password"));
        assert!(!summary.contains("new-fixture-secret"));
        assert!(!format!("{write:?}").contains("new-fixture-secret"));
        let doc = game_valheim::config::ConfigDocument::parse(write.contents).unwrap();
        let plan = game_valheim::launch::build_launch_plan(doc.settings()).unwrap();
        let args = plan.expose_arguments();
        for (key, value) in [
            ("-port", "2466"),
            ("-savedir", "D:/Valheim Saves"),
            ("-password", "new-fixture-secret"),
            ("-world", "Meadows"),
        ] {
            assert!(args.windows(2).any(|p| p[0] == key && p[1] == value));
        }
        draft.change("port", "65535".into());
        assert!(draft.reviewed(false).is_err());
    }
    #[cfg(feature = "arksa")]
    #[test]
    fn ark_keeps_profile_url_and_rcon_engine_consistent() {
        let mut draft = Draft::load("arksa", None, false, false, None).unwrap();
        draft.change("name", "My ARK Server".into());
        draft.change("port", "7787".into());
        draft.change("query_port", "27025".into());
        draft.change("rcon_port", "27030".into());
        draft.change("password", "new-fixture-secret".into());
        draft.change("admin_password", "new-admin-secret".into());
        let (write, summary, docs) = draft.reviewed(true).unwrap();
        assert_eq!(write.additional.len(), 1);
        assert!(!summary.contains("new-admin-secret"));
        let reg = Registration {
            id: Default::default(),
            game_id: "arksa".to_string().try_into().unwrap(),
            source_path: PathBuf::from("C:/Profile/fixture.ini"),
            source_sha256: String::new(),
        };
        let spec = game_arksa::local::resolve(&reg, &docs["manager"]).unwrap();
        assert!(spec.arguments[0].contains("SessionName=My ARK Server"));
        assert!(spec.arguments[0].contains("ServerPassword=new-fixture-secret"));
        assert_eq!(spec.ports, vec![7787, 27025, 27030]);
        assert_eq!(
            gsm_domain::config::ini(&docs["GameUserSettings.ini"], "ServerSettings", "RCONPort")
                .unwrap()
                .as_deref(),
            Some("27030")
        );
        assert_eq!(
            gsm_domain::config::ini(
                &docs["GameUserSettings.ini"],
                "ServerSettings",
                "ServerAdminPassword"
            )
            .unwrap()
            .as_deref(),
            Some("new-admin-secret")
        );
        draft.change("server_dir", "D:/Other ARK".into());
        assert!(draft.reviewed(false).is_err());
        draft.change(
            "server_dir",
            "C:/GsmFixture/Edit_Install_Location_Val".into(),
        );
        draft.change("name", "Injected?Port=1".into());
        assert!(draft.reviewed(false).is_err());
    }
    #[cfg(feature = "conan")]
    #[test]
    fn conan_writes_connection_passwords_to_their_actual_ini_files() {
        let mut draft = Draft::load("conan", None, false, false, None).unwrap();
        for (k, v) in [
            ("name", "My Conan"),
            ("port", "7797"),
            ("password", "join-secret"),
            ("admin_password", "admin-secret"),
            ("rcon_password", "rcon-secret"),
        ] {
            draft.change(k, v.into());
        }
        let (write, summary, docs) = draft.reviewed(true).unwrap();
        assert_eq!(write.additional.len(), 3);
        assert!(!summary.contains("join-secret"));
        for (file, section, key, value) in [
            ("Engine.ini", "OnlineSubsystem", "ServerName", "My Conan"),
            (
                "ServerSettings.ini",
                "ServerSettings",
                "ServerPassword",
                "join-secret",
            ),
            (
                "ServerSettings.ini",
                "ServerSettings",
                "AdminPassword",
                "admin-secret",
            ),
            ("Game.ini", "RconPlugin", "RconPassword", "rcon-secret"),
        ] {
            assert_eq!(
                gsm_domain::config::ini(&docs[file], section, key)
                    .unwrap()
                    .as_deref(),
                Some(value)
            );
        }
        assert_eq!(
            gsm_domain::config::Toml::parse(&docs["manager"])
                .unwrap()
                .port("server", "game_port", 0)
                .unwrap(),
            7797
        );
    }
    #[cfg(feature = "windrose")]
    #[test]
    fn windrose_updates_existing_nested_keys_without_inventing_top_level_keys() {
        let mut docs = mock_documents("windrose");
        docs.insert("ServerDescription.json".into(),r#"{"Settings":{"ServerName":"Fixture","Password":"old-secret","IsPasswordProtected":true,"MaxPlayerCount":4},"WorldIslandId":"ABC","Unknown":42}"#.into());
        let mut draft = Draft::load("windrose", None, false, false, Some(docs)).unwrap();
        draft.change("password", "new-secret".into());
        draft.change("name", "New Windrose".into());
        let (write, _, docs) = draft.reviewed(false).unwrap();
        assert_eq!(write.additional.len(), 1);
        let json: serde_json::Value =
            serde_json::from_str(&docs["ServerDescription.json"]).unwrap();
        assert_eq!(json["Settings"]["Password"], "new-secret");
        assert!(json.get("Password").is_none());
        assert_eq!(json["Unknown"], 42);
        assert_eq!(json["WorldIslandId"], "ABC");
    }
    #[cfg(feature = "satisfactory")]
    #[test]
    fn satisfactory_credentials_are_for_api_login_and_ports_reach_argv() {
        let mut draft = Draft::load("satisfactory", None, false, false, None).unwrap();
        assert!(!draft.values.iter().any(|(k, _)| k == "password"));
        draft.change("port", "7787".into());
        draft.change("api_password", "api-secret".into());
        let (write, summary, _) = draft.reviewed(true).unwrap();
        assert!(!summary.contains("api-secret"));
        let reg = Registration {
            id: Default::default(),
            game_id: "satisfactory".to_string().try_into().unwrap(),
            source_path: PathBuf::from("C:/fixture.toml"),
            source_sha256: String::new(),
        };
        let spec = game_satisfactory::local::resolve(&reg, &write.contents).unwrap();
        assert!(spec.arguments.contains(&"-Port=7787".into()));
        assert!(
            matches!(spec.shutdown,gsm_domain::local::Shutdown::Https{password,..} if password=="api-secret")
        );
    }
}
