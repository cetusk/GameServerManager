//! Log IO is performed by backend/clipboard workers, never GUI callbacks.
use super::paths::{err, validate};
use gsm_domain::local::LocalServer;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

pub(super) fn full(spec: &LocalServer, path: &Path) -> Result<String, String> {
    validate(path)?;
    let mut file = File::open(path).map_err(err)?;
    let meta = file.metadata().map_err(err)?;
    if !meta.is_file() {
        return Err("Log must be a regular file".into());
    }
    if meta.len() > 128 * 1024 * 1024 {
        return Err("Log exceeds 128 MiB; open it directly / ログが128 MiBを超えています。ログファイルを直接開いてください".into());
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(meta.len())
        .read_to_end(&mut bytes)
        .map_err(err)?;
    Ok(spec.redact(&String::from_utf8_lossy(&bytes)))
}

/// Only include bytes from this invocation in a failure; never reuse old errors.
pub(super) fn tail(spec: &LocalServer, path: &Path, from: u64) -> Result<String, String> {
    validate(path)?;
    let mut file = File::open(path).map_err(err)?;
    let meta = file.metadata().map_err(err)?;
    if !meta.is_file() {
        return Err("Log must be a regular file".into());
    }
    let start = from.max(meta.len().saturating_sub(8192));
    file.seek(SeekFrom::Start(start)).map_err(err)?;
    let mut bytes = Vec::new();
    file.take(8192).read_to_end(&mut bytes).map_err(err)?;
    // A tail may start inside a line or a secret. Drop that partial line.
    let bytes = if start > from {
        bytes
            .iter()
            .position(|b| *b == b'\n')
            .map_or(&[][..], |i| &bytes[i + 1..])
    } else {
        &bytes
    };
    let text = spec.redact(&String::from_utf8_lossy(bytes));
    let lines = text.lines().collect::<Vec<_>>();
    Ok(lines[lines.len().saturating_sub(30)..].join("\n"))
}

pub(super) fn update_failure(spec: &LocalServer, path: &Path, from: u64, status: &str) -> String {
    let detail = match tail(spec, path, from) {
        Ok(text) if !text.trim().is_empty() => text,
        Ok(_) => "今回の標準出力はありません / No output from this attempt".into(),
        Err(e) => format!("更新ログの読み取りに失敗 / Cannot read update log: {e}"),
    };
    spec.redact(&format!("SteamCMD の更新に失敗しました / SteamCMD update failed ({status})\n更新ログ / Update log: {}\n今回の出力（末尾） / Output from this attempt (tail):\n{detail}", path.display()))
}

pub(super) fn combined(spec: &LocalServer, update: &Path, events: &str) -> Result<String, String> {
    let mut sections = vec![format!(
        "管理ツールの操作記録 / Manager operations\n{events}"
    )];
    for (label, path) in [
        ("サーバーログ / Server log", spec.log_file.as_path()),
        ("SteamCMD 更新ログ / SteamCMD update log", update),
    ] {
        let content = match std::fs::metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                "ログはまだありません / No log yet".into()
            }
            Err(e) => return Err(spec.redact(&format!("{label} [{}]: {e}", path.display()))),
            Ok(_) => full(spec, path)
                .map_err(|e| spec.redact(&format!("{label} [{}]: {e}", path.display())))?,
        };
        sections.push(format!(
            "{label}（過去の記録を含みます / includes past records）\n{}\n{content}",
            path.display()
        ));
    }
    Ok(spec.redact(&sections.join("\n\n")))
}
