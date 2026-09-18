//! Inspect only the registered installation. Never search unrelated Steam libraries.
use gsm_domain::local::Installation;
use std::{fs::File, io::Read, path::Path};

pub(super) fn inspect(executable: &Path, root: &Path, app_id: u32) -> Installation {
    match executable.metadata() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Installation::NotInstalled,
        Ok(meta) if meta.is_file() => (),
        _ => return Installation::Unknown,
    }
    let path = root
        .join("steamapps")
        .join(format!("appmanifest_{app_id}.acf"));
    let mut text = String::new();
    if File::open(path)
        .and_then(|f| f.take(65537).read_to_string(&mut text))
        .is_err()
        || text.len() > 65536
    {
        return Installation::Unknown;
    }
    build_id(&text, app_id)
        .map(Installation::SteamBuild)
        .unwrap_or(Installation::Unknown)
}

// A bounded KeyValues reader: accept only direct AppState fields, not nested
// depot build IDs or manifests belonging to another application.
fn build_id(text: &str, app_id: u32) -> Option<String> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => (),
            '/' if chars.next()? == '/' => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '{' | '}' => tokens.push(c.to_string()),
            '"' => {
                let mut value = String::new();
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => value.push(chars.next()?),
                        c => value.push(c),
                    }
                }
                tokens.push(value);
            }
            _ => return None,
        }
    }
    if tokens.first()?.as_str() != "AppState" || tokens.get(1)? != "{" {
        return None;
    }
    let mut depth = 1;
    let mut i = 2;
    let mut id = None;
    let mut build = None;
    while i < tokens.len() {
        let key = &tokens[i];
        if key == "}" {
            depth -= 1;
            i += 1;
            if depth == 0 {
                break;
            }
            continue;
        }
        if key == "{" {
            return None;
        }
        let value = tokens.get(i + 1)?;
        if value == "{" {
            depth += 1;
        } else if value == "}" {
            return None;
        } else if depth == 1 {
            match key.as_str() {
                "appid" if id.is_none() => id = Some(value.as_str()),
                "buildid" if build.is_none() => build = Some(value.as_str()),
                "appid" | "buildid" => return None,
                _ => (),
            }
        }
        i += 2;
    }
    let build = build?;
    if depth != 0
        || i != tokens.len()
        || id? != app_id.to_string()
        || build.is_empty()
        || !build.bytes().all(|b| b.is_ascii_digit())
        || build.parse::<u64>().ok()? == 0
    {
        return None;
    }
    Some(build.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installation_is_not_guessed_from_missing_or_foreign_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("server.exe");
        assert_eq!(inspect(&exe, tmp.path(), 123), Installation::NotInstalled);
        std::fs::write(&exe, b"fixture").unwrap();
        assert_eq!(inspect(&exe, tmp.path(), 123), Installation::Unknown);
        std::fs::create_dir(tmp.path().join("steamapps")).unwrap();
        let manifest = tmp.path().join("steamapps/appmanifest_123.acf");
        std::fs::write(&manifest, r#""AppState" { "appid" "456" "buildid" "7" }"#).unwrap();
        assert_eq!(inspect(&exe, tmp.path(), 123), Installation::Unknown);
        std::fs::write(
            &manifest,
            r#""AppState" { "appid" "123" "Depots" { "buildid" "8" } "buildid" "99" }"#,
        )
        .unwrap();
        assert_eq!(
            inspect(&exe, tmp.path(), 123),
            Installation::SteamBuild("99".into())
        );
        assert_eq!(
            build_id(
                r#""AppState" { "appid" "123" "Depots" { "buildid" "8" } }"#,
                123
            ),
            None
        );
        assert_eq!(
            build_id(
                r#""AppState" { "appid" "123" "buildid" "99" "buildid" "10" }"#,
                123
            ),
            None
        );
        assert_eq!(
            build_id(r#""AppState" { "appid" "123" "buildid" "99""#, 123),
            None
        );
    }
}
