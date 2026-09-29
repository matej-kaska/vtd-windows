mod audio;
mod config;
mod engine;
mod runtime;

use anyhow::{Result, bail, ensure};
use std::{path::Path, time::Instant};

pub fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config_path = config::path()?;
    match args.first().map(String::as_str) {
        Some("help" | "--help" | "-h") => println!(
            "VTD Windows - offline Czech GPU dictation\n\n  vtd run                 Background process; hold F8 to dictate, Esc to cancel\n  vtd status / stop / copy  Query state, stop, or copy last transcript\n  vtd devices             List Vulkan GPUs and microphones\n  vtd init                Create vtd.json beside the executable\n  vtd transcribe FILE.wav [REPEATS]\n                          Print transcript and warm/cold timings\n  vtd autostart on|off     Enable/disable startup for this user\n\nConfiguration: vtd.json beside the executable; restart after changes.\nModel paths are relative to vtd.json. Default language: cs.\n"
        ),
        Some("devices") => {
            for g in engine::gpus() {
                println!("GPU {}: {} (discrete={})", g.index, g.name, g.discrete);
            }
            for name in audio::devices()? {
                println!("Microphone: {name}");
            }
        }
        Some("init") => {
            config::Config::write_default(&config_path)?;
            println!("{}", config_path.display());
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
        Some("autostart") => runtime::autostart(args.get(1).map(String::as_str))?,
        Some("copy") => runtime::control("copy")?,
        Some("status") => runtime::control("status")?,
        Some("stop") => runtime::control("stop")?,
        None | Some("run") => {
            config::Config::write_default(&config_path)?;
            runtime::run(config::Config::load(&config_path)?)?;
        }
        Some(other) => bail!("Unknown command: {other}; use vtd --help"),
    }
    Ok(())
}
