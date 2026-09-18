#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod catalog;
mod creation;
mod data_location;
mod data_setup;
mod editor;
mod language;
mod options;
mod preferences;
mod registration;
mod ui;

use gsm_application::Application;
use gsm_domain::GameBackend;
use gsm_infra::{
    FileSettingsStore,
    local::LocalBackend,
    registration::{ConfigSource, RegistrationStore},
};
use gsm_mock::MockBackend;
use std::sync::Arc;
slint::include_modules!();

fn main() {
    if let Err(error) = launch() {
        eprintln!("{error:#}");
        #[cfg(windows)]
        rfd::MessageDialog::new()
            .set_title("GameServerManager")
            .set_level(rfd::MessageLevel::Error)
            .set_description(format!(
                "起動・終了処理に失敗しました / Application error\n\n{error:#}"
            ))
            .show();
        std::process::exit(1);
    }
}

fn launch() -> anyhow::Result<()> {
    let Some(options) = options::Options::parse(std::env::args_os().skip(1))? else {
        println!("{}", options::HELP);
        return Ok(());
    };
    let mut initial_error = String::new();
    let root = if let Some(root) = &options.data_dir {
        Some(root.clone())
    } else {
        match data_location::locator_path().and_then(|p| data_location::read(&p)) {
            Ok(root) => root,
            Err(e) => {
                initial_error = format!("{e:#}");
                None
            }
        }
    };
    let mut setup_language = None;
    let Some(mut root) = (match root {
        Some(root) => Some(root),
        None => data_setup::choose(None, options.local, false, &initial_error)?.map(|choice| {
            setup_language = Some(choice.english);
            choice.root
        }),
    }) else {
        return Ok(());
    };
    loop {
        match run_session(&options, &root, setup_language.take()) {
            Ok(ui::SessionExit::Close) => return Ok(()),
            Ok(ui::SessionExit::Reload) => {
                // All settings/runtime locks have been dropped. Never reuse stale CLI paths.
                std::process::Command::new(std::env::current_exe()?)
                    .args([
                        "--backend",
                        if options.local { "local" } else { "mock" },
                        "--data-dir",
                    ])
                    .arg(&root)
                    .spawn()?;
                return Ok(());
            }
            Ok(ui::SessionExit::ChangeDirectory(english)) => {
                if let Some(next) = data_setup::choose(Some(&root), options.local, english, "")? {
                    setup_language = Some(next.english);
                    root = next.root;
                }
            }
            Err(e) if options.smoke_test => return Err(e),
            Err(e) => {
                let Some(next) =
                    data_setup::choose(Some(&root), options.local, false, &format!("{e:#}"))?
                else {
                    return Ok(());
                };
                setup_language = Some(next.english);
                root = next.root;
            }
        }
    }
}
fn run_session(
    options: &options::Options,
    root: &std::path::Path,
    setup_language: Option<bool>,
) -> anyhow::Result<ui::SessionExit> {
    let store = Arc::new(
        if options.local {
            FileSettingsStore::open_local(root)
        } else {
            FileSettingsStore::open(root)
        }
        .map_err(anyhow::Error::msg)?,
    );
    let data_root = store.root().display().to_string();
    let registrations = Arc::new(RegistrationStore::open(root).map_err(anyhow::Error::msg)?);
    let mut issues = vec![];
    let mut instances = vec![];
    let backend: Arc<dyn GameBackend> = if options.local {
        let mut servers = vec![];
        for reg in registrations.list() {
            let result = (|| {
                let state = root.join("instances").join(reg.id.to_string());
                if let Some(original) = gsm_infra::local::settings::recovery_source(&reg, &state)? {
                    return catalog::resolve(&reg, &original);
                }
                let source = ConfigSource::read(&reg.source_path)?;
                if source.digest() != reg.source_sha256 {
                    let state = root.join("instances").join(reg.id.to_string());
                    if ["process.json", "operation.json", "restore-journal.json"]
                        .iter()
                        .any(|file| state.join(file).exists())
                    {
                        return Err("プロセス／未完了操作の記録がある設定が変更されています。操作開始時の設定に戻してから再読み込みしてください".into());
                    }
                    issues.push(format!(
                        "{}: 登録後に設定が変わっています。停止後に再検証・登録してください",
                        reg.game_id
                    ));
                }
                catalog::resolve(&reg, source.text())
            })();
            match result {
                Ok(spec) => servers.push((reg, spec)),
                Err(e) => issues.push(format!("{}: {e}", reg.game_id)),
            }
        }
        let helper = std::env::current_exe()?
            .parent()
            .ok_or_else(|| anyhow::anyhow!("実行フォルダーが不明です"))?
            .join("gsm-ctrlc-helper.exe");
        let backend = match LocalBackend::open(root, helper.clone(), servers) {
            Ok(b) => b,
            Err(e) => {
                issues.push(e);
                LocalBackend::open(root, helper, vec![]).map_err(anyhow::Error::msg)?
            }
        };
        instances = backend.instances();
        Arc::new(backend)
    } else {
        Arc::new(MockBackend::default())
    };
    let app = Arc::new(Application::open(catalog::games(), backend, store)?);
    if options.local {
        app.bind_registered(instances)?;
    }
    ui::run(
        app,
        registrations,
        &data_root,
        options.smoke_test,
        issues.join("\n"),
        setup_language,
    )
}
