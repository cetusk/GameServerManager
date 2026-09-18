use gsm_domain::settings::{Field, Kind};
pub fn schema() -> Vec<Field> {
    let mut fields = vec![];
    fields.extend([
        Field::new(
            "profile_name",
            "manager",
            "@ini:General",
            "Edit_Profile",
            Kind::Text,
            "ARK SA",
        ),
        Field::new(
            "name",
            "manager",
            "@ini:Server",
            "Edit_SessionName",
            Kind::Text,
            "",
        ),
        Field::new(
            "port",
            "manager",
            "@ini:Server",
            "SE_Port",
            Kind::Number(1024, 65535),
            "7777",
        ),
        Field::new(
            "query_port",
            "manager",
            "@ini:Server",
            "SE_QueryPort",
            Kind::Number(1024, 65535),
            "27015",
        ),
        Field::new(
            "rcon_port",
            "manager",
            "@ini:Server",
            "SE_RCONPort",
            Kind::Number(1024, 65535),
            "27020",
        ),
        Field::new(
            "password",
            "manager",
            "@ini:Server",
            "Edit_ServerPassword",
            Kind::Secret,
            "",
        ),
        Field::new(
            "admin_password",
            "manager",
            "@ini:Server",
            "Edit_ServerAdminPassword",
            Kind::Secret,
            "",
        ),
        Field::new(
            "steamcmd",
            "manager",
            "@ini:Integration",
            "SteamCMD",
            Kind::Path,
            "",
        ),
        Field::new(
            "server_dir",
            "manager",
            "@ini:General",
            "Edit_Install_Location_Val",
            Kind::Path,
            "",
        ),
        Field::new(
            "backup_dir",
            "manager",
            "@ini:Integration",
            "BackupDir",
            Kind::Path,
            "",
        ),
    ]);
    fields
}
/// Update both profile and launch URL, plus RCON's engine-side values.
pub fn synchronize(
    docs: &mut gsm_domain::settings::Documents,
    changed: &[String],
) -> Result<(), String> {
    use gsm_domain::{
        config::{arguments, ini},
        settings::set_ini,
    };
    let mut profile = docs["manager"].clone();
    if changed
        .iter()
        .any(|s| ["name", "port", "query_port", "password"].contains(&s.as_str()))
    {
        let enabled = ini(&profile, "General", "ChB_CMD_override")?
            .is_some_and(|s| s == "1" || s.eq_ignore_ascii_case("true"));
        let command = if enabled {
            "MM_Command_Override"
        } else {
            "MM_Command_Val"
        };
        let mut args =
            arguments(&ini(&profile, "General", command)?.ok_or("Missing launch command")?)?;
        if args.len() < 2 {
            return Err("Invalid ARK launch command".into());
        }
        let mut url: Vec<String> = args[1].split('?').map(str::to_owned).collect();
        for (id, key, param) in [
            ("name", "Edit_SessionName", "SessionName"),
            ("port", "SE_Port", "Port"),
            ("query_port", "SE_QueryPort", "QueryPort"),
            ("password", "Edit_ServerPassword", "ServerPassword"),
        ] {
            if !changed.iter().any(|s| s == id) {
                continue;
            }
            let value = ini(&profile, "Server", key)?.unwrap_or_default();
            if value.contains(['?', '"', '\\']) || ((id == "name") && value.trim().is_empty()) {
                return Err("ARK name/password contains unsupported characters / ARK名・パスワードの文字を確認してください".into());
            }
            let mut found = false;
            for part in url.iter_mut().skip(1) {
                if part
                    .split('=')
                    .next()
                    .is_some_and(|k| k.eq_ignore_ascii_case(param))
                {
                    *part = format!("{param}={value}");
                    found = true;
                }
            }
            if !found {
                url.push(format!("{param}={value}"));
            }
        }
        args[1] = url.join("?");
        // Escape as Windows argv, never as shell text.
        fn quote(s: &str) -> String {
            let mut out = String::from("\"");
            let mut slashes = 0;
            for c in s.chars() {
                if c == '\\' {
                    slashes += 1;
                    continue;
                }
                out.extend(std::iter::repeat_n(
                    '\\',
                    if c == '"' { slashes * 2 + 1 } else { slashes },
                ));
                slashes = 0;
                out.push(c);
            }
            out.extend(std::iter::repeat_n('\\', slashes * 2));
            out.push('"');
            out
        }
        // INI permits a quoted command; set_ini deliberately rejects embedded quotes for normal values.
        let next = args
            .iter()
            .map(|s| {
                if s.chars().any(char::is_whitespace) || s.contains('"') {
                    quote(s)
                } else {
                    s.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        let placeholder = "GSM_REVIEWED_LAUNCH_COMMAND";
        profile = set_ini(&profile, "General", command, placeholder)?.replace(
            &format!("{command}={placeholder}"),
            &format!("{command}={next}"),
        );
    }
    if changed.iter().any(|s| s == "server_dir") {
        profile = set_ini(&profile, "General", "Sys_RelativePath", "0")?;
    }
    if changed
        .iter()
        .any(|s| s == "rcon_port" || s == "admin_password")
    {
        let mut engine = docs
            .get("GameUserSettings.ini")
            .ok_or("GameUserSettings.ini が必要です / GameUserSettings.ini is required")?
            .clone();
        for (id, key, target) in [
            ("rcon_port", "SE_RCONPort", "RCONPort"),
            (
                "admin_password",
                "Edit_ServerAdminPassword",
                "ServerAdminPassword",
            ),
        ] {
            if changed.iter().any(|s| s == id) {
                let value = ini(&profile, "Server", key)?.unwrap_or_default();
                if value.is_empty() {
                    return Err("ARK admin password is required".into());
                }
                engine = set_ini(&engine, "ServerSettings", target, &value)?;
            }
        }
        engine = set_ini(&engine, "ServerSettings", "RCONEnabled", "True")?;
        docs.insert("GameUserSettings.ini".into(), engine);
    }
    docs.insert("manager".into(), profile);
    Ok(())
}
pub fn server_name(text: &str) -> Result<String, String> {
    use gsm_domain::config::{arguments, ini};
    let enabled = ini(text, "General", "ChB_CMD_override")?
        .is_some_and(|s| s == "1" || s.eq_ignore_ascii_case("true"));
    let args = arguments(
        &ini(
            text,
            "General",
            if enabled {
                "MM_Command_Override"
            } else {
                "MM_Command_Val"
            },
        )?
        .unwrap_or_default(),
    )?;
    Ok(args
        .get(1)
        .and_then(|s| {
            s.split('?').skip(1).find_map(|s| {
                let (k, v) = s.split_once('=')?;
                k.eq_ignore_ascii_case("SessionName").then(|| v.to_owned())
            })
        })
        .unwrap_or_default())
}
pub fn creation_schema() -> Vec<Field> {
    let mut fields = schema();
    fields.push(Field::new(
        "new_map",
        "manager",
        "@ini:General",
        "CB_MapName_Text",
        Kind::Text,
        "TheIsland_WP",
    ));
    fields
}
pub fn create(values: &[(String, String)]) -> Result<gsm_domain::settings::Documents, String> {
    use gsm_domain::settings::{Documents, new_documents, set_ini};
    let mut seed = Documents::new();
    seed.insert(
        "manager".into(),
        "[Server]\nCB_RCONEnabled=1\n[General]\nSys_RelativePath=0\n".into(),
    );
    seed.insert("GameUserSettings.ini".into(), String::new());
    let mut docs = new_documents(&creation_schema(), seed, values)?;
    let map = gsm_domain::config::ini(&docs["manager"], "General", "CB_MapName_Text")?
        .ok_or("Map is required")?;
    gsm_domain::config::name(&map)?;
    let command = format!("ArkAscendedServer.exe {map}?listen?Port=7777?QueryPort=27015 -log");
    docs.insert(
        "manager".into(),
        set_ini(&docs["manager"], "General", "MM_Command_Val", &command)?,
    );
    synchronize(
        &mut docs,
        &[
            "name",
            "port",
            "query_port",
            "rcon_port",
            "password",
            "admin_password",
        ]
        .map(str::to_owned),
    )?;
    Ok(docs)
}
pub fn creation_paths(root: &std::path::Path) -> Vec<(&'static str, std::path::PathBuf)> {
    vec![(
        "GameUserSettings.ini",
        root.join("ShooterGame/Saved/Config/WindowsServer/GameUserSettings.ini"),
    )]
}
