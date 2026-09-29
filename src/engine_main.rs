#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
#[path = "windows/engine.rs"]
mod engine;
#[cfg(windows)]
#[path = "windows/inference.rs"]
mod inference;

#[cfg(windows)]
fn main() {
    vtd::attach_console();
    if let Err(e) = inference::main() {
        eprintln!("vtd-engine: {e:#}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {}
