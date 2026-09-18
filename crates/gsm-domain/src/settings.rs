//! Pure, allowlisted configuration editing. Game crates own the field schemas.
use std::collections::BTreeMap;
pub type Documents = BTreeMap<String, String>;
#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Text,
    Secret,
    Number(u64, u64),
    Bool,
    Path,
    ReadOnly,
}
#[derive(Clone)]
pub struct Field {
    pub id: &'static str,
    pub file: &'static str,
    pub section: &'static str,
    pub key: &'static str,
    pub kind: Kind,
    pub default: &'static str,
}
impl Field {
    pub fn new(
        id: &'static str,
        file: &'static str,
        section: &'static str,
        key: &'static str,
        kind: Kind,
        default: &'static str,
    ) -> Self {
        Self {
            id,
            file,
            section,
            key,
            kind,
            default,
        }
    }
}
pub fn paths(save: bool, log: bool) -> Vec<Field> {
    let mut out = vec![
        Field::new("steamcmd", "manager", "paths", "steamcmd", Kind::Path, ""),
        Field::new(
            "server_dir",
            "manager",
            "paths",
            "server_dir",
            Kind::Path,
            "",
        ),
        Field::new(
            "backup_dir",
            "manager",
            "paths",
            "backup_dir",
            Kind::Path,
            "",
        ),
    ];
    if save {
        out.push(Field::new(
            "save_dir",
            "manager",
            "paths",
            "save_dir",
            Kind::Path,
            "",
        ));
    }
    if log {
        out.push(Field::new(
            "log_file",
            "manager",
            "paths",
            "log_file",
            Kind::Path,
            "",
        ));
    }
    out
}
fn json_pointer(doc: &serde_json::Value, key: &str) -> Result<String, String> {
    fn visit(v: &serde_json::Value, key: &str, path: String, out: &mut Vec<String>) {
        match v {
            serde_json::Value::Object(m) => {
                for (k, v) in m {
                    let p = format!("{}/{}", path, k.replace('~', "~0").replace('/', "~1"));
                    if k == key {
                        out.push(p.clone());
                    }
                    visit(v, key, p, out);
                }
            }
            serde_json::Value::Array(a) => {
                for (i, v) in a.iter().enumerate() {
                    visit(v, key, format!("{path}/{i}"), out);
                }
            }
            _ => (),
        }
    }
    let mut found = vec![];
    visit(doc, key, String::new(), &mut found);
    if found.len() != 1 {
        return Err(format!(
            "{key}: field missing or ambiguous / 項目が存在しないか重複しています"
        ));
    }
    Ok(found.remove(0))
}
pub fn read(text: &str, f: &Field) -> Result<String, String> {
    if f.section == "@json" {
        let doc: serde_json::Value = serde_json::from_str(text).map_err(|_| "Invalid JSON")?;
        let p = json_pointer(&doc, f.key)?;
        let v = &doc.pointer(&p).unwrap();
        return match v {
            serde_json::Value::String(s) => Ok(s.clone()),
            serde_json::Value::Bool(b) => Ok(b.to_string()),
            serde_json::Value::Number(n) => Ok(n.to_string()),
            _ => Err("Unsupported JSON value / 未対応の型です".into()),
        };
    }
    if f.section.starts_with("@ini:") {
        return Ok(
            crate::config::ini(text, &f.section[5..], f.key)?.unwrap_or_else(|| f.default.into())
        );
    }
    let doc = crate::config::Toml::parse(text)?;
    Ok(match doc.value(f.section, f.key) {
        Some(toml::Value::String(s)) => s.clone(),
        Some(toml::Value::Integer(n)) => n.to_string(),
        Some(toml::Value::Boolean(b)) => b.to_string(),
        None => f.default.into(),
        _ => return Err("Unsupported setting type / 設定の型が不正です".into()),
    })
}
pub fn set_ini(text: &str, section: &str, key: &str, value: &str) -> Result<String, String> {
    crate::config::ini(text, section, key)?;
    if value.chars().any(char::is_control) || value.contains('"') {
        return Err(
            "INI value contains unsupported characters / 改行や引用符は使用できません".into(),
        );
    }
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let mut active = false;
    let mut insert = None;
    for (i, line) in lines.iter_mut().enumerate() {
        let t = line.trim().trim_start_matches('\u{feff}');
        if t.starts_with('[') && t.ends_with(']') {
            if active && insert.is_none() {
                insert = Some(i);
            }
            active = t[1..t.len() - 1].eq_ignore_ascii_case(section);
        } else if active
            && !t.starts_with([';', '#'])
            && let Some((k, _)) = t.split_once('=')
            && k.trim().eq_ignore_ascii_case(key)
        {
            let prefix = line.split_once('=').unwrap().0.to_owned();
            *line = format!("{prefix}={value}");
            return Ok(lines.join(newline) + newline);
        }
    }
    if active {
        insert = Some(lines.len());
    }
    if let Some(i) = insert {
        lines.insert(i, format!("{key}={value}"));
    } else {
        lines.push(format!("[{section}]"));
        lines.push(format!("{key}={value}"));
    }
    Ok(lines.join(newline) + newline)
}
pub fn write(text: &str, f: &Field, value: &str) -> Result<String, String> {
    if value.chars().any(char::is_control) {
        return Err(format!(
            "{}: control characters are not allowed / 改行・制御文字は使用できません",
            f.id
        ));
    }
    match f.kind {
        Kind::ReadOnly => {
            return Err("This path is determined by the game / ゲーム側で決まる項目です".into());
        }
        Kind::Number(min, max) => {
            let n = value
                .parse::<u64>()
                .map_err(|_| format!("{}: enter an integer / 整数を入力してください", f.id))?;
            if !(min..=max).contains(&n) {
                return Err(format!("{}: {min}–{max}", f.id));
            }
        }
        Kind::Bool => {
            if !["true", "false"].contains(&value) {
                return Err("Invalid boolean".into());
            }
        }
        Kind::Path => {
            crate::config::absolute(value)?;
        }
        _ => (),
    }
    if f.section.starts_with("@ini:") {
        return set_ini(text, &f.section[5..], f.key, value);
    }
    if f.section == "@json" {
        let mut doc: serde_json::Value = serde_json::from_str(text).map_err(|_| "Invalid JSON")?;
        let p = json_pointer(&doc, f.key)?;
        let old = doc.pointer_mut(&p).unwrap();
        let next = match f.kind {
            Kind::Number(..) => serde_json::Value::from(value.parse::<u64>().unwrap()),
            Kind::Bool => serde_json::Value::from(value == "true"),
            _ => serde_json::Value::String(value.into()),
        };
        if std::mem::discriminant(old) != std::mem::discriminant(&next) {
            return Err("Unexpected JSON field type / JSON項目の型が想定と異なります".into());
        }
        *old = next;
        return serde_json::to_string_pretty(&doc)
            .map(|s| s + "\n")
            .map_err(|_| "JSON serialization failed".into());
    }
    let mut doc = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|_| "Invalid TOML")?;
    let mut next = match f.kind {
        Kind::Number(..) => {
            toml_edit::Value::from(value.parse::<i64>().map_err(|_| "Integer overflow")?)
        }
        Kind::Bool => toml_edit::Value::from(value == "true"),
        _ => toml_edit::Value::from(value),
    };
    let item = &mut doc[f.section][f.key];
    if let Some(old) = item.as_value() {
        *next.decor_mut() = old.decor().clone();
    }
    *item = toml_edit::Item::Value(next);
    Ok(doc.to_string())
}
/// Build an explicit new document set; no fixtures or stored credentials are used.
pub fn new_documents(
    schema: &[Field],
    mut docs: Documents,
    values: &[(String, String)],
) -> Result<Documents, String> {
    let mut seen = std::collections::BTreeSet::new();
    for (id, value) in values {
        let f = schema
            .iter()
            .find(|f| f.id == id)
            .ok_or("Unsupported creation field")?;
        if !seen.insert(id) {
            return Err("Duplicate creation field".into());
        }
        let old = docs.entry(f.file.into()).or_default();
        *old = write(old, f, value)?;
    }
    Ok(docs)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_unknown_ini_and_toml_and_rejects_ambiguous_json() {
        let text = "; header\r\n[Server]\r\nPassword=old\r\nUnknown=untouched\r\n";
        let edited = set_ini(text, "Server", "Password", "new").unwrap();
        assert!(edited.starts_with("; header\r\n"));
        assert!(edited.contains("Unknown=untouched\r\n"));
        assert!(set_ini(text, "Server", "Password", "bad\nInjected=1").is_err());
        let f = Field::new(
            "port",
            "manager",
            "server",
            "port",
            Kind::Number(1024, 65534),
            "2456",
        );
        let text = "# keep\n[server]\nport = 2456 # port comment\nunknown = 'keep-me'\n";
        let edited = write(text, &f, "2466").unwrap();
        assert!(edited.contains("port = 2466 # port comment"));
        assert!(edited.contains("unknown = 'keep-me'"));
        let f = Field::new("name", "description", "@json", "ServerName", Kind::Text, "");
        for text in [
            r#"{"ServerName":"same","nested":{"ServerName":"same"}}"#,
            r#"{"Unknown":42}"#,
        ] {
            assert!(read(text, &f).is_err());
            assert!(write(text, &f, "next").is_err());
        }
    }
}
