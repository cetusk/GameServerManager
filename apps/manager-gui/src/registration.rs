//! UI-independent review/commit controller. IO always runs outside the UI thread.
use gsm_domain::{GameId, JobStatus, Registration};
use gsm_infra::registration::{ConfigSource, RegistrationStore};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
};

struct Reviewed {
    game_id: GameId,
    input: String,
    source: ConfigSource,
    preview: String,
}
enum Completed {
    Reviewed(Result<Reviewed, String>),
    Registered(Result<Registration, String>),
}
pub struct RegistrationPanel {
    store: Arc<RegistrationStore>,
    saved: Vec<Registration>,
    reviewed: Option<Reviewed>,
    receiver: Option<Receiver<Completed>>,
    worker: Option<std::thread::JoinHandle<()>>,
    pub report: String,
    pub reload_ready: bool,
    pub activity: Option<(String, &'static str, JobStatus)>,
}
impl RegistrationPanel {
    pub fn new(store: Arc<RegistrationStore>) -> Self {
        Self {
            saved: store.list(),
            store,
            reload_ready: false,
            reviewed: None,
            receiver: None,
            worker: None,
            activity: None,
            report: "「参照…」で旧管理設定を選ぶか、絶対パスを入力してください。ARK は Profile/*.ini、他のゲームは config.toml です。".into(),
        }
    }
    pub fn busy(&self) -> bool {
        self.receiver.is_some()
    }
    pub fn can_register(&self) -> bool {
        !self.busy() && self.reviewed.is_some()
    }
    pub fn can_register_for(&self, game: &str, input: &str) -> bool {
        self.can_register()
            && self
                .reviewed
                .as_ref()
                .is_some_and(|r| r.game_id.as_ref() == game && r.input == input)
    }
    pub fn saved(&self, game: &str) -> Option<Registration> {
        self.saved
            .iter()
            .find(|r| r.game_id.as_ref() == game)
            .cloned()
    }
    pub fn invalidate(&mut self) {
        if !self.busy() {
            self.activity = None;
            self.reviewed = None;
            self.report = "パスを変更しました。再検証してください。".into();
        }
    }
    pub fn review(&mut self, game: &str, input: String) {
        if self.busy() {
            return;
        }
        self.reviewed = None;
        self.report = "設定を検証中…".into();
        self.activity = Some((game.into(), "設定の検証", JobStatus::Running));
        let game = game.to_owned();
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("gsm-config-review".into())
            .spawn(move || {
                let result = (|| {
                    let game_id = GameId::try_from(game.clone())?;
                    let source = ConfigSource::read(&PathBuf::from(&input))?;
                    let preview = review_source(&game, &source)?;
                    Ok(Reviewed {
                        game_id,
                        input,
                        source,
                        preview,
                    })
                })();
                let _ = sender.send(Completed::Reviewed(result));
            }) {
            Ok(worker) => {
                self.receiver = Some(receiver);
                self.worker = Some(worker);
            }
            Err(_) => {
                self.report = "検証ワーカーを開始できませんでした。".into();
                self.finish_activity(JobStatus::Failed(self.report.clone()));
            }
        }
    }
    pub fn register(&mut self, game: &str, input: &str) {
        if self.busy() {
            return;
        }
        let Some(reviewed) = self.reviewed.take() else {
            return;
        };
        if reviewed.game_id.as_ref() != game || reviewed.input != input {
            self.report = "検証時の対象と一致しません。再検証してください。".into();
            return;
        }
        self.report = "設定の変更有無を確認して登録中…".into();
        self.activity = Some((game.into(), "設定の登録", JobStatus::Running));
        let store = self.store.clone();
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("gsm-config-register".into())
            .spawn(move || {
                let result = store.register(reviewed.game_id, &reviewed.source);
                let _ = sender.send(Completed::Registered(result));
            }) {
            Ok(worker) => {
                self.receiver = Some(receiver);
                self.worker = Some(worker);
            }
            Err(_) => {
                self.report = "登録ワーカーを開始できませんでした。再検証してください。".into();
                self.finish_activity(JobStatus::Failed(self.report.clone()));
            }
        }
    }
    /// An already approved GUI edit must still match exactly before re-registering.
    pub fn register_applied(&mut self, game: &str, expected: String) {
        if self.busy() {
            return;
        }
        let Some(registration) = self.saved(game) else {
            return;
        };
        let store = self.store.clone();
        let (sender, receiver) = mpsc::channel();
        self.activity = Some((game.into(), "設定の登録", JobStatus::Running));
        match std::thread::Builder::new()
            .name("gsm-register-edit".into())
            .spawn(move || {
                let result = (|| {
                    let source = ConfigSource::read(&registration.source_path)?;
                    if source.text() != expected {
                        return Err(
                            "保存後に設定が変わりました。確認して登録し直してください".into()
                        );
                    }
                    review_source(registration.game_id.as_ref(), &source)?;
                    store.register(registration.game_id, &source)
                })();
                let _ = sender.send(Completed::Registered(result));
            }) {
            Ok(worker) => {
                self.receiver = Some(receiver);
                self.worker = Some(worker);
            }
            Err(_) => {
                self.report = "登録処理を開始できません".into();
                self.finish_activity(JobStatus::Failed(self.report.clone()));
            }
        }
    }
    pub fn create(&mut self, plan: crate::creation::Plan, data: PathBuf) {
        if self.busy() || self.saved(&plan.game).is_some() {
            return;
        }
        self.report =
            "設定ファイルを新規作成して登録中… / Creating and registering configuration…".into();
        self.activity = Some((plan.game.clone(), "設定の登録", JobStatus::Running));
        let store = self.store.clone();
        let (tx, rx) = mpsc::channel();
        match std::thread::Builder::new()
            .name("gsm-create-server".into())
            .spawn(move || {
                let result = (|| {
                    crate::creation::ensure_empty(&plan.server_dir)?;
                    if let Some(path) = &plan.save_dir {
                        crate::creation::ensure_empty(path)?;
                    }
                    // Recheck the registry and other resources after the confirmation dialog.
                    let reg = Registration {
                        id: Default::default(),
                        game_id: plan.game.clone().try_into()?,
                        source_path: plan.source.clone(),
                        source_sha256: String::new(),
                    };
                    let spec = crate::catalog::resolve(&reg, &plan.documents["manager"])?;
                    gsm_infra::local::LocalBackend::validate_spec(&spec, &data)?;
                    for other in store.list() {
                        if other.game_id == reg.game_id {
                            return Err("This game was registered while creating settings".into());
                        }
                        let text = ConfigSource::read(&other.source_path)?;
                        gsm_infra::local::LocalBackend::validate_pair(
                            &spec,
                            &crate::catalog::resolve(&other, text.text())?,
                        )?;
                    }
                    gsm_infra::registration::create_files(&plan.files)?;
                    for (path,expected) in &plan.files {if ConfigSource::read(path)?.text()!=expected {return Err(format!("作成後に設定が変更されています。登録前に再確認してください / Settings changed after creation: {}",path.display()));}}
                    let source = ConfigSource::read(&plan.source)?;
                    // Keep successfully created files if registration itself fails, so the
                    // user can register that exact file again without re-entering secrets.
                    review_source(&plan.game, &source).map_err(|e| {
                        format!(
                            "{e}\n作成した設定 / Created configuration: {}",
                            plan.source.display()
                        )
                    })?;
                    store.register(reg.game_id, &source).map_err(|e| {
                        format!(
                            "{e}\n作成した設定 / Created configuration: {}",
                            plan.source.display()
                        )
                    })
                })();
                let _ = tx.send(Completed::Registered(result));
            }) {
            Ok(worker) => {
                self.worker = Some(worker);
                self.receiver = Some(rx);
            }
            Err(_) => {
                self.report = "作成処理を開始できません / Cannot start creation worker".into();
                self.finish_activity(JobStatus::Failed(self.report.clone()));
            }
        }
    }
    pub fn poll(&mut self) {
        let Some(receiver) = &self.receiver else {
            return;
        };
        let completed = match receiver.try_recv() {
            Ok(completed) => completed,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.receiver = None;
                self.reviewed = None;
                self.report = "処理が中断されました。登録状況を確認して再検証してください。".into();
                self.finish_activity(JobStatus::Failed(self.report.clone()));
                return;
            }
        };
        self.receiver = None;
        self.finish_activity(JobStatus::Completed);
        match completed {
            Completed::Reviewed(Ok(reviewed)) => {
                self.report = reviewed.preview.clone();
                self.reviewed = Some(reviewed);
            }
            Completed::Registered(Ok(registration)) => {
                self.saved.retain(|r| r.game_id != registration.game_id);
                self.saved.push(registration.clone());
                self.reload_ready = true;
                self.report = format!(
                    "登録しました。\n登録 ID: {}\n管理画面を自動で開き直して反映します。",
                    registration.id
                );
            }
            Completed::Reviewed(Err(error)) | Completed::Registered(Err(error)) => {
                self.report = error;
                self.finish_activity(JobStatus::Failed(self.report.clone()));
            }
        }
    }
    fn finish_activity(&mut self, status: JobStatus) {
        if let Some((_, _, phase)) = &mut self.activity {
            *phase = status;
        }
    }
    /// Finish a requested registration before normal application exit.
    pub fn shutdown(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.poll();
    }
}

