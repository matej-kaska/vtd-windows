use super::{audio, config::Config};
use anyhow::{Context, Result, ensure};
use std::{ffi::CStr, time::Instant};
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
            "Use a multilingual model for Czech, not an .en model"
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
        if samples.len() < 4800 || !audio::audible(samples, cfg.silence_rms) {
            return Ok(String::new());
        }
        ensure!(samples.len() <= 16000 * 300, "Recording exceeds 5 minutes");
        self.decode(cfg, samples)
    }

    fn decode(&mut self, cfg: &Config, samples: &[f32]) -> Result<String> {
        let mut params = FullParams::new(SamplingStrategy::BeamSearch {
            beam_size: 5,
            patience: -1.0,
        });
        params.set_n_threads(cfg.threads);
        params.set_language(Some(&cfg.language));
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
        params.set_temperature_inc(0.0);
        self.state.full(params, samples)?;
        let mut text = String::new();
        for segment in self.state.as_iter() {
            text.push_str(&segment.to_str_lossy()?);
        }
        Ok(text.trim().to_owned())
    }
}
