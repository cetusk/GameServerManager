mod diagnostics;
mod installation;
mod logs;
mod network;
pub mod paths;
pub mod process;
pub mod settings;
pub mod snapshots;
mod updater;
use crate::registration::ConfigSource;
use fs2::FileExt;
use gsm_domain::{
    local::{BackendState, LocalServer, Shutdown},
    *,
};
use paths::*;
use std::{
    collections::{BTreeMap, VecDeque},
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

struct Entry {
    diagnostics: Mutex<diagnostics::Watch>,
    registration: Registration,
    spec: LocalServer,
    state: PathBuf,
    operation: Mutex<()>,
    identity: Mutex<Option<process::Identity>>,
    events: Mutex<VecDeque<String>>,
    _lock: File,
    restart_required: std::sync::atomic::AtomicBool,
}
pub struct LocalBackend {
    entries: BTreeMap<InstanceId, Entry>,
    helper: PathBuf,
    _global: File,
    operation_gate: Mutex<()>,
}
impl LocalBackend {
    pub fn open(
        data_dir: &Path,
        helper: PathBuf,
        servers: Vec<(Registration, LocalServer)>,
    ) -> Result<Self, String> {
        if !cfg!(windows) {
            return Err("実サーバー管理モードは Windows 専用です".into());
        }
        validate(data_dir)?;
        validate(&helper)?;
        // Across data directories, serialize local managers for this Windows user.
        let global =
            PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA がありません")?)
                .join("GameServerManager");
        validate(&global)?;
        fs::create_dir_all(&global).map_err(err)?;
        let global = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(global.join("local-runtime.lock"))
            .map_err(err)?;
        global
            .try_lock_exclusive()
            .map_err(|_| "実管理モードはこの Windows ユーザーで既に起動しています")?;
        for (i, (_, a)) in servers.iter().enumerate() {
            for (_, b) in servers.iter().skip(i + 1) {
                Self::validate_pair(a, b)?;
            }
        }
        let mut entries = BTreeMap::new();
        for (registration, spec) in servers {
            if registration.id != spec.instance.id || registration.game_id != spec.instance.game_id
            {
                return Err("登録と実サーバーの識別子が一致しません".into());
            }
            Self::validate_spec(&spec, data_dir)?;
            fs::create_dir_all(&spec.cwd).map_err(err)?;
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(spec.cwd.join(".gsm-server.lock"))
                .map_err(err)?;
            lock.try_lock_exclusive()
                .map_err(|_| "この設置先は別の統合マネージャーが使用中です")?;
            let state = data_dir
                .join("instances")
                .join(spec.instance.id.to_string());
            validate(&state)?;
            fs::create_dir_all(&state).map_err(err)?;
            let identity = match fs::read(state.join("process.json")) {
                Ok(data) => {
                    Some(serde_json::from_slice(&data).map_err(|_| "プロセス識別記録が不正です")?)
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(err(e)),
            };
            entries.insert(
                spec.instance.id,
                Entry {
                    diagnostics: Mutex::new(diagnostics::Watch::default()),
                    registration,
                    spec,
                    state,
                    identity: Mutex::new(identity),
                    operation: Mutex::new(()),
                    events: Mutex::new(VecDeque::new()),
                    _lock: lock,
                    restart_required: std::sync::atomic::AtomicBool::new(false),
                },
            );
        }
        Ok(Self {
            entries,
            helper,
            _global: global,
            operation_gate: Mutex::new(()),
        })
    }
    /// Validate resource separation before registration or changing a saved path.
    pub fn validate_pair(a: &LocalServer, b: &LocalServer) -> Result<(), String> {
        let saves_overlap = a
            .save_targets
            .iter()
            .any(|x| b.save_targets.iter().any(|y| overlap(&x.path, &y.path)));
        let backup_overlap = overlap(&a.backup_dir, &b.cwd)
            || overlap(&b.backup_dir, &a.cwd)
            || b.save_targets
                .iter()
                .any(|t| overlap(&a.backup_dir, &t.path))
            || a.save_targets
                .iter()
                .any(|t| overlap(&b.backup_dir, &t.path));
        if overlap(&a.cwd, &b.cwd) || saves_overlap || backup_overlap {
            return Err("他の登録サーバーの設置先・保存先と重なっています / Paths overlap another registered server".into());
        }
        Ok(())
    }
    pub fn validate_spec(spec: &LocalServer, data: &Path) -> Result<(), String> {
        for p in [
            &spec.cwd,
            &spec.working_directory,
            &spec.executable,
            &spec.backup_dir,
            &spec.log_file,
            &spec.steamcmd,
        ] {
            validate(p)?;
        }
        if !spec.executable.starts_with(&spec.cwd) || !spec.working_directory.starts_with(&spec.cwd)
        {
            return Err("実行ファイルが登録した設置先の外にあります".into());
        }
        if spec.arguments.iter().any(|v| v.contains('\0')) {
            return Err("起動引数に NUL が含まれています".into());
        }
        if spec.save_targets.is_empty() {
            return Err("保存対象が定義されていません".into());
        }
        for (i, t) in spec.save_targets.iter().enumerate() {
            validate(&t.path)?;
            if !component(&t.key)
                || overlap(&t.path, &spec.backup_dir)
                || overlap(&t.path, data)
                || overlap(&t.path, &spec.executable)
                || overlap(&t.path, &spec.log_file)
            {
                return Err(
                    "保存対象とバックアップ・管理データ・実行ファイル・ログが重なっています".into(),
                );
            }
            if spec
                .save_targets
                .iter()
                .skip(i + 1)
                .any(|other| other.key == t.key || overlap(&other.path, &t.path))
            {
                return Err("保存対象が重複しています".into());
            }
        }
        if overlap(&spec.backup_dir, &spec.cwd) || overlap(&spec.backup_dir, data) {
            return Err("バックアップは設置先・管理データとは別の場所にしてください".into());
        }
        Ok(())
    }
    pub fn instances(&self) -> Vec<Instance> {
        self.entries
            .values()
            .map(|e| e.spec.instance.clone())
            .collect()
    }
    fn current(entry: &Entry) -> Result<ProcessState, String> {
        process::observe(&entry.spec, entry.identity.lock().unwrap().as_ref())
    }
    fn event(entry: &Entry, text: String) {
        let mut events = entry.events.lock().unwrap();
        events.push_back(entry.spec.redact(&text));
        while events.len() > 200 {
            events.pop_front();
        }
    }
    fn verify_source(entry: &Entry) -> Result<(), String> {
        let source = ConfigSource::read(&entry.registration.source_path)?;
        if source.digest() != entry.registration.source_sha256 {
            return Err(
                "登録後に設定が変更されています。サーバー停止後に再登録してください".into(),
            );
        }
        Ok(())
    }
    fn stop(&self, entry: &Entry) -> Result<(), String> {
        let identity = entry
            .identity
            .lock()
            .unwrap()
            .clone()
            .ok_or("管理対象プロセスが記録されていません")?;
        if Self::current(entry)? != ProcessState::Alive {
            return Err("実行ファイル・PID・作成時刻の照合に失敗しました".into());
        }
        let waiter = process::ExitWaiter::open(&identity)
            .map_err(|e| format!("停止要求前の終了待機準備に失敗しました: {e}"))?;
        match &entry.spec.shutdown {
            Shutdown::Console => {
                if !self.helper.is_file() {
                    return Err("gsm-ctrlc-helper.exe がありません".into());
                }
                let mut command = std::process::Command::new(&self.helper);
                command
                    .arg(identity.pid.to_string())
                    .arg(identity.created.to_string());
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    command.creation_flags(0x08000000);
                }
                let output = command.output().map_err(|e| {
                    format!(
                        "停止 helper を実行できません [{}]: {e}",
                        self.helper.display()
                    )
                })?;
                if !output.status.success() {
                    return Err(format!(
                        "正常停止イベントを送信できませんでした: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    ));
                }
            }
            Shutdown::Rcon {
                port,
                password,
                commands,
            } => network::rcon(*port, password, commands)?,
            stop @ Shutdown::Https { .. } => network::api_shutdown(stop)?,
        }
        entry.diagnostics.lock().unwrap().stopping();
        if !waiter
            .wait(Duration::from_secs(
                entry.spec.stop_timeout_secs.clamp(5, 600),
            ))
            .map_err(|e| format!("停止要求送信後の終了確認に失敗しました: {e}"))?
        {
            return Err("停止待ちがタイムアウトしました。強制終了はしていません".into());
        }
        let path = entry.state.join("process.json");
        if exists(&path).map_err(|e| {
            format!(
                "サーバー終了後のプロセス記録を確認できません [{}]: {e}",
                path.display()
            )
        })? {
            fs::remove_file(&path).map_err(|e| {
                format!(
                    "サーバーは終了しましたが、プロセス記録を削除できません [{}]: {e}",
                    path.display()
                )
            })?;
        }
        *entry.identity.lock().unwrap() = None;
        Ok(())
    }
    fn stopped(entry: &Entry) -> Result<(), String> {
        if Self::current(entry)? != ProcessState::Absent {
            return Err("停止確認ができないためファイル操作を中止しました".into());
        }
        Ok(())
    }
    fn work(&self, entry: &Entry, command: &Command) -> Result<(), String> {
        // Stopping the already verified process remains possible after source edits.
        if !matches!(command, Command::Stop | Command::Recover) {
            Self::verify_source(entry)?;
        }
        let marker = entry.state.join("operation.json");
        if ((snapshots::pending(&entry.state) || settings::pending(&entry.state))
            || exists(&marker)?)
            && !matches!(command, Command::Recover | Command::Stop)
        {
            return Err("前回の操作が未完了です。回復を実行してください".into());
        }
        if matches!(command, Command::Stop) {
            return self.stop(entry);
        }
        Self::stopped(entry)?;
        let mut updater = entry.spec.clone();
        updater.executable = entry.spec.steamcmd.clone();
        if process::observe(&updater, None)? != ProcessState::Absent {
            return Err("SteamCMD が動作中です。終了を待ってください".into());
        }
        if matches!(command, Command::Recover) {
            settings::recover(&entry.spec, &entry.state)?;
            snapshots::recover(&entry.spec, &entry.state)?;
            let identity = entry.state.join("process.json");
            if exists(&identity)? {
                fs::remove_file(identity).map_err(err)?;
            }
            *entry.identity.lock().unwrap() = None;
            if exists(&marker)? {
                fs::remove_file(marker).map_err(err)?;
            }
            return Ok(());
        }
        (entry.spec.validate_layout)(&entry.spec)?;
        write_json(
            &marker,
            &serde_json::json!({"operation":command.label(),"instance":entry.spec.instance.id}),
        )?;
        let result = (|| match command {
            Command::Start => {
                if entry
                    .restart_required
                    .load(std::sync::atomic::Ordering::SeqCst)
                {
                    return Err("設定・復元・更新の反映が必要です。設定を確認し「管理画面を再読み込み」でアプリを再起動してください".into());
                }
                (entry.spec.validate_start)(&entry.spec)?;
                for other in self.entries.values() {
                    if other.spec.instance.id != entry.spec.instance.id
                        && entry
                            .spec
                            .ports
                            .iter()
                            .any(|p| other.spec.ports.contains(p))
                        && Self::current(other)? != ProcessState::Absent
                    {
                        return Err(
                            "同じポートを使う登録サーバーが稼働中、または状態不明です".into()
                        );
                    }
                }
                if !entry.spec.executable.is_file() {
                    return Err("サーバー実行ファイルがありません。先に更新・インストールを実行してください".into());
                }
                if matches!(entry.spec.shutdown, Shutdown::Console) && !self.helper.is_file() {
                    return Err("正常停止 helper がありません".into());
                }
                for port in &entry.spec.ports {
                    let _udp = std::net::UdpSocket::bind(("127.0.0.1", *port))
                        .map_err(|_| format!("UDP ポート {port} が使用中です"))?;
                    let _tcp = std::net::TcpListener::bind(("127.0.0.1", *port))
                        .map_err(|_| format!("TCP ポート {port} が使用中です"))?;
                }
                if entry.spec.save_targets.iter().any(|t| t.path.exists()) {
                    snapshots::create(&entry.spec)
                        .map_err(|e| format!("起動前バックアップに失敗しました: {e}"))?;
                }
                Self::stopped(entry)?;
                *entry.diagnostics.lock().unwrap() =
                    diagnostics::Watch::begin(&entry.spec.log_file);
                let identity = process::start(&entry.spec)?;
                entry.diagnostics.lock().unwrap().started();
                *entry.identity.lock().unwrap() = Some(identity.clone());
                let identity_path = entry.state.join("process.json");
                write_json(&identity_path, &identity).map_err(|e| {
                    format!(
                        "サーバー起動後のプロセス記録を保存できません [{}]: {e}",
                        identity_path.display()
                    )
                })?;
                Ok(())
            }
            Command::Backup => {
                snapshots::create(&entry.spec)?;
                Self::stopped(entry)
            }
            Command::Restore(id) => {
                entry
                    .restart_required
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                snapshots::restore(&entry.spec, &entry.state, *id)
            }
            Command::WriteSettings(change) => {
                let writes =
                    settings::prepare(&entry.spec, &entry.registration.source_path, change)?;
                snapshots::create(&entry.spec)?;
                Self::stopped(entry)?;
                settings::apply(&entry.spec, &entry.state, &writes)?;
                entry
                    .restart_required
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
            Command::EditSettings => {
                snapshots::create(&entry.spec)?;
                entry
                    .restart_required
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                for path in &entry.spec.edit_files {
                    validate(path)?;
                    if !path.is_file() {
                        return Err("編集対象の設定ファイルがありません".into());
                    }
                    if !entry.spec.save_targets.iter().any(|t| {
                        key(path) == key(&t.path) || key(path).starts_with(&(key(&t.path) + "\\"))
                    }) {
                        return Err("退避対象にない設定ファイルは編集できません".into());
                    }
                    let editor = PathBuf::from(
                        std::env::var_os("SystemRoot").ok_or("SystemRoot がありません")?,
                    )
                    .join("System32/notepad.exe");
                    if !std::process::Command::new(editor)
                        .arg(path)
                        .status()
                        .map_err(err)?
                        .success()
                    {
                        return Err("設定エディターを終了できません".into());
                    }
                }
                Self::event(entry,"編集を保存してエディターを閉じ、設定を再検証・登録して実管理モードを再起動してください".into());
                Ok(())
            }
            Command::Update => {
                if !entry.spec.steamcmd.is_file() {
                    return Err("指定した steamcmd.exe がありません。アプリ設定 → SteamCMD で用意し、サーバー設定のパスを確認してください / SteamCMD is missing. Set it up in App settings → SteamCMD and check the server-specific path".into());
                }
                let _steamcmd_lease = crate::steamcmd::Lease::acquire(&entry.spec.steamcmd)?;
                if entry.spec.save_targets.iter().any(|t| t.path.exists()) {
                    snapshots::create(&entry.spec)?;
                }
                entry
                    .restart_required
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                let log_path = entry.state.join("steamcmd.log");
                Self::event(
                    entry,
                    format!(
                        "SteamCMD: {}\n更新ログ / Update log: {}",
                        entry.spec.steamcmd.display(),
                        log_path.display()
                    ),
                );
                updater::run(&entry.spec, &log_path, |message| {
                    Self::event(entry, message)
                })?;
                Ok(())
            }
            _ => Err("未対応の操作です".into()),
        })();
        if !(snapshots::pending(&entry.state) || settings::pending(&entry.state))
            && let Err(error) = fs::remove_file(&marker)
        {
            let cleanup = format!("操作記録を削除できません [{}]: {error}", marker.display());
            return Err(match result {
                Ok(()) => format!("操作本体は完了しましたが、{cleanup}"),
                Err(original) => format!("{original}\n後処理にも失敗しました: {cleanup}"),
            });
        }
        result
    }
}
impl GameBackend for LocalBackend {
    fn full_log(&self, instance: &Instance) -> Result<String, String> {
        let entry = self
            .entries
            .get(&instance.id)
            .ok_or("Server not registered / サーバー未登録")?;
        if entry.spec.instance != *instance {
            return Err("Server mismatch / 対象が一致しません".into());
        }
        let events = entry
            .events
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        logs::combined(&entry.spec, &entry.state.join("steamcmd.log"), &events)
    }

    fn is_local(&self) -> bool {
        true
    }
    fn execute(&self, request: &OperationRequest) -> Result<Observation, String> {
        let _operation = self
            .operation_gate
            .try_lock()
            .map_err(|_| "別の実サーバーで操作中です。完了を待ってください")?;
        let entry = self
            .entries
            .get(&request.instance.id)
            .ok_or("実サーバーが登録されていません")?;
        if entry.spec.instance.game_id != request.instance.game_id
            || entry.spec.instance.world != request.instance.world
        {
            return Err("実サーバーの操作対象が一致しません".into());
        }
        let _guard = entry
            .operation
            // The global gate already excludes other commands. A monitor may still
            // hold this entry briefly; wait on the worker rather than reject the click.
            .lock()
            .map_err(|_| "状態確認処理が異常終了しました。管理ツールを再起動してください")?;
        Self::event(entry, format!("{} を開始", request.command.label()));
        let result = self.work(entry, &request.command);
        Self::event(
            entry,
            match &result {
                Ok(_) => format!("{} が完了", request.command.label()),
                Err(e) => format!("{} が失敗: {e}", request.command.label()),
            },
        );
        result.map_err(|e| entry.spec.redact(&e))?;
        Ok(Observation {
            process: Self::current(entry)?,
            readiness: Readiness::Unknown,
        })
    }
    fn inspect(&self, instance: &Instance) -> Result<Option<BackendState>, String> {
        let Some(entry) = self.entries.get(&instance.id) else {
            return Ok(Some(BackendState {
                info: gsm_domain::local::ServerInfo {
                    installation: gsm_domain::local::Installation::Unregistered,
                    alert: None,
                },
                observation: Observation {
                    process: ProcessState::Unknown,
                    readiness: Readiness::Unknown,
                },
                backups: vec![],
                logs: vec!["実サーバー設定を登録して実管理モードを再起動してください".into()],
            }));
        };
        let Ok(_guard) = entry.operation.try_lock() else {
            return Ok(None);
        };
        let process = Self::current(entry)?;
        let mut logs = entry
            .events
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        if let Ok(mut f) = File::open(&entry.spec.log_file) {
            logs.push("サーバーログ（過去の起動・停止の記録を含みます）".into());
            let len = f.metadata().map_err(err)?.len();
            f.seek(SeekFrom::Start(len.saturating_sub(32768)))
                .map_err(err)?;
            let mut bytes = vec![];
            f.take(32768).read_to_end(&mut bytes).map_err(err)?;
            logs.extend(
                String::from_utf8_lossy(&bytes)
                    .lines()
                    .rev()
                    .take(100)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .map(|s| entry.spec.redact(s)),
            );
        }
        let update_log = entry.state.join("steamcmd.log");
        if update_log.exists() {
            logs.push(format!("SteamCMD 更新ログ（過去の更新を含みます） / SteamCMD update log (includes previous updates): {}", update_log.display()));
            match logs::tail(&entry.spec, &update_log, 0) {
                Ok(text) => logs.extend(text.lines().map(str::to_owned)),
                Err(e) => logs.push(entry.spec.redact(&e)),
            }
        }
        // Old log lines cannot establish readiness for the current process.
        let readiness = Readiness::Unknown;
        if (snapshots::pending(&entry.state) || settings::pending(&entry.state))
            || entry.state.join("operation.json").exists()
        {
            logs.push("未完了の処理があります。停止状態で「回復」を実行してください".into());
        }
        if entry
            .restart_required
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            logs.push(
                "設定・復元・更新後は設定を確認し「管理画面を再読み込み」で再起動してください"
                    .into(),
            );
        }
        let backups = match snapshots::list(&entry.spec) {
            Ok(b) => b,
            Err(e) => {
                logs.push(
                    entry
                        .spec
                        .redact(&format!("バックアップ一覧の読込に失敗: {e}")),
                );
                vec![]
            }
        };
        let mut alert = entry
            .diagnostics
            .lock()
            .unwrap()
            .poll(&entry.spec.log_file, process);
        if let Some(alert) = &mut alert {
            alert.detail = entry.spec.redact(&alert.detail);
        }
        Ok(Some(BackendState {
            info: gsm_domain::local::ServerInfo {
                installation: installation::inspect(
                    &entry.spec.executable,
                    &entry.spec.cwd,
                    entry.spec.steam_app_id,
                ),
                alert,
            },
            observation: Observation { process, readiness },
            backups,
            logs,
        }))
    }
}

#[cfg(test)]
mod tests;
