//! Shared SteamCMD defaults and installation from Valve's HTTPS distribution.
//! All functions perform IO and must run outside GUI callbacks.
use crate::local::paths::{err, validate, write_json};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

pub const DEFAULT_EXE: &str = r"C:\steamcmd\steamcmd.exe";
pub const DOWNLOAD_URL: &str = "https://steamcdn-a.akamaihd.net/client/installer/steamcmd.zip";
const MAX_ARCHIVE: u64 = 16 * 1024 * 1024;
const MAX_EXE: u64 = 32 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    version: u32,
    executable: PathBuf,
}

pub fn settings_path(root: &Path, local: bool) -> PathBuf {
    root.join(if local {
        "local-steamcmd.json"
    } else {
        "mock-steamcmd.json"
    })
}

pub fn read_setting(path: &Path) -> Result<Option<PathBuf>, String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(err(e)),
    };
    if !metadata.is_file() || metadata.len() > 65536 {
        return Err("Invalid SteamCMD settings file".into());
    }
    let file = File::open(path).map_err(err)?;
    let settings: Settings = serde_json::from_reader(file.take(65537))
        .map_err(|_| "SteamCMD の共通設定が不正です / Invalid shared SteamCMD settings")?;
    if settings.version != 1 || !settings.executable.is_absolute() {
        return Err("SteamCMD の共通設定が未対応です / Unsupported SteamCMD settings".into());
    }
    Ok(Some(settings.executable))
}

pub fn save_setting(path: &Path, executable: &Path) -> Result<(), String> {
    check_executable(executable)?;
    write_json(
        path,
        &Settings {
            version: 1,
            executable: executable.into(),
        },
    )
}

fn check_path(executable: &Path) -> Result<&Path, String> {
    validate(executable)?;
    if !executable
        .file_name()
        .is_some_and(|s| s.eq_ignore_ascii_case("steamcmd.exe"))
    {
        return Err(
            "steamcmd.exe の絶対パスを指定してください / Specify the absolute path to steamcmd.exe"
                .into(),
        );
    }
    executable
        .parent()
        .ok_or_else(|| "SteamCMD folder is missing".into())
}

fn check_pe(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 64 || &bytes[..2] != b"MZ" {
        return Err("Invalid SteamCMD executable / 実行ファイルの形式が不正です".into());
    }
    let offset = u32::from_le_bytes(bytes[60..64].try_into().unwrap()) as usize;
    let header = bytes.get(offset..offset.saturating_add(6));
    if !header
        .is_some_and(|h| &h[..4] == b"PE\0\0" && matches!(&h[4..6], [0x4c, 0x01] | [0x64, 0x86]))
    {
        return Err("Invalid Windows executable / Windows 実行ファイルではありません".into());
    }
    Ok(())
}

pub fn check_executable(executable: &Path) -> Result<(), String> {
    check_path(executable)?;
    if !fs::metadata(executable)
        .map_err(|e| format!("steamcmd.exe を確認できません / Cannot access steamcmd.exe: {e}"))?
        .is_file()
    {
        return Err(
            "steamcmd.exe は通常のファイルを指定してください / Select a regular file".into(),
        );
    }
    let mut bytes = Vec::new();
    File::open(executable)
        .map_err(err)?
        .take(MAX_EXE + 1)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() as u64 > MAX_EXE {
        return Err("SteamCMD executable is too large".into());
    }
    check_pe(&bytes)
}

/// Held through deployment or the complete SteamCMD invocation, including self-update.
pub struct Lease {
    _file: File,
}
impl Lease {
    pub fn acquire(executable: &Path) -> Result<Self, String> {
        // Runtime paths may use an existing, individually named executable.
        validate(executable)?;
        let directory = executable.parent().ok_or("SteamCMD folder is missing")?;
        fs::create_dir_all(directory).map_err(err)?;
        let lock = directory.join(".gsm-steamcmd.lock");
        validate(&lock)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock)
            .map_err(err)?;
        file.try_lock_exclusive()
            .map_err(|_| "この SteamCMD は使用中です / This SteamCMD installation is in use")?;
        Ok(Self { _file: file })
    }
}

#[derive(Clone, Debug)]
pub enum Progress {
    Download { bytes: u64, total: Option<u64> },
    Extract,
}

pub fn install(executable: &Path, progress: impl Fn(Progress)) -> Result<(), String> {
    check_path(executable)?;
    let _lease = Lease::acquire(executable)?;
    ensure_empty_installation(executable)?;
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(err)?;
    let response = client
        .get(DOWNLOAD_URL)
        .send()
        .map_err(err)?
        .error_for_status()
        .map_err(err)?;
    if response.status() != reqwest::StatusCode::OK {
        return Err("Unexpected download response / ダウンロード応答が不正です".into());
    }
    let total = response.content_length();
    if total.is_some_and(|n| n > MAX_ARCHIVE) {
        return Err("SteamCMD archive is too large".into());
    }
    let mut reader = response.take(MAX_ARCHIVE + 1);
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = reader.read(&mut buffer).map_err(err)?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..n]);
        if bytes.len() as u64 > MAX_ARCHIVE {
            return Err("SteamCMD archive is too large".into());
        }
        progress(Progress::Download {
            bytes: bytes.len() as u64,
            total,
        });
    }
    if total.is_some_and(|n| n != bytes.len() as u64) {
        return Err("SteamCMD download was incomplete".into());
    }
    progress(Progress::Extract);
    let binary = unpack(&bytes)?;
    // Check again after the network operation; never replace an existing file.
    ensure_empty_installation(executable)?;
    deploy(executable, &binary)
}

