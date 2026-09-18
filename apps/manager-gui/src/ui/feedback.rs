use gsm_application::InstanceView;
use gsm_domain::{Job, JobStatus, local::Installation};

pub(super) fn version_label(value: Option<&Installation>, local: bool, en: bool) -> String {
    if !local {
        return if en {
            "Server version: unknown (mock)"
        } else {
            "サーバーバージョン: 不明（模擬）"
        }
        .into();
    }
    match value {
        Some(Installation::SteamBuild(id)) => {
            if en {
                format!("Server: Steam Build {id}")
            } else {
                format!("サーバー: Steam Build {id}")
            }
        }
        Some(Installation::NotInstalled) => if en {
            "Server: not installed"
        } else {
            "サーバー: 未インストール"
        }
        .into(),
        None | Some(Installation::Unregistered) => if en {
            "Version unknown (not registered)"
        } else {
            "バージョン不明（未登録）"
        }
        .into(),
        Some(Installation::Unknown) => if en {
            "Server version: unknown"
        } else {
            "サーバーバージョン: 不明"
        }
        .into(),
    }
}
pub(super) fn failure(
    view: &InstanceView,
    jobs: &[Job],
    en: bool,
) -> Option<(String, String, bool)> {
    if let Some(job) = jobs
        .iter()
        .rev()
        .find(|j| j.instance.id == view.instance.id)
        && let JobStatus::Failed(detail) = &job.status
    {
        let lower = detail.to_ascii_lowercase();
        let update = [
            "version mismatch",
            "incompatible version",
            "wrong version",
            "更新・インストール",
        ]
        .iter()
        .any(|s| lower.contains(s));
        return Some((
            format!(
                "{} · {} {}",
                view.instance.name,
                crate::language::command(&job.command, en),
                if en {
                    "failed"
                } else {
                    "に失敗しました"
                }
            ),
            detail.clone(),
            update,
        ));
    }
    view.info.alert.as_ref().map(|a| {
        (
            format!(
                "{} · {}",
                view.instance.name,
                if a.update_suggested {
                    if en {
                        "Version mismatch reported"
                    } else {
                        "バージョン不一致を検出"
                    }
                } else if en {
                    "Server needs attention"
                } else {
                    "サーバーの確認が必要です"
                }
            ),
            a.detail.clone(),
            a.update_suggested,
        )
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use gsm_domain::{
        Command, Instance, InstanceId, Observation, OperationId,
        local::{ServerAlert, ServerInfo},
    };
    #[test]
    fn failures_follow_server_identity_and_keep_runtime_mismatches_visible() {
        let view = InstanceView {
            instance: Instance {
                id: InstanceId::new(),
                game_id: "valheim".to_string().try_into().unwrap(),
                name: "selected".into(),
                world: "world".into(),
            },
            info: ServerInfo::default(),
            observation: Observation::default(),
            active: None,
            backups: vec![],
            logs: vec![],
        };
        let mut jobs = vec![Job {
            id: OperationId::new(),
            instance: view.instance.clone(),
            command: Command::Start,
            status: JobStatus::Failed("access denied".into()),
            started_at: 0,
        }];
        assert_eq!(failure(&view, &jobs, true).unwrap().1, "access denied");
        jobs[0].instance.id = InstanceId::new();
        assert!(failure(&view, &jobs, true).is_none());
        let mut view = view;
        view.info.alert = Some(ServerAlert {
            detail: "incompatible version".into(),
            update_suggested: true,
        });
        assert!(failure(&view, &jobs, true).unwrap().2);
        jobs[0].instance.id = view.instance.id;
        jobs[0].status = JobStatus::Completed;
        assert!(failure(&view, &jobs, false).unwrap().2);
    }
    #[test]
    fn unknown_installations_and_mock_do_not_claim_an_installed_version() {
        assert!(version_label(None, true, false).contains("未登録"));
        assert!(
            version_label(Some(&Installation::NotInstalled), true, false)
                .contains("未インストール")
        );
        assert!(
            version_label(Some(&Installation::SteamBuild("123".into())), false, true)
                .contains("mock")
        );
        assert!(
            version_label(Some(&Installation::SteamBuild("123".into())), true, true)
                .contains("Steam Build 123")
        );
    }
}
