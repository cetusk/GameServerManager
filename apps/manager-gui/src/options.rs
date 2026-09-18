use anyhow::{Result, bail};
use std::{ffi::OsString, path::PathBuf};

pub const HELP: &str = "GameServerManager\n\n  manager-gui --data-dir <absolute-path> --backend <mock|local> [--smoke-test]\n\nlocal: Windows servers on this PC. mock: synthetic development data.\nAn explicit data directory is required. --smoke-test never performs server operations.";
pub struct Options {
    pub data_dir: PathBuf,
    pub smoke_test: bool,
    pub local: bool,
}
impl Options {
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Option<Self>> {
        let mut args = args.into_iter();
        let mut data_dir = None;
        let mut smoke_test = false;
        let mut backend = None;
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--help" | "-h") => return Ok(None),
                Some("--data-dir") => {
                    if data_dir.is_some() {
                        bail!("--data-dir was specified twice");
                    }
                    data_dir =
                        Some(PathBuf::from(args.next().ok_or_else(|| {
                            anyhow::anyhow!("--data-dir requires a path")
                        })?));
                }
                Some("--backend") => {
                    if backend.is_some() {
                        bail!("--backend was specified twice");
                    }
                    backend = Some(
                        match args
                            .next()
                            .and_then(|s| s.to_str().map(str::to_owned))
                            .as_deref()
                        {
                            Some("mock") => false,
                            Some("local") => true,
                            _ => bail!("--backend must be mock or local"),
                        },
                    );
                }
                Some("--smoke-test") => smoke_test = true,
                _ => bail!("Unknown argument: {}", arg.to_string_lossy()),
            }
        }
        let data_dir = data_dir.ok_or_else(|| anyhow::anyhow!("--data-dir <absolute-path> is required. Use run-dev.ps1 for an isolated development directory."))?;
        if !data_dir.is_absolute() {
            bail!("--data-dir must be an absolute path");
        }
        Ok(Some(Self {
            data_dir,
            smoke_test,
            local: backend.unwrap_or(false),
        }))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_roots_and_local_backend_cannot_fall_back_to_production() {
        assert!(Options::parse(Vec::<OsString>::new()).is_err());
        assert!(Options::parse(["--data-dir", "relative"].map(OsString::from)).is_err());
        assert!(Options::parse(["--backend", "local"].map(OsString::from)).is_err());
    }
}
