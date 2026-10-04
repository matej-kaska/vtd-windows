use super::protocol::*;
use anyhow::{Result, ensure};
use std::{cell::RefCell, ptr::null_mut};
use vtd::config::Config;
use windows_sys::Win32::{
    Foundation::*,
    System::{DataExchange::COPYDATASTRUCT, LibraryLoader::GetModuleHandleW, Threading::*},
    UI::WindowsAndMessaging::*,
};

pub fn parent() -> HWND {
    unsafe { FindWindowW(windows_sys::w!("VTDWindows"), null_mut()) }
}

pub fn send(parent: HWND, kind: usize, bytes: &[u8]) -> Result<()> {
    let packet = COPYDATASTRUCT {
        dwData: kind,
        cbData: bytes.len().try_into()?,
        lpData: bytes.as_ptr().cast_mut().cast(),
    };
    let mut accepted = 0;
    let sent = unsafe {
        SendMessageTimeoutW(
            parent,
            WM_COPYDATA,
            GetCurrentProcessId() as usize,
            &packet as *const _ as isize,
            SMTO_ABORTIFHUNG,
            3000,
            &mut accepted,
        )
    };
    ensure!(
        sent != 0 && accepted == 1,
        "VTD resident did not accept the message"
    );
    Ok(())
}

pub fn preferences(parent: HWND, cfg: &Config) -> Result<()> {
    let preferences = Preferences {
        trigger: cfg.trigger_key,
        toggle: cfg.toggle_key,
        replay: cfg.replay_key,
    };
    let bytes = unsafe {
        std::slice::from_raw_parts(
            (&preferences as *const Preferences).cast(),
            std::mem::size_of::<Preferences>(),
        )
    };
    send(parent, PREFERENCES, bytes)
}

pub fn status(parent: HWND, text: &str, active: bool, paused: bool) {
    let mut value = Status {
        active: u32::from(active),
        paused: u32::from(paused),
        text: [0; 128],
    };
    for (target, unit) in value.text.iter_mut().take(127).zip(text.encode_utf16()) {
        *target = unit;
    }
    let bytes = unsafe {
        std::slice::from_raw_parts(
            (&value as *const Status).cast(),
            std::mem::size_of::<Status>(),
        )
    };
    let _ = send(parent, STATUS, bytes);
}

pub fn epoch() -> usize {
    let mut epoch = usize::MAX;
    let sent = unsafe {
        SendMessageTimeoutW(
            parent(),
            GET_EPOCH,
            0,
            0,
            SMTO_ABORTIFHUNG,
            1000,
            &mut epoch,
        )
    };
    if sent == 0 { usize::MAX } else { epoch }
}

thread_local! { static LAST: RefCell<String> = const { RefCell::new(String::new()) }; }

unsafe extern "system" fn last_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_COPYDATA && l != 0 {
            let mut resident_pid = 0;
            GetWindowThreadProcessId(parent(), &mut resident_pid);
            let packet = &*(l as *const COPYDATASTRUCT);
            if w == resident_pid as usize && packet.dwData == TRANSCRIPT && !packet.lpData.is_null()
            {
                let bytes =
                    std::slice::from_raw_parts(packet.lpData.cast::<u8>(), packet.cbData as usize);
                if let Ok(text) = std::str::from_utf8(bytes) {
                    LAST.with(|last| *last.borrow_mut() = text.to_owned());
                    return 1;
                }
            }
        }
        DefWindowProcW(hwnd, msg, w, l)
    }
}

pub fn last() -> Result<String> {
    unsafe {
        LAST.with(|last| last.borrow_mut().clear());
        let instance = GetModuleHandleW(null_mut());
        let class = windows_sys::w!("VTDLastReader");
        let registration = WNDCLASSW {
            lpfnWndProc: Some(last_proc),
            hInstance: instance,
            lpszClassName: class,
            ..std::mem::zeroed()
        };
        RegisterClassW(&registration);
        let hwnd = CreateWindowExW(
            0,
            class,
            class,
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            null_mut(),
            instance,
            null_mut(),
        );
        ensure!(!hwnd.is_null(), "Cannot request the last transcript");
        let mut accepted = 0;
        let sent = SendMessageTimeoutW(
            parent(),
            LAST_REQUEST,
            hwnd as usize,
            0,
            SMTO_ABORTIFHUNG,
            3000,
            &mut accepted,
        );
        DestroyWindow(hwnd);
        ensure!(sent != 0 && accepted == 1, "No transcript available yet");
        Ok(LAST.with(|last| std::mem::take(&mut *last.borrow_mut())))
    }
}

pub fn bootstrap() -> Result<()> {
    let parent = parent();
    ensure!(!parent.is_null(), "VTD is not running");
    let result = (|| {
        let path = vtd::config::path()?;
        Config::write_default(&path)?;
        let cfg = Config::load(&path)?;
        preferences(parent, &cfg)
    })();
    unsafe {
        PostMessageW(parent, BOOTSTRAP_CLOSED, GetCurrentProcessId() as usize, 0);
    }
    result
}
