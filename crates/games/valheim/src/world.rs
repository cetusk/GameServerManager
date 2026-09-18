//! Read-only layout evidence, based on the supplied legacy world.rs and spec §12.
//! A complete set of filenames does not prove that the game can load its contents.
use crate::config::WorldName;
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

pub const WORLDS_SUBDIR: &str = "worlds_local";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldLayout {
    Missing,
    EmptyFolder,
    Legacy,
    IncompleteLegacy,
    Folder,
    Mixed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Generation {
    pub id: String,
    pub db2: bool,
    pub fwl2: bool,
    pub chunks: bool,
    pub ok: bool,
}
impl Generation {
    /// Filename/type evidence only, not content validation or restore permission.
    pub fn has_required_files(&self) -> bool {
        self.db2 && self.fwl2 && self.chunks && self.ok
    }
}

#[derive(Clone, Debug)]
pub struct WorldInspection {
    pub layout: WorldLayout,
    pub folder: Option<PathBuf>,
    pub folder_has_entries: bool,
    pub legacy_files: Vec<PathBuf>,
    pub legacy_pair_complete: bool,
    pub generations: Vec<Generation>,
}
impl WorldInspection {
    /// Preserve the legacy escape route until a same-generation file set exists.
    /// An empty shell or a partly written folder is not evidence of completed migration.
    pub fn needs_pre_migration_backup(&self) -> bool {
        self.legacy_pair_complete && !self.generations.iter().any(Generation::has_required_files)
    }
    /// Folder snapshots must include the entire tree, not just one generation.
    /// A future copy service must revalidate all paths and server state.
    pub fn candidate_paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.folder.iter().chain(self.legacy_files.iter())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WorldError {
    #[error("ワールド検索元には絶対パスの既存ディレクトリを指定してください")]
    InvalidRoot,
    #[error("リンク／reparse point はこの検査では扱えません: {0}")]
    LinkedPath(PathBuf),
    #[error("想定外のファイル種別です: {0}")]
    WrongType(PathBuf),
    #[error("ワールドを読み取れません: {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}
fn io_error(path: &Path, source: io::Error) -> WorldError {
    WorldError::Io {
        path: path.to_owned(),
        source,
    }
}
fn checked_metadata(path: &Path) -> Result<Option<fs::Metadata>, WorldError> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_error(path, e)),
    };
    let mut linked = meta.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        linked |= meta.file_attributes() & 0x400 != 0; // FILE_ATTRIBUTE_REPARSE_POINT
    }
    #[cfg(not(windows))]
    let _ = &mut linked;
    if linked {
        return Err(WorldError::LinkedPath(path.to_owned()));
    }
    Ok(Some(meta))
}

/// Inspect only the explicitly supplied world's top-level layout, without copying,
/// deleting, following links, discovering user profiles, or parsing save contents.
/// IO failures are errors, never evidence of an absent or stopped server.
pub fn inspect_world(root: &Path, world: &WorldName) -> Result<WorldInspection, WorldError> {
    if !root.is_absolute()
        || root
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(WorldError::InvalidRoot);
    }
    // Reject linked ancestors too. This remains a read-only probe, not a race-free IO sandbox.
    for ancestor in root.ancestors().collect::<Vec<_>>().into_iter().rev() {
        let meta = checked_metadata(ancestor)?.ok_or(WorldError::InvalidRoot)?;
        if !meta.is_dir() {
            return Err(WorldError::InvalidRoot);
        }
    }
    let world = world.as_str();
    let folder_path = root.join(world);
    let folder = match checked_metadata(&folder_path)? {
        Some(meta) if meta.is_dir() => Some(folder_path),
        Some(_) => return Err(WorldError::WrongType(folder_path)),
        None => None,
    };
    let mut folder_has_entries = false;
    let mut generations = BTreeMap::<String, Generation>::new();
    if let Some(folder) = &folder {
        for entry in fs::read_dir(folder).map_err(|e| io_error(folder, e))? {
            let entry = entry.map_err(|e| io_error(folder, e))?;
            let path = entry.path();
            let meta = checked_metadata(&path)?.ok_or_else(|| {
                io_error(
                    &path,
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        "entry disappeared during inspection",
                    ),
                )
            })?;
            folder_has_entries = true;
            let name = entry.file_name();
            let Some((id, suffix)) = name
                .to_str()
                .and_then(|n| n.strip_prefix("_main."))
                .and_then(|n| n.rsplit_once('.'))
            else {
                continue;
            };
            if id.is_empty()
                || !id.bytes().all(|b| b.is_ascii_digit())
                || !matches!(suffix, "db2" | "fwl2" | "chunks" | "ok")
            {
                continue;
            }
            if !meta.is_file() {
                return Err(WorldError::WrongType(path));
            }
            let generation = generations.entry(id.into()).or_insert_with(|| Generation {
                id: id.into(),
                db2: false,
                fwl2: false,
                chunks: false,
                ok: false,
            });
            match suffix {
                "db2" => generation.db2 = true,
                "fwl2" => generation.fwl2 = true,
                "chunks" => generation.chunks = true,
                "ok" => generation.ok = true,
                _ => unreachable!(),
            }
        }
    }
    let mut legacy_files = Vec::new();
    let mut pair = [false; 2];
    for (index, suffix) in ["db", "fwl", "db.old", "fwl.old"].iter().enumerate() {
        let path = root.join(format!("{world}.{suffix}"));
        if let Some(meta) = checked_metadata(&path)? {
            if !meta.is_file() {
                return Err(WorldError::WrongType(path));
            }
            if index < 2 {
                pair[index] = true;
            }
            legacy_files.push(path);
        }
    }
    let legacy_pair_complete = pair.iter().all(|v| *v);
    let layout = if folder_has_entries {
        if legacy_files.is_empty() {
            WorldLayout::Folder
        } else {
            WorldLayout::Mixed
        }
    } else if legacy_pair_complete {
        WorldLayout::Legacy
    } else if !legacy_files.is_empty() {
        WorldLayout::IncompleteLegacy
    } else if folder.is_some() {
        WorldLayout::EmptyFolder
    } else {
        WorldLayout::Missing
    };
    Ok(WorldInspection {
        layout,
        folder,
        folder_has_entries,
        legacy_files,
        legacy_pair_complete,
        generations: generations.into_values().collect(),
    })
}
