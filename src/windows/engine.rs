use anyhow::{Context, Result, ensure};
use std::{ffi::CStr, time::Instant};
use vtd::{audio, config::Config};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

#[derive(Debug)]
pub struct Gpu {
    pub index: usize,
    pub name: String,
    pub discrete: bool,
}

pub fn gpus() -> Vec<Gpu> {
    use whisper_rs::whisper_rs_sys::*;
    let mut result = Vec::new();
    unsafe {
        for i in 0..ggml_backend_dev_count() {
            let dev = ggml_backend_dev_get(i);
            let kind = ggml_backend_dev_type(dev);
            if kind == ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_GPU
                || kind == ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_IGPU
            {
                result.push(Gpu {
                    index: result.len(),
                    name: CStr::from_ptr(ggml_backend_dev_description(dev))
                        .to_string_lossy()
                        .into_owned(),
                    discrete: kind == ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_GPU,
                });
            }
        }
    }
    result
}

pub struct Engine {
    state: WhisperState,
}
impl Engine {
    pub fn load(cfg: &Config) -> Result<Self> {
        ensure!(
            cfg.language == "auto" || whisper_rs::get_lang_id(&cfg.language).is_some(),
            "Unknown language"
        );
        ensure!(
            cfg.model.is_file(),
            "Model missing: {}. Run scripts/download-model.ps1.",
            cfg.model.display()
        );
        let devices = gpus();
        let gpu = cfg
            .gpu
            .and_then(|i| devices.iter().find(|g| g.index == i))
            .or_else(|| {
                if cfg.gpu.is_none() {
                    devices.iter().find(|g| g.discrete).or(devices.first())
                } else {
                    None
                }
            })
            .context("Requested Vulkan GPU not available; no automatic CPU fallback")?;
        eprintln!("VTD GPU {}: {}", gpu.index, gpu.name);
        let start = Instant::now();
        let mut params = WhisperContextParameters::default();
        params
            .use_gpu(true)
            .gpu_device(gpu.index as i32)
            .flash_attn(true);
        let ctx = WhisperContext::new_with_params(&cfg.model, params).context("Loading Whisper")?;
        ensure!(
            ctx.is_multilingual(),
            "Use a multilingual model, not an .en model"
        );
        let state = ctx.create_state()?;
        let mut engine = Self { state };
        engine.decode(cfg, &[0.0; 16000])?;
        eprintln!(
            "VTD model loaded and warmed in {:.2}s",
            start.elapsed().as_secs_f64()
        );
        Ok(engine)
    }

    pub fn transcribe(&mut self, cfg: &Config, samples: &[f32]) -> Result<String> {
        ensure!(samples.len() <= 16000 * 300, "Recording exceeds 5 minutes");
        if samples.len() < 4800 || !audio::audible(samples, cfg.silence_rms) {
            return Ok(String::new());
        }
        let mut text = self.decode(cfg, samples)?;
        if cfg.filter_subtitle_credits {
            let len = without_subtitle_credit(&text).len();
            text.truncate(len);
        }
        Ok(text)
    }

    fn decode(&mut self, cfg: &Config, samples: &[f32]) -> Result<String> {
        let mut params = FullParams::new(SamplingStrategy::BeamSearch {
            beam_size: 5,
            patience: -1.0,
        });
        params.set_n_threads(cfg.threads);
        params.set_language((cfg.language != "auto").then_some(cfg.language.as_str()));
        params.set_translate(false);
        params.set_no_context(true);
        params.set_no_timestamps(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);
        params.set_temperature(0.0);
        params.set_temperature_inc(0.2);
        self.state.full(params, samples)?;
        let mut text = String::new();
        for segment in self.state.as_iter() {
            text.push_str(&segment.to_str_lossy()?);
        }
        Ok(text.trim().to_owned())
    }
}

fn without_subtitle_credit(text: &str) -> &str {
    let end = text
        .trim_end_matches(|c: char| c.is_whitespace() || matches!(c, '.' | ',' | '!' | ';' | '…'));
    let mut words = end.split_whitespace().rev();
    let Some(name) = words.next() else {
        return text;
    };
    let name = name.to_lowercase();
    let valid = match name.as_str() {
        "johnyx" | "johnnyx" => true,
        "x" => words
            .next()
            .is_some_and(|w| matches!(w.to_lowercase().as_str(), "johny" | "johnny")),
        _ => false,
    };
    if !valid
        || !words
            .next()
            .is_some_and(|w| matches!(w.to_lowercase().as_str(), "vytvořil" | "vytvoril"))
    {
        return text;
    }
    let Some(word) = words.next().filter(|w| w.to_lowercase() == "titulky") else {
        return text;
    };
    text[..word.as_ptr() as usize - text.as_ptr() as usize].trim_end()
}

#[cfg(test)]
mod tests {
    use super::without_subtitle_credit;
    #[test]
    fn removes_only_known_trailing_credit() {
        assert_eq!(
            without_subtitle_credit("Hotovo. Titulky vytvořil JohnyX."),
            "Hotovo."
        );
        assert_eq!(without_subtitle_credit("TITULKY VYTVOŘIL JOHNNY X!"), "");
        for text in [
            "Titulky vytvořil Petr.",
            "Děkuji za pozornost.",
            "Titulky vytvořil JohnyX. To je chyba.",
            "Řekl jsem JohnyX.",
        ] {
            assert_eq!(without_subtitle_credit(text), text);
        }
        assert_eq!(
            without_subtitle_credit("İstanbul. Titulky vytvořil JohnyX."),
            "İstanbul."
        );
    }
}