fn review_source(game: &str, source: &ConfigSource) -> Result<String, String> {
    let registration = Registration {
        id: Default::default(),
        game_id: game.to_owned().try_into()?,
        source_path: source.path().to_owned(),
        source_sha256: source.digest().into(),
    };
    let plan = crate::catalog::resolve(&registration, source.text())?;
    let shutdown = match plan.shutdown {
        gsm_domain::local::Shutdown::Console => "コンソール正常停止",
        gsm_domain::local::Shutdown::Rcon { .. } => "ローカル RCON",
        gsm_domain::local::Shutdown::Https { .. } => "ローカル HTTPS API",
    };
    Ok(plan.redact(&format!("ゲーム: {game}\nサーバー: {}\nワールド: {}\n実行ファイル: {}\n停止方式: {shutdown}\nSteam App ID: {}\nバックアップ先: {}\n\n保存・復元する範囲:\n{}\n\n設定形式を検証しました。プロセス稼働・ポート・実ゲームの読み込みは起動時と実機で確認します。「登録して反映」で管理画面に反映します。",
        plan.instance.name,plan.instance.world,plan.executable.display(),plan.steam_app_id,plan.backup_dir.display(),plan.save_targets.iter().map(|t|format!("{}: {}",t.key,t.path.display())).collect::<Vec<_>>().join("\n"))))
}

