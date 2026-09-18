//! SteamCMD self-update can exit/relaunch before it processes game arguments.
//! Initialize with +quit first, then run and verify a separate app_update.
use super::{logs, paths::err, process};
use gsm_domain::{ProcessState, local::LocalServer};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

struct Attempt {
    success: bool,
    status: String,
    start: u64,
    ready: bool,
    installed: bool,
}

pub(super) fn run(spec: &LocalServer, log: &Path, event: impl Fn(String)) -> Result<(), String> {
    initialize(|| {
        event("SteamCMD の初期化・自己更新を確認中 / Checking SteamCMD initialization and self-update".into());
        let attempt = invoke(spec, log, &["+quit"], "SteamCMD initialization")?;
        if attempt.success && attempt.ready {
            Ok(None)
        } else {
            Ok(Some(logs::update_failure(
                spec,
                log,
                attempt.start,
                &attempt.status,
            )))
        }
    })?;
    event("ゲームサーバーを更新・検証中 / Updating and validating game server".into());
    let app = spec.steam_app_id.to_string();
    let cwd = spec.cwd.to_string_lossy();
    let attempt = invoke(
        spec,
        log,
        &[
            "+force_install_dir",
            &cwd,
            "+login",
            "anonymous",
            "+app_update",
            &app,
            "validate",
            "+quit",
        ],
        "Game server update",
    )?;
    if !attempt.success || !attempt.installed {
        let reason = if attempt.success {
            format!(
                "{}; App {} の更新完了を確認できません / App update completion not confirmed",
                attempt.status, app
            )
        } else {
            attempt.status
        };
        return Err(logs::update_failure(spec, log, attempt.start, &reason));
    }
    Ok(())
}

fn initialize(mut attempt: impl FnMut() -> Result<Option<String>, String>) -> Result<(), String> {
    // +quit is safe to repeat: no login or game update is requested in this phase.
    // Two successive bootstrap packages are possible on a fresh installation.
    for index in 0..3 {
        match attempt()? {
            None => return Ok(()),
            Some(e) if index == 2 => {
                return Err(format!(
                    "SteamCMD の初期化を完了できません / SteamCMD initialization did not complete after 3 attempts\n{e}"
                ));
            }
            Some(_) => (),
        }
    }
    unreachable!()
}

fn invoke(
    spec: &LocalServer,
    log: &Path,
    arguments: &[&str],
    phase: &str,
) -> Result<Attempt, String> {
    // This also prevents a retry from overlapping a still-running bootstrap child.
    wait_until_idle(spec)?;
    let mut output = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .map_err(|e| {
            format!(
                "SteamCMD ログを開けません / Cannot open SteamCMD log [{}]: {e}",
                log.display()
            )
        })?;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(err)?
        .as_secs();
    writeln!(output, "\n--- GSM {phase} (Unix time: {timestamp}) ---").map_err(err)?;
    let start = output.metadata().map_err(err)?.len();
    let status = Command::new(&spec.steamcmd)
        .current_dir(spec.steamcmd.parent().ok_or("SteamCMD folder is missing")?)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(output.try_clone().map_err(err)?)
        .stderr(output)
        .status()
        .map_err(|e| format!("SteamCMD を実行できません / Cannot run SteamCMD [{}]: {e}\n更新ログ / Update log: {}", spec.steamcmd.display(), log.display()))?;
    writeln!(
        OpenOptions::new().append(true).open(log).map_err(err)?,
        "\n--- GSM {phase}: {status} ---"
    )
    .map_err(err)?;
    wait_until_idle(spec).map_err(|e| format!("{e}\n更新ログ / Update log: {}", log.display()))?;
    let ready = contains_since(log, start, b"Steam Console Client")?;
    let installed = contains_since(
        log,
        start,
        format!("Success! App '{}' fully installed.", spec.steam_app_id).as_bytes(),
    )?;
    Ok(Attempt {
        success: status.success(),
        status: status.to_string(),
        start,
        ready,
        installed,
    })
}

fn wait_until_idle(spec: &LocalServer) -> Result<(), String> {
    let mut updater = spec.clone();
    updater.executable = spec.steamcmd.clone();
    let deadline = Instant::now() + Duration::from_secs(600);
    let mut idle_since = None;
    loop {
        let state = process::observe(&updater, None);
        match &state {
            Ok(ProcessState::Absent) => {
                if idle_since.get_or_insert_with(Instant::now).elapsed() >= Duration::from_secs(2) {
                    return Ok(());
                }
            }
            // A relaunch may disappear between enumeration and identity lookup.
            // Require sustained absence; never start another updater on uncertainty.
            _ => idle_since = None,
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "SteamCMD の終了を確認できません。動作中の場合は終了を待って再試行してください / Cannot confirm SteamCMD has exited; wait before retrying. {}",
                state.err().unwrap_or_default()
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn contains_since(path: &Path, start: u64, needle: &[u8]) -> Result<bool, String> {
    let mut file = File::open(path).map_err(err)?;
    let length = file.metadata().map_err(err)?.len().saturating_sub(start);
    file.seek(SeekFrom::Start(start)).map_err(err)?;
    let mut reader = file.take(length);
    let mut carry = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = reader.read(&mut buffer).map_err(err)?;
        if read == 0 {
            return Ok(false);
        }
        carry.extend_from_slice(&buffer[..read]);
        if carry.windows(needle.len()).any(|window| window == needle) {
            return Ok(true);
        }
        let keep = carry.len().saturating_sub(needle.len() - 1);
        carry.drain(..keep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bootstrap_relaunches_are_bounded_and_must_reach_a_ready_client() {
        let mut count = 0;
        initialize(|| {
            count += 1;
            if count < 3 {
                Ok(Some("bootstrap relaunch".into()))
            } else {
                Ok(None)
            }
        })
        .unwrap();
        assert_eq!(count, 3);
        count = 0;
        let error = initialize(|| {
            count += 1;
            Ok(Some("network failure".into()))
        })
        .unwrap_err();
        assert_eq!(count, 3);
        assert!(error.contains("network failure"));
        count = 0;
        initialize(|| {
            count += 1;
            Ok(None)
        })
        .unwrap();
        assert_eq!(count, 1);
        count = 0;
        let error = initialize(|| {
            count += 1;
            Err("process still active".into())
        })
        .unwrap_err();
        assert_eq!(count, 1);
        assert!(error.contains("process still active"));
    }
    #[test]
    fn completion_is_specific_to_current_attempt_and_app_even_across_read_boundaries() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("steamcmd.log");
        let success = "Success! App '896660' fully installed.";
        let old = format!("{success}\n");
        std::fs::write(
            &path,
            format!("{old}Update complete, launching...\nSuccess! App '123' fully installed.\n"),
        )
        .unwrap();
        assert!(!contains_since(&path, old.len() as u64, success.as_bytes()).unwrap());
        std::fs::write(&path, format!("{}{success}", "x".repeat(8190))).unwrap();
        assert!(contains_since(&path, 0, success.as_bytes()).unwrap());
    }
}
