use super::config::Config;
use anyhow::{Context, Result, bail, ensure};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub fn devices() -> Result<Vec<String>> {
    cpal::default_host()
        .input_devices()?
        .map(|d| Ok(d.description()?.name().to_owned()))
        .collect()
}

struct Buffer {
    samples: Vec<f32>,
    error: Option<String>,
}
pub struct Recording {
    stream: cpal::Stream,
    data: Arc<Mutex<Buffer>>,
    rate: u32,
    started: Instant,
}

impl Recording {
    pub fn start(cfg: &Config) -> Result<Self> {
        let host = cpal::default_host();
        let device = if let Some(name) = &cfg.microphone {
            host.input_devices()?
                .find(|d| d.description().is_ok_and(|v| v.name() == name))
        } else {
            host.default_input_device()
        }
        .context("Microphone not found; use vtd devices")?;
        let supported = device.default_input_config()?;
        let config: cpal::StreamConfig = supported.clone().into();
        let rate = config.sample_rate;
        let channels = config.channels as usize;
        eprintln!(
            "VTD microphone: {}, {rate} Hz, {channels} channels",
            device.description()?.name()
        );
        let limit = rate as usize * cfg.max_recording_seconds as usize;
        let data = Arc::new(Mutex::new(Buffer {
            samples: Vec::with_capacity(limit),
            error: None,
        }));
        macro_rules! stream {
            ($ty:ty, $convert:expr) => {{
                let target = data.clone();
                let errors = data.clone();
                device.build_input_stream(
                    &config,
                    move |input: &[$ty], _: &cpal::InputCallbackInfo| {
                        if let Ok(mut buf) = target.lock() {
                            let left = limit.saturating_sub(buf.samples.len());
                            for frame in input.chunks_exact(channels).take(left) {
                                buf.samples.push(
                                    frame.iter().map($convert).sum::<f32>() / channels as f32,
                                );
                            }
                        }
                    },
                    move |err| {
                        if let Ok(mut buf) = errors.lock() {
                            buf.error = Some(err.to_string());
                        }
                    },
                    None,
                )?
            }};
        }
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => stream!(f32, |x: &f32| *x),
            cpal::SampleFormat::I16 => stream!(i16, |x: &i16| *x as f32 / 32768.0),
            cpal::SampleFormat::U16 => stream!(u16, |x: &u16| (*x as f32 - 32768.0) / 32768.0),
            other => bail!("Unsupported microphone format: {other}"),
        };
        stream.play()?;
        Ok(Self {
            stream,
            data,
            rate,
            started: Instant::now(),
        })
    }

    pub fn finish(self) -> Result<(Vec<f32>, u32)> {
        drop(self.stream);
        let mut buf = self
            .data
            .lock()
            .map_err(|_| anyhow::anyhow!("Audio lock poisoned"))?;
        if let Some(err) = &buf.error {
            bail!("Microphone: {err}");
        }
        let seconds = buf.samples.len() as f64 / self.rate as f64;
        let rms = (buf.samples.iter().map(|x| (*x as f64).powi(2)).sum::<f64>()
            / buf.samples.len().max(1) as f64)
            .sqrt();
        eprintln!(
            "VTD capture: {seconds:.2}s audio, {:.2}s elapsed, RMS {rms:.4}",
            self.started.elapsed().as_secs_f64()
        );
        Ok((std::mem::take(&mut buf.samples), self.rate))
    }
}

pub fn resample(input: &[f32], rate: u32) -> Vec<f32> {
    if rate == 16000 {
        return input.to_vec();
    }
    if rate == 0 || input.is_empty() {
        return Vec::new();
    }
    let ratio = rate as f64 / 16000.0;
    let cutoff = (1.0 / ratio).min(1.0) * 0.94;
    let radius = (16.0 / cutoff).ceil() as i64;
    let count = (input.len() as f64 / ratio) as usize;
    (0..count)
        .map(|i| {
            let pos = i as f64 * ratio;
            let mid = pos.floor() as i64;
            let (mut value, mut weight) = (0.0, 0.0);
            for n in mid - radius..=mid + radius {
                if n < 0 || n >= input.len() as i64 {
                    continue;
                }
                let d = pos - n as f64;
                let x = std::f64::consts::PI * d * cutoff;
                let sinc = if x.abs() < 1e-8 { 1.0 } else { x.sin() / x };
                let window = 0.5 * (1.0 + (std::f64::consts::PI * d / radius as f64).cos());
                let w = sinc * window;
                value += input[n as usize] as f64 * w;
                weight += w;
            }
            (value / weight) as f32
        })
        .collect()
}

pub fn audible(samples: &[f32], threshold: f32) -> bool {
    samples
        .chunks(320)
        .filter(|frame| {
            let rms = (frame.iter().map(|x| x * x).sum::<f32>() / frame.len() as f32).sqrt();
            rms > threshold
        })
        .take(5)
        .count()
        == 5
}

pub fn read_wav(path: &std::path::Path) -> Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    ensure!(
        spec.channels > 0 && spec.sample_rate > 0,
        "Invalid WAV format"
    );
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 2f32.powi(spec.bits_per_sample as i32 - 1);
            reader
                .samples::<i32>()
                .map(|s| s.map(|x| x as f32 / scale))
                .collect::<Result<_, _>>()?
        }
    };
    let mono: Vec<f32> = raw
        .chunks_exact(spec.channels as usize)
        .map(|f| f.iter().sum::<f32>() / spec.channels as f32)
        .collect();
    Ok(resample(&mono, spec.sample_rate))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn silence_and_click_do_not_trigger() {
        let mut x = vec![0.0; 16000];
        assert!(!audible(&x, 0.002));
        x[100] = 1.0;
        assert!(!audible(&x, 0.002));
        assert!(audible(&vec![0.1; 16000], 0.002));
    }
    #[test]
    fn resampling_preserves_duration_and_rejects_aliasing() {
        for rate in [44100, 48000] {
            let sine = |hz: f32| {
                (0..rate)
                    .map(|i| (2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin())
                    .collect::<Vec<_>>()
            };
            let low = resample(&sine(1000.0), rate);
            let high = resample(&sine(12000.0), rate);
            assert_eq!(low.len(), 16000);
            let energy = |x: &[f32]| x[100..15900].iter().map(|v| v * v).sum::<f32>() / 15800.0;
            assert!(energy(&low) > 0.45);
            assert!(energy(&high) < 0.001);
        }
    }
}
