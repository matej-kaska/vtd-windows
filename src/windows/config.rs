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
    pub toggle: bool,
    pub clipboard_paste: bool,
    pub idle_unload_seconds: u64,
    pub max_recording_seconds: u32,
    pub silence_rms: f32,
    pub threads: i32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: "models/ggml-large-v3-turbo-q5_0.bin".into(),
            microphone: None,
            gpu: None,
            language: "cs".into(),
            trigger_key: 0x77,
            toggle_key: 0x78,
            toggle: false,
            clipboard_paste: false,
            idle_unload_seconds: 300,
            max_recording_seconds: 120,
            silence_rms: 0.002,
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
            (0x70..=0x87).contains(&self.toggle_key) && self.toggle_key != self.trigger_key,
            "toggle_key must be F1-F24 and differ from trigger_key; default F9=120"
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
        ensure!(
            self.language == "auto" || whisper_rs::get_lang_id(&self.language).is_some(),
            "Unknown language"
        );
        Ok(())
    }

    pub fn write_default(path: &Path) -> Result<()> {
        if !path.exists() {
            std::fs::write(path, serde_json::to_string_pretty(&Self::default())?)?;
        }
        Ok(())
    }
}
