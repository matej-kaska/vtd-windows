#[cfg(windows)]
#[path = "windows/audio.rs"]
pub mod audio;
#[cfg(windows)]
#[path = "windows/config.rs"]
pub mod config;
#[cfg(windows)]
#[path = "windows/languages.rs"]
pub mod languages;
#[cfg(windows)]
#[path = "windows/models.rs"]
pub mod models;
#[cfg(windows)]
#[path = "windows/output.rs"]
mod output;
#[cfg(windows)]
#[path = "windows/recording.rs"]
pub mod recording;
#[cfg(windows)]
#[path = "windows/text.rs"]
pub mod text;

#[cfg(windows)]
pub fn attach_console() {
    unsafe {
        use windows_sys::Win32::System::Console::*;
        if GetStdHandle(STD_OUTPUT_HANDLE).is_null() {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}
