use super::{autostart, runtime, tray};
use anyhow::{Result, bail, ensure};
use std::{cell::Cell, ptr::null_mut};
use vtd::{config::Config, languages::LANGUAGES};
use windows_sys::Win32::{
    Foundation::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentProcessId},
    UI::{
        Controls::{
            CheckDlgButton, HKM_GETHOTKEY, HKM_SETHOTKEY, HKM_SETRULES, ICC_HOTKEY_CLASS,
            INITCOMMONCONTROLSEX, InitCommonControlsEx, IsDlgButtonChecked,
        },
        WindowsAndMessaging::*,
    },
};

const HOLD: i32 = 1001;
const TOGGLE: i32 = 1002;
const REPLAY: i32 = 1003;
const LANGUAGE: i32 = 1004;
const AUTOSTART: i32 = 1005;
const MUTE: i32 = 1006;
thread_local! { static WINDOW: Cell<HWND> = const { Cell::new(null_mut()) }; }
thread_local! { static PARENT: Cell<HWND> = const { Cell::new(null_mut()) }; }

fn dispatch(msg: &MSG) -> bool {
    WINDOW.with(|window| unsafe {
        let hwnd = window.get();
        !hwnd.is_null() && IsDialogMessageW(hwnd, msg) != 0
    })
}

fn close() {
    let hwnd = WINDOW.with(Cell::get);
    if !hwnd.is_null() {
        unsafe {
            DestroyWindow(hwnd);
        }
    }
}

pub fn run(menu: Option<(bool, bool)>) -> Result<()> {
    unsafe {
        let parent = FindWindowW(windows_sys::w!("VTDWindows"), null_mut());
        ensure!(!parent.is_null(), "VTD is not running.");
        PARENT.with(|window| window.set(parent));
        if let Some((paused, busy)) = menu {
            let command = tray::show_menu(paused, busy)?;
            if command != 4 {
                if command != 0 {
                    notify(runtime::MENU_COMMAND, command as LPARAM)?;
                }
                finished(parent);
                return Ok(());
            }
        }
        // Check current recording state before opening settings from a menu.
        notify(runtime::SETTINGS_READY, 0)?;
        let cfg = Config::load(&vtd::config::path()?)?;
        show(&cfg)?;
        if let Err(e) = notify(runtime::SETTINGS_READY, WINDOW.with(Cell::get) as LPARAM) {
            close();
            return Err(e);
        }
        let mut msg = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if !dispatch(&msg) {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        finished(parent);
        Ok(())
    }
}

unsafe fn finished(parent: HWND) {
    unsafe {
        PostMessageW(
            parent,
            runtime::UI_CLOSED,
            GetCurrentProcessId() as usize,
            0,
        );
    }
}

pub(super) fn notify(message: u32, parameter: LPARAM) -> Result<()> {
    let mut result = 0;
    let sent = unsafe {
        SendMessageTimeoutW(
            PARENT.with(Cell::get),
            message,
            GetCurrentProcessId() as usize,
            parameter,
            SMTO_ABORTIFHUNG,
            3000,
            &mut result,
        )
    };
    ensure!(
        sent != 0 && result == 1,
        "VTD could not apply the settings. Close this window and restart VTD if the problem persists."
    );
    Ok(())
}

fn show(cfg: &Config) -> Result<()> {
    unsafe {
        let existing = WINDOW.with(Cell::get);
        if !existing.is_null() {
            ShowWindow(existing, SW_RESTORE);
            SetForegroundWindow(existing);
            return Ok(());
        }
        ensure!(
            InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_HOTKEY_CLASS,
            }) != 0,
            "Cannot initialize shortcut fields."
        );
        let startup = autostart::enabled(&autostart::snapshot()?)?;
        // cfg is borrowed only synchronously during WM_INITDIALOG.
        let hwnd = CreateDialogParamW(
            GetModuleHandleW(null_mut()),
            std::ptr::without_provenance(2),
            null_mut(),
            Some(dialog_proc),
            cfg as *const Config as LPARAM,
        );
        ensure!(
            !hwnd.is_null(),
            "Cannot open settings: {}",
            std::io::Error::last_os_error()
        );
        WINDOW.with(|window| window.set(hwnd));
        // A dialog does not inherit the executable's icon. Set both sizes before
        // showing it so the taskbar, its preview and Alt+Tab use the VTD icon.
        for (kind, width, height) in [
            (ICON_SMALL, SM_CXSMICON, SM_CYSMICON),
            (ICON_BIG, SM_CXICON, SM_CYICON),
        ] {
            let icon = LoadImageW(
                GetModuleHandleW(null_mut()),
                std::ptr::without_provenance(1),
                IMAGE_ICON,
                GetSystemMetrics(width),
                GetSystemMetrics(height),
                LR_SHARED,
            );
            if !icon.is_null() {
                // Shared resource handles belong to Windows and need no DestroyIcon.
                SendMessageW(hwnd, WM_SETICON, kind as WPARAM, icon as LPARAM);
            }
        }
        CheckDlgButton(hwnd, AUTOSTART, u32::from(startup));
        CheckDlgButton(hwnd, MUTE, u32::from(cfg.mute_output));
        // Install/autostart can launch us with STARTF_USESHOWWINDOW / SW_HIDE.
        // ShowWindow's first call would honor that instead of opening settings.
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_SHOWWINDOW,
        );
        SetForegroundWindow(hwnd);
        Ok(())
    }
}

