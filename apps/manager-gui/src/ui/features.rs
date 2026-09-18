use super::*;

impl Session {
    pub(super) fn english(&self) -> bool {
        self.prefs.english()
    }
    pub(super) fn apply_preferences(&self, ui: &MainWindow) {
        ui.global::<T>().set_english(self.english());
        ui.global::<Theme>()
            .set_palette(match self.prefs.theme.as_str() {
                "blue" => 1,
                "violet" => 2,
                _ => 0,
            });
        let p = ui.global::<AppPreferences>();
        p.set_notify_completed(self.prefs.notify_completed);
        p.set_notify_failed(self.prefs.notify_failed);
        p.set_sound(self.prefs.sound);
        p.set_remember_game(self.prefs.remember_game);
        p.set_remember_tab(self.prefs.remember_tab);
        p.set_info(
            format!(
                "Version {}\nWindows · {}\n{}\n{}\n{}",
                env!("CARGO_PKG_VERSION"),
                if self.app.is_local() {
                    "Local server management"
                } else {
                    "Mock preview"
                },
                self.app
                    .catalog()
                    .iter()
                    .map(|g| g.name)
                    .collect::<Vec<_>>()
                    .join(" / "),
                if self.english() {
                    "App data:"
                } else {
                    "管理データ:"
                },
                self.prefs_path.parent().unwrap().display()
            )
            .into(),
        );
    }
    pub(super) fn save_preferences(&mut self) {
        if self.import_receiver.is_some() {
            return;
        }
        if !self.prefs_valid {
            self.error="App settings file is invalid; import a valid file to replace it / アプリ設定ファイルが不正です。正しい設定をインポートしてください".into();
            return;
        }
        if let Err(e) = self.prefs_writer.queue(self.prefs.clone()) {
            self.error = e;
        }
    }
    pub(super) fn remember_navigation(&mut self) {
        if self.import_receiver.is_some() {
            return;
        }
        let game = if self.prefs.remember_game {
            self.app
                .snapshot()
                .instances
                .iter()
                .find(|v| Some(v.instance.id) == self.active)
                .map(|v| v.instance.game_id.to_string())
        } else {
            None
        };
        let page = if self.prefs.remember_tab {
            self.page
        } else {
            0
        };
        if game != self.prefs.last_game || page != self.prefs.last_tab {
            self.prefs.last_game = game;
            self.prefs.last_tab = page;
            self.save_preferences();
        }
    }
    pub(super) fn toast(&mut self, ui: &MainWindow, message: String) {
        ui.set_notification(message.into());
        self.toast_deadline = Some(Instant::now() + Duration::from_secs(7));
        if self.prefs.sound {
            notification_sound();
        }
    }
    pub(super) fn editor_key(&self, ui: &MainWindow) -> (String, bool) {
        (ui.get_active_game().to_string(), self.page == 2)
    }
    pub(super) fn poll_features(&mut self, ui: &MainWindow) {
        self.poll_background(ui);
        if let Some(Err(error)) = self.prefs_writer.poll() {
            self.error = format!(
                "App settings could not be saved / アプリ設定を保存できませんでした: {error}"
            );
        }
        if let Some(rx) = &self.creation_receiver {
            match rx.try_recv() {
                Ok(result) => {
                    self.creation_receiver = None;
                    match result {
                        Ok(plan) => {
                            if self.creation.as_ref().is_some_and(|d| d.game == plan.game) {
                                ui.global::<ConfigEditor>()
                                    .set_summary(plan.summary.clone().into());
                                self.pending_creation = Some(plan);
                                ui.global::<ConfigEditor>().set_confirming(true);
                            }
                        }
                        Err(e) => ui.global::<ConfigEditor>().set_message(e.into()),
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.creation_receiver = None;
                    ui.global::<ConfigEditor>().set_message(
                        "作成設定の検証が中断されました / Configuration review interrupted".into(),
                    );
                }
                _ => (),
            }
        }
        if let Some(rx) = &self.editor_receiver {
            match rx.try_recv() {
                Ok((key, result)) => {
                    self.editor_receiver = None;
                    match result {
                        Ok(draft) => {
                            self.drafts.insert(key, draft);
                            ui.global::<ConfigEditor>().set_message("".into());
                        }
                        Err(e) => ui.global::<ConfigEditor>().set_message(e.into()),
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.editor_receiver = None;
                    self.error = "Editor worker interrupted / 編集処理が中断されました".into();
                }
                _ => (),
            }
        }
        if let Some(rx) = &self.review_receiver {
            match rx.try_recv() {
                Ok((key, result)) => {
                    self.review_receiver = None;
                    if key == self.editor_key(ui) && self.drafts.contains_key(&key) {
                        match result {
                            Ok((write, summary, documents)) => {
                                self.pending_edit = Some((key, Arc::new(write), documents));
                                ui.global::<ConfigEditor>().set_summary(summary.into());
                                ui.global::<ConfigEditor>().set_confirming(true);
                            }
                            Err(e) => ui.global::<ConfigEditor>().set_message(e.into()),
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.review_receiver = None;
                    self.error = "設定検証が中断されました / Settings review interrupted".into();
                }
                _ => (),
            }
        }
        if let Some(rx) = &self.copy_receiver {
            match rx.try_recv() {
                Ok(result) => {
                    self.copy_receiver = None;
                    match result {
                        Ok(clipboard) => {
                            self.clipboard = Some(clipboard);
                            self.toast(
                                ui,
                                if self.english() {
                                    "Full log copied"
                                } else {
                                    "ログ全文をコピーしました"
                                }
                                .into(),
                            );
                        }
                        Err(e) => self.error = format!("Copy failed / コピーできませんでした: {e}"),
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.copy_receiver = None;
                    self.error = "Log reader interrupted / ログの読み込みが中断されました".into();
                }
                _ => (),
            }
        }
        let snapshot = self.app.snapshot();
        if let Some((id, game, contents)) = &self.save_job
            && let Some(job) = snapshot.jobs.iter().find(|j| j.id == *id)
            && job.status != JobStatus::Running
        {
            let (game, contents) = (game.clone(), contents.clone());
            self.save_job = None;
            if job.status == JobStatus::Completed {
                self.drafts.retain(|(g, _), _| g != &game);
                if self.app.is_local() {
                    self.registration
                        .register_applied(&game, contents["manager"].clone());
                } else {
                    self.mock_settings.insert(game, contents);
                }
                ui.global::<ConfigEditor>().set_message(
                    if self.english() {
                        "Settings saved"
                    } else {
                        "設定を保存しました"
                    }
                    .into(),
                );
            }
        }
        for job in &snapshot.jobs {
            let previous = self.noticed.insert(job.id, job.status.clone());
            if previous.as_ref() != Some(&job.status) && job.status != JobStatus::Running {
                let failed = matches!(job.status, JobStatus::Failed(_));
                if (failed && self.prefs.notify_failed) || (!failed && self.prefs.notify_completed)
                {
                    self.toast(
                        ui,
                        format!(
                            "{} · {} · {}",
                            job.instance.name,
                            crate::language::command(&job.command, self.english()),
                            if failed {
                                if self.english() { "Failed" } else { "失敗" }
                            } else if self.english() {
                                "Completed"
                            } else {
                                "完了"
                            }
                        ),
                    );
                }
            }
        }
        self.noticed
            .retain(|id, _| snapshot.jobs.iter().any(|j| j.id == *id));
        if self.toast_deadline.is_some_and(|t| Instant::now() >= t) {
            ui.set_notification("".into());
            self.toast_deadline = None;
        }
    }
}
#[cfg(windows)]
fn notification_sound() {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBeep(kind: u32) -> i32;
    }
    unsafe {
        MessageBeep(0x40);
    }
}
#[cfg(not(windows))]
fn notification_sound() {}

pub(super) fn bind_features(ui: &MainWindow, session: &Rc<RefCell<Session>>) {
    macro_rules! bind {
        ($object:expr,$event:ident,|$s:ident,$u:ident $(,$arg:ident)*| $body:block)=>{{
            let state=session.clone();let weak=ui.as_weak();
            $object.$event(move |$($arg),*|{if let Some($u)=weak.upgrade(){let mut $s=state.borrow_mut();$body;$s.refresh(&$u);}});
        }};
    }
    bind!(ui, on_toggle_preferences, |s, u| {
        s.navigate(Destination::Preferences, &u);
        let _ = &u;
    });
    bind!(ui, on_language_changed, |s, u, en| {
        s.prefs.language = if en { "en" } else { "ja" }.into();
        s.save_preferences();
        s.apply_preferences(&u);
    });
    bind!(ui, on_dismiss_notification, |s, u| {
        s.toast_deadline = None;
        u.set_notification("".into());
    });
    bind!(ui.global::<AppPreferences>(), on_changed, |s, u| {
        let p = u.global::<AppPreferences>();
        s.prefs.theme = match u.global::<Theme>().get_palette() {
            1 => "blue",
            2 => "violet",
            _ => "turquoise",
        }
        .into();
        s.prefs.notify_completed = p.get_notify_completed();
        s.prefs.notify_failed = p.get_notify_failed();
        s.prefs.sound = p.get_sound();
        s.prefs.remember_game = p.get_remember_game();
        s.prefs.remember_tab = p.get_remember_tab();
        s.remember_navigation();
        s.save_preferences();
    });
    bind!(
        ui.global::<AppPreferences>(),
        on_test_notification,
        |s, u| {
            let message = if s.english() {
                "Example: job completed"
            } else {
                "通知の表示例: 作業が完了しました"
            }
            .into();
            s.toast(&u, message);
        }
    );
    bind!(ui.global::<AppPreferences>(), on_cancel_import, |s, u| {
        s.pending_import = None;
        u.global::<AppPreferences>().set_transfer_ready(false);
        u.global::<AppPreferences>().set_transfer_summary("".into());
    });
    bind!(ui.global::<AppPreferences>(), on_apply_import, |s, u| {
        if s.import_receiver.is_none()
            && !u.global::<AppPreferences>().get_transfer_busy()
            && let Some(p) = s.pending_import.take()
        {
            match s.prefs_writer.queue_confirmed(p.clone()) {
                Ok(receiver) => {
                    s.import_receiver = Some((p, receiver));
                    u.global::<AppPreferences>().set_transfer_busy(true);
                }
                Err(e) => {
                    s.pending_import = Some(p);
                    s.error = e;
                }
            }
        }
    });
    for export in [false, true] {
        let state = session.clone();
        let weak = ui.as_weak();
        let callback = move || {
            let Some(ui) = weak.upgrade() else { return };
            if ui.global::<AppPreferences>().get_transfer_busy() {
                return;
            }
            ui.global::<AppPreferences>().set_transfer_busy(true);
            let prefs = state.borrow().prefs.clone();
            let state = state.clone();
            let weak = weak.clone();
            let dialog = rfd::AsyncFileDialog::new()
                .set_parent(&ui.window().window_handle())
                .set_title(if export {
                    "Export app settings / アプリ設定のエクスポート"
                } else {
                    "Import app settings / アプリ設定のインポート"
                })
                .add_filter("JSON", &["json"])
                .set_file_name("gsm-app-preferences.json");
            let _ = slint::spawn_local(async move {
                let file = if export {
                    dialog.save_file().await
                } else {
                    dialog.pick_file().await
                };
                if let Some(ui) = weak.upgrade() {
                    let mut s = state.borrow_mut();
                    let p = ui.global::<AppPreferences>();
                    if let Some(file) = file {
                        let path = file.path().to_path_buf();
                        let (tx, rx) = mpsc::channel();
                        match std::thread::Builder::new()
                            .name("gsm-preference-transfer".into())
                            .spawn(move || {
                                let result = if export {
                                    prefs.save(&path).map(|()| None)
                                } else {
                                    Preferences::read(&path).map(Some)
                                };
                                let _ = tx.send(result);
                            }) {
                            Ok(_) => s.transfer_receiver = Some(rx),
                            Err(e) => {
                                s.error = e.to_string();
                                p.set_transfer_busy(false);
                            }
                        }
                    } else {
                        p.set_transfer_busy(false);
                    }
                    s.refresh(&ui);
                }
            });
        };
        if export {
            ui.global::<AppPreferences>().on_export_settings(callback);
        } else {
            ui.global::<AppPreferences>().on_import_settings(callback);
        }
    }
    bind!(ui, on_copy_log, |s, u| {
        if s.copy_receiver.is_none()
            && let Some(id) = s.active
        {
            let app = s.app.clone();
            let clipboard = s.clipboard.take();
            let (tx, rx) = mpsc::channel();
            match std::thread::Builder::new()
                .name("gsm-full-log".into())
                .spawn(move || {
                    let result = app.full_log(id).and_then(|text| {
                        let mut clipboard = match clipboard {
                            Some(clipboard) => clipboard,
                            None => arboard::Clipboard::new().map_err(|e| e.to_string())?,
                        };
                        clipboard.set_text(text).map_err(|e| e.to_string())?;
                        Ok(clipboard)
                    });
                    let _ = tx.send(result);
                }) {
                Ok(_) => s.copy_receiver = Some(rx),
                Err(e) => s.error = e.to_string(),
            }
        }
        let _ = &u;
    });
    bind!(ui.global::<ConfigEditor>(), on_load, |s, u, world| {
        if u.get_can_backup() && s.editor_receiver.is_none() {
            let key = (u.get_active_game().to_string(), world);
            {
                let local = s.app.is_local();
                let reg = s.registration.saved(&key.0);
                let mock_source = s.mock_settings.get(&key.0).cloned();
                let (tx, rx) = mpsc::channel();
                match std::thread::Builder::new()
                    .name("gsm-edit-read".into())
                    .spawn(move || {
                        let result = Draft::load(&key.0, reg, local, world, mock_source);
                        let _ = tx.send((key, result));
                    }) {
                    Ok(_) => s.editor_receiver = Some(rx),
                    Err(e) => s.error = e.to_string(),
                }
            }
        }
    });
    bind!(
        ui.global::<ConfigEditor>(),
        on_changed,
        |s, u, key, value| {
            if s.pending_navigation.is_some()
                || s.review_receiver.is_some()
                || s.creation_busy()
                || s.save_job.is_some()
            {
                return;
            }
            let target = s.editor_key(&u);
            let raw = crate::editor::choice_value(&key, &value, s.english());
            if let Some(draft) = &mut s.creation {
                draft.change(&key, raw.clone());
            }
            if let Some(draft) = s.drafts.get_mut(&target) {
                draft.change(&key, raw);
            }
        }
    );
    bind!(ui.global::<ConfigEditor>(), on_cancel, |s, u| {
        let key = s.editor_key(&u);
        s.drafts.remove(&key);
        s.creation = None;
        s.pending_creation = None;
        s.pending_edit = None;
        u.global::<ConfigEditor>().set_message("".into());
    });
    {
        let state = session.clone();
        let weak = ui.as_weak();
        ui.global::<ConfigEditor>().on_browse(move |field| {
            let Some(ui) = weak.upgrade() else { return };
            if ui.global::<ConfigEditor>().get_busy() {
                return;
            }
            let key = state.borrow().editor_key(&ui);
            let state = state.clone();
            let weak = weak.clone();
            let dialog = rfd::AsyncFileDialog::new()
                .set_parent(&ui.window().window_handle())
                .set_file_name(if key.0 == "arksa" {
                    "profile.ini"
                } else {
                    "config.toml"
                });
            let _ = slint::spawn_local(async move {
                let file = if field == "new_config" {
                    dialog.save_file().await
                } else if field == "steamcmd" || field == "log_file" {
                    dialog.pick_file().await
                } else {
                    dialog.pick_folder().await
                };
                if let Some(ui) = weak.upgrade()
                    && let Some(file) = file
                {
                    let mut s = state.borrow_mut();
                    if !ui.global::<ConfigEditor>().get_busy() && key == s.editor_key(&ui) {
                        let value = file.path().to_string_lossy().into_owned();
                        if let Some(draft) = &mut s.creation {
                            draft.change(&field, value.clone());
                        }
                        if let Some(draft) = s.drafts.get_mut(&key) {
                            draft.change(&field, value);
                        }
                    }
                    s.refresh(&ui);
                }
            });
        });
    }
    bind!(ui, on_new_configuration, |s, u| {
        if u.get_can_create() {
            s.navigate(Destination::Create(u.get_active_game().to_string()), &u);
        }
    });
    bind!(ui.global::<ConfigEditor>(), on_review, |s, u| {
        if let Some(draft) = s.creation.clone() {
            if s.creation_busy() {
                return;
            }
            let local = s.app.is_local();
            let english = s.english();
            let data = s.prefs_path.parent().unwrap().to_path_buf();
            let registrations = crate::catalog::games()
                .iter()
                .filter_map(|g| s.registration.saved(g.id.as_ref()))
                .collect();
            let (tx, rx) = mpsc::channel();
            match std::thread::Builder::new()
                .name("gsm-create-review".into())
                .spawn(move || {
                    let _ = tx.send(draft.review(data, registrations, local, english));
                }) {
                Ok(_) => s.creation_receiver = Some(rx),
                Err(e) => s.error = e.to_string(),
            };
            s.refresh(&u);
            return;
        }

        if u.get_can_backup() && s.review_receiver.is_none() {
            let key = s.editor_key(&u);
            if let Some(mut draft) = s.drafts.get(&key).cloned() {
                draft.validation_root = s.prefs_path.parent().map(PathBuf::from);
                draft.other_registrations = crate::catalog::games()
                    .iter()
                    .filter_map(|g| s.registration.saved(g.id.as_ref()))
                    .collect();
                let english = s.english();
                let (tx, rx) = mpsc::channel();
                match std::thread::Builder::new()
                    .name("gsm-edit-review".into())
                    .spawn(move || {
                        let result = draft.reviewed(english);
                        let _ = tx.send((key, result));
                    }) {
                    Ok(_) => s.review_receiver = Some(rx),
                    Err(e) => s.error = e.to_string(),
                }
            }
        }
    });
    bind!(ui.global::<ConfigEditor>(), on_confirm, |s, u| {
        u.global::<ConfigEditor>().set_confirming(false);
        if let Some(plan) = s.pending_creation.take() {
            if !s.creation.as_ref().is_some_and(|d| d.game == plan.game) || s.creation_busy() {
                return;
            }
            if s.app.is_local() {
                let data = s.prefs_path.parent().unwrap().to_path_buf();
                s.registration.create(plan, data);
            } else {
                s.mock_settings.insert(plan.game, plan.documents);
                s.creation = None;
                u.global::<ConfigEditor>().set_message("模擬設定を作成しました。実ファイルは作成しません / Mock configuration created; no files were written".into());
            }
            s.refresh(&u);
            return;
        }

        if let Some((key, change, documents)) = s.pending_edit.take()
            && key == s.editor_key(&u)
            && u.get_can_backup()
            && let Some(id) = s.active
        {
            s.registration.activity = None;
            match s.app.submit(id, Command::WriteSettings(change.clone())) {
                Ok(job) => {
                    s.save_job = Some((job, key.0, documents));
                }
                Err(e) => s.error = e.to_string(),
            }
        }
    });
}
