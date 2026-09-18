//! Synthetic observations only: no game executable, network or save-file access.
use gsm_domain::{Command, GameBackend, Observation, OperationRequest, ProcessState, Readiness};
use std::time::Duration;

pub struct MockBackend {
    delay: Duration,
}
impl MockBackend {
    pub fn new(delay: Duration) -> Self {
        Self { delay }
    }
}
impl Default for MockBackend {
    fn default() -> Self {
        Self::new(Duration::from_millis(750))
    }
}
impl GameBackend for MockBackend {
    fn execute(&self, request: &OperationRequest) -> Result<Observation, String> {
        std::thread::sleep(self.delay);
        match (&request.command, request.before.process) {
            (Command::Start, ProcessState::Absent) => Ok(Observation {
                process: ProcessState::Alive,
                readiness: Readiness::Ready,
            }),
            (Command::Stop, ProcessState::Alive) => Ok(Observation::default()),
            (
                Command::Backup | Command::Restore(_) | Command::WriteSettings(_),
                ProcessState::Absent,
            ) => Ok(request.before),
            _ => Err("模擬サーバーの現在の状態では実行できません".into()),
        }
    }
}
