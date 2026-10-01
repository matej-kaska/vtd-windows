use anyhow::{Context, Result, ensure};
use std::{
    ffi::{CStr, CString, c_char, c_void},
    time::Instant,
};
use vtd::{audio, config::Config};

unsafe extern "C" {
    fn vtd_native_error() -> *const c_char;
    fn vtd_native_gpu(index: i32, name: *mut c_char, capacity: usize, discrete: *mut i32) -> i32;
    fn vtd_native_load(
        path: *const c_char,
        language: *const c_char,
        threads: i32,
        device: i32,
    ) -> *mut c_void;
    fn vtd_native_run(ctx: *mut c_void, pcm: *const f32, count: i32) -> *const c_char;
    fn vtd_native_free(ctx: *mut c_void);
}

pub struct Gpu {
    pub index: usize,
    pub name: String,
    pub discrete: bool,
}
pub fn gpus() -> Vec<Gpu> {
    let mut devices = Vec::new();
    loop {
        let mut name = [0; 512];
        let mut discrete = 0;
        if unsafe {
            vtd_native_gpu(
                devices.len() as i32,
                name.as_mut_ptr(),
                name.len(),
                &mut discrete,
            )
        } != 1
        {
            break;
        }
        devices.push(Gpu {
            index: devices.len(),
            name: unsafe { CStr::from_ptr(name.as_ptr()) }
                .to_string_lossy()
                .into_owned(),
            discrete: discrete != 0,
        });
    }
    devices
}
fn native_error() -> String {
    unsafe { CStr::from_ptr(vtd_native_error()) }
        .to_string_lossy()
        .into_owned()
}

pub struct Engine {
    ctx: *mut c_void,
}
impl Drop for Engine {
    fn drop(&mut self) {
        unsafe {
            vtd_native_free(self.ctx);
        }
    }
}
impl Engine {
    pub fn load(cfg: &Config) -> Result<Self> {
        ensure!(
            cfg.model.is_file(),
            "Model missing: {}. Choose and download a model in Settings.",
            cfg.model.display()
        );
        let devices = gpus();
        let gpu = if let Some(i) = cfg.gpu {
            devices.iter().find(|d| d.index == i)
        } else {
            devices.iter().find(|d| d.discrete).or(devices.first())
        }
        .context("Requested Vulkan GPU not available; no automatic CPU fallback")?;
        eprintln!("VTD GPU {}: {}", gpu.index, gpu.name);
        let start = Instant::now();
        let path = CString::new(cfg.model.to_str().context("Invalid model path")?)?;
        let language = CString::new(cfg.language.as_str())?;
        let ctx = unsafe {
            vtd_native_load(
                path.as_ptr(),
                language.as_ptr(),
                cfg.threads,
                gpu.index as i32,
            )
        };
        ensure!(!ctx.is_null(), "Loading speech model: {}", native_error());
        let mut engine = Self { ctx };
        engine.decode(&[0.0; 16000])?;
        eprintln!(
            "VTD model loaded and warmed in {:.2}s",
            start.elapsed().as_secs_f64()
        );
        Ok(engine)
    }

    pub fn transcribe(&mut self, cfg: &Config, samples: impl AsRef<[f32]>) -> Result<String> {
        let samples = samples.as_ref();
        ensure!(samples.len() <= 16000 * 300, "Recording exceeds 5 minutes");
        ensure!(
            samples.iter().all(|s| s.is_finite()),
            "Recording contains invalid audio samples"
        );
        if samples.len() < 4800 || !audio::audible(samples, cfg.silence_rms) {
            return Ok(String::new());
        }
        let mut remaining = samples;
        let mut text = String::new();
        while !remaining.is_empty() {
            let end = chunk_end(remaining);
            let chunk = &remaining[..end];
            if audio::audible(chunk, cfg.silence_rms) {
                let part = self.decode(chunk)?;
                if !part.is_empty() {
                    if !text.is_empty() {
                        text.push(' ');
                    }
                    text.push_str(&part);
                }
            }
            remaining = &remaining[end..];
        }
        if cfg.filter_subtitle_credits {
            text.truncate(vtd::text::without_subtitle_credit(&text).len());
        }
        Ok(text)
    }

    fn decode(&mut self, samples: &[f32]) -> Result<String> {
        let text = unsafe { vtd_native_run(self.ctx, samples.as_ptr(), samples.len() as i32) };
        ensure!(!text.is_null(), "Transcribing: {}", native_error());
        Ok(unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .trim()
            .to_owned())
    }
}

// Bound encoder workspace and decoder output at long-dictation lengths. Prefer
// a quiet 80 ms interval between 20 and 30 seconds; every input sample is retained.
fn chunk_end(samples: &[f32]) -> usize {
    const RATE: usize = 16000;
    if samples.len() <= 30 * RATE {
        return samples.len();
    }
    let max = (30 * RATE).min(samples.len() - 3 * RATE);
    let mut best = 20 * RATE;
    let mut energy = f64::INFINITY;
    for start in (20 * RATE..=max - 1280).step_by(320) {
        let next = samples[start..start + 1280]
            .iter()
            .map(|&x| f64::from(x).powi(2))
            .sum::<f64>();
        if next <= energy {
            best = start + 640;
            energy = next;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::chunk_end;
    #[test]
    fn long_dictation_splits_in_silence_and_keeps_every_sample() {
        let mut audio = vec![0.2; 16000 * 65];
        audio[16000 * 25..16000 * 26].fill(0.0);
        let split = chunk_end(&audio);
        assert!((16000 * 25..16000 * 26).contains(&split));
        let mut remaining = audio.as_slice();
        let mut total = 0;
        while !remaining.is_empty() {
            let end = chunk_end(remaining);
            assert!((4800..=16000 * 30).contains(&end));
            total += end;
            remaining = &remaining[end..];
        }
        assert_eq!(total, audio.len());
    }
}
