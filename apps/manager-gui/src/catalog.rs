use gsm_domain::GameDescriptor;
pub fn games() -> Vec<GameDescriptor> {
    vec![
        #[cfg(feature = "arksa")]
        game_arksa::descriptor(),
        #[cfg(feature = "valheim")]
        game_valheim::descriptor(),
        #[cfg(feature = "windrose")]
        game_windrose::descriptor(),
        #[cfg(feature = "satisfactory")]
        game_satisfactory::descriptor(),
        #[cfg(feature = "conan")]
        game_conan::descriptor(),
    ]
}

pub fn resolve(
    reg: &gsm_domain::Registration,
    text: &str,
) -> Result<gsm_domain::local::LocalServer, String> {
    match reg.game_id.as_ref() {
        #[cfg(feature = "arksa")]
        "arksa" => game_arksa::local::resolve(reg, text),
        #[cfg(feature = "valheim")]
        "valheim" => game_valheim::local::resolve(reg, text),
        #[cfg(feature = "windrose")]
        "windrose" => game_windrose::local::resolve(reg, text),
        #[cfg(feature = "satisfactory")]
        "satisfactory" => game_satisfactory::local::resolve(reg, text),
        #[cfg(feature = "conan")]
        "conan" => game_conan::local::resolve(reg, text),
        _ => Err("このビルドには対象ゲームの実管理機能がありません".into()),
    }
}

pub fn settings_schema(game: &str) -> Vec<gsm_domain::settings::Field> {
    match game {
        #[cfg(feature = "valheim")]
        "valheim" => game_valheim::settings::schema(),
        #[cfg(feature = "arksa")]
        "arksa" => game_arksa::settings::schema(),
        #[cfg(feature = "conan")]
        "conan" => game_conan::settings::schema(),
        #[cfg(feature = "windrose")]
        "windrose" => game_windrose::settings::schema(),
        #[cfg(feature = "satisfactory")]
        "satisfactory" => game_satisfactory::settings::schema(),
        _ => vec![],
    }
}
pub fn synchronize_settings(
    game: &str,
    docs: &mut gsm_domain::settings::Documents,
    changed: &[String],
) -> Result<(), String> {
    #[cfg(feature = "arksa")]
    if game == "arksa" {
        game_arksa::settings::synchronize(docs, changed)?;
    }
    #[cfg(feature = "valheim")]
    if game == "valheim" {
        let doc = game_valheim::config::ConfigDocument::parse(docs["manager"].clone())
            .map_err(|e| e.to_string())?;
        game_valheim::launch::build_launch_plan(doc.settings()).map_err(|e| e.to_string())?;
    }
    let _ = (game, docs, changed);
    Ok(())
}
pub fn settings_name(game: &str, text: &str) -> Result<Option<String>, String> {
    #[cfg(feature = "arksa")]
    if game == "arksa" {
        return game_arksa::settings::server_name(text).map(Some);
    }
    let _ = (game, text);
    Ok(None)
}
pub fn creation_schema(game: &str) -> Vec<gsm_domain::settings::Field> {
    match game {
        #[cfg(feature = "valheim")]
        "valheim" => game_valheim::settings::creation_schema(),
        #[cfg(feature = "arksa")]
        "arksa" => game_arksa::settings::creation_schema(),
        #[cfg(feature = "conan")]
        "conan" => game_conan::settings::creation_schema(),
        #[cfg(feature = "windrose")]
        "windrose" => game_windrose::settings::creation_schema(),
        #[cfg(feature = "satisfactory")]
        "satisfactory" => game_satisfactory::settings::creation_schema(),
        _ => vec![],
    }
}
pub fn create_configuration(
    game: &str,
    values: &[(String, String)],
) -> Result<gsm_domain::settings::Documents, String> {
    match game {
        #[cfg(feature = "valheim")]
        "valheim" => game_valheim::settings::create(values),
        #[cfg(feature = "arksa")]
        "arksa" => game_arksa::settings::create(values),
        #[cfg(feature = "conan")]
        "conan" => game_conan::settings::create(values),
        #[cfg(feature = "windrose")]
        "windrose" => game_windrose::settings::create(values),
        #[cfg(feature = "satisfactory")]
        "satisfactory" => game_satisfactory::settings::create(values),
        _ => Err("Game is unavailable in this build".into()),
    }
}
pub fn creation_paths(
    game: &str,
    root: &std::path::Path,
) -> Vec<(&'static str, std::path::PathBuf)> {
    match game {
        #[cfg(feature = "arksa")]
        "arksa" => game_arksa::settings::creation_paths(root),
        #[cfg(feature = "conan")]
        "conan" => game_conan::settings::creation_paths(root),
        _ => {
            let _ = root;
            vec![]
        }
    }
}
