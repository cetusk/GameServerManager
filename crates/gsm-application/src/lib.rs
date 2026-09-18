//! Registration, selection and per-instance operation ownership.
use gsm_domain::*;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const HISTORY_LIMIT: usize = 200;
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Invalid(String),
    #[error("対象のサーバーが見つかりません")]
    NotFound,
    #[error("このサーバーでは別の操作を実行中です")]
    Busy,
    #[error("現在の状態ではこの操作を実行できません")]
    InvalidState,
    #[error("復元元がこのサーバー／ワールドのバックアップではありません")]
    WrongBackup,
    #[error("設定を保存できませんでした: {0}")]
    Storage(String),
}
#[derive(Clone, Debug, Default)]
struct Runtime {
    info: local::ServerInfo,
    observation: Observation,
    active: Option<OperationId>,
    backups: Vec<Backup>,
    logs: VecDeque<String>,
}
#[derive(Clone, Debug)]
pub struct InstanceView {
    pub info: local::ServerInfo,
    pub instance: Instance,
    pub observation: Observation,
    pub active: Option<OperationId>,
    pub backups: Vec<Backup>,
    pub logs: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub config: AppConfig,
    pub instances: Vec<InstanceView>,
    pub jobs: Vec<Job>,
}
struct State {
    config: AppConfig,
    runtimes: BTreeMap<InstanceId, Runtime>,
    jobs: VecDeque<Job>,
    closing: bool,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}
