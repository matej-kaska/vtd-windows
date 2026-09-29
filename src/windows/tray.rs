use super::runtime::wide;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Shell::*, WindowsAndMessaging::*},
};

pub const EVENT: u32 = WM_APP + 4;

fn symbol(kind: u8) -> HBITMAP {
    unsafe {
        let size = GetSystemMetrics(SM_CYMENUCHECK).max(16);
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = size;
        info.bmiHeader.biHeight = -size;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(
            std::ptr::null_mut(),
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        if bitmap.is_null() {
            return bitmap;
        }
        let color = GetSysColor(COLOR_MENUTEXT);
        let pixels = std::slice::from_raw_parts_mut(bits.cast::<u32>(), (size * size) as usize);
        for y in 0..size {
            for x in 0..size {
                let mut coverage = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let px = (x as f32 + (sx as f32 + 0.5) / 4.0) * 16.0 / size as f32;
                        let py = (y as f32 + (sy as f32 + 0.5) / 4.0) * 16.0 / size as f32;
                        let inside = match kind {
                            0 => {
                                (3.0..13.0).contains(&py)
                                    && ((4.0..6.5).contains(&px) || (9.5..12.0).contains(&px))
                            }
                            1 => (4.0..12.0).contains(&px) && (py - 8.0).abs() < (12.0 - px) * 0.65,
                            _ => {
                                (3.5..12.5).contains(&px)
                                    && (3.5..12.5).contains(&py)
                                    && ((px - py).abs() < 1.0 || (px + py - 16.0).abs() < 1.0)
                            }
                        };
                        coverage += u32::from(inside);
                    }
                }
                let alpha = coverage * 255 / 16;
                let red = (color & 255) * alpha / 255;
                let green = ((color >> 8) & 255) * alpha / 255;
                let blue = ((color >> 16) & 255) * alpha / 255;
                pixels[(y * size + x) as usize] = (alpha << 24) | (red << 16) | (green << 8) | blue;
            }
        }
        bitmap
    }
}

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
        let mut info: MENUINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of_val(&info) as u32;
        info.fMask = MIM_STYLE;
        info.dwStyle = MNS_CHECKORBMP;
        SetMenuInfo(menu, &info);
        AppendMenuW(
            menu,
            MF_STRING,
            1,
            wide(if paused { "Spustit" } else { "Pozastavit" }).as_ptr(),
        );
        AppendMenuW(menu, MF_STRING, 2, wide("Ukončit").as_ptr());
        let icons = [symbol(u8::from(paused)), symbol(2)];
        for (position, &bitmap) in icons.iter().enumerate() {
            let mut item: MENUITEMINFOW = std::mem::zeroed();
            item.cbSize = std::mem::size_of_val(&item) as u32;
            item.fMask = MIIM_BITMAP;
            item.hbmpItem = bitmap;
            SetMenuItemInfoW(menu, position as u32, 1, &item);
        }
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
        for icon in icons {
            if !icon.is_null() {
                DeleteObject(icon);
            }
        }
        PostMessageW(hwnd, WM_NULL, 0, 0);
        command as u32
    }
}
