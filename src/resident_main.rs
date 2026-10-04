#![cfg_attr(windows, no_std)]
#![cfg_attr(windows, no_main)]
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
#[path = "windows/resident.rs"]
mod resident;

#[cfg(target_os = "linux")]
#[path = "main.rs"]
mod platform;

#[cfg(target_os = "linux")]
fn main() {
    platform::main();
}
