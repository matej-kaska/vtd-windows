mod runtime;
mod tray;
mod worker;

use anyhow::{Result, bail, ensure};
use vtd::{config, recording};

pub fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config_path = config::path()?;
    match args.first().map(String::as_str) {
        Some("__worker" | "transcribe") => {
            ensure!(
                worker::command()?.args(&args).status()?.success(),
                "Transcription process failed"
            );
        }
        Some("help" | "--help" | "-h") => println!(
            "VTD Windows - offline multilingual GPU dictation\n\n  vtd run                 Background process; hold F8 or tap F9 to start/stop, Esc to cancel\n  vtd status / stop / copy  Query state, stop, or copy last transcript\n  vtd devices             List Vulkan GPUs and microphones\n  vtd init                Create vtd.json beside the executable\n  vtd transcribe FILE.wav [REPEATS]\n                          Print transcript and warm/cold timings\n  vtd autostart on|off     Enable/disable startup for this user\n\nConfiguration: vtd.json beside the executable; restart after changes.\nModel paths are relative to vtd.json. Default language: cs.\n"
        ),
        Some("devices") => {
            ensure!(
                worker::command()?.arg("devices").status()?.success(),
                "GPU detection failed"
            );
            for name in recording::devices()? {
                println!("Microphone: {name}");
            }
        }
        Some("init") => {
            config::Config::write_default(&config_path)?;
            println!("{}", config_path.display());
        }
        Some("autostart") => runtime::autostart(args.get(1).map(String::as_str))?,
        Some("copy") => runtime::control("copy")?,
        Some(command @ ("status" | "pause" | "resume")) => runtime::control(command)?,
        Some("stop") => runtime::control("stop")?,
        None | Some("run") => {
            let capture_next = match &args[args.len().min(1)..] {
                [] => None,
                [flag, path] if flag == "--capture-next" => Some(std::path::PathBuf::from(path)),
                _ => bail!("Use: vtd run [--capture-next FILE.wav]"),
            };
            config::Config::write_default(&config_path)?;
            runtime::run(config::Config::load(&config_path)?, capture_next)?;
        }
        Some(other) => bail!("Unknown command: {other}; use vtd --help"),
    }
    Ok(())
}
