use gsm_domain::{Command, Job, JobStatus, OperationId};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct Pending {
    job: Option<OperationId>,
    due: Option<Instant>,
}
#[derive(Debug, PartialEq)]
pub(super) enum Action {
    Wait,
    Announce,
    Reload,
}
impl Pending {
    pub fn submitted(&mut self, id: OperationId) {
        self.job = Some(id);
        self.due = None;
    }
    pub fn poll(&mut self, jobs: &[Job], now: Instant, safe_to_reload: bool) -> Action {
        let Some(id) = self.job else {
            return Action::Wait;
        };
        let Some(job) = jobs
            .iter()
            .find(|job| job.id == id && job.command == Command::Update)
        else {
            *self = Self::default();
            return Action::Wait;
        };
        match job.status {
            JobStatus::Running => Action::Wait,
            JobStatus::Failed(_) => {
                *self = Self::default();
                Action::Wait
            }
            JobStatus::Completed => match self.due {
                None => {
                    self.due = Some(now + Duration::from_secs(3));
                    Action::Announce
                }
                Some(due) if now >= due && safe_to_reload => {
                    *self = Self::default();
                    Action::Reload
                }
                _ => Action::Wait,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gsm_domain::{Instance, InstanceId};
    #[test]
    fn reloads_only_submitted_success_once_after_notice_and_pending_work() {
        let mut jobs = vec![Job {
            id: OperationId::new(),
            instance: Instance {
                id: InstanceId::new(),
                game_id: "valheim".to_owned().try_into().unwrap(),
                name: "Fixture".into(),
                world: "Fixture".into(),
            },
            command: Command::Update,
            status: JobStatus::Completed,
            started_at: 1,
        }];
        let now = Instant::now();
        let later = now + Duration::from_secs(4);
        let mut pending = Pending::default();
        // Persisted successes from previous sessions must never trigger a reload loop.
        assert_eq!(pending.poll(&jobs, now, true), Action::Wait);
        pending.submitted(jobs[0].id);
        jobs[0].status = JobStatus::Running;
        assert_eq!(pending.poll(&jobs, now, true), Action::Wait);
        jobs[0].status = JobStatus::Completed;
        assert_eq!(pending.poll(&jobs, now, true), Action::Announce);
        assert_eq!(pending.poll(&jobs, now, true), Action::Wait);
        assert_eq!(pending.poll(&jobs, later, false), Action::Wait);
        assert_eq!(pending.poll(&jobs, later, true), Action::Reload);
        assert_eq!(pending.poll(&jobs, later, true), Action::Wait);
        pending.submitted(jobs[0].id);
        jobs[0].status = JobStatus::Failed("fixture".into());
        assert_eq!(pending.poll(&jobs, now, true), Action::Wait);
        assert_eq!(pending.poll(&jobs, later, true), Action::Wait);
    }
}
