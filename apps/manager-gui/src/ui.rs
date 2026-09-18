mod background;
mod features;
mod feedback;
mod navigation;
mod steamcmd;
use navigation::Destination;

use crate::{
    AppPreferences, BackupRow, ConfigEditor, EditorField, GamePair, GameRow, JobRow, MainWindow, T,
    Theme,
};
use crate::{
    editor::{Draft, Drafts},
    preferences::{PreferenceWriter, Preferences},
};
use gsm_application::{Application, InstanceView};
use gsm_domain::{BackupId, Command, GameId, InstanceId, Job, JobStatus, ProcessState};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{cell::RefCell, collections::BTreeSet, rc::Rc, sync::Arc, time::Duration};
use std::{collections::BTreeMap, path::PathBuf, sync::mpsc, time::Instant};

type EditorReply = ((String, bool), Result<Draft, String>);

pub enum SessionExit {
    Close,
    Reload,
    ChangeDirectory(bool),
}

struct Session {
    app: Arc<Application>,
    registration: crate::registration::RegistrationPanel,
    steamcmd: steamcmd::Panel,
    file_picker_open: bool,
    draft: BTreeSet<GameId>,
    query: String,
    choosing: bool,
    active: Option<InstanceId>,
    overview: bool,
    page: i32,
    error: String,
    pending_navigation: Option<Destination>,
    pending_restore: Option<(InstanceId, BackupId, String)>,
    reload: bool,
    change_directory: bool,
    prefs: Preferences,
    prefs_writer: PreferenceWriter,
    prefs_path: PathBuf,
    prefs_valid: bool,
    preferences_open: bool,
    pending_import: Option<Preferences>,
    drafts: Drafts,
    creation: Option<crate::creation::Draft>,
    creation_receiver: Option<mpsc::Receiver<crate::creation::Reply>>,
    pending_creation: Option<crate::creation::Plan>,
    mock_settings: BTreeMap<String, gsm_domain::settings::Documents>,
    editor_receiver: Option<mpsc::Receiver<EditorReply>>,
    review_receiver: Option<mpsc::Receiver<crate::editor::ReviewReply>>,
    pending_edit: Option<(
        (String, bool),
        Arc<gsm_domain::SettingsWrite>,
        gsm_domain::settings::Documents,
    )>,
    save_job: Option<(
        gsm_domain::OperationId,
        String,
        gsm_domain::settings::Documents,
    )>,
    copy_receiver: Option<mpsc::Receiver<Result<arboard::Clipboard, String>>>,
    selection_receiver: Option<mpsc::Receiver<Result<(), String>>>,
    transfer_receiver: Option<mpsc::Receiver<Result<Option<Preferences>, String>>>,
    import_receiver: Option<(Preferences, mpsc::Receiver<Result<(), String>>)>,
    clipboard: Option<arboard::Clipboard>,
    noticed: BTreeMap<gsm_domain::OperationId, JobStatus>,
    toast_deadline: Option<Instant>,
}
fn model<T: Clone + PartialEq + 'static>(current: ModelRc<T>, items: Vec<T>) -> ModelRc<T> {
    // Preserve delegates and keyboard focus while polling background jobs.
    if let Some(rows) = current.as_any().downcast_ref::<VecModel<T>>() {
        if rows.row_count() == items.len() {
            for (index, item) in items.into_iter().enumerate() {
                if rows.row_data(index).as_ref() != Some(&item) {
                    rows.set_row_data(index, item);
                }
            }
        } else {
            rows.set_vec(items);
        }
        current
    } else {
        ModelRc::new(VecModel::from(items))
    }
}
fn clock(seconds: u64) -> String {
    i64::try_from(seconds)
        .ok()
        .and_then(|n| chrono::DateTime::from_timestamp(n, 0))
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S %:z")
                .to_string()
        })
        .unwrap_or_else(|| "日時不明".into())
}

