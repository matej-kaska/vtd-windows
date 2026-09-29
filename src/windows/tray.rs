use super::runtime::wide;
use windows_sys::Win32::{
    Foundation::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Shell::*, WindowsAndMessaging::*},
};

pub const EVENT: u32 = WM_APP + 4;

pub fn update(hwnd: HWND, text: &str, operation: u32) -> bool {
    unsafe {
        let mut icon: NOTIFYICONDATAW = std::mem::zeroed();
        icon.cbSize = std::mem::size_of_val(&icon) as u32;
        icon.hWnd = hwnd;
        icon.uID = 1;
        icon.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        icon.uCallbackMessage = EVENT;
        icon.hIcon = LoadImageW(
            GetModuleHandleW(std::ptr::null()),
            std::ptr::without_provenance(1),
            IMAGE_ICON,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            LR_SHARED,
        ) as HICON;
        if icon.hIcon.is_null() {
            return false;
        }
        for (dest, unit) in icon.szTip.iter_mut().take(127).zip(text.encode_utf16()) {
            *dest = unit;
        }
        Shell_NotifyIconW(operation, &icon) != 0
    }
}

pub fn menu(hwnd: HWND, paused: bool) -> u32 {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return 0;
        }
        AppendMenuW(
            menu,
            MF_STRING,
            1,
            wide(if paused {
                "▶  Spustit"
            } else {
                "⏸  Pozastavit"
            })
            .as_ptr(),
        );
        AppendMenuW(menu, MF_STRING, 2, wide("✕  Ukončit").as_ptr());
        let mut point: POINT = std::mem::zeroed();
        GetCursorPos(&mut point);
        SetForegroundWindow(hwnd);
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        DestroyMenu(menu);
        PostMessageW(hwnd, WM_NULL, 0, 0);
        command as u32
    }
}
