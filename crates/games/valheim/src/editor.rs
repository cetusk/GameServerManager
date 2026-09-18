//! Allowlisted editor preserving comments, unknown fields and secrets.
use crate::{config::ConfigDocument, launch::build_launch_plan};

pub const SERVER_FIELDS: &[&str] = &[
    "name",
    "port",
    "password",
    "save_interval",
    "backups",
    "public",
    "crossplay",
];
pub const WORLD_FIELDS: &[&str] = &[
    "mod_combat",
    "mod_deathpenalty",
    "mod_resources",
    "mod_raids",
    "mod_portals",
];
pub fn choices(key: &str) -> &'static [&'static str] {
    match key {
        "public" => &["0", "1"],
        "crossplay" => &["false", "true"],
        "mod_combat" => &["default", "veryeasy", "easy", "hard", "veryhard"],
        "mod_deathpenalty" => &["default", "casual", "veryeasy", "hard", "hardcore"],
        "mod_resources" => &["default", "muchless", "less", "more", "muchmore"],
        "mod_raids" => &["default", "none", "muchless", "less", "more", "muchmore"],
        "mod_portals" => &["default", "casual", "hard", "veryhard"],
        _ => &[],
    }
}
pub fn fields(text: &str, world: bool) -> Result<Vec<(String, String)>, String> {
    let doc = ConfigDocument::parse(text.into()).map_err(|e| e.to_string())?;
    build_launch_plan(doc.settings()).map_err(|e| e.to_string())?;
    if world
        && ((!doc.settings().server.preset.is_empty() && doc.settings().server.preset != "default")
            || doc
                .settings()
                .server
                .world_keys
                .iter()
                .any(|s| !s.is_empty()))
    {
        return Err("Preset or world keys are configured. Edit these in the original file first / プリセット・追加ワールドキーが設定されています。優先関係を確認して元ファイルで編集してください".into());
    }
    let server = &doc.settings().server;
    Ok(if world {
        vec![
            ("mod_combat", server.mod_combat.clone()),
            ("mod_deathpenalty", server.mod_deathpenalty.clone()),
            ("mod_resources", server.mod_resources.clone()),
            ("mod_raids", server.mod_raids.clone()),
            ("mod_portals", server.mod_portals.clone()),
        ]
    } else {
        vec![
            ("name", server.name.clone()),
            ("port", server.port.to_string()),
            // Never populate the editor with the existing credential.
            ("password", String::new()),
            ("save_interval", server.save_interval.to_string()),
            ("backups", server.backups.to_string()),
            ("public", server.public.to_string()),
            ("crossplay", server.crossplay.to_string()),
        ]
    }
    .into_iter()
    .map(|(k, v)| {
        (
            k.into(),
            if world && v.is_empty() {
                "default".into()
            } else {
                v
            },
        )
    })
    .collect())
}
pub fn apply(text: &str, changes: &[(String, String)], world: bool) -> Result<String, String> {
    let fields = fields(text, world)?;
    let mut doc = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|_| "Invalid TOML / TOML形式が不正です")?;
    let mut seen = std::collections::BTreeSet::new();
    let mut modified = false;
    for (key, value) in changes {
        if !fields.iter().any(|(k, _)| k == key) || !seen.insert(key) {
            return Err("Unsupported or duplicate field / 未対応または重複する項目です".into());
        }
        // Empty password means keep the existing value, never remove it.
        if fields.iter().any(|(k, old)| k == key && old == value) {
            continue;
        }
        let next = if key == "name" || key == "password" {
            toml_edit::Value::from(value.as_str())
        } else if ["port", "save_interval", "backups"].contains(&key.as_str()) {
            let n = value
                .parse::<u32>()
                .map_err(|_| "Enter a nonnegative integer / 0以上の整数を入力してください")?;
            toml_edit::Value::from(i64::from(n))
        } else {
            if !choices(key).contains(&value.as_str()) {
                return Err("Unsupported value / 未対応の値です".into());
            }
            match key.as_str() {
                "public" => toml_edit::Value::from(if value == "1" { 1 } else { 0 }),
                "crossplay" => toml_edit::Value::from(value == "true"),
                _ => toml_edit::Value::from(value.as_str()),
            }
        };
        let item = &mut doc["server"][key.as_str()];
        let mut next = next;
        if let Some(old) = item.as_value() {
            *next.decor_mut() = old.decor().clone();
        }
        *item = toml_edit::Item::Value(next);
        modified = true;
    }
    // A no-op must preserve the original bytes, including Windows CRLFs.
    // toml_edit normalizes line endings when serializing even without edits.
    let result = if modified {
        doc.to_string()
    } else {
        text.into()
    };
    let verified = ConfigDocument::parse(result.clone()).map_err(|e| e.to_string())?;
    build_launch_plan(verified.settings()).map_err(|e| e.to_string())?;
    Ok(result)
}
/// Remove old and new credentials even if a non-secret field repeats them.
pub fn redact_review(original: &str, updated: &str, summary: &str) -> Result<String, String> {
    let before = ConfigDocument::parse(original.into()).map_err(|e| e.to_string())?;
    let after = ConfigDocument::parse(updated.into()).map_err(|e| e.to_string())?;
    let mut secrets = [
        before.settings().server.password.expose(),
        after.settings().server.password.expose(),
    ];
    secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    let mut result = summary.to_owned();
    for secret in secrets {
        if !secret.is_empty() {
            result = result.replace(secret, "[非表示]");
        }
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    const FIXTURE: &str = include_str!("../tests/fixtures/legacy-config.toml");
    #[test]
    fn edits_preserve_unknown_fields_comments_and_secrets() {
        let original = format!("{FIXTURE}\nunknown_future = 'keep-me' # untouched\n");
        let result = apply(&original, &[("save_interval".into(), "1200".into())], false).unwrap();
        assert!(result.contains("unknown_future = 'keep-me' # untouched"));
        assert!(result.contains("password = \"fixture-only-password\""));
        assert!(result.contains("save_interval = 1200"));
        assert!(apply(&original, &[("password".into(), "bad".into())], false).is_err());
        assert!(apply(&original, &[("save_interval".into(), "0".into())], false).is_err());
    }
    #[test]
    fn connection_settings_reach_launch_arguments_without_changing_world_or_paths() {
        let changes = [
            ("name".into(), "New Server".into()),
            ("port".into(), "2466".into()),
            ("password".into(), "new-fixture-secret".into()),
        ];
        let updated = apply(FIXTURE, &changes, false).unwrap();
        let doc = ConfigDocument::parse(updated).unwrap();
        let plan = build_launch_plan(doc.settings()).unwrap();
        let args = plan.expose_arguments();
        for (flag, value) in [
            ("-name", "New Server"),
            ("-port", "2466"),
            ("-password", "new-fixture-secret"),
            ("-world", "Meadows"),
        ] {
            assert!(
                args.windows(2)
                    .any(|pair| pair[0] == flag && pair[1] == value)
            );
        }
        assert_eq!(
            doc.settings().paths.save_dir.as_str(),
            r"C:\GsmFixture\Data"
        );
        assert!(
            fields(FIXTURE, false)
                .unwrap()
                .iter()
                .any(|(k, v)| k == "password" && v.is_empty())
        );
        let unchanged = apply(FIXTURE, &[("password".into(), String::new())], false).unwrap();
        assert_eq!(unchanged, FIXTURE);
    }
    #[test]
    fn no_op_edits_preserve_lf_and_crlf_without_changing_credentials() {
        let lf = FIXTURE.replace("\r\n", "\n");
        for original in [&lf, &lf.replace('\n', "\r\n")] {
            for changes in [
                vec![],
                vec![("password".into(), String::new())],
                fields(original, false).unwrap(),
                fields(original, true).unwrap(),
            ] {
                let world = changes.iter().any(|(key, _)| key.starts_with("mod_"));
                assert_eq!(apply(original, &changes, world).unwrap(), *original);
            }
            let changed = apply(original, &[("port".into(), "2466".into())], false).unwrap();
            let parsed = ConfigDocument::parse(changed).unwrap();
            assert_eq!(parsed.settings().server.port, 2466);
            assert_eq!(
                parsed.settings().server.password.expose(),
                "fixture-only-password"
            );
        }
    }
    #[test]
    fn invalid_connection_values_are_rejected_without_echoing_them() {
        for (key, value) in [
            ("port", "1023"),
            ("port", "65535"),
            ("port", "x"),
            ("name", " "),
            ("name", "-bad-name"),
            ("password", "bad"),
            ("password", "-bad-secret"),
            ("password", "Fixture Server"),
            ("name", "injected\nname"),
        ] {
            let error = apply(FIXTURE, &[(key.into(), value.into())], false).unwrap_err();
            assert!(!error.contains("fixture-only-password"));
            assert!(!error.contains("-bad-secret"));
        }
        assert!(apply(FIXTURE, &[("name".into(), "New".into())], true).is_err());
        assert!(
            apply(
                FIXTURE,
                &[
                    ("port".into(), "2466".into()),
                    ("port".into(), "2467".into())
                ],
                false
            )
            .is_err()
        );
        assert!(apply(FIXTURE, &[("save_dir".into(), "other".into())], false).is_err());
    }

    #[test]
    fn preset_and_extra_keys_are_never_silently_overridden() {
        for extra in ["preset='hard'", "world_keys=['noportals']"] {
            assert!(fields(&format!("{FIXTURE}\n{extra}"), true).is_err());
            assert!(fields(&format!("{FIXTURE}\n{extra}"), false).is_ok());
        }
        assert!(apply(FIXTURE, &[("mod_combat".into(), "impossible".into())], true).is_err());
        for key in WORLD_FIELDS {
            for value in choices(key) {
                assert!(apply(FIXTURE, &[(key.to_string(), value.to_string())], true).is_ok());
            }
        }
    }
}
