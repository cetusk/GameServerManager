//! UI callbacks enqueue IO; the timer only consumes completed results.
use super::*;

fn completed<T>(
    receiver: &mut Option<mpsc::Receiver<Result<T, String>>>,
) -> Option<Result<T, String>> {
    let result = match receiver.as_ref()?.try_recv() {
        Ok(result) => result,
        Err(mpsc::TryRecvError::Empty) => return None,
        Err(mpsc::TryRecvError::Disconnected) => {
            Err("Background worker interrupted / バックグラウンド処理が中断されました".into())
        }
    };
    *receiver = None;
    Some(result)
}
impl Session {
    pub(super) fn apply_selection(&mut self) {
        if self.selection_receiver.is_some() {
            return;
        }
        let app = self.app.clone();
        let games = self.draft.clone();
        let (tx, rx) = mpsc::channel();
        match std::thread::Builder::new()
            .name("gsm-game-selection".into())
            .spawn(move || {
                let _ = tx.send(app.select_games(games).map_err(|e| e.to_string()));
            }) {
            Ok(_) => self.selection_receiver = Some(rx),
            Err(e) => self.error = e.to_string(),
        }
    }
    pub(super) fn poll_background(&mut self, ui: &MainWindow) {
        if let Some(result) = completed(&mut self.selection_receiver) {
            match result {
                Ok(()) => {
                    let config = self.app.snapshot().config;
                    self.draft = config.enabled_games.iter().cloned().collect();
                    if !config.instances.iter().any(|i| {
                        Some(i.id) == self.active && config.enabled_games.contains(&i.game_id)
                    }) {
                        self.active = config
                            .instances
                            .iter()
                            .find(|i| config.enabled_games.contains(&i.game_id))
                            .map(|i| i.id);
                        self.overview = true;
                    }
                    if self.choosing {
                        self.choosing = false;
                        self.page = 0;
                        self.overview = config.enabled_games.len() != 1;
                    }
                    self.remember_navigation();
                }
                Err(e) => self.error = e,
            }
        }
        if let Some(result) = completed(&mut self.transfer_receiver) {
            let p = ui.global::<AppPreferences>();
            p.set_transfer_busy(false);
            match result {
                Ok(Some(next)) => {
                    p.set_transfer_summary(
                        format!(
                            "{}\n{}\n\n{}\n{}",
                            if self.english() {
                                "Current settings"
                            } else {
                                "現在の設定"
                            },
                            self.prefs.summary(),
                            if self.english() {
                                "Imported settings"
                            } else {
                                "読み込む設定"
                            },
                            next.summary()
                        )
                        .into(),
                    );
                    p.set_transfer_ready(next != self.prefs);
                    self.pending_import = Some(next);
                }
                Ok(None) => p.set_transfer_summary(
                    if self.english() {
                        "Settings exported"
                    } else {
                        "設定を書き出しました"
                    }
                    .into(),
                ),
                Err(e) => {
                    self.pending_import = None;
                    p.set_transfer_ready(false);
                    p.set_transfer_summary(e.into());
                }
            }
        }
        let result = self
            .import_receiver
            .as_ref()
            .and_then(|(_, rx)| match rx.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                    "Settings writer interrupted / 設定保存が中断されました".into(),
                )),
            });
        if let Some(result) = result {
            let (prefs, _) = self.import_receiver.take().unwrap();
            let p = ui.global::<AppPreferences>();
            p.set_transfer_busy(false);
            match result {
                Ok(()) => {
                    self.prefs = prefs;
                    self.prefs_valid = true;
                    self.apply_preferences(ui);
                    // Apply remembered navigation only if the user stayed on settings.
                    if self.preferences_open {
                        if self.prefs.remember_game
                            && let Some(game) = self.prefs.last_game.clone()
                        {
                            self.select_game(&game);
                            self.preferences_open = true;
                        }
                        if self.prefs.remember_tab {
                            self.page = self.prefs.last_tab;
                        }
                    }
                    p.set_transfer_ready(false);
                    p.set_transfer_summary(
                        if self.english() {
                            "Settings imported"
                        } else {
                            "設定を読み込みました"
                        }
                        .into(),
                    );
                }
                Err(e) => {
                    self.pending_import = Some(prefs);
                    self.error = e;
                }
            }
        }
    }
}