pub struct Application {
    catalog: Vec<GameDescriptor>,
    backend: Arc<dyn GameBackend>,
    store: Arc<dyn SettingsStore>,
    shared: Arc<Shared>,
    config_gate: Mutex<()>,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl Application {
    /// May read a large file; call from a worker.
    pub fn full_log(&self, id: InstanceId) -> Result<String, String> {
        let snapshot = self.snapshot();
        let view = snapshot
            .instances
            .iter()
            .find(|v| v.instance.id == id)
            .ok_or("Server not found")?;
        if self.is_local() {
            self.backend.full_log(&view.instance)
        } else {
            Ok(view.logs.join("\n"))
        }
    }
    pub fn is_local(&self) -> bool {
        self.backend.is_local()
    }
    /// Startup-only binding: actual identities never inherit a mock instance ID.
    pub fn bind_registered(&self, instances: Vec<Instance>) -> Result<(), AppError> {
        let _config = self.config_gate.lock().unwrap();
        let mut state = self.shared.state.lock().unwrap();
        if !state.jobs.is_empty() {
            return Err(AppError::Busy);
        }
        let mut config = state.config.clone();
        for instance in instances {
            config.instances.retain(|i| i.game_id != instance.game_id);
            config.instances.push(instance);
        }
        self.store.save(&config).map_err(AppError::Storage)?;
        for instance in &config.instances {
            let mut runtime = Runtime::default();
            runtime.observation.process = ProcessState::Unknown;
            state.runtimes.insert(instance.id, runtime);
        }
        state.config = config;
        Ok(())
    }
    /// Called by the monitor worker, never the GUI event thread.
    pub fn refresh_backend(&self) {
        if !self.backend.is_local() {
            return;
        }
        let instances = self.shared.state.lock().unwrap().config.instances.clone();
        for instance in instances {
            let result = self.backend.inspect(&instance);
            let mut state = self.shared.state.lock().unwrap();
            if let Some(runtime) = state.runtimes.get_mut(&instance.id) {
                if runtime.active.is_some() {
                    continue;
                }
                match result {
                    Ok(Some(actual)) => {
                        runtime.info = actual.info;
                        runtime.observation = actual.observation;
                        runtime.backups = actual.backups;
                        runtime.logs = actual.logs.into();
                    }
                    Ok(None) => (),
                    Err(e) => {
                        runtime.observation.process = ProcessState::Unknown;
                        push_log(runtime, e);
                    }
                }
            }
        }
    }
    pub fn open(
        catalog: Vec<GameDescriptor>,
        backend: Arc<dyn GameBackend>,
        store: Arc<dyn SettingsStore>,
    ) -> Result<Self, AppError> {
        let known: BTreeSet<_> = catalog.iter().map(|g| g.id.clone()).collect();
        if known.is_empty() || known.len() != catalog.len() {
            return Err(AppError::Invalid(
                "ゲーム登録が空、または重複しています".into(),
            ));
        }
        let config = store.load().map_err(AppError::Storage)?;
        if config.schema_version != CONFIG_VERSION {
            return Err(AppError::Invalid("未対応の設定バージョンです".into()));
        }
        let mut ids = BTreeSet::new();
        let mut instance_games = BTreeSet::new();
        for instance in &config.instances {
            if !known.contains(&instance.game_id)
                || !ids.insert(instance.id)
                || !instance_games.insert(instance.game_id.clone())
            {
                return Err(AppError::Invalid("設定に未搭載ゲーム／重複インスタンスがあります。必要なゲーム機能を有効にして起動してください".into()));
            }
        }
        let selected: BTreeSet<_> = config.enabled_games.iter().cloned().collect();
        if selected.len() != config.enabled_games.len() || !selected.is_subset(&instance_games) {
            return Err(AppError::Invalid(
                "使用ゲームと登録インスタンスが一致していません".into(),
            ));
        }
        let runtimes = config
            .instances
            .iter()
            .map(|i| (i.id, Runtime::default()))
            .collect();
        Ok(Self {
            catalog,
            config_gate: Mutex::new(()),
            backend,
            store,
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    config,
                    runtimes,
                    jobs: VecDeque::new(),
                    closing: false,
                }),
                changed: Condvar::new(),
            }),
        })
    }
    pub fn catalog(&self) -> &[GameDescriptor] {
        &self.catalog
    }
    /// Saving succeeds before the visible selection changes. Unselected instances remain intact.
    pub fn select_games(&self, games: BTreeSet<GameId>) -> Result<(), AppError> {
        if games.is_empty()
            || games
                .iter()
                .any(|id| !self.catalog.iter().any(|g| &g.id == id))
        {
            return Err(AppError::Invalid("使用するゲームを選択してください".into()));
        }
        // Serialize configuration writers, but let snapshots/jobs proceed during disk IO.
        let _config = self.config_gate.lock().unwrap();
        let mut config = {
            let state = self.shared.state.lock().unwrap();
            if state.closing {
                return Err(AppError::Invalid("終了処理中です".into()));
            }
            state.config.clone()
        };
        config.enabled_games = games.iter().cloned().collect();
        for game_id in games {
            if !config.instances.iter().any(|i| i.game_id == game_id) {
                let game = self.catalog.iter().find(|g| g.id == game_id).unwrap();
                config.instances.push(Instance {
                    id: InstanceId::new(),
                    game_id,
                    name: format!(
                        "{} / {}",
                        game.name,
                        if self.backend.is_local() {
                            "未登録"
                        } else {
                            "検証用"
                        }
                    ),
                    world: game.sample_world.into(),
                });
            }
        }
        self.store.save(&config).map_err(AppError::Storage)?;
        let mut state = self.shared.state.lock().unwrap();
        for instance in &config.instances {
            state.runtimes.entry(instance.id).or_insert_with(|| {
                let mut runtime = Runtime::default();
                if self.backend.is_local() {
                    runtime.observation.process = ProcessState::Unknown;
                }
                runtime
            });
        }
        state.config = config;
        Ok(())
    }
    pub fn snapshot(&self) -> Snapshot {
        let state = self.shared.state.lock().unwrap();
        Snapshot {
            config: state.config.clone(),
            jobs: state.jobs.iter().cloned().collect(),
            instances: state
                .config
                .instances
                .iter()
                .map(|instance| {
                    let runtime = &state.runtimes[&instance.id];
                    InstanceView {
                        info: runtime.info.clone(),
                        instance: instance.clone(),
                        observation: runtime.observation,
                        active: runtime.active,
                        backups: runtime.backups.clone(),
                        logs: runtime.logs.iter().cloned().collect(),
                    }
                })
                .collect(),
        }
    }
    /// The request captures an immutable instance ID before leaving the UI thread.
    pub fn submit(
        &self,
        instance_id: InstanceId,
        command: Command,
    ) -> Result<OperationId, AppError> {
        let request = {
            let mut state = self.shared.state.lock().unwrap();
            if state.closing {
                return Err(AppError::Invalid("終了処理中です".into()));
            }
            let instance = state
                .config
                .instances
                .iter()
                .find(|i| i.id == instance_id)
                .cloned()
                .ok_or(AppError::NotFound)?;
            if !state.config.enabled_games.contains(&instance.game_id) {
                return Err(AppError::Invalid(
                    "使用ゲームとして選択されていません".into(),
                ));
            }
            let runtime = state.runtimes.get_mut(&instance_id).unwrap();
            if runtime.active.is_some() {
                return Err(AppError::Busy);
            }
            match (&command, runtime.observation.process) {
                (Command::Start, ProcessState::Absent)
                | (Command::Stop, ProcessState::Alive)
                | (
                    Command::Backup
                    | Command::Restore(_)
                    | Command::Update
                    | Command::Recover
                    | Command::EditSettings
                    | Command::WriteSettings(_),
                    ProcessState::Absent,
                ) => (),
                _ => return Err(AppError::InvalidState),
            }
            if let Command::Restore(id) = command
                && !runtime.backups.iter().any(|b| {
                    b.id == id && b.instance_id == instance_id && b.world == instance.world
                })
            {
                return Err(AppError::WrongBackup);
            }
            let id = OperationId::new();
            let request = OperationRequest {
                id,
                instance: instance.clone(),
                command: command.clone(),
                before: runtime.observation,
            };
            runtime.active = Some(id);
            push_log(runtime, format!("{} を開始 / 操作 {}", command.label(), id));
            while state.jobs.len() >= HISTORY_LIMIT {
                if let Some(index) = state
                    .jobs
                    .iter()
                    .position(|j| j.status != JobStatus::Running)
                {
                    state.jobs.remove(index);
                } else {
                    break;
                }
            }
            state.jobs.push_back(Job {
                id,
                instance,
                command,
                status: JobStatus::Running,
                started_at: now(),
            });
            request
        };
        let shared = self.shared.clone();
        let backend = self.backend.clone();
        let worker_request = request.clone();
        if let Err(error) = std::thread::Builder::new()
            .name(format!("gsm-job-{}", request.id))
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    backend.execute(&worker_request)
                }))
                .unwrap_or_else(|_| Err("操作ワーカーが異常終了しました".into()));
                let actual = backend.inspect(&worker_request.instance).ok().flatten();
                finish(&shared, &worker_request, result, actual, backend.is_local());
            })
        {
            finish(
                &self.shared,
                &request,
                Err(format!("操作ワーカーを開始できません: {error}")),
                None,
                self.backend.is_local(),
            );
        }
        Ok(request.id)
    }
    pub fn wait_for_idle(&self, timeout: Duration) -> bool {
        let state = self.shared.state.lock().unwrap();
        let (state, _) = self
            .shared
            .changed
            .wait_timeout_while(state, timeout, |s| {
                s.runtimes.values().any(|r| r.active.is_some())
            })
            .unwrap();
        state.runtimes.values().all(|r| r.active.is_none())
    }
    pub fn shutdown(&self) {
        let mut state = self.shared.state.lock().unwrap();
        state.closing = true;
        while state.runtimes.values().any(|r| r.active.is_some()) {
            state = self.shared.changed.wait(state).unwrap();
        }
    }
}
fn push_log(runtime: &mut Runtime, message: String) {
    runtime.logs.push_back(message);
    while runtime.logs.len() > HISTORY_LIMIT {
        runtime.logs.pop_front();
    }
}
fn finish(
    shared: &Shared,
    request: &OperationRequest,
    result: Result<Observation, String>,
    actual: Option<local::BackendState>,
    is_local: bool,
) {
    let mut state = shared.state.lock().unwrap();
    let runtime = state.runtimes.get_mut(&request.instance.id).unwrap();
    if runtime.active != Some(request.id) {
        return;
    }
    let status = match result {
        Ok(observation) => {
            runtime.observation = observation;
            if request.command == Command::Backup && !is_local {
                runtime.backups.push(Backup {
                    id: BackupId::new(),
                    instance_id: request.instance.id,
                    world: request.instance.world.clone(),
                    created_at: now(),
                });
            }
            push_log(
                runtime,
                format!("{} が完了 / 操作 {}", request.command.label(), request.id),
            );
            JobStatus::Completed
        }
        Err(error) => {
            if is_local {
                runtime.observation.process = ProcessState::Unknown;
            }
            push_log(
                runtime,
                format!(
                    "{} が失敗: {} / 操作 {}",
                    request.command.label(),
                    error,
                    request.id
                ),
            );
            JobStatus::Failed(error)
        }
    };
    runtime.active = None;
    if let Some(actual) = actual {
        runtime.info = actual.info;
        runtime.observation = actual.observation;
        runtime.backups = actual.backups;
        runtime.logs = actual.logs.into();
    }
    if let Some(job) = state.jobs.iter_mut().find(|j| j.id == request.id) {
        job.status = status;
    }
    shared.changed.notify_all();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    #[derive(Default)]
    struct Store {
        value: Mutex<AppConfig>,
        fail: AtomicBool,
    }
    impl SettingsStore for Store {
        fn load(&self) -> Result<AppConfig, String> {
            Ok(self.value.lock().unwrap().clone())
        }
        fn save(&self, config: &AppConfig) -> Result<(), String> {
            if self.fail.load(Ordering::SeqCst) {
                return Err("disk unavailable".into());
            }
            *self.value.lock().unwrap() = config.clone();
            Ok(())
        }
    }
    fn catalog() -> Vec<GameDescriptor> {
        ["alpha", "beta"]
            .map(|id| GameDescriptor {
                id: id.to_owned().try_into().unwrap(),
                name: id,
                description: "fixture",
                sample_world: "World",
                backup_scope: "mock",
            })
            .into()
    }
    fn ids(keys: &[&str]) -> BTreeSet<GameId> {
        keys.iter()
            .map(|k| k.to_string().try_into().unwrap())
            .collect()
    }
    fn app(backend: Arc<dyn GameBackend>) -> Application {
        let app = Application::open(catalog(), backend, Arc::new(Store::default())).unwrap();
        app.select_games(ids(&["alpha", "beta"])).unwrap();
        app
    }
    fn run(app: &Application, id: InstanceId, command: Command) {
        app.submit(id, command).unwrap();
        assert!(app.wait_for_idle(Duration::from_secs(3)));
    }
    #[test]
    fn slow_selection_save_does_not_block_snapshots_or_job_completion() {
        use std::sync::mpsc;
        struct SlowStore {
            entered: mpsc::Sender<()>,
            release: Mutex<mpsc::Receiver<()>>,
            slow: AtomicBool,
        }
        impl SettingsStore for SlowStore {
            fn load(&self) -> Result<AppConfig, String> {
                Ok(AppConfig::default())
            }
            fn save(&self, _: &AppConfig) -> Result<(), String> {
                if self.slow.load(Ordering::SeqCst) {
                    self.entered.send(()).unwrap();
                    self.release
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap();
                }
                Ok(())
            }
        }
        let (entered, started) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let store = Arc::new(SlowStore {
            entered,
            release: Mutex::new(wait),
            slow: AtomicBool::new(false),
        });
        let app = Arc::new(
            Application::open(
                catalog(),
                Arc::new(gsm_mock::MockBackend::new(Duration::ZERO)),
                store.clone(),
            )
            .unwrap(),
        );
        app.select_games(ids(&["alpha", "beta"])).unwrap();
        let instance = app.snapshot().instances[0].instance.id;
        store.slow.store(true, Ordering::SeqCst);
        let writer_app = app.clone();
        let writer = std::thread::spawn(move || writer_app.select_games(ids(&["beta"])));
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let (reply, snapshot) = mpsc::channel();
        let reader_app = app.clone();
        let reader = std::thread::spawn(move || {
            let old_selection = reader_app.snapshot().config.enabled_games.len();
            reader_app.submit(instance, Command::Start).unwrap();
            let idle = reader_app.wait_for_idle(Duration::from_secs(1));
            reply.send((old_selection, idle)).unwrap();
        });
        let result = snapshot.recv_timeout(Duration::from_secs(2));
        // Always release IO before asserting so a failure cannot strand threads.
        release.send(()).unwrap();
        writer.join().unwrap().unwrap();
        reader.join().unwrap();
        assert_eq!(result.unwrap(), (2, true));
        assert_eq!(
            app.snapshot().config.enabled_games,
            ids(&["beta"]).into_iter().collect::<Vec<_>>()
        );
        assert_eq!(
            app.snapshot().instances[0].observation.process,
            ProcessState::Alive
        );
    }

    #[test]
    fn switching_selection_never_redirects_an_inflight_operation() {
        let app = app(Arc::new(gsm_mock::MockBackend::new(Duration::from_millis(
            100,
        ))));
        let initial = app.snapshot();
        let a = initial.instances[0].instance.id;
        let b = initial.instances[1].instance.id;
        let operation = app.submit(a, Command::Start).unwrap();
        assert!(matches!(
            app.submit(a, Command::Backup),
            Err(AppError::Busy)
        ));
        app.select_games(ids(&["beta"])).unwrap();
        run(&app, b, Command::Backup);
        let state = app.snapshot();
        assert_eq!(state.instances[0].observation.process, ProcessState::Alive);
        assert_eq!(state.instances[1].observation.process, ProcessState::Absent);
        assert!(
            state.instances[1]
                .logs
                .iter()
                .all(|l| !l.contains(&operation.to_string()))
        );
        assert_eq!(
            state
                .jobs
                .iter()
                .find(|j| j.id == operation)
                .unwrap()
                .instance
                .id,
            a
        );
        app.select_games(ids(&["alpha"])).unwrap();
        assert_eq!(app.snapshot().instances[0].instance.id, a);
    }
    #[test]
    fn restore_requires_stopped_state_and_matching_instance_backup() {
        let app = app(Arc::new(gsm_mock::MockBackend::new(Duration::ZERO)));
        let snapshot = app.snapshot();
        let a = snapshot.instances[0].instance.id;
        let b = snapshot.instances[1].instance.id;
        run(&app, a, Command::Backup);
        let backup = app.snapshot().instances[0].backups[0].id;
        assert!(matches!(
            app.submit(b, Command::Restore(backup)),
            Err(AppError::WrongBackup)
        ));
        run(&app, a, Command::Start);
        assert!(matches!(
            app.submit(a, Command::Restore(backup)),
            Err(AppError::InvalidState)
        ));
        run(&app, a, Command::Stop);
        run(&app, a, Command::Restore(backup));
    }
    #[test]
    fn failed_save_does_not_change_selection_or_registration() {
        let store = Arc::new(Store::default());
        let app = Application::open(
            catalog(),
            Arc::new(gsm_mock::MockBackend::default()),
            store.clone(),
        )
        .unwrap();
        app.select_games(ids(&["alpha"])).unwrap();
        let before = app.snapshot().config;
        store.fail.store(true, Ordering::SeqCst);
        assert!(app.select_games(ids(&["beta"])).is_err());
        assert_eq!(app.snapshot().config, before);
    }
    struct Failing;
    impl GameBackend for Failing {
        fn execute(&self, _: &OperationRequest) -> Result<Observation, String> {
            Err("injected failure".into())
        }
    }
    #[test]
    fn failure_is_not_treated_as_process_absence_and_releases_operation_slot() {
        let app = app(Arc::new(Failing));
        let id = app.snapshot().instances[0].instance.id;
        // Seed an observed running server, then fail its stop operation.
        app.shared
            .state
            .lock()
            .unwrap()
            .runtimes
            .get_mut(&id)
            .unwrap()
            .observation
            .process = ProcessState::Alive;
        run(&app, id, Command::Stop);
        let snapshot = app.snapshot();
        assert_eq!(
            snapshot.instances[0].observation.process,
            ProcessState::Alive
        );
        assert!(snapshot.instances[0].active.is_none());
        assert!(matches!(snapshot.jobs[0].status, JobStatus::Failed(_)));
        assert!(matches!(
            app.submit(id, Command::Backup),
            Err(AppError::InvalidState)
        ));
    }
    #[test]
    fn unknown_process_state_blocks_destructive_operations() {
        let app = app(Arc::new(gsm_mock::MockBackend::default()));
        let id = app.snapshot().instances[0].instance.id;
        app.shared
            .state
            .lock()
            .unwrap()
            .runtimes
            .get_mut(&id)
            .unwrap()
            .observation
            .process = ProcessState::Unknown;
        assert!(matches!(
            app.submit(id, Command::Backup),
            Err(AppError::InvalidState)
        ));
        assert!(matches!(
            app.submit(
                id,
                Command::WriteSettings(Arc::new(SettingsWrite {
                    additional: vec![],
                    expected_sha256: "fixture".into(),
                    contents: "secret".into()
                }))
            ),
            Err(AppError::InvalidState)
        ));
        assert!(matches!(
            app.submit(id, Command::Start),
            Err(AppError::InvalidState)
        ));
    }
    #[test]
    fn registration_ids_survive_restart_without_persisting_mock_processes() {
        let store = Arc::new(Store::default());
        let app = Application::open(
            catalog(),
            Arc::new(gsm_mock::MockBackend::new(Duration::ZERO)),
            store.clone(),
        )
        .unwrap();
        app.select_games(ids(&["alpha"])).unwrap();
        let id = app.snapshot().instances[0].instance.id;
        run(&app, id, Command::Start);
        app.shutdown();
        let next = Application::open(catalog(), Arc::new(gsm_mock::MockBackend::default()), store)
            .unwrap();
        assert_eq!(next.snapshot().instances[0].instance.id, id);
        assert_eq!(
            next.snapshot().instances[0].observation.process,
            ProcessState::Absent
        );
    }
}