pub fn error(hwnd: HWND, error: &anyhow::Error) {
    unsafe {
        MessageBoxW(
            hwnd,
            runtime::wide(&format!("{error:#}")).as_ptr(),
            windows_sys::w!("VTD - Settings"),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn save(hwnd: HWND) -> Result<()> {
    let config_path = vtd::config::path()?;
    let mut cfg = Config::load(&config_path)?;
    unsafe {
        for (id, key) in [
            (HOLD, &mut cfg.trigger_key),
            (TOGGLE, &mut cfg.toggle_key),
            (REPLAY, &mut cfg.replay_key),
        ] {
            *key = SendDlgItemMessageW(hwnd, id, HKM_GETHOTKEY, 0, 0) as u32 & 0x7ff;
        }
        let selected = SendDlgItemMessageW(hwnd, LANGUAGE, CB_GETCURSEL, 0, 0);
        ensure!(selected >= 0, "Select a speech language.");
        if let Some(&(code, _)) = LANGUAGES.get(selected as usize) {
            cfg.language = code.into();
        } // An unlisted existing language alias remains untouched.
        cfg.mute_output = IsDlgButtonChecked(hwnd, MUTE) == 1;
        cfg.validate()?;
        let previous_startup = autostart::snapshot()?;
        let startup = IsDlgButtonChecked(hwnd, AUTOSTART) == 1;
        let change_startup = autostart::enabled(&previous_startup)? != startup;
        if change_startup {
            autostart::set_enabled(startup)?;
        }
        if let Err(error) = cfg.save_preferences(&config_path) {
            if change_startup && let Err(rollback) = autostart::restore(&previous_startup) {
                bail!("{error:#}; could not restore the startup setting: {rollback:#}");
            }
            return Err(error);
        }
        notify(runtime::SETTINGS_APPLY, 0)
    }
}

unsafe extern "system" fn dialog_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> isize {
    unsafe {
        match msg {
            WM_INITDIALOG => {
                let cfg = &*(l as *const Config);
                for (id, key) in [
                    (HOLD, cfg.trigger_key),
                    (TOGGLE, cfg.toggle_key),
                    (REPLAY, cfg.replay_key),
                ] {
                    SendDlgItemMessageW(hwnd, id, HKM_SETRULES, 0, 0);
                    SendDlgItemMessageW(hwnd, id, HKM_SETHOTKEY, key as usize, 0);
                }
                let mut selected = LANGUAGES.len();
                for (index, &(code, name)) in LANGUAGES.iter().enumerate() {
                    SendDlgItemMessageW(
                        hwnd,
                        LANGUAGE,
                        CB_ADDSTRING,
                        0,
                        runtime::wide(name).as_ptr() as LPARAM,
                    );
                    if cfg.language == code {
                        selected = index;
                    }
                }
                if selected == LANGUAGES.len() {
                    SendDlgItemMessageW(
                        hwnd,
                        LANGUAGE,
                        CB_ADDSTRING,
                        0,
                        runtime::wide(&cfg.language).as_ptr() as LPARAM,
                    );
                }
                SendDlgItemMessageW(hwnd, LANGUAGE, CB_SETCURSEL, selected, 0);
                if cfg.toggle {
                    SetDlgItemTextW(hwnd, 1011, windows_sys::w!("Start / stop (&primary):"));
                }
                1
            }
            WM_COMMAND => {
                match (w & 0xffff) as i32 {
                    IDOK => match save(hwnd) {
                        Ok(()) => {
                            DestroyWindow(hwnd);
                        }
                        Err(e) => error(hwnd, &e),
                    },
                    IDCANCEL => {
                        DestroyWindow(hwnd);
                    }
                    _ => return 0,
                }
                1
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                1
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            WM_NCDESTROY => {
                WINDOW.with(|window| {
                    if window.get() == hwnd {
                        window.set(null_mut());
                    }
                });
                0
            }
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_shortcut_control_captures_function_and_media_keys() {
        unsafe {
            assert_ne!(
                InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                    dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                    dwICC: ICC_HOTKEY_CLASS,
                }),
                0
            );
            let hwnd = CreateWindowExW(
                0,
                windows_sys::w!("msctls_hotkey32"),
                null_mut(),
                0,
                0,
                0,
                180,
                24,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null_mut()),
                null_mut(),
            );
            assert!(!hwnd.is_null());
            SendMessageW(hwnd, HKM_SETRULES, 0, 0);
            for key in [120, 0xad, 0xae, 0xb3] {
                SendMessageW(hwnd, WM_KEYDOWN, key, 0);
                assert_eq!(
                    SendMessageW(hwnd, HKM_GETHOTKEY, 0, 0) & 0x7ff,
                    key as isize
                );
                SendMessageW(hwnd, WM_KEYUP, key, 0);
            }
            SendMessageW(hwnd, HKM_SETHOTKEY, 0x378, 0);
            assert_eq!(SendMessageW(hwnd, HKM_GETHOTKEY, 0, 0), 0x378);
            DestroyWindow(hwnd);
        }
    }
}