fn status(view: &InstanceView, en: bool) -> String {
    let text = match view.observation.process {
        ProcessState::Absent => {
            if en {
                "Stopped"
            } else {
                "停止中"
            }
        }
        ProcessState::Alive => {
            if en {
                "Running"
            } else {
                "稼働中"
            }
        }
        ProcessState::Unknown => {
            if en {
                "Unknown"
            } else {
                "状態不明"
            }
        }
    };
    if view.active.is_some() {
        format!("{text} · {}", if en { "Working" } else { "操作中" })
    } else {
        text.into()
    }
}
fn job_phase(status: &JobStatus) -> &'static str {
    match status {
        JobStatus::Running => "running",
        JobStatus::Completed => "completed",
        JobStatus::Failed(_) => "failed",
    }
}
fn activity(jobs: &[Job], instance: InstanceId) -> (&'static str, String) {
    match jobs.iter().rev().find(|job| job.instance.id == instance) {
        Some(job) => activity_label(job.command.label(), &job.status),
        None => ("idle", "現在処理中の作業はありません".into()),
    }
}
fn activity_label(label: &str, status: &JobStatus) -> (&'static str, String) {
    let text = match status {
        JobStatus::Running => format!("{label}を実行中…"),
        JobStatus::Completed => format!("現在処理中の作業はありません · 直前: {label} 完了"),
        JobStatus::Failed(_) => format!("現在処理中の作業はありません · 直前: {label} 失敗"),
    };
    (job_phase(status), text)
}
impl Session {
    fn refresh(&self, ui: &MainWindow) {
        let snapshot = self.app.snapshot();
        self.steamcmd.refresh(
            ui,
            !self.registration.busy()
                && !self.file_picker_open
                && self.save_job.is_none()
                && snapshot.instances.iter().all(|v| v.active.is_none()),
            self.app.is_local() && cfg!(windows),
            self.english(),
        );
        ui.set_leave_visible(self.pending_navigation.is_some());
        ui.set_leave_busy(
            self.editor_receiver.is_some()
                || self.review_receiver.is_some()
                || self.save_job.is_some()
                || self.creation_busy(),
        );
        ui.set_preferences_open(self.preferences_open);
        ui.global::<AppPreferences>()
            .set_data_can_change(self.can_change_directory());
        ui.set_copy_busy(self.copy_receiver.is_some());
        ui.set_selection_busy(self.selection_receiver.is_some());
        ui.global::<AppPreferences>()
            .set_applying(self.import_receiver.is_some());
        let editor = ui.global::<ConfigEditor>();
        editor.set_shared_steamcmd(self.steamcmd.default_path().into());
        let key = (
            snapshot
                .instances
                .iter()
                .find(|v| Some(v.instance.id) == self.active)
                .map(|v| v.instance.game_id.to_string())
                .unwrap_or_default(),
            self.page == 2,
        );
        editor.set_creating(self.creation.is_some());
        ui.set_creation_open(self.creation.is_some());
        editor.set_busy(
            self.creation_busy()
                || self.editor_receiver.is_some()
                || self.review_receiver.is_some()
                || self.save_job.is_some(),
        );
        if let Some(draft) = &self.creation {
            editor.set_loaded(true);
            editor.set_notes("".into());
            let mut fields = vec![EditorField {
                key: "new_config".into(),
                label_ja: crate::editor::label("new_config", false).into(),
                label_en: crate::editor::label("new_config", true).into(),
                value: draft.source.clone().into(),
                path: true,
                ..Default::default()
            }];
            fields.extend(draft.values.iter().enumerate().map(|(index, (k, v))| {
                let f = draft.schema.iter().find(|f| f.id == k).unwrap();
                EditorField {
                    key: k.clone().into(),
                    label_ja: crate::editor::label(k, false)
                        .replace("（変更用）", "")
                        .into(),
                    label_en: crate::editor::label(k, true).into(),
                    value: crate::editor::option_label(k, v, self.english()).into(),
                    secret: f.kind == gsm_domain::settings::Kind::Secret,
                    path: f.kind == gsm_domain::settings::Kind::Path,
                    read_only: false,
                    group: if k == "steamcmd" {
                        2
                    } else if index == 0 {
                        1
                    } else {
                        0
                    },
                    options: ModelRc::new(VecModel::from(crate::editor::display_choices(
                        k,
                        self.english(),
                    ))),
                }
            }));
            // Options retain model identity, including while typing and changing language.
            for field in &mut fields {
                if let Some(old) = editor.get_fields().iter().find(|old| old.key == field.key) {
                    field.options = model(old.options, field.options.iter().collect());
                }
            }
            editor.set_fields(model(editor.get_fields(), fields));
        } else if let Some(draft) = self.drafts.get(&key) {
            editor.set_loaded(true);
            editor.set_notes(draft.notes.clone().into());
            editor.set_fields(model(
                editor.get_fields(),
                draft
                    .values
                    .iter()
                    .enumerate()
                    .map(|(index, (k, v))| EditorField {
                        group: if self.page == 2 {
                            0
                        } else if k == "steamcmd" {
                            2
                        } else if index == 0 {
                            1
                        } else {
                            0
                        },
                        key: k.clone().into(),
                        secret: draft
                            .field(k)
                            .is_some_and(|f| f.kind == gsm_domain::settings::Kind::Secret),
                        path: draft
                            .field(k)
                            .is_some_and(|f| f.kind == gsm_domain::settings::Kind::Path),
                        read_only: draft
                            .field(k)
                            .is_some_and(|f| f.kind == gsm_domain::settings::Kind::ReadOnly),
                        label_ja: crate::editor::label(k, false).into(),
                        label_en: crate::editor::label(k, true).into(),
                        value: crate::editor::option_label(k, v, self.english()).into(),
                        options: editor
                            .get_fields()
                            .iter()
                            .find(|f| f.key.as_str() == k)
                            .map(|f| f.options)
                            .map(|current| {
                                model(current, crate::editor::display_choices(k, self.english()))
                            })
                            .unwrap_or_else(|| {
                                ModelRc::new(VecModel::from(crate::editor::display_choices(
                                    k,
                                    self.english(),
                                )))
                            }),
                    })
                    .collect(),
            ));
        } else {
            editor.set_loaded(false);
            editor.set_notes("".into());
        }
        ui.set_choosing(self.choosing);
        ui.set_can_cancel(!snapshot.config.enabled_games.is_empty());
        ui.set_can_apply(!self.draft.is_empty());
        ui.set_chosen_count(self.draft.len() as i32);
        ui.set_overview(self.overview);
        ui.set_page_index(self.page);
        ui.set_error_text(self.error.clone().into());
        ui.set_registration_busy(self.registration.busy() || self.file_picker_open);
        let active = snapshot
            .instances
            .iter()
            .find(|v| Some(v.instance.id) == self.active);
        let active_game = active.map(|v| v.instance.game_id.as_ref()).unwrap_or("");
        if ui.get_active_game() != active_game {
            ui.set_registration_path(
                self.registration
                    .saved(active_game)
                    .map(|r| r.source_path.to_string_lossy().to_string())
                    .unwrap_or_default()
                    .into(),
            );
        }
        let registration_allowed = (!self.app.is_local()
            || active.is_some_and(|v| {
                v.active.is_none() && v.observation.process != ProcessState::Alive
            }))
            && !self.editing()
            && self.pending_navigation.is_none();
        ui.set_registration_allowed(registration_allowed);
        ui.set_can_register(
            registration_allowed
                && snapshot.instances.iter().all(|v| v.active.is_none())
                && self
                    .registration
                    .can_register_for(active_game, ui.get_registration_path().as_str()),
        );
        ui.set_registration_report(
            crate::language::registration_report(&self.registration.report, self.english()).into(),
        );
        ui.set_registration_supported(!active_game.is_empty());
        ui.set_can_create(
            !active_game.is_empty()
                && (!self.app.is_local() || self.registration.saved(active_game).is_none())
                && !self.registration.busy()
                && snapshot.instances.iter().all(|v| v.active.is_none()),
        );
        ui.set_setup_needed(
            self.app.is_local()
                && active.is_some_and(|v| {
                    self.registration
                        .saved(v.instance.game_id.as_ref())
                        .is_none()
                        || v.info.installation == gsm_domain::local::Installation::Unregistered
                }),
        );
        ui.set_registration_saved(
            self.registration
                .saved(active_game)
                .map(|r| {
                    format!(
                        "{}: {}",
                        if self.english() {
                            "Registered ID"
                        } else {
                            "登録済み ID"
                        },
                        r.id
                    )
                })
                .unwrap_or_else(|| {
                    if self.english() {
                        "Not registered"
                    } else {
                        "未登録"
                    }
                    .into()
                })
                .into(),
        );
        ui.set_can_reload(
            !self.steamcmd.busy()
                && !self.registration.busy()
                && !self.file_picker_open
                && snapshot.instances.iter().all(|v| v.active.is_none()),
        );
        let rows: Vec<_> = self
            .app
            .catalog()
            .iter()
            .filter(|g| {
                format!("{} {}", g.name, g.id)
                    .to_lowercase()
                    .contains(&self.query.to_lowercase())
            })
            .map(|g| GameRow {
                id: g.id.to_string().into(),
                title: g.name.into(),
                description: feedback::version_label(
                    snapshot
                        .instances
                        .iter()
                        .find(|v| v.instance.game_id == g.id)
                        .map(|v| &v.info.installation),
                    self.app.is_local(),
                    self.english(),
                )
                .into(),
                picked: self.draft.contains(&g.id),
                status: "".into(),
            })
            .collect();
        let pairs = rows
            .chunks(2)
            .map(|items| GamePair {
                left: items[0].clone(),
                right: items.get(1).cloned().unwrap_or_default(),
            })
            .collect();
        ui.set_catalog(model(ui.get_catalog(), pairs));
        ui.set_servers(model(
            ui.get_servers(),
            self.app
                .catalog()
                .iter()
                .filter(|g| snapshot.config.enabled_games.contains(&g.id))
                .filter_map(|g| {
                    snapshot
                        .instances
                        .iter()
                        .find(|v| v.instance.game_id == g.id)
                        .map(|view| GameRow {
                            id: g.id.to_string().into(),
                            title: g.name.into(),
                            description: feedback::version_label(
                                Some(&view.info.installation),
                                self.app.is_local(),
                                self.english(),
                            )
                            .into(),
                            picked: false,
                            status: status(view, self.english()).into(),
                        })
                })
                .collect(),
        ));
        let library: Vec<_> = ui.get_servers().iter().collect();
        ui.set_library(model(
            ui.get_library(),
            library
                .chunks(2)
                .map(|items| GamePair {
                    left: items[0].clone(),
                    right: items.get(1).cloned().unwrap_or_default(),
                })
                .collect(),
        ));
        let (title, detail, update) = active
            .and_then(|view| feedback::failure(view, &snapshot.jobs, self.english()))
            .unwrap_or_default();
        ui.set_failure_title(title.into());
        ui.set_server_error(detail.into());
        ui.set_suggest_update(update);
        if let Some(view) = snapshot
            .instances
            .iter()
            .find(|v| Some(v.instance.id) == self.active)
        {
            let game = self
                .app
                .catalog()
                .iter()
                .find(|g| g.id == view.instance.game_id)
                .unwrap();
            ui.set_active_game(game.id.to_string().into());
            ui.set_active_name(view.instance.name.clone().into());
            ui.set_active_world(view.instance.world.clone().into());
            ui.set_active_status(status(view, self.english()).into());
            let (phase, text) = if view.active.is_none()
                && let Some((game, label, status)) = &self.registration.activity
                && game == view.instance.game_id.as_ref()
            {
                activity_label(label, status)
            } else {
                activity(&snapshot.jobs, view.instance.id)
            };
            ui.set_activity_phase(phase.into());
            ui.set_activity_text(
                if self.english() {
                    if self
                        .registration
                        .activity
                        .as_ref()
                        .is_some_and(|(g, _, _)| g == view.instance.game_id.as_ref())
                        && view.active.is_none()
                    {
                        if phase == "running" {
                            "Reviewing / registering settings…".into()
                        } else {
                            format!(
                                "No active jobs · Settings {}",
                                if phase == "failed" {
                                    "failed"
                                } else {
                                    "reviewed"
                                }
                            )
                        }
                    } else if let Some(job) = snapshot
                        .jobs
                        .iter()
                        .rev()
                        .find(|j| j.instance.id == view.instance.id)
                    {
                        let label = crate::language::command(&job.command, true);
                        match job.status {
                            JobStatus::Running => format!("{label} in progress…"),
                            JobStatus::Completed => {
                                format!("No active jobs · Last: {label} completed")
                            }
                            JobStatus::Failed(_) => {
                                format!("No active jobs · Last: {label} failed")
                            }
                        }
                    } else if phase == "running" {
                        "Reviewing / registering settings…".into()
                    } else {
                        "No active jobs".into()
                    }
                } else {
                    text
                }
                .into(),
            );
            ui.set_backup_scope(
                crate::language::scope(game.id.as_ref(), self.english(), game.backup_scope).into(),
            );
            let idle = !self.steamcmd.busy()
                && view.active.is_none()
                && !self.registration.busy()
                && !self.file_picker_open
                && self.editor_receiver.is_none()
                && self.review_receiver.is_none()
                && self.save_job.is_none()
                && (!self.app.is_local() || snapshot.instances.iter().all(|v| v.active.is_none()));
            ui.set_can_start(idle && view.observation.process == ProcessState::Absent);
            ui.set_can_stop(idle && view.observation.process == ProcessState::Alive);
            ui.set_can_backup(idle && view.observation.process == ProcessState::Absent);
            ui.set_can_restore(idle && view.observation.process == ProcessState::Absent);
            ui.set_backups(model(
                ui.get_backups(),
                view.backups
                    .iter()
                    .rev()
                    .map(|b| BackupRow {
                        id: b.id.to_string().into(),
                        label: format!("{} / {}", clock(b.created_at), b.world).into(),
                    })
                    .collect(),
            ));
            ui.set_logs(if view.logs.is_empty() {
                if self.english() {
                    "No log entries yet."
                } else {
                    "まだログがありません。"
                }
                .into()
            } else {
                view.logs.join("\n").into()
            });
        }
        ui.set_jobs(model(
            ui.get_jobs(),
            snapshot
                .jobs
                .iter()
                .rev()
                .map(|j| {
                    let state = match &j.status {
                        JobStatus::Running => "実行中".to_owned(),
                        JobStatus::Completed => "完了".to_owned(),
                        JobStatus::Failed(e) => {
                            format!("{}: {e}", if self.english() { "Failed" } else { "失敗" })
                        }
                    };
                    JobRow {
                        phase: job_phase(&j.status).into(),
                        title: format!(
                            "{} · {} · {}",
                            j.instance.name,
                            crate::language::command(&j.command, self.english()),
                            if matches!(j.status, JobStatus::Running) {
                                if self.english() {
                                    "Running"
                                } else {
                                    "実行中"
                                }
                            } else if matches!(j.status, JobStatus::Completed) {
                                if self.english() {
                                    "Completed"
                                } else {
                                    "完了"
                                }
                            } else {
                                if self.english() { "Failed" } else { "失敗" }
                            }
                        )
                        .into(),
                        detail: format!(
                            "{}: {}{}",
                            if self.english() { "Started" } else { "開始" },
                            clock(j.started_at),
                            if let JobStatus::Failed(_) = &j.status {
                                format!("\n{state}")
                            } else {
                                String::new()
                            }
                        )
                        .into(),
                    }
                })
                .collect(),
        ));
        ui.set_restore_visible(self.pending_restore.is_some());
        ui.set_restore_description(
            self.pending_restore
                .as_ref()
                .map(|(_, _, text)| text.clone())
                .unwrap_or_default()
                .into(),
        );
    }
    fn submit(&mut self, command: Command) {
        if self.steamcmd.busy() || self.pending_restore.is_some() {
            return;
        }
        if let Some(id) = self.active {
            self.registration.activity = None;
            self.error = self
                .app
                .submit(id, command)
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
        }
    }
    fn cancel_selection(&mut self) {
        self.draft = self
            .app
            .snapshot()
            .config
            .enabled_games
            .into_iter()
            .collect();
        self.query.clear();
        self.choosing = false;
        self.error.clear();
    }
    fn select_game(&mut self, game_id: &str) {
        if self.pending_restore.is_some() {
            return;
        }
        let snapshot = self.app.snapshot();
        if let Some(view) = snapshot.instances.iter().find(|v| {
            v.instance.game_id.as_ref() == game_id
                && snapshot.config.enabled_games.contains(&v.instance.game_id)
        }) {
            if self.active != Some(view.instance.id) {
                self.registration.invalidate();
            }
            self.cancel_selection();
            self.active = Some(view.instance.id);
            self.overview = false;
            self.preferences_open = false;
            if self.app.is_local() && self.registration.saved(game_id).is_none() {
                self.page = 1;
            }
            self.error.clear();
        }
    }
}