fn ensure_empty_installation(executable: &Path) -> Result<(), String> {
    let folder = check_path(executable)?;
    for entry in fs::read_dir(folder).map_err(err)? {
        if entry.map_err(err)?.file_name() != ".gsm-steamcmd.lock" {
            return Err("空のフォルダーを指定してください。既存の SteamCMD は「このパスを保存」で使用できます / Choose an empty folder, or use Save this path for an existing SteamCMD".into());
        }
    }
    Ok(())
}

fn unpack(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() as u64 > MAX_ARCHIVE {
        return Err("SteamCMD archive is too large".into());
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(err)?;
    // Valve's Windows bootstrap archive contains exactly one file. No arbitrary extraction.
    if archive.len() != 1 {
        return Err("Unexpected SteamCMD archive contents".into());
    }
    let mut file = archive.by_index(0).map_err(err)?;
    if file.name() != "steamcmd.exe" || file.is_dir() || file.is_symlink() || file.size() > MAX_EXE
    {
        return Err("Unexpected SteamCMD archive entry".into());
    }
    let expected = file.size();
    let mut result = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_EXE + 1)
        .read_to_end(&mut result)
        .map_err(err)?;
    if result.len() as u64 != expected {
        return Err("Invalid SteamCMD executable size".into());
    }
    check_pe(&result)?;
    Ok(result)
}

fn deploy(executable: &Path, bytes: &[u8]) -> Result<(), String> {
    let folder = check_path(executable)?;
    check_pe(bytes)?;
    let mut file = tempfile::NamedTempFile::new_in(folder).map_err(err)?;
    file.write_all(bytes).map_err(err)?;
    file.as_file().sync_all().map_err(err)?;
    file.persist_noclobber(executable).map_err(|e| format!("SteamCMD を配置できません（既存ファイルは上書きしません） / Cannot deploy SteamCMD without overwriting: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binary() -> Vec<u8> {
        let mut b = vec![0; 128];
        b[..2].copy_from_slice(b"MZ");
        b[60..64].copy_from_slice(&64u32.to_le_bytes());
        b[64..70].copy_from_slice(b"PE\0\0\x4c\x01");
        b
    }
    fn archive(name: &str, bytes: &[u8]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
        writer.finish().unwrap().into_inner()
    }
    #[test]
    #[ignore = "Downloads Valve's current bootstrap into a temporary directory; never executes it"]
    fn downloads_official_bootstrap_without_executing_or_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("steamcmd.exe");
        install(&exe, |_| {}).unwrap();
        check_executable(&exe).unwrap();
        let original = fs::read(&exe).unwrap();
        assert!(original.len() > 100_000);
        assert!(install(&exe, |_| {}).is_err());
        assert_eq!(fs::read(&exe).unwrap(), original);
    }
    #[test]
    fn rejects_unexpected_archives_traversal_and_non_executables() {
        assert_eq!(
            unpack(&archive("steamcmd.exe", &binary())).unwrap(),
            binary()
        );
        for name in [
            "../steamcmd.exe",
            "/steamcmd.exe",
            "other.exe",
            "dir/steamcmd.exe",
        ] {
            assert!(unpack(&archive(name, &binary())).is_err());
        }
        assert!(unpack(&archive("steamcmd.exe", b"not an executable")).is_err());
        assert!(unpack(b"not a zip").is_err());
        let mut b = binary();
        b[60..64].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(check_pe(&b).is_err());
    }
    #[test]
    fn installation_is_exclusive_and_never_replaces_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("steamcmd.exe");
        let lease = Lease::acquire(&exe).unwrap();
        assert!(Lease::acquire(&exe).is_err());
        ensure_empty_installation(&exe).unwrap();
        deploy(&exe, &binary()).unwrap();
        assert!(ensure_empty_installation(&exe).is_err());
        assert!(deploy(&exe, &binary()).is_err());
        assert_eq!(fs::read(&exe).unwrap(), binary());
        drop(lease);
        assert!(Lease::acquire(&exe).is_ok());
    }
    #[test]
    fn shared_path_is_opt_in_and_invalid_save_preserves_last_setting() {
        let dir = tempfile::tempdir().unwrap();
        let file = settings_path(dir.path(), true);
        assert_eq!(read_setting(&file).unwrap(), None);
        let exe = dir.path().join("steamcmd.exe");
        fs::write(&exe, binary()).unwrap();
        save_setting(&file, &exe).unwrap();
        assert_eq!(read_setting(&file).unwrap(), Some(exe.clone()));
        assert!(save_setting(&file, &dir.path().join("missing/steamcmd.exe")).is_err());
        assert_eq!(read_setting(&file).unwrap(), Some(exe));
        assert!(!settings_path(dir.path(), false).exists());
        fs::write(&file, "invalid").unwrap();
        assert!(read_setting(&file).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "invalid");
    }
    #[test]
    fn interrupted_or_failed_download_does_not_publish_partial_executable() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("steamcmd.exe");
        assert!(deploy(&exe, b"partial").is_err());
        assert!(!exe.exists());
        fs::write(dir.path().join("existing.cfg"), "keep").unwrap();
        assert!(ensure_empty_installation(&exe).is_err());
        assert_eq!(
            fs::read_to_string(dir.path().join("existing.cfg")).unwrap(),
            "keep"
        );
    }
}
