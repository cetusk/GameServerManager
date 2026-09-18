//! Fresh server configuration drafts. Reading/checking paths runs on workers.
use gsm_domain::{
    Registration,
    settings::{self, Documents, Field, Kind},
};
use gsm_infra::{
    local::{LocalBackend, paths},
    registration::ConfigSource,
};
use std::path::PathBuf;
#[derive(Clone)]
pub struct Draft {
    pub game: String,
    pub values: Vec<(String, String)>,
    pub schema: Vec<Field>,
    pub source: String,
}
pub struct Plan {
    pub game: String,
    pub source: PathBuf,
    pub documents: Documents,
    pub files: Vec<(PathBuf, String)>,
    pub summary: String,
    pub server_dir: PathBuf,
    pub save_dir: Option<PathBuf>,
}
pub type Reply = Result<Plan, String>;
impl Draft {
    #[cfg(test)]
    pub fn new(game: &str) -> Self {
        Self::with_steamcmd(game, gsm_infra::steamcmd::DEFAULT_EXE)
    }
    pub fn with_steamcmd(game: &str, steamcmd: &str) -> Self {
        let mut schema = crate::catalog::creation_schema(game);
        schema.sort_by_key(|f| f.kind == Kind::Path);
        let values = schema
            .iter()
            .map(|f| {
                let value = if f.id == "steamcmd" {
                    steamcmd.into()
                } else if f.kind == Kind::Secret {
                    String::new()
                } else if ["name", "display_name", "profile_name"].contains(&f.id) {
                    format!("{game} Server")
                } else if game == "satisfactory" && f.id == "save_dir" {
                    std::env::var_os("LOCALAPPDATA")
                        .map(|p| {
                            PathBuf::from(p)
                                .join("FactoryGame/Saved/SaveGames")
                                .to_string_lossy()
                                .into_owned()
                        })
                        .unwrap_or_default()
                } else {
                    f.default.into()
                };
                (f.id.into(), value)
            })
            .collect();
        Self {
            game: game.into(),
            values,
            schema,
            source: String::new(),
        }
    }
    pub fn change(&mut self, key: &str, value: String) {
        if key == "new_config" {
            self.source = value;
        } else if let Some((_, v)) = self.values.iter_mut().find(|(k, _)| k == key) {
            *v = value;
        }
    }
    pub fn review(
        &self,
        data: PathBuf,
        registrations: Vec<Registration>,
        local: bool,
        english: bool,
    ) -> Result<Plan, String> {
        let source = gsm_domain::config::absolute(&self.source)?;
        let docs = crate::catalog::create_configuration(&self.game, &self.values)?;
        let reg = Registration {
            id: Default::default(),
            game_id: self.game.clone().try_into()?,
            source_path: source.clone(),
            source_sha256: String::new(),
        };
        for f in self.schema.iter().filter(|f| {
            matches!(
                f.id,
                "name" | "display_name" | "profile_name" | "new_world" | "new_map"
            )
        }) {
            let value = settings::read(&docs[f.file], f)?;
            if value.trim().is_empty() {
                return Err(format!(
                    "{}: 入力してください / Value required",
                    crate::editor::label(f.id, english)
                ));
            }
        }
        let spec = crate::catalog::resolve(&reg, &docs["manager"])?;
        let save_dir = if self.game == "valheim" || self.game == "satisfactory" {
            Some(gsm_domain::config::Toml::parse(&docs["manager"])?.path("paths", "save_dir")?)
        } else {
            None
        };
        let mut files = vec![];
        for (key, path) in crate::catalog::creation_paths(&self.game, &spec.cwd) {
            files.push((
                path,
                docs.get(key).ok_or("Missing generated document")?.clone(),
            ));
        }
        files.push((source.clone(), docs["manager"].clone()));
        if local {
            LocalBackend::validate_spec(&spec, &data)?;
            if paths::overlap(&source, &spec.cwd) {
                return Err("設定ファイルは新しいサーバー設置先の外に保存してください / Save the manager configuration outside the new installation folder".into());
            }
            ensure_empty(&spec.cwd)?;
            if let Some(path) = &save_dir {
                ensure_empty(path)?;
            }
            for (path, _) in &files {
                paths::validate(path)?;
                if path
                    .try_exists()
                    .map_err(|_| "Cannot inspect configuration path")?
                {
                    return Err("既存ファイルは上書きしません。別の保存名を指定してください / Choose a new file name; existing files are not overwritten".into());
                }
            }
            for other in registrations {
                if other.game_id == reg.game_id {
                    return Err("このゲームは登録済みです。既存の設定を編集してください / This game is already registered".into());
                }
                let source = ConfigSource::read(&other.source_path)?;
                if source.digest() != other.source_sha256 {
                    return Err("他の登録設定が変更されています。先に再登録してください / Another registration has changed".into());
                }
                let other = crate::catalog::resolve(&other, source.text())?;
                LocalBackend::validate_pair(&spec, &other)?;
            }
        }
        let mut summary = format!(
            "{}: {}\n",
            if english {
                "New configuration"
            } else {
                "作成する設定ファイル"
            },
            source.display()
        );
        for (key, value) in &self.values {
            let f = self.schema.iter().find(|f| f.id == key).unwrap();
            let shown = if f.kind == Kind::Secret {
                if value.is_empty() {
                    "—"
                } else {
                    "[非表示 / hidden]"
                }
            } else {
                value
            };
            summary.push_str(&format!(
                "{}: {}\n",
                crate::editor::label(key, english),
                shown
            ));
        }
        for f in self.schema.iter().filter(|f| f.kind == Kind::Secret) {
            let secret = settings::read(&docs[f.file], f)?;
            if !secret.is_empty() {
                summary = summary.replace(&secret, "[非表示]");
            }
        }
        Ok(Plan {
            game: self.game.clone(),
            source,
            documents: docs,
            files,
            summary,
            server_dir: spec.cwd,
            save_dir,
        })
    }
}
pub fn ensure_empty(path: &std::path::Path) -> Result<(), String> {
    paths::validate(path)?;
    if path
        .try_exists()
        .map_err(|_| "Cannot inspect new server folder")?
        && (!path.is_dir()
            || std::fs::read_dir(path)
                .map_err(|_| "Cannot inspect new server folder")?
                .next()
                .is_some())
    {
        return Err(format!(
            "新規作成には空のフォルダーが必要です。既存サーバーは設定ファイルを登録してください / Choose an empty folder for a new server: {}",
            path.display()
        ));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    pub fn filled(game: &str) -> Draft {
        let mut d = Draft::new(game);
        d.source = format!("Z:/GsmNewFixture/{game}/config.toml");
        for (k, v) in &mut d.values {
            let f = d.schema.iter().find(|f| f.id == k).unwrap();
            if f.kind == Kind::Path {
                *v = format!("Z:/GsmNewFixture/{game}/{k}");
            }
            if f.kind == Kind::Secret {
                *v = "new-fixture-credential".into();
            }
        }
        d
    }
    #[test]
    fn every_game_uses_shared_default_and_keeps_individual_overrides() {
        for game in crate::catalog::games() {
            let id = game.id.to_string();
            let common = "C:/Shared SteamCMD/steamcmd.exe";
            let individual = "D:/Dedicated SteamCMD/steamcmd.exe";
            let mut draft = Draft::with_steamcmd(&id, common);
            assert_eq!(
                draft
                    .values
                    .iter()
                    .find(|(k, _)| k == "steamcmd")
                    .unwrap()
                    .1,
                common
            );
            draft.change("steamcmd", individual.into());
            assert_eq!(
                draft
                    .values
                    .iter()
                    .find(|(k, _)| k == "steamcmd")
                    .unwrap()
                    .1,
                individual
            );
            let next = Draft::with_steamcmd(&id, "E:/NewShared/steamcmd.exe");
            assert_eq!(
                next.values.iter().find(|(k, _)| k == "steamcmd").unwrap().1,
                "E:/NewShared/steamcmd.exe"
            );
            assert_eq!(
                draft
                    .values
                    .iter()
                    .find(|(k, _)| k == "steamcmd")
                    .unwrap()
                    .1,
                individual
            );
            // The individual path must survive serialization and reach the runtime plan.
            let mut ready = filled(&id);
            ready.change("steamcmd", individual.into());
            let plan = ready
                .review(PathBuf::from("Z:/AppData"), vec![], false, false)
                .unwrap();
            let reg = Registration {
                id: Default::default(),
                game_id: game.id,
                source_path: plan.source,
                source_sha256: String::new(),
            };
            let spec = crate::catalog::resolve(&reg, &plan.documents["manager"]).unwrap();
            assert_eq!(
                spec.steamcmd.to_string_lossy().replace('\\', "/"),
                individual
            );
        }
    }
    #[test]
    fn every_game_builds_new_settings_without_legacy_files_or_fake_world_ids() {
        for descriptor in crate::catalog::games() {
            let game = descriptor.id.to_string();
            let draft = filled(&game);
            let plan = draft
                .review(PathBuf::from("Z:/AppData"), vec![], false, false)
                .unwrap();
            assert_eq!(plan.files.last().unwrap().0, plan.source);
            assert!(!plan.summary.contains("new-fixture-credential"));
            assert!(!plan.documents["manager"].contains("fixture-only-password"));
            let reg = Registration {
                id: Default::default(),
                game_id: game.clone().try_into().unwrap(),
                source_path: plan.source.clone(),
                source_sha256: String::new(),
            };
            let spec = crate::catalog::resolve(&reg, &plan.documents["manager"]).unwrap();
            assert!(spec.steamcmd.to_string_lossy().contains("GsmNewFixture"));
            if game == "windrose" {
                assert!(!plan.documents.contains_key("ServerDescription.json"));
                assert!(spec.save_targets.iter().any(|t| t.key == "profiles"));
                assert!(matches!(
                    spec.shutdown,
                    gsm_domain::local::Shutdown::Console
                ));
            }
            if game == "satisfactory" {
                assert!(matches!(
                    spec.shutdown,
                    gsm_domain::local::Shutdown::Console
                ));
            }
            if game == "arksa" {
                assert_eq!(plan.files.len(), 2);
                assert!(plan.documents["GameUserSettings.ini"].contains("RCONEnabled=True"));
            }
            if game == "conan" {
                assert_eq!(plan.files.len(), 4);
            }
            if game == "valheim" {
                assert!(
                    spec.arguments
                        .windows(2)
                        .any(|a| a[0] == "-world" && a[1] == "NewWorld")
                );
            }
        }
    }
    #[test]
    fn existing_data_is_never_accepted_as_a_new_installation() {
        let dir = tempfile::tempdir().unwrap();
        ensure_empty(dir.path()).unwrap();
        let path = dir.path().join("world.db");
        std::fs::write(&path, "existing-world").unwrap();
        assert!(ensure_empty(dir.path()).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "existing-world");
    }
    #[cfg(feature = "valheim")]
    #[test]
    fn missing_password_or_invalid_ports_do_not_create_a_plan() {
        let mut draft = filled("valheim");
        draft.change("password", String::new());
        assert!(
            draft
                .review(PathBuf::from("Z:/AppData"), vec![], false, false)
                .is_err()
        );
        draft.change("password", "another-fixture-password".into());
        draft.change("port", "1".into());
        assert!(
            draft
                .review(PathBuf::from("Z:/AppData"), vec![], false, false)
                .is_err()
        );
    }
}
