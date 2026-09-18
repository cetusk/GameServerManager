use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};
pub fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub fn validate(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("絶対パス（親ディレクトリ移動なし）が必要です".into());
    }
    for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) => {
                let linked = meta.file_type().is_symlink();
                #[cfg(windows)]
                let linked = {
                    use std::os::windows::fs::MetadataExt;
                    linked || meta.file_attributes() & 0x400 != 0
                };
                if linked {
                    return Err(format!(
                        "リンク／reparse point は操作対象にできません: {}",
                        ancestor.display()
                    ));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(err(e)),
        }
    }
    Ok(path.to_owned())
}
pub fn exists(path: &Path) -> Result<bool, String> {
    validate(path)?;
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(err(e)),
    }
}
pub fn key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_start_matches("\\\\?\\")
        .trim_end_matches('\\')
        .to_lowercase()
}
pub fn overlap(a: &Path, b: &Path) -> bool {
    let (a, b) = (key(a), key(b));
    a == b || a.starts_with(&(b.clone() + "\\")) || b.starts_with(&(a + "\\"))
}
pub fn component(value: &str) -> bool {
    !value.is_empty()
        && value.len() < 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
pub fn remove(path: &Path) -> Result<(), String> {
    validate(path)?;
    if !exists(path)? {
        return Ok(());
    }
    if path.is_dir() {
        fs::remove_dir_all(path).map_err(err)
    } else {
        fs::remove_file(path).map_err(err)
    }
}
pub fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), String> {
    use std::io::Write;
    validate(path)?;
    let parent = path.parent().ok_or("保存先に親ディレクトリがありません")?;
    fs::create_dir_all(parent).map_err(err)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(err)?;
    serde_json::to_writer_pretty(&mut tmp, value).map_err(err)?;
    tmp.flush().map_err(err)?;
    tmp.as_file().sync_all().map_err(err)?;
    tmp.persist(path).map_err(err)?;
    Ok(())
}
