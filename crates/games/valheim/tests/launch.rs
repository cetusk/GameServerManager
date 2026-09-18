use game_valheim::{config::ConfigDocument, launch::build_launch_plan};
const FIXTURE: &str = include_str!("fixtures/legacy-config.toml");

#[test]
fn spaced_values_are_single_arguments_and_password_is_never_in_preview_or_debug() {
    let source = FIXTURE
        .replace("Fixture Server", "Fixture & Server")
        .replace("Meadows", "My World");
    let document = ConfigDocument::parse(source).unwrap();
    let plan = build_launch_plan(document.settings()).unwrap();
    let args = plan.expose_arguments();
    assert!(args.windows(2).any(|p| p == ["-name", "Fixture & Server"]));
    assert!(args.windows(2).any(|p| p == ["-world", "My World"]));
    assert!(
        args.windows(2)
            .any(|p| p == ["-password", "fixture-only-password"])
    );
    assert!(!format!("{plan:?}").contains("fixture-only-password"));
    assert!(!plan.redacted_preview().contains("fixture-only-password"));
    assert_eq!(
        plan.executable(),
        r"C:\GsmFixture\Server\valheim_server.exe"
    );
    for omitted in [
        "-crossplay",
        "-modifier",
        "-preset",
        "-setkey",
        "-backupshort",
        "-backuplong",
    ] {
        assert!(!args.iter().any(|v| v == omitted));
    }
}
#[test]
fn custom_modifiers_keys_intervals_and_crossplay_follow_the_legacy_argument_contract() {
    let source = format!(
        "{FIXTURE}\nmod_combat = 'hard'\npreset = 'immersive'\nworld_keys = ['nomap', 'nobuildcost']\ncrossplay = true\n[manager]\ngraceful_stop_timeout_secs = 30\nauto_backup_before_update = true\nbackup_short_secs = 3600\nbackup_long_secs = 86400\n"
    );
    let document = ConfigDocument::parse(source).unwrap();
    let plan = build_launch_plan(document.settings()).unwrap();
    let args = plan.expose_arguments();
    assert!(
        args.windows(3)
            .any(|p| p == ["-modifier", "combat", "hard"])
    );
    for pair in [
        ["-preset", "immersive"],
        ["-setkey", "nomap"],
        ["-setkey", "nobuildcost"],
        ["-backupshort", "3600"],
        ["-backuplong", "86400"],
    ] {
        assert!(args.windows(2).any(|p| p == pair));
    }
    assert_eq!(args.last().unwrap(), "-crossplay");
}
#[test]
fn unsafe_or_unknown_launch_values_fail_without_echoing_the_value() {
    for source in [
        FIXTURE.replace("Meadows", "-world"),
        FIXTURE.replace("fixture-only-password", "-secret-password"),
        format!("{FIXTURE}\nmod_combat = 'secret-unknown-value'"),
        format!("{FIXTURE}\nworld_keys = ['secret-unknown-value']"),
        FIXTURE.replace(r"C:\GsmFixture\Backups", r"c:/gsmfixture/data/Backups"),
        FIXTURE.replace(r"C:\GsmFixture\Backups", r"C:\GsmFixture"),
        FIXTURE.replace(
            r"C:\GsmFixture\Logs\server.log",
            r"C:\GsmFixture\Server\valheim_server.exe",
        ),
    ] {
        let document = ConfigDocument::parse(source).unwrap();
        let error = build_launch_plan(document.settings()).unwrap_err();
        assert!(!error.to_string().contains("secret-unknown-value"));
    }
}
#[test]
fn redaction_also_scrubs_password_repeated_in_other_fields() {
    let document =
        ConfigDocument::parse(FIXTURE.replace("Meadows", "fixture-only-password")).unwrap();
    let plan = build_launch_plan(document.settings()).unwrap();
    assert!(!format!("{plan:?}").contains("fixture-only-password"));
}