#[cfg(all(test, feature = "valheim"))]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{Duration, Instant},
    };
    fn wait(panel: &mut RegistrationPanel) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while panel.busy() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
            panel.poll();
        }
    }
    #[test]
    fn review_requires_same_target_and_unchanged_content_before_registration() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.toml");
        let fixture =
            include_str!("../../../crates/games/valheim/tests/fixtures/legacy-config.toml");
        fs::write(&path, fixture).unwrap();
        let input = path.to_str().unwrap().to_owned();
        let store = Arc::new(RegistrationStore::open(&root.path().join("manager")).unwrap());
        let mut panel = RegistrationPanel::new(store.clone());
        panel.review("valheim", input.clone());
        wait(&mut panel);
        assert!(panel.can_register());
        assert!(!panel.report.contains("fixture-only-password"));
        panel.register("windrose", &input);
        assert!(store.list().is_empty());
        assert!(!panel.can_register());
        panel.review("valheim", input.clone());
        wait(&mut panel);
        fs::write(&path, format!("{fixture}\n# changed after review")).unwrap();
        panel.register("valheim", &input);
        wait(&mut panel);
        assert!(store.list().is_empty());
        assert!(panel.report.contains("再検証"));
        panel.review("valheim", input.clone());
        wait(&mut panel);
        panel.register("valheim", &input);
        wait(&mut panel);
        assert_eq!(store.list().len(), 1);
        let id = store.list()[0].id;
        panel.review("valheim", input.clone());
        wait(&mut panel);
        panel.register("valheim", &input);
        wait(&mut panel);
        assert_eq!(store.list()[0].id, id);
    }
    #[test]
    fn invalid_config_never_produces_a_committable_review_or_echoes_password() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.toml");
        fs::write(&path, "password = [secret-password").unwrap();
        let mut panel =
            RegistrationPanel::new(Arc::new(RegistrationStore::open(root.path()).unwrap()));
        panel.review("valheim", path.to_str().unwrap().into());
        wait(&mut panel);
        assert!(!panel.can_register());
        assert!(!panel.report.contains("secret-password"));
    }
}