#[cfg(test)]
mod local_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    #[derive(Default)]
    struct Store(Mutex<AppConfig>);
    impl SettingsStore for Store {
        fn load(&self) -> Result<AppConfig, String> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn save(&self, c: &AppConfig) -> Result<(), String> {
            *self.0.lock().unwrap() = c.clone();
            Ok(())
        }
    }
    struct Local {
        busy_inspection: AtomicBool,
    }
    impl GameBackend for Local {
        fn is_local(&self) -> bool {
            true
        }
        fn execute(&self, _: &OperationRequest) -> Result<Observation, String> {
            self.busy_inspection.store(true, Ordering::SeqCst);
            Ok(Observation::default())
        }
        fn inspect(&self, _: &Instance) -> Result<Option<local::BackendState>, String> {
            if self.busy_inspection.load(Ordering::SeqCst) {
                Ok(None)
            } else {
                Ok(Some(local::BackendState {
                    info: local::ServerInfo::default(),
                    observation: Observation::default(),
                    backups: vec![],
                    logs: vec![],
                }))
            }
        }
    }
    #[test]
    fn local_backup_never_creates_a_synthetic_record_when_monitor_is_busy() {
        let game = GameId::try_from("fixture".to_string()).unwrap();
        let backend = Arc::new(Local {
            busy_inspection: AtomicBool::new(false),
        });
        let app = Application::open(
            vec![GameDescriptor {
                id: game.clone(),
                name: "fixture",
                description: "fixture",
                sample_world: "world",
                backup_scope: "fixture",
            }],
            backend,
            Arc::new(Store::default()),
        )
        .unwrap();
        app.select_games([game.clone()].into()).unwrap();
        let instance = Instance {
            id: Default::default(),
            game_id: game,
            name: "registered".into(),
            world: "world".into(),
        };
        app.bind_registered(vec![instance.clone()]).unwrap();
        assert_eq!(
            app.snapshot().instances[0].observation.process,
            ProcessState::Unknown
        );
        assert!(app.submit(instance.id, Command::Backup).is_err());
        app.refresh_backend();
        app.submit(instance.id, Command::Backup).unwrap();
        assert!(app.wait_for_idle(Duration::from_secs(2)));
        let snapshot = app.snapshot();
        assert!(snapshot.instances[0].backups.is_empty());
        assert_eq!(snapshot.instances[0].instance.id, instance.id);
        assert_eq!(snapshot.jobs[0].status, JobStatus::Completed);
    }
}
