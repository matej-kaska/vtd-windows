use anyhow::{Result, ensure};

pub fn resample(mut input: Vec<f32>, rate: u32) -> Vec<f32> {
    if rate == 16000 {
        return input;
    }
    if rate == 0 || input.is_empty() {
        return Vec::new();
    }
    let ratio = rate as f64 / 16000.0;
    let cutoff = (1.0 / ratio).min(1.0) * 0.94;
    let radius = (16.0 / cutoff).ceil() as i64;
    let count = (input.len() as f64 / ratio) as usize;
    let (mut a, mut b) = (rate, 16000);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    let phases = 16000 / a;
    let width = (2 * radius + 1) as usize;
    let mut weights = Vec::with_capacity(phases as usize * (width + 1));
    for phase in 0..phases {
        let start = weights.len();
        weights.extend((-radius..=radius).map(|n| {
            let d = phase as f64 / phases as f64 - n as f64;
            let x = std::f64::consts::PI * d * cutoff;
            let sinc = if x.abs() < 1e-8 { 1.0 } else { x.sin() / x };
            sinc * 0.5 * (1.0 + (std::f64::consts::PI * d / radius as f64).cos())
        }));
        weights.push(weights[start..].iter().sum());
    }
    let down = rate > 16000;
    let mut output = Vec::with_capacity(if down { 0 } else { count });
    let mut history = vec![0.0; if down { radius as usize } else { 0 }];
    for i in 0..count {
        let pos = i as u64 * rate as u64;
        let mid = (pos / 16000) as i64;
        let phase = ((pos % 16000) / a as u64) as usize * (width + 1);
        let begin = (mid - radius).max(0) as usize;
        let end = (mid + radius + 1).min(input.len() as i64) as usize;
        let filter = &weights[phase + (begin as i64 - mid + radius) as usize
            ..phase + (end as i64 - mid + radius) as usize];
        let weight = if filter.len() == width {
            weights[phase + width]
        } else {
            filter.iter().sum()
        };
        let mut value = 0.0;
        if down && begin < i {
            for (n, &w) in (begin..end).zip(filter) {
                let sample = if n < i {
                    history[n % history.len()]
                } else {
                    input[n]
                };
                value += sample as f64 * w;
            }
        } else {
            for (&sample, &w) in input[begin..end].iter().zip(filter) {
                value += sample as f64 * w;
            }
        }
        let sample = (value / weight) as f32;
        if down {
            if mid - radius < i as i64 {
                let index = i % history.len();
                history[index] = input[i];
            }
            input[i] = sample;
        } else {
            output.push(sample);
        }
    }
    if down {
        input.truncate(count);
        input.shrink_to_fit();
        input
    } else {
        output
    }
}

pub fn audible(samples: &[f32], threshold: f32) -> bool {
    samples
        .chunks(320)
        .filter(|frame| {
            frame.iter().map(|x| x * x).sum::<f32>() > threshold * threshold * frame.len() as f32
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
    let mut raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 2f32.powi(spec.bits_per_sample as i32 - 1);
            reader
                .samples::<i32>()
                .map(|s| s.map(|x| x as f32 / scale))
                .collect::<Result<_, _>>()?
        }
    };
    if spec.channels > 1 {
        let channels = spec.channels as usize;
        let frames = raw.len() / channels;
        for i in 0..frames {
            raw[i] = raw[i * channels..(i + 1) * channels].iter().sum::<f32>() / channels as f32;
        }
        raw.truncate(frames);
    }
    Ok(resample(raw, spec.sample_rate))
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
            let low = resample(sine(1000.0), rate);
            let high = resample(sine(12000.0), rate);
            assert_eq!(low.len(), 16000);
            let energy = |x: &[f32]| x[100..15900].iter().map(|v| v * v).sum::<f32>() / 15800.0;
            assert!(energy(&low) > 0.45);
            assert!(energy(&high) < 0.001);
        }
    }
}
