use anyhow::{Context, Result, ensure};
use std::{
    ffi::{CStr, CString, c_char},
    ptr::{null, null_mut},
    time::Instant,
};
use vtd::{audio, config::Config};
use whisper_rs::whisper_rs_sys::*;

#[derive(Debug)]
pub struct Gpu {
    pub index: usize,
    pub name: String,
    pub discrete: bool,
}

pub fn gpus() -> Vec<Gpu> {
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
    ctx: *mut whisper_context,
    state: *mut whisper_state,
    language: *const c_char,
}
impl Drop for Engine {
    fn drop(&mut self) {
        unsafe {
            if !self.state.is_null() {
                whisper_free_state(self.state);
            }
            whisper_free(self.ctx);
        }
    }
}
impl Engine {
    pub fn load(cfg: &Config) -> Result<Self> {
        let language = (cfg.language != "auto")
            .then(|| whisper_rs::get_lang_id(&cfg.language).context("Unknown language"))
            .transpose()?;
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
        let path = CString::new(cfg.model.to_str().context("Invalid model path")?)?;
        let mut engine = unsafe {
            let mut params = whisper_context_default_params();
            params.use_gpu = true;
            params.gpu_device = gpu.index as i32;
            params.flash_attn = true;
            let ctx = whisper_init_from_file_with_params_no_state(path.as_ptr(), params);
            ensure!(!ctx.is_null(), "Loading Whisper");
            let mut engine = Self {
                ctx,
                state: null_mut(),
                language: language.map_or(null(), |id| whisper_lang_str(id)),
            };
            ensure!(
                whisper_is_multilingual(ctx) != 0,
                "Use a multilingual model, not an .en model"
            );
            engine.state = whisper_init_state(ctx);
            ensure!(!engine.state.is_null(), "Creating Whisper state");
            engine
        };
        engine.prepare(cfg, &[0.0; 16000])?;
        engine.decode(cfg)?;
        eprintln!(
            "VTD model loaded and warmed in {:.2}s",
            start.elapsed().as_secs_f64()
        );
        Ok(engine)
    }

    pub fn transcribe(&mut self, cfg: &Config, samples: impl AsRef<[f32]>) -> Result<String> {
        ensure!(
            samples.as_ref().len() <= 16000 * 300,
            "Recording exceeds 5 minutes"
        );
        if samples.as_ref().len() < 4800 || !audio::audible(samples.as_ref(), cfg.silence_rms) {
            return Ok(String::new());
        }
        self.prepare(cfg, samples.as_ref())?;
        drop(samples);
        let mut text = self.decode(cfg)?;
        if cfg.filter_subtitle_credits {
            let len = without_subtitle_credit(&text).len();
            text.truncate(len);
        }
        Ok(text)
    }

    fn prepare(&mut self, cfg: &Config, samples: &[f32]) -> Result<()> {
        let result = unsafe {
            whisper_pcm_to_mel_with_state(
                self.ctx,
                self.state,
                samples.as_ptr(),
                samples.len() as i32,
                cfg.threads,
            )
        };
        ensure!(result == 0, "Calculating spectrogram: {result}");
        Ok(())
    }

    fn decode(&mut self, cfg: &Config) -> Result<String> {
        let mut text = String::new();
        unsafe {
            let mut params =
                whisper_full_default_params(whisper_sampling_strategy_WHISPER_SAMPLING_BEAM_SEARCH);
            params.beam_search.beam_size = 5;
            params.beam_search.patience = -1.0;
            params.n_threads = cfg.threads;
            params.language = self.language;
            params.translate = false;
            params.no_context = true;
            params.no_timestamps = true;
            params.print_special = false;
            params.print_progress = false;
            params.print_realtime = false;
            params.print_timestamps = false;
            params.suppress_blank = true;
            params.suppress_nst = true;
            params.temperature = 0.0;
            params.temperature_inc = 0.2;
            let result = whisper_full_with_state(self.ctx, self.state, params, null(), 0);
            ensure!(result == 0, "Transcribing: {result}");
            let result = whisper_set_mel_with_state(self.ctx, self.state, null(), 0, 80);
            ensure!(result == 0, "Releasing spectrogram: {result}");
            for i in 0..whisper_full_n_segments_from_state(self.state) {
                let segment = whisper_full_get_segment_text_from_state(self.state, i);
                ensure!(!segment.is_null(), "Missing segment text");
                text.push_str(&CStr::from_ptr(segment).to_string_lossy());
            }
        }
        let start = text.len() - text.trim_start().len();
        text.truncate(text.trim_end().len());
        text.drain(..start.min(text.len()));
        Ok(text)
    }
}

fn without_subtitle_credit(text: &str) -> &str {
    let end = text
        .trim_end_matches(|c: char| c.is_whitespace() || matches!(c, '.' | ',' | '!' | ';' | '…'));
    let mut words = end.split_whitespace().rev();
    let Some(name) = words.next() else {
        return text;
    };
    let equal = |a: &str, b: &str| a.chars().flat_map(char::to_lowercase).eq(b.chars());
    let valid = equal(name, "johnyx")
        || equal(name, "johnnyx")
        || (equal(name, "x")
            && words
                .next()
                .is_some_and(|w| equal(w, "johny") || equal(w, "johnny")));
    if !valid
        || !words
            .next()
            .is_some_and(|w| equal(w, "vytvořil") || equal(w, "vytvoril"))
    {
        return text;
    }
    let Some(word) = words.next().filter(|w| equal(w, "titulky")) else {
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