#[cfg(all(
    test,
    feature = "arksa",
    feature = "valheim",
    feature = "windrose",
    feature = "satisfactory",
    feature = "conan"
))]
mod all_game_tests {
    use super::*;
    use std::{
        fs,
        path::Path,
        time::{Duration, Instant},
    };
    fn put(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fn wait(panel: &mut RegistrationPanel) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while panel.busy() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
            panel.poll();
        }
    }
    #[test]
    fn all_five_legacy_sources_register_without_copying_credentials_or_changing_files() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(RegistrationStore::open(&temp.path().join("manager")).unwrap());
        let mut panel = RegistrationPanel::new(store.clone());
        for game in ["arksa", "valheim", "windrose", "satisfactory", "conan"] {
            let base = temp.path().join(game);
            let server = base.join("server");
            let source = base.join(if game == "arksa" {
                "profile.ini"
            } else {
                "config.toml"
            });
            let paths = format!(
                "[paths]\nserver_dir='{}'\nsteamcmd='{}'\nbackup_dir='{}'\n",
                server.display(),
                base.join("steamcmd.exe").display(),
                base.join("backup").display()
            );
            let text = match game {
                "valheim" => {
                    include_str!("../../../crates/games/valheim/tests/fixtures/legacy-config.toml")
                        .to_string()
                }
                "conan" => paths,
                "satisfactory" => format!(
                    "{paths}save_dir='{}'\nlog_file='{}'\n[server]\nname='Fixture'\nadmin_password='never-persist-this-secret'\n",
                    base.join("saves").display(),
                    server.join("game.log").display()
                ),
                "windrose" => {
                    put(
                        &server.join("R5/ServerDescription.json"),
                        r#"{"ServerName":"Fixture","WorldIslandId":"ABC","Password":"never-persist-this-secret"}"#,
                    );
                    put(&server.join("R5/Saved/SaveProfiles/Default/RocksDB_v2/0.10.0/Worlds/ABC/WorldDescription.json"),r#"{"WorldDescription":{"islandId":"ABC"}}"#);
                    paths
                }
                "arksa" => format!(
                    "[General]\nEdit_Install_Location_Val={}\nCB_MapName_Text=TheIsland_WP\nMM_Command_Val=ArkAscendedServer.exe TheIsland_WP?listen?Port=7777?QueryPort=27015 -log\n[Server]\nSE_Port=7777\nSE_QueryPort=27015\nSE_RCONPort=27020\nCB_RCONEnabled=1\nEdit_ServerAdminPassword=never-persist-this-secret\n[Integration]\nSteamCMD={}\nBackupDir={}\n",
                    server.display(),
                    base.join("steamcmd.exe").display(),
                    base.join("backup").display()
                ),
                _ => unreachable!(),
            };
            put(&source, &text);
            let input = source.to_string_lossy().to_string();
            panel.review(game, input.clone());
            wait(&mut panel);
            assert!(panel.can_register(), "{game}: {}", panel.report);
            assert!(!panel.report.contains("never-persist-this-secret"));
            panel.register(game, &input);
            wait(&mut panel);
            assert!(
                store.list().iter().any(|r| r.game_id.as_ref() == game),
                "{game}: {}",
                panel.report
            );
            assert_eq!(fs::read_to_string(source).unwrap(), text);
        }
        assert_eq!(store.list().len(), 5);
        let saved =
            fs::read_to_string(temp.path().join("manager/local-registrations.json")).unwrap();
        assert!(!saved.contains("never-persist-this-secret"));
        assert!(!saved.contains("fixture-only-password"));
    }
}
#[cfg(all(test, feature = "conan"))]
mod creation_tests {
    use super::*;
    #[test]
    fn fresh_conan_configuration_registers_without_a_legacy_source() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("app");
        let mut draft = crate::creation::Draft::new("conan");
        draft.source = temp
            .path()
            .join("config.toml")
            .to_string_lossy()
            .into_owned();
        for (key, part) in [
            ("steamcmd", "steamcmd.exe"),
            ("server_dir", "server"),
            ("backup_dir", "backup"),
        ] {
            draft.change(key, temp.path().join(part).to_string_lossy().into_owned());
        }
        draft.change("name", "Fresh Conan".into());
        draft.change("admin_password", "creation-test-secret".into());
        let plan = draft.review(data.clone(), vec![], true, false).unwrap();
        let source = plan.source.clone();
        assert!(!source.exists());
        let store = Arc::new(RegistrationStore::open(&data).unwrap());
        let mut panel = RegistrationPanel::new(store.clone());
        panel.create(plan, data);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while panel.busy() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
            panel.poll();
        }
        assert!(panel.reload_ready, "{}", panel.report);
        assert_eq!(store.list().len(), 1);
        let reg = store.list().remove(0);
        let source = ConfigSource::read(&source).unwrap();
        let spec = crate::catalog::resolve(&reg, source.text()).unwrap();
        assert_eq!(spec.instance.name, "Fresh Conan");
        assert!(!panel.report.contains("creation-test-secret"));
    }
}
