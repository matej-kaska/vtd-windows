use crate::config::Config;
use anyhow::{Context, Result, bail};
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
    first_sample: Option<std::time::Duration>,
}

impl Buffer {
    fn append<T>(
        &mut self,
        input: &[T],
        channels: usize,
        limit: usize,
        convert: impl Fn(&T) -> f32,
    ) {
        let needed = (self.samples.len() + input.len() / channels).min(limit);
        if needed > self.samples.capacity() {
            let capacity = (self.samples.capacity() * 2).max(needed).min(limit);
            self.samples.reserve_exact(capacity - self.samples.len());
        }
        let left = limit.saturating_sub(self.samples.len());
        if channels == 1 {
            self.samples.extend(input.iter().take(left).map(convert));
            return;
        }
        for frame in input.chunks_exact(channels).take(left) {
            self.samples
                .push(frame.iter().map(&convert).sum::<f32>() / channels as f32);
        }
    }
}

pub struct Recording {
    stream: cpal::Stream,
    data: Arc<Mutex<Buffer>>,
    rate: u32,
    started: Instant,
}

impl Recording {
    pub fn start(cfg: &Config) -> Result<Self> {
        let started = Instant::now();
        let host = cpal::default_host();
        let device = if let Some(name) = &cfg.microphone {
            host.input_devices()?
                .find(|d| d.description().is_ok_and(|v| v.name() == name))
        } else {
            host.default_input_device()
        }
        .context("Microphone not found; use vtd devices")?;
        Self::open(
            &device,
            cfg,
            cpal::StreamConfig {
                channels: 1,
                sample_rate: 16000,
                buffer_size: cpal::BufferSize::Default,
            },
            cpal::SampleFormat::F32,
            started,
        )
        .or_else(|_| {
            let supported = device.default_input_config()?;
            Self::open(
                &device,
                cfg,
                supported.clone().into(),
                supported.sample_format(),
                started,
            )
        })
    }

    fn open(
        device: &cpal::Device,
        cfg: &Config,
        config: cpal::StreamConfig,
        format: cpal::SampleFormat,
        started: Instant,
    ) -> Result<Self> {
        let rate = config.sample_rate;
        let channels = config.channels as usize;
        let limit = rate as usize * cfg.max_recording_seconds as usize;
        let data = Arc::new(Mutex::new(Buffer {
            samples: Vec::with_capacity(limit.min(rate as usize * 30)),
            error: None,
            first_sample: None,
        }));
        macro_rules! stream {
            ($ty:ty, $convert:expr) => {{
                let target = data.clone();
                let errors = data.clone();
                device.build_input_stream(
                    &config,
                    move |input: &[$ty], _: &cpal::InputCallbackInfo| {
                        if let Ok(mut buf) = target.lock() {
                            if buf.first_sample.is_none() && !input.is_empty() {
                                buf.first_sample = Some(started.elapsed());
                            }
                            buf.append(input, channels, limit, $convert);
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
        let stream = match format {
            cpal::SampleFormat::F32 => stream!(f32, |x: &f32| *x),
            cpal::SampleFormat::I16 => stream!(i16, |x: &i16| *x as f32 / 32768.0),
            cpal::SampleFormat::U16 => stream!(u16, |x: &u16| (*x as f32 - 32768.0) / 32768.0),
            other => bail!("Unsupported microphone format: {other}"),
        };
        stream.play()?;
        eprintln!(
            "VTD audio open: {:.2}ms, {rate} Hz, {channels} channels",
            started.elapsed().as_secs_f64() * 1000.0
        );
        Ok(Self {
            stream,
            data,
            rate,
            started,
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
            "VTD capture: {seconds:.2}s audio, {:.2}s elapsed, first_packet={:.2}ms, RMS {rms:.4}",
            self.started.elapsed().as_secs_f64(),
            buf.first_sample.unwrap_or_default().as_secs_f64() * 1000.0
        );
        Ok((std::mem::take(&mut buf.samples), self.rate))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mono_capture_keeps_limit_and_16k_storage() {
        let mut buffer = Buffer {
            samples: Vec::with_capacity(480000),
            error: None,
            first_sample: None,
        };
        for _ in 0..30001 {
            buffer.append(&[0.1; 160], 1, 4800000, |x| *x);
        }
        assert_eq!(buffer.samples.len(), 4800000);
        assert_eq!(buffer.samples.capacity(), 4800000);
        assert!(buffer.samples.iter().all(|&x| x == 0.1));
        let pointer = buffer.samples.as_ptr();
        let samples = crate::audio::resample(buffer.samples, 16000);
        assert_eq!(samples.as_ptr(), pointer);
        assert_eq!(samples.len(), 4800000);
    }

    #[test]
    fn growing_capture_preserves_samples_and_stops_at_limit() {
        let mut buffer = Buffer {
            samples: Vec::with_capacity(3000),
            error: None,
            first_sample: None,
        };
        let input: Vec<_> = (0..31000)
            .flat_map(|i| [i as f32 - 0.5, i as f32 + 0.5])
            .collect();
        for chunk in input.chunks(2048) {
            buffer.append(chunk, 2, 30000, |x| *x);
        }
        assert_eq!(buffer.samples.len(), 30000);
        assert_eq!(buffer.samples.capacity(), 30000);
        assert!(
            buffer
                .samples
                .iter()
                .enumerate()
                .all(|(i, &x)| x == i as f32)
        );
    }
}
