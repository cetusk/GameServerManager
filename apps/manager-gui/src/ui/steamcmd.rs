//! Non-blocking shared-tool settings. No game-specific path is overwritten here.
use super::*;
use gsm_infra::steamcmd::{self as infra, Progress};

enum Reply {
    Progress(Progress),
    Picked(Option<PathBuf>),
    Saved(PathBuf),
    Failed(String),
}
pub(super) struct Panel {
    pub path: String,
    pub saved: Option<String>,
    settings: PathBuf,
    receiver: Option<mpsc::Receiver<Reply>>,
    progress: Option<Progress>,
    message: String,
    phase: &'static str,
}
impl Panel {
    pub fn new(root: &std::path::Path, local: bool) -> Self {
        let settings = infra::settings_path(root, local);
        let (saved, message) = match infra::read_setting(&settings) {
            Ok(path) => (path.map(|p| p.display().to_string()), String::new()),
            Err(e) => (None, e),
        };
        Self {
            path: saved.clone().unwrap_or_else(|| infra::DEFAULT_EXE.into()),
            saved,
            settings,
            receiver: None,
            progress: None,
            phase: if message.is_empty() { "idle" } else { "failed" },
            message,
        }
    }
    pub fn busy(&self) -> bool {
        self.receiver.is_some()
    }
    pub fn default_path(&self) -> &str {
        self.saved.as_deref().unwrap_or(infra::DEFAULT_EXE)
    }
    fn spawn(
        &mut self,
        task: impl FnOnce(mpsc::Sender<Reply>) -> Result<(), String> + Send + 'static,
    ) {
        if self.busy() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.message.clear();
        self.progress = None;
        match std::thread::Builder::new()
            .name("gsm-steamcmd".into())
            .spawn(move || {
                if let Err(e) = task(tx.clone()) {
                    let _ = tx.send(Reply::Failed(e));
                }
            }) {
            Ok(_) => {
                self.receiver = Some(rx);
                self.phase = "running";
            }
            Err(e) => {
                self.message = e.to_string();
                self.phase = "failed";
            }
        }
    }
    fn browse(&mut self, existing: bool) {
        self.spawn(move |tx| {
            let dialog = rfd::FileDialog::new();
            let picked = if existing {
                dialog.add_filter("SteamCMD", &["exe"]).pick_file()
            } else {
                dialog.pick_folder().map(|p| p.join("steamcmd.exe"))
            };
            tx.send(Reply::Picked(picked)).map_err(|e| e.to_string())
        });
    }
    fn apply(&mut self, download: bool) {
        let path = PathBuf::from(&self.path);
        let settings = self.settings.clone();
        self.spawn(move |tx| {
            if download {
                infra::install(&path, |p| { let _ = tx.send(Reply::Progress(p)); })?;
            }
            infra::save_setting(&settings, &path).map_err(|e| if download {
                format!("SteamCMD は配置済みですが共通設定を保存できません。「このパスを保存」で再試行してください / SteamCMD deployed; retry Save this path: {e}")
            } else { e })?;
            tx.send(Reply::Saved(path)).map_err(|e| e.to_string())
        });
    }
    pub fn poll(&mut self) {
        loop {
            let Some(rx) = &self.receiver else { return };
            let reply = match rx.try_recv() {
                Ok(reply) => reply,
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => Reply::Failed(
                    "SteamCMD の処理が中断されました / SteamCMD operation interrupted".into(),
                ),
            };
            match reply {
                Reply::Progress(progress) => {
                    self.progress = Some(progress);
                    continue;
                }
                Reply::Picked(path) => {
                    if let Some(path) = path {
                        self.path = path.display().to_string();
                    }
                    self.phase = "idle";
                }
                Reply::Saved(path) => {
                    self.path = path.display().to_string();
                    self.saved = Some(self.path.clone());
                    self.phase = "completed";
                }
                Reply::Failed(e) => {
                    self.message = e;
                    self.phase = "failed";
                }
            }
            self.receiver = None;
            self.progress = None;
        }
    }
    pub fn refresh(&self, ui: &MainWindow, can_work: bool, supported: bool, english: bool) {
        let p = ui.global::<AppPreferences>();
        p.set_steamcmd_path(self.path.clone().into());
        p.set_steamcmd_saved(self.saved.clone().unwrap_or_default().into());
        p.set_steamcmd_busy(self.busy());
        p.set_steamcmd_supported(supported);
        p.set_steamcmd_can_work(can_work && supported && !self.busy());
        p.set_steamcmd_phase(self.phase.into());
        let message = if !self.message.is_empty() {
            self.message.clone()
        } else if let Some(progress) = &self.progress {
            match progress {
                Progress::Download { bytes, total } => format!(
                    "{}: {} KiB{}",
                    if english {
                        "Downloading"
                    } else {
                        "ダウンロード中"
                    },
                    bytes / 1024,
                    total
                        .map(|t| format!(" / {} KiB", t / 1024))
                        .unwrap_or_default()
                ),
                Progress::Extract => if english {
                    "Checking and extracting…"
                } else {
                    "ファイルを検証・展開中…"
                }
                .into(),
            }
        } else if self.busy() {
            if english {
                "Working…"
            } else {
                "処理中…"
            }
            .into()
        } else if self.phase == "completed" {
            if english {
                "Shared path saved. Used as the default for new server configurations."
            } else {
                "共通パスを保存しました。新しく作成するサーバー設定の初期値になります。"
            }
            .into()
        } else if self.saved.as_deref() != Some(self.path.as_str()) {
            if english {
                "This path is not saved. Choose an existing EXE or download into an empty folder."
            } else {
                "このパスは未保存です。既存の exe を選ぶか、空のフォルダーにダウンロードしてください。"
            }
            .into()
        } else {
            String::new()
        };
        p.set_steamcmd_status(message.into());
    }
}

