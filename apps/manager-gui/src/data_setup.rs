//! First-run and directory-switch UI. File dialogs and validation run off-thread.
use crate::{DataSetupWindow, T, data_location};
use slint::ComponentHandle;
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::mpsc,
    time::Duration,
};

pub struct Selection {
    pub root: PathBuf,
    pub english: bool,
}

enum Reply {
    Picked(Option<PathBuf>),
    Ready(PathBuf),
    Failed(String),
}

pub fn choose(
    current: Option<&Path>,
    local: bool,
    english: bool,
    error: &str,
) -> anyhow::Result<Option<Selection>> {
    let ui = DataSetupWindow::new()?;
    ui.global::<T>().set_english(english);
    ui.set_changing(current.is_some() && error.is_empty());
    ui.set_local_mode(local);
    ui.set_folder(
        current
            .map(|p| p.display().to_string())
            .unwrap_or_default()
            .into(),
    );
    ui.set_error(error.into());
    let chosen = Rc::new(RefCell::new(None));
    let (sender, receiver) = mpsc::channel();
    let weak = ui.as_weak();
    let pick_sender = sender.clone();
    ui.on_browse(move || {
        let Some(ui) = weak.upgrade() else { return };
        if ui.get_busy() {
            return;
        }
        ui.set_busy(true);
        let sender = pick_sender.clone();
        if let Err(e) = std::thread::Builder::new()
            .name("gsm-data-picker".into())
            .spawn(move || {
                let _ = sender.send(Reply::Picked(rfd::FileDialog::new().pick_folder()));
            })
        {
            ui.set_busy(false);
            ui.set_error(e.to_string().into());
        }
    });
    let weak = ui.as_weak();
    let current = current.map(Path::to_path_buf);
    ui.on_accept(move || {
        let Some(ui) = weak.upgrade() else { return };
        if ui.get_busy() {
            return;
        }
        let folder = PathBuf::from(ui.get_folder().as_str());
        let old = current.clone();
        let sender = sender.clone();
        ui.set_busy(true);
        ui.set_error("".into());
        if let Err(e) = std::thread::Builder::new()
            .name("gsm-data-setup".into())
            .spawn(move || {
                let result = (|| {
                    let locator = if local {
                        Some(data_location::locator_path()?)
                    } else {
                        None
                    };
                    data_location::prepare(&folder, old.as_deref(), local, locator.as_deref())
                })();
                let _ = sender.send(match result {
                    Ok(root) => Reply::Ready(root),
                    Err(e) => Reply::Failed(format!("{e:#}")),
                });
            })
        {
            ui.set_busy(false);
            ui.set_error(e.to_string().into());
        }
    });
    let weak = ui.as_weak();
    ui.on_cancel(move || {
        if let Some(ui) = weak.upgrade()
            && !ui.get_busy()
        {
            let _ = ui.hide();
        }
    });
    let weak = ui.as_weak();
    ui.window().on_close_requested(move || {
        if weak.upgrade().is_some_and(|ui| ui.get_busy()) {
            slint::CloseRequestResponse::KeepWindowShown
        } else {
            slint::CloseRequestResponse::HideWindow
        }
    });
    let weak = ui.as_weak();
    let result = chosen.clone();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(100),
        move || {
            let Some(ui) = weak.upgrade() else { return };
            if let Ok(reply) = receiver.try_recv() {
                ui.set_busy(false);
                match reply {
                    Reply::Picked(Some(path)) => ui.set_folder(path.display().to_string().into()),
                    Reply::Picked(None) => {}
                    Reply::Failed(message) => ui.set_error(message.into()),
                    Reply::Ready(path) => {
                        *result.borrow_mut() = Some(Selection {
                            root: path,
                            english: ui.global::<T>().get_english(),
                        });
                        let _ = ui.hide();
                    }
                }
            }
        },
    );
    ui.run()?;
    timer.stop();
    let result = chosen.borrow_mut().take();
    Ok(result)
}
