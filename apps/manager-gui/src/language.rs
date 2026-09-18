use gsm_domain::Command;
pub fn command(command: &Command, en: bool) -> &str {
    if !en {
        return command.label();
    }
    match command {
        Command::Start => "Start",
        Command::Stop => "Stop",
        Command::Backup => "Backup",
        Command::Restore(_) => "Restore",
        Command::Update => "Update",
        Command::Recover => "Recover interrupted job",
        Command::EditSettings => "Edit settings",
        Command::WriteSettings(_) => "Save settings",
    }
}
pub fn scope(game: &str, en: bool, original: &str) -> String {
    if !en {
        return original.into();
    }
    match game {
        "valheim" => "World files and manager configuration",
        "arksa" => "Saved worlds and server configuration",
        "windrose" => "World saves and server configuration",
        "satisfactory" => "Saved sessions and server configuration",
        "conan" => "World database and server configuration",
        _ => original,
    }
    .into()
}

/// Translate controller-owned labels only; names, paths and game output stay verbatim.
pub fn registration_report(report: &str, en: bool) -> String {
    if !en {
        return report.into();
    }
    report.lines().map(|line| {
        for (ja,en) in [
            ("ゲーム: ","Game: "),("サーバー: ","Server: "),("ワールド: ","World: "),("実行ファイル: ","Executable: "),("停止方式: ","Shutdown method: "),("バックアップ先: ","Backup location: "),("登録 ID: ","Registration ID: "),
        ] {if let Some(value)=line.strip_prefix(ja){return format!("{en}{value}");}}
        match line {
            "「参照…」で旧管理設定を選ぶか、絶対パスを入力してください。ARK は Profile/*.ini、他のゲームは config.toml です。"=>"Choose a configuration file or enter its absolute path. ARK uses Profile/*.ini; other games use config.toml.",
            "パスを変更しました。再検証してください。"=>"The path changed. Load and review the settings again.",
            "設定を検証中…"=>"Reviewing settings…",
            "設定の変更有無を確認して登録中…"=>"Checking for changes and registering…",
            "保存・復元する範囲:"=>"Backup and restore scope:",
            "登録しました。"=>"Registered.",
            "管理画面を自動で開き直して反映します。"=>"The manager will reopen automatically to apply the registration.",
            "設定形式を検証しました。プロセス稼働・ポート・実ゲームの読み込みは起動時と実機で確認します。「登録して反映」で管理画面に反映します。"=>"Configuration format checked. Process state, ports and game loading are checked at startup and on the target PC. Choose Register and apply to use this configuration.",
            _=>line,
        }.to_owned()
    }).collect::<Vec<_>>().join("\n")
}
