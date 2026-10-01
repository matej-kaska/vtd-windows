use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineKind {
    Whisper,
    Transcribe,
}

pub struct Model {
    pub id: &'static str,
    pub name: &'static str,
    pub file: &'static str,
    pub engine: EngineKind,
    pub url: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
    pub details: &'static str,
    pub autodetect: bool,
    pub languages: &'static [&'static str],
}

include!(concat!(env!("OUT_DIR"), "/model_catalog.rs"));

impl Model {
    pub fn path(&self, config: &Path) -> PathBuf {
        config
            .parent()
            .unwrap_or(Path::new("."))
            .join("models")
            .join(self.file)
    }

    pub fn supports_language(&self, language: &str) -> bool {
        if language == "auto" {
            self.autodetect
        } else {
            self.languages.is_empty() || self.languages.contains(&language)
        }
    }
}

pub fn from_path(path: &Path) -> Option<&'static Model> {
    let file = path.file_name()?.to_str()?;
    MODELS.iter().find(|m| m.file.eq_ignore_ascii_case(file))
}

pub fn engine(path: &Path) -> EngineKind {
    use std::io::Read;
    let mut header = [0u8; 4];
    if std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .is_ok()
        && &header == b"GGUF"
    {
        return EngineKind::Transcribe;
    }
    from_path(path).map_or_else(
        || {
            if path
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("gguf"))
            {
                EngineKind::Transcribe
            } else {
                EngineKind::Whisper
            }
        },
        |m| m.engine,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_custom_gguf_by_content_and_checks_language_coverage() {
        let path =
            std::env::temp_dir().join(format!("vtd-custom-model-{}.bin", std::process::id()));
        std::fs::write(&path, b"GGUF").unwrap();
        assert_eq!(engine(&path), EngineKind::Transcribe);
        std::fs::write(&path, b"lmgg").unwrap();
        assert_eq!(engine(&path), EngineKind::Whisper);
        std::fs::remove_file(path).unwrap();
        assert!(MODELS.iter().all(|m| m.supports_language("cs")));
        assert!(!MODELS[0].supports_language("auto"));
        assert!(MODELS[1].supports_language("auto"));
        assert!(!MODELS[1].supports_language("ja"));
        assert!(MODELS[2].supports_language("ja"));
    }
}
