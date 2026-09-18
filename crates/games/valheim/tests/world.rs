use game_valheim::{
    config::WorldName,
    world::{WorldError, WorldLayout, inspect_world},
};
use std::{fs, path::Path};

fn world() -> WorldName {
    "Meadows".to_owned().try_into().unwrap()
}
fn write(root: &Path, name: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, b"synthetic fixture, not a playable save").unwrap();
}

#[test]
fn legacy_pair_empty_migration_shell_and_other_worlds_are_distinguished() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "Other.db");
    write(dir.path(), "Other.fwl");
    assert_eq!(
        inspect_world(dir.path(), &world()).unwrap().layout,
        WorldLayout::Missing
    );
    fs::create_dir(dir.path().join("Meadows")).unwrap();
    let empty = inspect_world(dir.path(), &world()).unwrap();
    assert_eq!(empty.layout, WorldLayout::EmptyFolder);
    assert!(!empty.needs_pre_migration_backup());
    write(dir.path(), "Meadows.db");
    assert_eq!(
        inspect_world(dir.path(), &world()).unwrap().layout,
        WorldLayout::IncompleteLegacy
    );
    write(dir.path(), "Meadows.fwl");
    write(dir.path(), "Meadows.db.old");
    let legacy = inspect_world(dir.path(), &world()).unwrap();
    assert_eq!(legacy.layout, WorldLayout::Legacy);
    assert!(legacy.needs_pre_migration_backup());
    assert_eq!(legacy.legacy_files.len(), 3);
    assert!(legacy.candidate_paths().all(|p| {
        p.file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("Meadows")
    }));
}

#[test]
fn old_backups_alone_do_not_substitute_for_the_active_pair() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "Meadows.db.old");
    write(dir.path(), "Meadows.fwl.old");
    let report = inspect_world(dir.path(), &world()).unwrap();
    assert_eq!(report.layout, WorldLayout::IncompleteLegacy);
    assert!(!report.legacy_pair_complete);
}

#[test]
fn generations_must_match_and_all_folder_contents_remain_in_scope() {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "_main.7.db2",
        "_main.7.fwl2",
        "_main.7.chunks",
        "_main.8.ok",
        "0_0.chunk",
        "foreign.ok",
    ] {
        write(dir.path(), &format!("Meadows/{name}"));
    }
    let incomplete = inspect_world(dir.path(), &world()).unwrap();
    assert_eq!(incomplete.layout, WorldLayout::Folder);
    assert!(
        incomplete
            .generations
            .iter()
            .all(|g| !g.has_required_files())
    );
    // An .ok marker can be empty. We inspect filenames, not save contents.
    fs::write(dir.path().join("Meadows/_main.7.ok"), b"").unwrap();
    write(dir.path(), "Meadows.db");
    write(dir.path(), "Meadows.fwl");
    let mixed = inspect_world(dir.path(), &world()).unwrap();
    assert_eq!(mixed.layout, WorldLayout::Mixed);
    assert!(!mixed.needs_pre_migration_backup());
    assert_eq!(
        mixed
            .generations
            .iter()
            .filter(|g| g.has_required_files())
            .map(|g| g.id.as_str())
            .collect::<Vec<_>>(),
        ["7"]
    );
    assert_eq!(mixed.candidate_paths().count(), 3);
    assert_eq!(
        mixed.folder.as_deref(),
        Some(dir.path().join("Meadows").as_path())
    );
    assert!(dir.path().join("Meadows/0_0.chunk").exists());
}

#[test]
fn arbitrary_folder_entry_is_not_a_completed_generation() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "Meadows/readme.txt");
    let report = inspect_world(dir.path(), &world()).unwrap();
    assert_eq!(report.layout, WorldLayout::Folder);
    assert!(report.generations.is_empty());
    write(dir.path(), "Meadows.db");
    write(dir.path(), "Meadows.fwl");
    let mixed = inspect_world(dir.path(), &world()).unwrap();
    assert_eq!(mixed.layout, WorldLayout::Mixed);
    assert!(mixed.needs_pre_migration_backup());
    fs::create_dir(dir.path().join("Meadows/_main.1.ok")).unwrap();
    assert!(matches!(
        inspect_world(dir.path(), &world()),
        Err(WorldError::WrongType(_))
    ));
}

#[test]
fn missing_or_wrong_type_roots_are_not_reported_as_missing_worlds() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        inspect_world(Path::new("relative"), &world()),
        Err(WorldError::InvalidRoot)
    ));
    assert!(inspect_world(&dir.path().join("missing"), &world()).is_err());
    write(dir.path(), "file");
    assert!(inspect_world(&dir.path().join("file"), &world()).is_err());
    write(dir.path(), "Meadows");
    assert!(matches!(
        inspect_world(dir.path(), &world()),
        Err(WorldError::WrongType(_))
    ));
}

#[cfg(unix)]
#[test]
fn linked_roots_worlds_and_top_level_save_files_are_rejected() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    symlink(elsewhere.path(), dir.path().join("linked-root")).unwrap();
    assert!(matches!(
        inspect_world(&dir.path().join("linked-root"), &world()),
        Err(WorldError::LinkedPath(_))
    ));
    symlink(elsewhere.path(), dir.path().join("Meadows")).unwrap();
    assert!(matches!(
        inspect_world(dir.path(), &world()),
        Err(WorldError::LinkedPath(_))
    ));
    fs::remove_file(dir.path().join("Meadows")).unwrap();
    fs::create_dir(dir.path().join("Meadows")).unwrap();
    write(elsewhere.path(), "_main.1.ok");
    symlink(
        elsewhere.path().join("_main.1.ok"),
        dir.path().join("Meadows/_main.1.ok"),
    )
    .unwrap();
    assert!(matches!(
        inspect_world(dir.path(), &world()),
        Err(WorldError::LinkedPath(_))
    ));
    assert!(elsewhere.path().join("_main.1.ok").exists());
}
