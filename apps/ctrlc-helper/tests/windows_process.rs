#![cfg(windows)]
//! Runs only an isolated copy of this test binary, never a game executable.
use gsm_domain::{
    Instance, InstanceId, ProcessState,
    local::{LocalServer, Shutdown},
};
use gsm_infra::local::process;
use std::{
    fs,
    os::windows::process::CommandExt,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
static STOP: AtomicBool = AtomicBool::new(false);
unsafe extern "system" fn handler(event: u32) -> i32 {
    if event == 0 {
        STOP.store(true, Ordering::SeqCst);
        1
    } else {
        0
    }
}
#[test]
#[ignore = "Child fixture, invoked only by the parent lifecycle test"]
fn fixture_server() {
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
    assert_ne!(unsafe { SetConsoleCtrlHandler(Some(handler), 1) }, 0);
    let root = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    fs::write(root.join("ready"), b"ready").unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !STOP.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "fixture helper timeout");
        std::thread::sleep(Duration::from_millis(10));
    }
    // Model a server that spends time saving after receiving Ctrl+C.
    std::thread::sleep(Duration::from_millis(500));
    fs::write(root.join("saved"), b"flushed after ctrl-c").unwrap();
}
struct Cleanup(process::Identity);
impl Drop for Cleanup {
    fn drop(&mut self) {
        if process::identity(self.0.pid).is_ok_and(|i| i.created == self.0.created) {
            let _ = process::signal(self.0.pid, self.0.created);
        }
    }
}
#[test]
fn windows_identity_and_helper_preserve_the_selected_process() {
    let temp = tempfile::tempdir().unwrap();
    let exe = temp.path().join("gsm-isolated-fixture.exe");
    fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();
    let spec = LocalServer {
        instance: Instance {
            id: InstanceId::new(),
            game_id: "fixture".to_owned().try_into().unwrap(),
            name: "fixture".into(),
            world: "fixture".into(),
        },
        executable: exe,
        cwd: temp.path().to_owned(),
        working_directory: temp.path().to_owned(),
        arguments: vec![
            "--ignored".into(),
            "--exact".into(),
            "fixture_server".into(),
            "--nocapture".into(),
        ],
        save_targets: vec![],
        backup_dir: temp.path().join("backups"),
        log_file: temp.path().join("log"),
        shutdown: Shutdown::Console,
        stop_timeout_secs: 5,
        ports: vec![],
        steamcmd: PathBuf::new(),
        steam_app_id: 0,
        secrets: vec![],
        edit_files: vec![],
        validate_layout: |_| Ok(()),
        validate_start: |_| Ok(()),
        validate_saves: |_| Ok(()),
    };
    assert_eq!(process::observe(&spec, None).unwrap(), ProcessState::Absent);
    let identity = process::start(&spec).unwrap();
    let cleanup = Cleanup(identity.clone());
    let deadline = Instant::now() + Duration::from_secs(10);
    while !temp.path().join("ready").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        process::observe(&spec, Some(&identity)).unwrap(),
        ProcessState::Alive
    );
    assert_eq!(
        process::observe(&spec, None).unwrap(),
        ProcessState::Unknown
    );
    let mut incorrect = identity.clone();
    incorrect.created += 1;
    assert!(process::ExitWaiter::open(&incorrect).is_err());
    incorrect = identity.clone();
    incorrect.executable = temp.path().join("different-server.exe");
    assert!(process::ExitWaiter::open(&incorrect).is_err());
    let waiter = process::ExitWaiter::open(&identity).unwrap();
    assert!(!waiter.wait(Duration::from_millis(20)).unwrap());
    assert!(!temp.path().join("saved").exists());
    let helper = env!("CARGO_BIN_EXE_gsm-ctrlc-helper");
    assert!(
        !std::process::Command::new(helper)
            .args([identity.pid.to_string(), (identity.created + 1).to_string()])
            .creation_flags(0x08000000) // Same CREATE_NO_WINDOW as the GUI.
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(
        process::observe(&spec, Some(&identity)).unwrap(),
        ProcessState::Alive
    );
    assert!(!temp.path().join("saved").exists());
    assert!(
        std::process::Command::new(helper)
            .args([identity.pid.to_string(), identity.created.to_string()])
            .creation_flags(0x08000000)
            .status()
            .unwrap()
            .success()
    );
    assert!(waiter.wait(Duration::from_secs(10)).unwrap());
    // The retained handle still identifies the exited process without reopening its PID.
    assert!(waiter.wait(Duration::ZERO).unwrap());
    assert_eq!(
        process::observe(&spec, Some(&identity)).unwrap(),
        ProcessState::Absent
    );
    assert_eq!(
        fs::read(temp.path().join("saved")).unwrap(),
        b"flushed after ctrl-c"
    );
    drop(waiter);
    drop(cleanup);
}
