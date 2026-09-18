use super::*;
use std::{
    sync::{Arc, mpsc},
    time::Instant,
};

#[test]
fn command_waits_for_monitor_but_other_commands_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let instance = Instance {
        id: InstanceId::new(),
        game_id: "fixture".to_owned().try_into().unwrap(),
        name: "fixture".into(),
        world: "fixture".into(),
    };
    let spec = LocalServer {
        instance: instance.clone(),
        cwd: temp.path().join("server"),
        working_directory: temp.path().join("server"),
        executable: temp.path().join("never-run.exe"),
        arguments: vec![],
        save_targets: vec![],
        backup_dir: temp.path().join("backups"),
        log_file: temp.path().join("server.log"),
        shutdown: Shutdown::Console,
        stop_timeout_secs: 5,
        ports: vec![],
        steamcmd: temp.path().join("never-run-steamcmd.exe"),
        steam_app_id: 0,
        secrets: vec!["fixture-secret".into()],
        edit_files: vec![],
        validate_layout: |_| Ok(()),
        validate_start: |_| Ok(()),
        validate_saves: |_| Ok(()),
    };
    // Full-log copy includes the first line and content beyond the preview tail,
    // while retaining the same secret redaction as the on-screen log.
    fs::write(
        &spec.log_file,
        format!(
            "first line fixture-secret\n{}\nlast line",
            "long-log\n".repeat(10000)
        ),
    )
    .unwrap();
    let full = logs::full(&spec, &spec.log_file).unwrap();
    assert!(full.starts_with("first line"));
    assert!(full.ends_with("last line"));
    assert!(full.len() > 32768);
    assert!(!full.contains("fixture-secret"));
    File::create(&spec.log_file)
        .unwrap()
        .set_len(128 * 1024 * 1024 + 1)
        .unwrap();
    assert!(logs::full(&spec, &spec.log_file).is_err());
    fs::remove_file(&spec.log_file).unwrap();
    let update_log = temp.path().join("steamcmd.log");
    let old = "ERROR: previous attempt\n";
    fs::write(
        &update_log,
        format!("{old}ERROR: new failure fixture-secret\n"),
    )
    .unwrap();
    let failure = logs::update_failure(&spec, &update_log, old.len() as u64, "exit code: 7");
    assert!(failure.contains("exit code: 7"));
    assert!(failure.contains("new failure"));
    assert!(failure.contains(&update_log.display().to_string()));
    assert!(!failure.contains("previous attempt"));
    assert!(!failure.contains("fixture-secret"));
    let copied = logs::combined(&spec, &update_log, "update failed fixture-secret").unwrap();
    assert!(copied.contains("Manager operations"));
    assert!(copied.contains("No log yet"));
    assert!(copied.contains("previous attempt"));
    assert!(copied.contains("new failure"));
    assert!(!copied.contains("fixture-secret"));
    let size = fs::metadata(&update_log).unwrap().len();
    let no_output = logs::update_failure(&spec, &update_log, size, "exit code: 1");
    assert!(no_output.contains("No output from this attempt"));
    assert!(!no_output.contains("new failure"));
    fs::write(
        &update_log,
        format!("{}fixture-secret\nlast line", "x".repeat(9000)),
    )
    .unwrap();
    assert_eq!(logs::tail(&spec, &update_log, 0).unwrap(), "last line");
    // Missing config stops execution immediately after lock acquisition, before
    // process inspection or server IO on either Windows or Linux.
    let entry = Entry {
        diagnostics: Mutex::new(diagnostics::Watch::default()),
        registration: Registration {
            id: instance.id,
            game_id: instance.game_id.clone(),
            source_path: temp.path().join("missing-config.toml"),
            source_sha256: String::new(),
        },
        spec,
        state: temp.path().join("state"),
        operation: Mutex::new(()),
        identity: Mutex::new(None),
        events: Mutex::new(VecDeque::new()),
        _lock: tempfile::tempfile().unwrap(),
        restart_required: std::sync::atomic::AtomicBool::new(false),
    };
    let backend = Arc::new(LocalBackend {
        entries: BTreeMap::from([(instance.id, entry)]),
        helper: temp.path().join("never-run-helper.exe"),
        _global: tempfile::tempfile().unwrap(),
        operation_gate: Mutex::new(()),
    });
    let request = OperationRequest {
        id: OperationId::new(),
        instance: instance.clone(),
        command: Command::Start,
        before: Observation {
            process: ProcessState::Absent,
            readiness: Readiness::Unknown,
        },
    };
    let entry = &backend.entries[&instance.id];
    let monitoring = entry.operation.lock().unwrap();
    let (finished, result) = mpsc::channel();
    let worker_backend = backend.clone();
    let worker_request = request.clone();
    let worker = std::thread::spawn(move || {
        finished
            .send(worker_backend.execute(&worker_request))
            .unwrap();
    });
    // Wait for the real execute path to hold the global command gate while the
    // monitor still owns the entry. The old try_lock path instead returns an error.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
        if matches!(
            backend.operation_gate.try_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        ) {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        backend.execute(&request).unwrap_err(),
        "別の実サーバーで操作中です。完了を待ってください"
    );
    assert!(backend.inspect(&instance).unwrap().is_none());
    assert!(matches!(
        result.recv_timeout(Duration::from_millis(50)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    drop(monitoring);
    let error = result
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .unwrap_err();
    worker.join().unwrap();
    assert_eq!(error, "設定ファイルを開けません");
    assert_eq!(entry.events.lock().unwrap().front().unwrap(), "起動 を開始");
    assert!(backend.operation_gate.try_lock().is_ok());
    assert!(entry.operation.try_lock().is_ok());
    assert!(!temp.path().join("state").exists());
    assert!(!temp.path().join("backups").exists());
}