pub(super) fn bind(ui: &MainWindow, session: &Rc<RefCell<Session>>) {
    let s = session.clone();
    let weak = ui.as_weak();
    ui.global::<AppPreferences>()
        .on_steamcmd_changed(move |value| {
            let mut s = s.borrow_mut();
            if !s.steamcmd.busy() {
                s.steamcmd.path = value.into();
                s.steamcmd.message.clear();
                s.steamcmd.phase = "idle";
            }
            if let Some(ui) = weak.upgrade() {
                s.refresh(&ui);
            }
        });
    let s = session.clone();
    let weak = ui.as_weak();
    ui.global::<AppPreferences>()
        .on_steamcmd_browse(move |existing| {
            let mut s = s.borrow_mut();
            if !s.steamcmd.busy() {
                s.steamcmd.browse(existing);
            }
            if let Some(ui) = weak.upgrade() {
                s.refresh(&ui);
            }
        });
    let s = session.clone();
    let weak = ui.as_weak();
    ui.global::<AppPreferences>()
        .on_steamcmd_apply(move |download| {
            let Some(ui) = weak.upgrade() else { return };
            let mut s = s.borrow_mut();
            if ui.global::<AppPreferences>().get_steamcmd_can_work() && !s.steamcmd.busy() {
                s.steamcmd.apply(download);
            }
            s.refresh(&ui);
        });
    let s = session.clone();
    let weak = ui.as_weak();
    ui.global::<AppPreferences>().on_steamcmd_default(move || {
        let mut s = s.borrow_mut();
        if !s.steamcmd.busy() {
            s.steamcmd.path = infra::DEFAULT_EXE.into();
            s.steamcmd.message.clear();
            s.steamcmd.phase = "idle";
        }
        if let Some(ui) = weak.upgrade() {
            s.refresh(&ui);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_saved_paths_become_defaults_and_failed_worker_keeps_previous_setting() {
        let dir = tempfile::tempdir().unwrap();
        let path = infra::settings_path(dir.path(), true);
        let saved = dir.path().join("existing/steamcmd.exe");
        let bytes =
            serde_json::to_vec(&serde_json::json!({"version":1,"executable":saved})).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let mut panel = Panel::new(dir.path(), true);
        assert_eq!(panel.default_path(), saved.to_str().unwrap());
        panel.path = dir
            .path()
            .join("missing/steamcmd.exe")
            .display()
            .to_string();
        assert_eq!(panel.default_path(), saved.to_str().unwrap());
        panel.apply(false);
        let deadline = Instant::now() + Duration::from_secs(5);
        while panel.busy() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
            panel.poll();
        }
        assert_eq!(panel.phase, "failed");
        assert!(!panel.message.is_empty());
        assert_eq!(panel.default_path(), saved.to_str().unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(
            Panel::new(dir.path(), false).default_path(),
            infra::DEFAULT_EXE
        );
    }
}
