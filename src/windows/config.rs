use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub model: PathBuf,
    pub microphone: Option<String>,
    pub gpu: Option<usize>,
    pub language: String,
    pub trigger_key: u32,
    pub toggle_key: u32,
    pub replay_key: u32,
    pub toggle: bool,
    pub clipboard_paste: bool,
    pub mute_output: bool,
    pub idle_unload_seconds: u64,
    pub max_recording_seconds: u32,
    pub silence_rms: f32,
    pub filter_subtitle_credits: bool,
    pub threads: i32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: "models/ggml-large-v3-turbo-q5_0.bin".into(),
            microphone: None,
            gpu: None,
            language: crate::languages::system_language().into(),
            trigger_key: 0x77,
            toggle_key: 0x78,
            replay_key: 0x79,
            toggle: false,
            clipboard_paste: true,
            mute_output: false,
            idle_unload_seconds: 30,
            max_recording_seconds: 300,
            silence_rms: 0.002,
            filter_subtitle_credits: true,
            threads: 4,
        }
    }
}

pub fn path() -> Result<PathBuf> {
    Ok(std::env::current_exe()?
        .parent()
        .context("Executable has no parent")?
        .join("vtd.json"))
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let mut cfg: Self = if path.exists() {
            let bytes = std::fs::read(path)?;
            serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))
                .context("Invalid vtd.json")?
        } else {
            Self::default()
        };
        cfg.validate()?;
        if cfg.model.is_relative() {
            cfg.model = path.parent().unwrap_or(Path::new(".")).join(&cfg.model);
        }
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            (0x70..=0x87).contains(&self.trigger_key),
            "trigger_key must be F1-F24 (112-135); default F8=119"
        );
        ensure!(
            (0x70..=0x87).contains(&self.toggle_key),
            "toggle_key must be F1-F24; default F9=120"
        );
        ensure!(
            (0x70..=0x87).contains(&self.replay_key),
            "replay_key must be F1-F24; default F10=121"
        );
        ensure!(
            self.trigger_key != self.toggle_key
                && self.trigger_key != self.replay_key
                && self.toggle_key != self.replay_key,
            "Each action must use a different key."
        );
        ensure!(
            (1..=300).contains(&self.max_recording_seconds),
            "max_recording_seconds must be 1-300"
        );
        ensure!((1..=16).contains(&self.threads), "threads must be 1-16");
        ensure!(
            self.silence_rms.is_finite() && (0.0..=0.1).contains(&self.silence_rms),
            "silence_rms must be 0-0.1"
        );
        Ok(())
    }

    pub fn write_default(path: &Path) -> Result<()> {
        use std::io::Write;
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(mut file) => file.write_all(&serde_json::to_vec_pretty(&Self::default())?)?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }

    pub fn save_preferences(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let bytes = std::fs::read(path)?;
        let mut value: serde_json::Value =
            serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))?;
        let object = value.as_object_mut().context("Invalid vtd.json")?;
        object.insert("trigger_key".into(), self.trigger_key.into());
        object.insert("toggle_key".into(), self.toggle_key.into());
        object.insert("replay_key".into(), self.replay_key.into());
        object.insert("language".into(), self.language.clone().into());
        object.insert("mute_output".into(), self.mute_output.into());
        serde_json::from_value::<Self>(value.clone())?.validate()?;
        // Preserve unrelated preferences and the original relative model path.
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, serde_json::to_vec_pretty(&value)?)?;
        if let Err(e) = std::fs::rename(&temp, path) {
            let _ = std::fs::remove_file(temp);
            return Err(e.into());
        }
        Ok(())
    }

    pub fn set_mute_output(enabled: bool) -> Result<()> {
        let path = path()?;
        let bytes = std::fs::read(&path)?;
        let mut value: serde_json::Value =
            serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))?;
        value
            .as_object_mut()
            .context("Invalid vtd.json")?
            .insert("mute_output".into(), enabled.into());
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, serde_json::to_vec_pretty(&value)?)?;
        std::fs::rename(temp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_are_distinct_and_replay_can_move_off_f10() {
        let mut cfg = Config {
            trigger_key: 0x79,
            replay_key: 0x70,
            ..Config::default()
        };
        assert!(cfg.validate().is_ok());
        for (trigger, toggle, replay) in [
            (119, 119, 121),
            (119, 120, 119),
            (119, 120, 120),
            (119, 120, 27),
        ] {
            cfg.trigger_key = trigger;
            cfg.toggle_key = toggle;
            cfg.replay_key = replay;
            assert!(cfg.validate().is_err());
        }
    }

    #[test]
    fn preferences_preserve_existing_language_paths_and_other_settings() {
        let dir = std::env::temp_dir().join(format!("vtd-config-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vtd.json");
        std::fs::write(
            &path,
            br#"{"language":"de","model":"models/custom.bin","threads":7,"mute_output":true}"#,
        )
        .unwrap();
        Config::write_default(&path).unwrap();
        let mut cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.language, "de");
        assert_eq!(cfg.replay_key, 121); // Backwards compatibility with older configs.
        cfg.language = "en".into();
        cfg.trigger_key = 121;
        cfg.replay_key = 122;
        cfg.mute_output = false;
        cfg.save_preferences(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["model"], "models/custom.bin");
        assert_eq!(saved["threads"], 7);
        assert_eq!(saved["mute_output"], false);
        assert_eq!(saved["language"], "en");
        assert_eq!(Config::load(&path).unwrap().replay_key, 122);
        let before = std::fs::read(&path).unwrap();
        cfg.toggle_key = cfg.trigger_key;
        assert!(cfg.save_preferences(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::remove_file(&path).unwrap();
        Config::write_default(&path).unwrap();
        assert_eq!(
            Config::load(&path).unwrap().language,
            crate::languages::system_language()
        );
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
