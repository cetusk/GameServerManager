use anyhow::{Result, bail};
use std::{ffi::OsString, path::PathBuf};

pub const HELP: &str = "GameServerManager\n\n  manager-gui --data-dir <absolute-path> --backend <mock|local> [--smoke-test]\n\nlocal: Windows servers on this PC. mock: synthetic development data.\nWithout arguments, local mode opens the saved directory or first-run setup. Mock mode and --smoke-test require --data-dir. --smoke-test never performs server operations.";
pub struct Options {
    pub data_dir: Option<PathBuf>,
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
        let local = backend.unwrap_or(data_dir.is_none());
        if data_dir.as_ref().is_some_and(|path| !path.is_absolute()) {
            bail!("--data-dir must be an absolute path");
        }
        if data_dir.is_none() && (!local || smoke_test) {
            bail!("Mock mode and smoke tests require an explicit --data-dir");
        }
        Ok(Some(Self {
            data_dir,
            smoke_test,
            local,
        }))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_launch_is_local_but_mock_and_smoke_remain_isolated() {
        let direct = Options::parse(Vec::<OsString>::new()).unwrap().unwrap();
        assert!(direct.local);
        assert!(direct.data_dir.is_none());
        assert!(Options::parse(["--data-dir", "relative"].map(OsString::from)).is_err());
        assert!(Options::parse(["--backend", "mock"].map(OsString::from)).is_err());
        assert!(Options::parse(["--smoke-test"].map(OsString::from)).is_err());
        let root = std::env::temp_dir();
        let explicit = Options::parse([OsString::from("--data-dir"), root.into_os_string()])
            .unwrap()
            .unwrap();
        assert!(!explicit.local); // Preserve existing development invocations.
    }
}