pub fn run(
    app: Arc<Application>,
    registrations: Arc<gsm_infra::registration::RegistrationStore>,
    data_root: &str,
    smoke_test: bool,
    startup_error: String,
    setup_language: Option<bool>,
) -> anyhow::Result<SessionExit> {
    let ui = MainWindow::new()?;
    ui.set_data_root(data_root.into());
    ui.set_local_mode(app.is_local());
    let config = app.snapshot().config;
    let prefs_path = PathBuf::from(data_root).join(if app.is_local() {
        "local-preferences.json"
    } else {
        "mock-preferences.json"
    });
    let new_preferences = !prefs_path.exists();
    let (prefs, prefs_valid, prefs_error) = if !new_preferences {
        match Preferences::read(&prefs_path) {
            Ok(p) => (p, true, String::new()),
            Err(e) => (Preferences::default(), false, e),
        }
    } else {
        let mut prefs = Preferences::default();
        if let Some(english) = setup_language {
            prefs.language = if english { "en" } else { "ja" }.into();
        }
        (prefs, true, String::new())
    };
    let initial_game = if prefs.remember_game {
        prefs.last_game.clone()
    } else {
        None
    };
    let initial_tab = if prefs.remember_tab {
        prefs.last_tab
    } else {
        0
    };
    let registration = crate::registration::RegistrationPanel::new(registrations);
    let session = Rc::new(RefCell::new(Session {
        app: app.clone(),
        registration,
        steamcmd: steamcmd::Panel::new(std::path::Path::new(data_root), app.is_local()),
        file_picker_open: false,
        draft: config.enabled_games.iter().cloned().collect(),
        query: String::new(),
        choosing: config.enabled_games.is_empty(),
        active: config
            .instances
            .iter()
            .find(|i| config.enabled_games.contains(&i.game_id))
            .map(|i| i.id),
        overview: false,
        page: initial_tab,
        error: [startup_error, prefs_error]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        pending_restore: None,
        pending_navigation: None,
        reload: false,
        change_directory: false,
        prefs,
        prefs_writer: PreferenceWriter::new(prefs_path.clone())?,
        prefs_path,
        prefs_valid,
        preferences_open: false,
        pending_import: None,
        drafts: Drafts::new(),
        creation: None,
        creation_receiver: None,
        pending_creation: None,
        mock_settings: BTreeMap::new(),
        editor_receiver: None,
        pending_edit: None,
        review_receiver: None,
        save_job: None,
        copy_receiver: None,
        selection_receiver: None,
        transfer_receiver: None,
        import_receiver: None,
        clipboard: None,
        noticed: BTreeMap::new(),
        toast_deadline: None,
    }));
    {
        let mut state = session.borrow_mut();
        if let Some(game) = initial_game {
            state.select_game(&game);
        }
        state.apply_preferences(&ui);
        if new_preferences && setup_language.is_some() {
            state.save_preferences();
        }
    }
    features::bind_features(&ui, &session);
    steamcmd::bind(&ui, &session);
    macro_rules! handler {
        ($method:ident, |$state:ident| $body:block) => {{
            let state = session.clone();
            let weak = ui.as_weak();
            ui.$method(move || {
                if let Some(ui) = weak.upgrade() {
                    let mut $state = state.borrow_mut();
                    $body;
                    $state.remember_navigation();
                    $state.refresh(&ui);
                }
            });
        }};
        ($method:ident, |$state:ident, $arg:ident| $body:block) => {{
            let state = session.clone();
            let weak = ui.as_weak();
            ui.$method(move |$arg| {
                if let Some(ui) = weak.upgrade() {
                    let mut $state = state.borrow_mut();
                    $body;
                    $state.remember_navigation();
                    $state.refresh(&ui);
                }
            });
        }};
    }
    handler!(on_search_changed, |state, query| {
        state.query = query.to_string();
    });
    handler!(on_registration_path_changed, |state| {
        state.registration.invalidate();
    });
    {
        let session = session.clone();
        let weak = ui.as_weak();
        ui.on_browse_registration(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let mut state = session.borrow_mut();
            if state.pending_restore.is_some()
                || ui.get_registration_busy()
                || !ui.get_registration_allowed()
            {
                return;
            }
            let active = state.active;
            let original_path = ui.get_registration_path();
            let game = ui.get_active_game();
            let mut dialog = rfd::AsyncFileDialog::new()
                .set_parent(&ui.window().window_handle())
                .set_title("旧管理ツールの設定ファイルを選択")
                .add_filter(
                    if game == "arksa" {
                        "ARK profile (*.ini)"
                    } else {
                        "Config (*.toml)"
                    },
                    &[if game == "arksa" { "ini" } else { "toml" }],
                )
                .add_filter("All files", &["*"]);
            let current = std::path::Path::new(original_path.as_str());
            if current.is_absolute()
                && let Some(parent) = current.parent()
            {
                dialog = dialog.set_directory(parent);
            }
            state.file_picker_open = true;
            state.refresh(&ui);
            let picker_session = session.clone();
            let picker_weak = weak.clone();
            if let Err(error) = slint::spawn_local(async move {
                let selected = dialog.pick_file().await;
                let mut state = picker_session.borrow_mut();
                state.file_picker_open = false;
                if let Some(ui) = picker_weak.upgrade() {
                    // A late dialog result must not replace another game's input.
                    if state.active == active
                        && ui.get_active_game() == game
                        && ui.get_registration_path() == original_path
                        && ui.get_registration_allowed()
                        && let Some(file) = selected
                    {
                        if let Some(path) = file.path().to_str() {
                            ui.set_registration_path(path.into());
                            state.registration.invalidate();
                        } else {
                            state.error = "選択したファイルのパスを表示できません。".into();
                        }
                    }
                    state.refresh(&ui);
                }
            }) {
                state.file_picker_open = false;
                state.error = format!("ファイル選択を開始できませんでした: {error}");
                state.refresh(&ui);
            }
        });
    }
    // Capture the game and path at the click; workers never read the changing UI target.
    {
        let session = session.clone();
        let weak = ui.as_weak();
        ui.on_review_registration(move || {
            if let Some(ui) = weak.upgrade() {
                let mut state = session.borrow_mut();
                if state.pending_restore.is_none()
                    && !state.file_picker_open
                    && ui.get_registration_allowed()
                {
                    state.registration.review(
                        ui.get_active_game().as_str(),
                        ui.get_registration_path().to_string(),
                    );
                }
                state.refresh(&ui);
            }
        });
    }
    {
        let session = session.clone();
        let weak = ui.as_weak();
        ui.on_commit_registration(move || {
            if let Some(ui) = weak.upgrade() {
                let mut state = session.borrow_mut();
                if state.pending_restore.is_none()
                    && !state.file_picker_open
                    && ui.get_registration_allowed()
                {
                    state.registration.register(
                        ui.get_active_game().as_str(),
                        ui.get_registration_path().as_str(),
                    );
                }
                state.refresh(&ui);
            }
        });
    }
    handler!(on_toggle_game, |state, id| {
        if let Ok(id) = GameId::try_from(id.to_string()) {
            if state.draft.contains(&id) {
                state.draft.remove(&id);
            } else {
                state.draft.insert(id);
            }
        }
    });
    handler!(on_apply_selection, |state| {
        state.apply_selection();
    });
    handler!(on_cancel_selection, |state| {
        state.cancel_selection();
    });
    {
        let state = session.clone();
        let weak = ui.as_weak();
        ui.on_edit_selection(move || {
            if let Some(ui) = weak.upgrade() {
                let mut s = state.borrow_mut();
                s.navigate(Destination::Choose, &ui);
                s.refresh(&ui);
            }
        });
    }
    {
        let state = session.clone();
        let weak = ui.as_weak();
        ui.on_open_game(move |id| {
            if let Some(ui) = weak.upgrade() {
                let mut s = state.borrow_mut();
                s.navigate(Destination::Game(id.to_string()), &ui);
                s.refresh(&ui);
            }
        });
    }
    {
        let state = session.clone();
        let weak = ui.as_weak();
        ui.on_open_overview(move || {
            if let Some(ui) = weak.upgrade() {
                let mut s = state.borrow_mut();
                s.navigate(Destination::Overview, &ui);
                s.refresh(&ui);
            }
        });
    }
    {
        let state = session.clone();
        let weak = ui.as_weak();
        ui.on_change_page(move |page| {
            if let Some(ui) = weak.upgrade() {
                let mut s = state.borrow_mut();
                s.navigate(Destination::Page(page), &ui);
                s.refresh(&ui);
            }
        });
    }
    {
        let state = session.clone();
        let weak = ui.as_weak();
        ui.on_cancel_leave(move || {
            if let Some(ui) = weak.upgrade() {
                let mut s = state.borrow_mut();
                s.pending_navigation = None;
                s.refresh(&ui);
            }
        });
    }
    {
        let state = session.clone();
        let weak = ui.as_weak();
        ui.on_confirm_leave(move || {
            if let Some(ui) = weak.upgrade() {
                let mut s = state.borrow_mut();
                s.discard_and_leave(&ui);
                s.refresh(&ui);
            }
        });
    }
    handler!(on_start_server, |state| {
        state.submit(Command::Start);
    });
    handler!(on_stop_server, |state| {
        state.submit(Command::Stop);
    });
    handler!(on_update_server, |state| {
        state.submit(Command::Update);
    });
    handler!(on_recover_server, |state| {
        state.submit(Command::Recover);
    });
    handler!(on_edit_settings, |state| {
        state.submit(Command::EditSettings);
    });
    {
        let state = session.clone();
        let weak = ui.as_weak();
        ui.on_reload_registrations(move || {
            if let Some(ui) = weak.upgrade() {
                let mut s = state.borrow_mut();
                if ui.get_can_reload() {
                    s.navigate(Destination::Reload, &ui);
                }
                s.refresh(&ui);
            }
        });
    }
    handler!(on_backup_server, |state| {
        state.submit(Command::Backup);
    });
    handler!(on_request_restore, |state, backup_id| {
        if state.pending_restore.is_none()
            && let Some(view) = state
                .app
                .snapshot()
                .instances
                .iter()
                .find(|v| Some(v.instance.id) == state.active)
            && let Some(backup) = view
                .backups
                .iter()
                .find(|b| b.id.to_string() == backup_id.as_str())
        {
            let scope = state
                .app
                .catalog()
                .iter()
                .find(|g| g.id == view.instance.game_id)
                .unwrap()
                .backup_scope;
            state.pending_restore = Some((
                view.instance.id,
                backup.id,
                format!(
                    "{}: {} / {}\n{}: {}\n{}: {}",
                    if state.english() {
                        "Destination"
                    } else {
                        "復元先"
                    },
                    view.instance.name,
                    view.instance.world,
                    if state.english() {
                        "Backup"
                    } else {
                        "復元元"
                    },
                    clock(backup.created_at),
                    if state.english() { "Scope" } else { "対象" },
                    crate::language::scope(view.instance.game_id.as_ref(), state.english(), scope)
                ),
            ));
        }
    });
    handler!(on_cancel_restore, |state| {
        state.pending_restore = None;
    });
    handler!(on_confirm_restore, |state| {
        if state.steamcmd.busy() {
            return;
        }
        if let Some((instance, backup, _)) = state.pending_restore.take() {
            state.registration.activity = None;
            state.error = state
                .app
                .submit(instance, Command::Restore(backup))
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
        }
    });
    session.borrow().refresh(&ui);
    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    let tick_state = session.clone();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(150),
        move || {
            if let Some(ui) = weak.upgrade() {
                let mut state = tick_state.borrow_mut();
                state.registration.poll();
                state.steamcmd.poll();
                state.poll_features(&ui);
                if state.registration.reload_ready
                    && !state.steamcmd.busy()
                    && state.selection_receiver.is_none()
                    && state.transfer_receiver.is_none()
                    && state.import_receiver.is_none()
                    && state
                        .app
                        .snapshot()
                        .instances
                        .iter()
                        .all(|v| v.active.is_none())
                {
                    state.remember_navigation();
                    state.reload = true;
                    let _ = slint::quit_event_loop();
                }
                state.refresh(&ui);
            }
        },
    );
    if smoke_test {
        slint::Timer::single_shot(Duration::from_secs(3), || {
            println!("Slint UI smoke check completed");
            let _ = slint::quit_event_loop();
        });
    }
    let (monitor_stop, monitor_receiver) = std::sync::mpsc::channel();
    let monitored = app.clone();
    let monitor = std::thread::Builder::new()
        .name("gsm-observer".into())
        .spawn(move || {
            loop {
                monitored.refresh_backend();
                if monitor_receiver.recv_timeout(Duration::from_secs(1))
                    != Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                {
                    break;
                }
            }
        })?;
    let closing_app = app.clone();
    let close_session = session.clone();
    let weak = ui.as_weak();
    ui.window().on_close_requested(move || {
        if close_session.borrow().steamcmd.busy()
            || close_session.borrow().selection_receiver.is_some()
            || close_session.borrow().transfer_receiver.is_some()
            || close_session.borrow().import_receiver.is_some()
            || close_session.borrow().copy_receiver.is_some()
            || close_session.borrow().registration.busy()
            || close_session.borrow().save_job.is_some()
            || closing_app
                .snapshot()
                .instances
                .iter()
                .any(|v| v.active.is_some())
        {
            if let Some(ui) = weak.upgrade() {
                ui.set_error_text("操作の完了を待ってから終了してください。".into());
            }
            slint::CloseRequestResponse::KeepWindowShown
        } else if close_session.borrow().editing()
            || close_session.borrow().pending_navigation.is_some()
        {
            if let Some(ui) = weak.upgrade() {
                let mut s = close_session.borrow_mut();
                s.navigate(Destination::Close, &ui);
                s.refresh(&ui);
            }
            slint::CloseRequestResponse::KeepWindowShown
        } else {
            slint::CloseRequestResponse::HideWindow
        }
    });
    let result = ui.run();
    let _ = monitor_stop.send(());
    let _ = monitor.join();
    timer.stop();
    session.borrow_mut().registration.shutdown();
    app.shutdown();
    let saved = session.borrow().prefs_writer.flush();
    result?;
    saved.map_err(anyhow::Error::msg)?;
    let state = session.borrow();
    Ok(if state.change_directory {
        SessionExit::ChangeDirectory(state.english())
    } else if state.reload {
        SessionExit::Reload
    } else {
        SessionExit::Close
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gsm_domain::{Instance, OperationId};

    #[test]
    fn activity_follows_the_selected_instance_and_its_latest_result() {
        let selected = Instance {
            id: InstanceId::new(),
            game_id: "valheim".to_owned().try_into().unwrap(),
            name: "Selected".into(),
            world: "World".into(),
        };
        let other = Instance {
            id: InstanceId::new(),
            ..selected.clone()
        };
        let mut jobs = vec![
            Job {
                id: OperationId::new(),
                instance: selected.clone(),
                command: Command::Backup,
                status: JobStatus::Running,
                started_at: 1,
            },
            Job {
                id: OperationId::new(),
                instance: other.clone(),
                command: Command::Start,
                status: JobStatus::Failed("other error".into()),
                started_at: 2,
            },
        ];
        assert_eq!(
            activity(&jobs, selected.id),
            ("running", "バックアップを実行中…".into())
        );
        assert_eq!(activity(&jobs, other.id).0, "failed");
        assert_eq!(activity(&jobs, InstanceId::new()).0, "idle");
        jobs[0].status = JobStatus::Completed;
        let (phase, text) = activity(&jobs, selected.id);
        assert_eq!(phase, "completed");
        assert!(text.contains("現在処理中の作業はありません"));
        assert!(text.contains("バックアップ 完了"));
        jobs.push(Job {
            id: OperationId::new(),
            instance: selected.clone(),
            command: Command::Start,
            status: JobStatus::Failed("start error".into()),
            started_at: 3,
        });
        assert_eq!(activity(&jobs, selected.id).0, "failed");
        assert!(activity(&jobs, selected.id).1.contains("起動 失敗"));
    }
}
