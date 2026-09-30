use crate::engine::{self, Engine};
use anyhow::{Result, bail, ensure};
use std::{
    io::{Read, Write},
    path::Path,
    time::Instant,
};
use vtd::{
    audio,
    config::{self, Config},
};

pub fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config_path = config::path()?;
    match args.first().map(String::as_str) {
        Some("__worker") => {
            let cfg = Config::load(&config_path)?;
            drop(config_path);
            drop(args);
            run(&cfg)?;
        }
        Some("devices") => {
            for g in engine::gpus() {
                println!("GPU {}: {} (discrete={})", g.index, g.name, g.discrete);
            }
        }
        Some("transcribe") => {
            let wav = args
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("Expected WAV path"))?;
            let repeats: usize = args.get(2).map(|s| s.parse()).transpose()?.unwrap_or(1);
            ensure!((1..=20).contains(&repeats), "Repeats must be 1-20");
            let cfg = config::Config::load(&config_path)?;
            let samples = audio::read_wav(Path::new(wav))?;
            let mut engine = engine::Engine::load(&cfg)?;
            for i in 0..repeats {
                let start = Instant::now();
                let text = engine.transcribe(&cfg, &samples)?;
                println!("{text}");
                eprintln!(
                    "VTD run={} audio_s={:.3} inference_s={:.3}",
                    i + 1,
                    samples.len() as f64 / 16000.0,
                    start.elapsed().as_secs_f64()
                );
            }
        }
        _ => bail!("Use vtd.exe"),
    }
    Ok(())
}

pub fn run(cfg: &Config) -> Result<()> {
    let mut engine = Engine::load(cfg)?;
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    loop {
        let mut count = [0; 4];
        match input.read_exact(&mut count) {
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            result => result?,
        }
        let count = u32::from_le_bytes(count) as usize;
        ensure!(count <= 16000 * 300, "Recording exceeds 5 minutes");
        let mut samples = vec![0.0f32; count];
        let bytes =
            unsafe { std::slice::from_raw_parts_mut(samples.as_mut_ptr().cast::<u8>(), count * 4) };
        input.read_exact(bytes)?;
        let result = engine
            .transcribe(cfg, samples)
            .map_err(|e| format!("{e:#}"));
        serde_json::to_writer(&mut output, &result)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}
