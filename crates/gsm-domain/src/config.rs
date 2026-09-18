//! Small, format-preserving consumers for imported settings. No filesystem access.
use std::path::PathBuf;
pub struct Toml(pub toml::Value);
impl Toml {
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text)
            .map(Self)
            .map_err(|_| "TOML 設定の構文・型を確認してください（本文は表示しません）".into())
    }
    pub fn value(&self, section: &str, key: &str) -> Option<&toml::Value> {
        self.0.get(section)?.get(key)
    }
    pub fn text(&self, section: &str, key: &str) -> Result<String, String> {
        self.value(section, key)
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .ok_or_else(|| format!("{section}.{key} がありません／文字列ではありません"))
    }
    pub fn optional(&self, section: &str, key: &str, default: &str) -> Result<String, String> {
        match self.value(section, key) {
            None => Ok(default.into()),
            Some(v) => v
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{section}.{key} は文字列が必要です")),
        }
    }
    pub fn number(&self, section: &str, key: &str, default: u64) -> Result<u64, String> {
        match self.value(section, key) {
            None => Ok(default),
            Some(v) => v
                .as_integer()
                .and_then(|n| n.try_into().ok())
                .ok_or_else(|| format!("{section}.{key} は非負整数が必要です")),
        }
    }
    pub fn boolean(&self, section: &str, key: &str, default: bool) -> Result<bool, String> {
        match self.value(section, key) {
            None => Ok(default),
            Some(v) => v
                .as_bool()
                .ok_or_else(|| format!("{section}.{key} は真偽値が必要です")),
        }
    }
    pub fn path(&self, section: &str, key: &str) -> Result<PathBuf, String> {
        absolute(&self.text(section, key)?)
    }
    pub fn port(&self, section: &str, key: &str, default: u16) -> Result<u16, String> {
        let n = self.number(section, key, u64::from(default))?;
        if !(1024..=65534).contains(&n) {
            return Err(format!("{section}.{key} のポート範囲が不正です"));
        }
        Ok(n as u16)
    }
}
pub fn absolute(text: &str) -> Result<PathBuf, String> {
    let normalized = text.replace('\\', "/");
    let b = normalized.as_bytes();
    let windows = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'/';
    let native = std::path::Path::new(text).is_absolute();
    if text.chars().any(char::is_control)
        || (!windows && !native)
        || normalized.split('/').any(|p| p == "..")
    {
        return Err("設定パスは明示した絶対パスにしてください".into());
    }
    Ok(PathBuf::from(text))
}
pub fn name(text: &str) -> Result<String, String> {
    if text.is_empty()
        || text.len() > 200
        || text.starts_with('-')
        || text.ends_with(['.', ' '])
        || text
            .chars()
            .any(|c| c.is_control() || "/\\:?*\"<>|".contains(c))
        || text == "."
        || text == ".."
    {
        return Err("ワールド／プロファイル名が不正です".into());
    }
    let stem = text
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ((stem.starts_with("COM") || stem.starts_with("LPT"))
        && matches!(
            &stem[3..],
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
        ))
    {
        return Err("Windows の予約名は使用できません".into());
    }
    Ok(text.into())
}
/// Get one key without rewriting unknown lines/comments. Ambiguous duplicates fail.
pub fn ini(text: &str, section: &str, key: &str) -> Result<Option<String>, String> {
    let mut current = "";
    let mut found = None;
    for line in text.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.starts_with([';', '#']) {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            current = &line[1..line.len() - 1];
            continue;
        }
        if current.eq_ignore_ascii_case(section)
            && let Some((k, v)) = line.split_once('=')
            && k.trim().eq_ignore_ascii_case(key)
        {
            if found.is_some() {
                return Err(format!("INI の {section}.{key} が重複しています"));
            }
            let value = v.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(value);
            found = Some(value.to_owned());
        }
    }
    Ok(found)
}

/// Windows-style quoted arguments, returned as argv rather than a shell command.
/// Backslashes are literal except immediately before a double quote.
pub fn arguments(text: &str) -> Result<Vec<String>, String> {
    if text.chars().any(|c| c.is_control() && c != '\t') {
        return Err("追加引数に制御文字があります".into());
    }
    let mut chars = text.chars().peekable();
    let mut out = vec![];
    let mut value = String::new();
    let mut quoted = false;
    let mut started = false;
    while let Some(c) = chars.next() {
        if c == '\\' {
            let mut count = 1;
            while chars.peek() == Some(&'\\') {
                chars.next();
                count += 1;
            }
            if chars.peek() == Some(&'"') {
                value.extend(std::iter::repeat_n('\\', count / 2));
                chars.next();
                if count % 2 == 0 {
                    quoted = !quoted
                } else {
                    value.push('"');
                }
            } else {
                value.extend(std::iter::repeat_n('\\', count));
            }
            started = true;
        } else if c == '"' {
            quoted = !quoted;
            started = true;
        } else if c.is_whitespace() && !quoted {
            if started {
                out.push(std::mem::take(&mut value));
                started = false;
            }
        } else {
            value.push(c);
            started = true;
        }
    }
    if quoted {
        return Err("追加引数の引用符が閉じていません".into());
    }
    if started {
        out.push(value);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_arguments_preserve_quoted_spaces_and_backslashes() {
        assert_eq!(
            arguments(r#"ArkAscendedServer.exe "Map?SessionName=My Server" -mods=3,1,2"#).unwrap(),
            [
                "ArkAscendedServer.exe",
                "Map?SessionName=My Server",
                "-mods=3,1,2"
            ]
        );
        assert_eq!(
            arguments(r#"-path "C:\Some Folder\data""#).unwrap(),
            ["-path", r"C:\Some Folder\data"]
        );
        assert!(arguments("-flag \"unfinished").is_err());
        assert!(arguments("-flag\n-injected").is_err());
    }
    #[test]
    fn names_and_duplicate_ini_settings_fail_without_echoing_secrets() {
        for value in ["CON", "LPT1.txt", "../world", "a:b", "a."] {
            assert!(name(value).is_err());
        }
        let text = "[ServerSettings]\nPassword=secret\nPassword=other\n";
        let error = ini(text, "ServerSettings", "Password").unwrap_err();
        assert!(!error.contains("secret"));
        assert!(
            ini("[Extra]\nUnknown=value\n", "Extra", "Unknown")
                .unwrap()
                .is_some()
        );
    }
}
