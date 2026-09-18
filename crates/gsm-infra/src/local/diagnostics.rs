//! Alerts use bytes written after this manager's start request, never old log tails.
use gsm_domain::{ProcessState, local::ServerAlert};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

#[derive(Default)]
pub(super) struct Watch {
    cursor: u64,
    prefix: Vec<u8>,
    pending: Vec<u8>,
    discard_line: bool,
    started: bool,
    stopping: bool,
    alert: Option<ServerAlert>,
}
impl Watch {
    pub fn begin(path: &Path) -> Self {
        let mut watch = Self::default();
        if let Ok(mut f) = File::open(path) {
            watch.cursor = f.metadata().map(|m| m.len()).unwrap_or(0);
            let _ = f.by_ref().take(256).read_to_end(&mut watch.prefix);
        }
        watch
    }
    pub fn started(&mut self) {
        self.started = true;
    }
    pub fn stopping(&mut self) {
        self.stopping = true;
    }
    pub fn poll(&mut self, path: &Path, process: ProcessState) -> Option<ServerAlert> {
        if !self.started {
            return self.alert.clone();
        }
        if let Ok(mut f) = File::open(path) {
            let mut prefix = Vec::new();
            let _ = f.by_ref().take(256).read_to_end(&mut prefix);
            let len = f.metadata().map(|m| m.len()).unwrap_or(0);
            if len < self.cursor || !prefix.starts_with(&self.prefix) {
                self.cursor = 0;
                self.pending.clear();
                self.discard_line = false;
            }
            self.prefix = prefix;
            if f.seek(SeekFrom::Start(self.cursor)).is_ok() {
                let mut bytes = Vec::new();
                if f.take(256 * 1024).read_to_end(&mut bytes).is_ok() {
                    self.cursor += bytes.len() as u64;
                    for byte in bytes {
                        if byte == b'\n' {
                            if !self.discard_line
                                && let Some(alert) =
                                    classify(&String::from_utf8_lossy(&self.pending))
                                && (self.alert.is_none() || alert.update_suggested)
                            {
                                self.alert = Some(alert);
                            }
                            self.pending.clear();
                            self.discard_line = false;
                        } else if self.pending.len() < 8192 && !self.discard_line {
                            self.pending.push(byte);
                        } else {
                            self.pending.clear();
                            self.discard_line = true;
                        }
                    }
                }
            }
        }
        if process == ProcessState::Absent && !self.stopping && self.alert.is_none() {
            self.alert = Some(ServerAlert {
                detail: "起動したサーバープロセスが終了しました。今回のサーバーログを確認してください。 / The started server process exited. Check its current log.".into(),
                update_suggested: false,
            });
        }
        self.alert.clone()
    }
}
fn classify(line: &str) -> Option<ServerAlert> {
    let lower = line.to_ascii_lowercase();
    // Do not treat version check messages, Unity graphics warnings or ordinary
    // disconnects as errors. Only explicit failure phrases are recognized.
    let mismatch = [
        "incompatible version",
        "version mismatch",
        "wrong version",
        "バージョンが一致しません",
    ]
    .iter()
    .any(|p| lower.contains(p));
    let failure = [
        "fatal error",
        "failed to bind",
        "address already in use",
        "failed to load world",
    ]
    .iter()
    .any(|p| lower.contains(p));
    (mismatch || failure).then(|| ServerAlert {
        detail: line.trim().to_owned(),
        update_suggested: mismatch,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn ignores_old_errors_but_tracks_new_partial_lines_and_rotation() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("server.log");
        std::fs::write(&p, "old version mismatch\n").unwrap();
        let mut w = Watch::begin(&p);
        w.started();
        assert!(w.poll(&p, ProcessState::Alive).is_none());
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        f.write_all(b"new incompatible ver").unwrap();
        assert!(w.poll(&p, ProcessState::Alive).is_none());
        f.write_all(b"sion: client 40 server 39\n").unwrap();
        let a = w.poll(&p, ProcessState::Alive).unwrap();
        assert!(a.update_suggested);
        assert!(a.detail.starts_with("new"));
        let mut w = Watch::begin(&p);
        w.started();
        std::fs::write(&p, "new log\nfatal error: world unavailable\n").unwrap();
        assert!(!w.poll(&p, ProcessState::Alive).unwrap().update_suggested);
    }
    #[test]
    fn expected_shutdown_and_graphics_warnings_do_not_raise_alerts() {
        assert!(classify("Network version check, their:40, mine:40").is_none());
        assert!(classify("The shader is not supported on this platform!").is_none());
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("absent.log");
        let mut w = Watch::begin(&p);
        assert!(w.poll(&p, ProcessState::Absent).is_none());
        w.started();
        w.stopping();
        assert!(w.poll(&p, ProcessState::Absent).is_none());
        let mut w = Watch::begin(&p);
        w.started();
        assert!(w.poll(&p, ProcessState::Absent).is_some());
    }
}
