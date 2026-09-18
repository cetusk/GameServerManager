//! Explicit, read-only inspection; never resolves paths from the legacy configuration.
use game_valheim::{config::ConfigDocument, world::inspect_world};
use std::{error::Error, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 4 || args[0] != "--config" || args[2] != "--worlds-dir" {
        return Err("usage: inspect --config <absolute config.toml> --worlds-dir <absolute worlds_local directory>".into());
    }
    let config_path = PathBuf::from(&args[1]);
    let worlds_dir = PathBuf::from(&args[3]);
    if !config_path.is_absolute() || !worlds_dir.is_absolute() {
        return Err("--config and --worlds-dir must be absolute paths".into());
    }
    let document = ConfigDocument::parse(std::fs::read_to_string(config_path)?)?;
    let config = document.settings();
    let report = inspect_world(&worlds_dir, &config.server.world)?;
    println!("Config: parsed and validated (values and credentials omitted)");
    println!("World layout: {:?}", report.layout);
    println!("Legacy pair complete: {}", report.legacy_pair_complete);
    println!(
        "Pre-migration backup needed: {}",
        report.needs_pre_migration_backup()
    );
    println!(
        "Generations with required filenames: {}",
        report
            .generations
            .iter()
            .filter(|g| g.has_required_files())
            .count()
    );
    println!("Read-only evidence; game loadability and restore safety are not verified.");
    Ok(())
}
