mod catalog;
mod creation;
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

fn main() -> anyhow::Result<()> {
    let Some(options) = options::Options::parse(std::env::args_os().skip(1))? else {
        println!("{}", options::HELP);
        return Ok(());
    };
    if run_session(&options)? {
        // All settings/runtime locks have been dropped before the new GUI opens.
        std::process::Command::new(std::env::current_exe()?)
            .args(std::env::args_os().skip(1))
            .spawn()?;
    }
    Ok(())
}
fn run_session(options: &options::Options) -> anyhow::Result<bool> {
    let store = Arc::new(
        if options.local {
            FileSettingsStore::open_local(&options.data_dir)
        } else {
            FileSettingsStore::open(&options.data_dir)
        }
        .map_err(anyhow::Error::msg)?,
    );
    let data_root = store.root().display().to_string();
    let registrations =
        Arc::new(RegistrationStore::open(&options.data_dir).map_err(anyhow::Error::msg)?);
    let mut issues = vec![];
    let mut instances = vec![];
    let backend: Arc<dyn GameBackend> = if options.local {
        let mut servers = vec![];
        for reg in registrations.list() {
            let result = (|| {
                let state = options.data_dir.join("instances").join(reg.id.to_string());
                if let Some(original) = gsm_infra::local::settings::recovery_source(&reg, &state)? {
                    return catalog::resolve(&reg, &original);
                }
                let source = ConfigSource::read(&reg.source_path)?;
                if source.digest() != reg.source_sha256 {
                    let state = options.data_dir.join("instances").join(reg.id.to_string());
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
        let backend = match LocalBackend::open(&options.data_dir, helper.clone(), servers) {
            Ok(b) => b,
            Err(e) => {
                issues.push(e);
                LocalBackend::open(&options.data_dir, helper, vec![]).map_err(anyhow::Error::msg)?
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
    )
}
