//! App-only preferences. Never includes server paths, credentials or world data.
mod writer;
pub use writer::PreferenceWriter;

use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub format: String,
    pub version: u32,
    pub theme: String,
    pub language: String,
    pub notify_completed: bool,
    pub notify_failed: bool,
    pub sound: bool,
    pub remember_game: bool,
    pub remember_tab: bool,
    pub last_game: Option<String>,
    pub last_tab: i32,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            format: "gsm-app-preferences".into(),
            version: 1,
            theme: "turquoise".into(),
            language: "ja".into(),
            notify_completed: true,
            notify_failed: true,
            sound: false,
            remember_game: false,
            remember_tab: false,
            last_game: None,
            last_tab: 0,
        }
    }
}
impl Preferences {
    pub fn english(&self) -> bool {
        self.language == "en"
    }
    pub fn read(path: &Path) -> Result<Self, String> {
        if !std::fs::metadata(path)
            .map_err(|e| e.to_string())?
            .is_file()
        {
            return Err(
                "Settings must be a regular file / 設定は通常のファイルを選択してください".into(),
            );
        }
        let mut bytes = Vec::new();
        File::open(path)
            .map_err(|e| e.to_string())?
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        Self::parse(&bytes)
    }
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 65536 {
            return Err("Settings file exceeds 64 KiB / 設定ファイルが大きすぎます".into());
        }
        let prefs: Self = serde_json::from_slice(bytes)
            .map_err(|_| "Invalid settings JSON / 設定 JSON の形式が不正です")?;
        if prefs.format != "gsm-app-preferences"
            || prefs.version != 1
            || !["turquoise", "blue", "violet"].contains(&prefs.theme.as_str())
            || !["ja", "en"].contains(&prefs.language.as_str())
            || !(0..=4).contains(&prefs.last_tab)
            || prefs.last_game.as_ref().is_some_and(|s| {
                !["arksa", "valheim", "windrose", "satisfactory", "conan"].contains(&s.as_str())
            })
        {
            return Err(
                "Unsupported settings value or version / 未対応の設定値またはバージョンです".into(),
            );
        }
        Ok(prefs)
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let mut value = self.clone();
        if !value.remember_game {
            value.last_game = None;
        }
        if !value.remember_tab {
            value.last_tab = 0;
        }
        let mut file =
            tempfile::NamedTempFile::new_in(path.parent().ok_or("Missing settings directory")?)
                .map_err(|e| e.to_string())?;
        serde_json::to_writer_pretty(file.as_file_mut(), &value).map_err(|e| e.to_string())?;
        file.write_all(b"\n").map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.persist(path).map_err(|e| e.error.to_string())?;
        Ok(())
    }
    pub fn summary(&self) -> String {
        format!(
            "Theme / テーマ: {}\nLanguage / 言語: {}\nCompletion / 完了通知: {}\nFailure / 失敗通知: {}\nSound / 通知音: {}\nRemember game / ゲームを記憶: {}\nRemember tab / タブを記憶: {}\nGame / ゲーム: {}\nTab / タブ: {}",
            self.theme,
            self.language,
            self.notify_completed,
            self.notify_failed,
            self.sound,
            self.remember_game,
            self.remember_tab,
            self.last_game.as_deref().unwrap_or("—"),
            self.last_tab + 1
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transfer_rejects_unknown_fields_versions_and_oversize_without_changing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prefs.json");
        let prefs = Preferences::default();
        prefs.save(&path).unwrap();
        assert_eq!(Preferences::read(&path).unwrap(), prefs);
        for invalid in [
            serde_json::json!({"extra": true}),
            serde_json::json!({"version":99}),
            serde_json::json!({"theme":"white"}),
            serde_json::json!({"last_game":"unknown"}),
        ] {
            let mut value = serde_json::to_value(&prefs).unwrap();
            value
                .as_object_mut()
                .unwrap()
                .extend(invalid.as_object().unwrap().clone());
            assert!(Preferences::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        assert!(Preferences::parse(&vec![b' '; 65537]).is_err());
        assert_eq!(Preferences::read(&path).unwrap(), prefs);
    }
    #[test]
    fn navigation_memory_is_opt_in_by_default() {
        let p = Preferences::default();
        assert!(!p.remember_game && !p.remember_tab);
        assert_eq!(p.last_game, None);
        assert_eq!(p.last_tab, 0);
        let enabled = Preferences {
            remember_game: true,
            remember_tab: true,
            ..p
        };
        assert!(
            Preferences::parse(&serde_json::to_vec(&enabled).unwrap())
                .unwrap()
                .remember_tab
        );
    }
    #[test]
    fn disabled_memory_does_not_export_navigation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prefs.json");
        let p = Preferences {
            remember_game: false,
            remember_tab: false,
            last_game: Some("valheim".into()),
            last_tab: 4,
            ..Default::default()
        };
        p.save(&path).unwrap();
        let actual = Preferences::read(&path).unwrap();
        assert_eq!(actual.last_game, None);
        assert_eq!(actual.last_tab, 0);
    }
}
