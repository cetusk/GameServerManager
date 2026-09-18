use game_valheim::config::{ConfigDocument, Language, WindowsPath, WorldName};

const LEGACY: &str = include_str!("fixtures/legacy-config.toml");

#[test]
fn legacy_defaults_and_original_comments_survive() {
    let document = ConfigDocument::parse(LEGACY.into()).unwrap();
    let config = document.settings();
    assert_eq!(config.server.mod_combat, "default");
    assert!(config.server.world_keys.is_empty());
    assert_eq!(config.manager.language, Language::Ja);
    assert_eq!(config.manager.graceful_stop_timeout_secs, 30);
    assert_eq!(config.manager.backup_short_secs, 7200);
    assert_eq!(config.manager.backup_long_secs, 43200);
    assert_eq!(document.original_toml(), LEGACY);
}

#[test]
fn all_sections_preserve_unknown_fields_and_custom_game_values() {
    let source = format!(
        "root_extension = {{ version = 2 }}\n{}\nmod_combat = 'future-combat'\nworld_keys = ['nomap', 'future-key']\ncrossplay = true\nserver_extension = {{ enabled = true }}\n[manager]\ngraceful_stop_timeout_secs = 45\nauto_backup_before_update = false\nlanguage = 'en'\npublic_address = 'fixture.example:1234'\nbackup_short_secs = 1200\nbackup_long_secs = 7200\nfuture_token = 'keep-me'\n[future_section]\nvalues = [1, 2]\n",
        LEGACY.replace("[server]", "path_extension = 'retained'\n[server]")
    );
    let document = ConfigDocument::parse(source.clone()).unwrap();
    let config = document.settings();
    assert_eq!(document.original_toml(), source);
    assert!(config.server.crossplay);
    assert_eq!(config.server.mod_combat, "future-combat");
    assert_eq!(config.server.world_keys, ["nomap", "future-key"]);
    assert_eq!(config.manager.language, Language::En);
    // A future typed editor must not lose extensions even if it serializes the schema.
    let encoded = toml::to_string(config).unwrap();
    let actual: toml::Value = toml::from_str(&encoded).unwrap();
    let expected: toml::Value = toml::from_str(&source).unwrap();
    for (section, key) in [
        ("paths", "path_extension"),
        ("server", "server_extension"),
        ("manager", "future_token"),
    ] {
        assert_eq!(actual[section][key], expected[section][key]);
    }
    assert_eq!(actual["root_extension"], expected["root_extension"]);
    assert_eq!(actual["future_section"], expected["future_section"]);
}

#[test]
fn windows_path_rules_are_identical_on_non_windows_hosts() {
    for value in [
        r"C:\GsmFixture\Data",
        "D:/日本語/Worlds",
        r"\\server\share\Data",
        r"C:\",
    ] {
        assert!(WindowsPath::try_from(value.to_owned()).is_ok(), "{value}");
    }
    for value in [
        "relative",
        r"C:Data",
        r"\Data",
        "/tmp/data",
        r"\\server",
        r"\\?\C:\Data",
        r"C:\a\..\b",
        r"C:\a:stream",
        r"C:\NUL",
        "C:\\Data. ",
        "C:\\a\0b",
    ] {
        assert!(WindowsPath::try_from(value.to_owned()).is_err(), "{value}");
    }
}

#[test]
fn world_name_cannot_escape_or_alias_another_windows_file() {
    for value in ["Meadows", "日本語の世界", "My World", "world.v2"] {
        assert!(WorldName::try_from(value.to_owned()).is_ok());
    }
    for value in [
        "",
        " ",
        ".",
        "..",
        "../Other",
        r"a\b",
        "C:Other",
        "World:stream",
        "NUL.db",
        "COM1",
        "LPT²",
        "World.",
        "World ",
        "x\ny",
    ] {
        assert!(WorldName::try_from(value.to_owned()).is_err(), "{value}");
    }
    assert!(WorldName::try_from("a".repeat(248)).is_err());
}

#[test]
fn invalid_values_and_types_fail_without_exposing_source_or_password() {
    for source in [
        LEGACY.replace("2456", "65535"),
        LEGACY.replace("public = 0", "public = 2"),
        LEGACY.replace("900", "59"),
        LEGACY.replace("fixture-only-password", "tiny"),
        LEGACY.replace("Fixture Server", "fixture-only-password"),
        LEGACY.replace("world = \"Meadows\"", "world = '../Other'"),
        LEGACY.replace("port = 2456", "port = 'fixture-only-password'"),
        LEGACY.replace(
            "password = \"fixture-only-password\"",
            "password = [\"fixture-only-password\"]",
        ),
        format!("{LEGACY}\nbad = \"fixture-only-password"),
    ] {
        let error = match ConfigDocument::parse(source) {
            Err(e) => e,
            Ok(_) => panic!("invalid config accepted"),
        };
        assert!(!format!("{error:?}: {error}").contains("fixture-only-password"));
    }
    let document = ConfigDocument::parse(LEGACY.into()).unwrap();
    assert_eq!(
        format!("{:?}", document.settings().server.password),
        "[redacted]"
    );
}
