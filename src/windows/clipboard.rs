use anyhow::{Result, ensure};
use std::{cell::RefCell, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{DataExchange::*, Memory::*, Ole::OleDuplicateData},
    UI::WindowsAndMessaging::*,
};

const TIMER: usize = 3;
thread_local! { static PENDING: RefCell<Option<Paste>> = const { RefCell::new(None) }; }

struct Data(u32, HANDLE);

impl Drop for Data {
    fn drop(&mut self) {
        if self.1.is_null() {
            return;
        }
        unsafe {
            match self.0 {
                2 | 9 | 0x82 => {
                    DeleteObject(self.1);
                }
                14 | 0x8e => {
                    DeleteEnhMetaFile(self.1);
                }
                3 | 0x83 => {
                    let picture = GlobalLock(self.1) as *const METAFILEPICT;
                    if !picture.is_null() {
                        DeleteMetaFile((*picture).hMF);
                        GlobalUnlock(self.1);
                    }
                    GlobalFree(self.1);
                }
                _ => {
                    GlobalFree(self.1);
                }
            }
        }
    }
}

struct Open;
impl Open {
    fn new(hwnd: HWND) -> Result<Self> {
        ensure!(
            unsafe { OpenClipboard(hwnd) } != 0,
            "Clipboard is busy; try inserting the last transcript again"
        );
        Ok(Self)
    }
}
impl Drop for Open {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}

struct Paste {
    saved: Vec<Data>,
    text: Data,
    sequence: u32,
    restoring: bool,
    published: HANDLE,
    target_process: u32,
}

fn snapshot() -> Result<Vec<Data>> {
    let mut saved = Vec::new();
    unsafe {
        let dib = IsClipboardFormatAvailable(8) != 0 || IsClipboardFormatAvailable(17) != 0;
        let dib_v5 = IsClipboardFormatAvailable(17) != 0;
        let mut format = 0;
        loop {
            SetLastError(0);
            format = EnumClipboardFormats(format);
            if format == 0 {
                ensure!(GetLastError() == 0, "Cannot enumerate clipboard formats");
                break;
            }
            if (format == 2 && dib) || (format == 8 && dib_v5) {
                continue;
            }
            let mut name = [0u16; 80];
            let len = GetClipboardFormatNameW(format, name.as_mut_ptr(), name.len() as i32);
            if len > 0
                && ["DataObject", "Ole Private Data"]
                    .iter()
                    .any(|s| name[..len as usize].iter().copied().eq(s.encode_utf16()))
            {
                continue;
            }
            ensure!(
                format != 0x80 && !(0x200..0x400).contains(&format),
                "Clipboard contains an unsupported private format; original preserved"
            );
            SetLastError(0);
            let source = GetClipboardData(format);
            if source.is_null()
                && GetLastError() == 0
                && name[..len as usize]
                    .iter()
                    .copied()
                    .eq("EnterpriseDataProtectionId".encode_utf16())
            {
                saved.push(Data(format, null_mut()));
                continue;
            }
            ensure!(
                !source.is_null(),
                "Cannot save clipboard format {format}; original preserved"
            );
            let copy = if matches!(format, 14 | 0x8e) {
                CopyEnhMetaFileW(source, std::ptr::null())
            } else {
                OleDuplicateData(source, format as u16, GMEM_MOVEABLE)
            };
            ensure!(
                !copy.is_null(),
                "Cannot save clipboard format {format}; original preserved"
            );
            saved.push(Data(format, copy));
        }
    }
    Ok(saved)
}

pub fn prepare(hwnd: HWND, text: &str, target: HWND) -> Result<()> {
    ensure!(
        !PENDING.with(|p| p.borrow().is_some()),
        "Previous paste is still finishing; try F10 again"
    );
    unsafe {
        let memory = Data(
            13,
            GlobalAlloc(GMEM_MOVEABLE, (text.encode_utf16().count() + 1) * 2),
        );
        ensure!(!memory.1.is_null(), "Clipboard allocation failed");
        let dest = GlobalLock(memory.1) as *mut u16;
        ensure!(!dest.is_null(), "Clipboard lock failed");
        for (i, unit) in text.encode_utf16().chain(Some(0)).enumerate() {
            *dest.add(i) = unit;
        }
        GlobalUnlock(memory.1);
        let marker_format = RegisterClipboardFormatW(windows_sys::w!(
            "ExcludeClipboardContentFromMonitorProcessing"
        ));
        let mut marker = Data(marker_format, GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, 4));
        ensure!(
            marker_format != 0 && !marker.1.is_null(),
            "Cannot mark temporary clipboard content"
        );
        let mut target_process = 0;
        GetWindowThreadProcessId(target, &mut target_process);
        ensure!(
            target_process != 0,
            "Target application is no longer available"
        );
        let _open = Open::new(hwnd)?;
        let saved = snapshot()?;
        ensure!(
            SetTimer(hwnd, TIMER, 2000, None) != 0,
            "Cannot schedule clipboard restoration"
        );
        if EmptyClipboard() == 0 {
            KillTimer(hwnd, TIMER);
            anyhow::bail!("Cannot prepare clipboard");
        }
        SetClipboardData(13, null_mut());
        PENDING.with(|p| {
            *p.borrow_mut() = Some(Paste {
                saved,
                text: memory,
                sequence: GetClipboardSequenceNumber(),
                restoring: false,
                published: null_mut(),
                target_process,
            })
        });
        if SetClipboardData(marker_format, marker.1).is_null() {
            drop(_open);
            restore(hwnd)?;
            anyhow::bail!("Cannot publish clipboard change");
        }
        marker.1 = null_mut();
        drop(_open);
        PENDING.with(|p| p.borrow_mut().as_mut().unwrap().sequence = GetClipboardSequenceNumber());
    }
    Ok(())
}

pub fn pending() -> bool {
    PENDING.with(|pending| pending.borrow().is_some())
}

pub fn restore(hwnd: HWND) -> Result<()> {
    PENDING.with(|p| {
        let mut pending = p.borrow_mut();
        let Some(paste) = pending.as_mut() else {
            return Ok(());
        };
        unsafe {
            let _open = Open::new(hwnd)?;
            if GetClipboardOwner() == hwnd
                && (GetClipboardSequenceNumber() == paste.sequence
                    || (!paste.restoring
                        && !paste.published.is_null()
                        && GetClipboardData(13) == paste.published))
            {
                if !paste.restoring {
                    ensure!(EmptyClipboard() != 0, "Cannot restore clipboard");
                    paste.restoring = true;
                }
                for data in &mut paste.saved {
                    if data.0 == 0 {
                        continue;
                    }
                    SetLastError(0);
                    if !SetClipboardData(data.0, data.1).is_null()
                        || (data.1.is_null()
                            && GetLastError() == 0
                            && IsClipboardFormatAvailable(data.0) != 0)
                    {
                        data.0 = 0;
                        data.1 = null_mut();
                    }
                }
                paste.sequence = GetClipboardSequenceNumber();
                ensure!(
                    paste.saved.iter().all(|data| data.0 == 0),
                    "Clipboard restoration incomplete; retrying"
                );
            }
            KillTimer(hwnd, TIMER);
        }
        *pending = None;
        Ok(())
    })
}

pub fn message(hwnd: HWND, msg: u32, w: WPARAM) -> bool {
    if msg == WM_RENDERFORMAT && w == 13 {
        PENDING.with(|p| {
            if let Some(paste) = p.borrow_mut().as_mut() {
                unsafe {
                    let mut reader = 0;
                    GetWindowThreadProcessId(GetOpenClipboardWindow(), &mut reader);
                    if reader != 0 && reader != paste.target_process {
                        return;
                    }
                    if !paste.text.1.is_null() && !SetClipboardData(13, paste.text.1).is_null() {
                        paste.published = paste.text.1;
                        paste.text.1 = null_mut();
                        paste.sequence = GetClipboardSequenceNumber();
                        if reader != 0 {
                            SetTimer(hwnd, TIMER, 300, None);
                        }
                    }
                }
            }
        });
        return true;
    }
    if msg == WM_TIMER && w == TIMER {
        if let Err(e) = restore(hwnd) {
            eprintln!("VTD clipboard: {e}");
        }
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::{LibraryLoader::GetModuleHandleW, StationsAndDesktops::*};

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        if message(hwnd, msg, w) {
            0
        } else {
            unsafe { DefWindowProcW(hwnd, msg, w, l) }
        }
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    fn write(hwnd: HWND, values: &[(u32, Vec<u8>)]) {
        let _open = Open::new(hwnd).unwrap();
        unsafe {
            assert_ne!(EmptyClipboard(), 0);
            for (format, bytes) in values {
                let data = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
                assert!(!data.is_null());
                let dest = GlobalLock(data) as *mut u8;
                assert!(!dest.is_null());
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), dest, bytes.len());
                GlobalUnlock(data);
                assert!(!SetClipboardData(*format, data).is_null());
            }
        }
    }

    fn read(hwnd: HWND, format: u32) -> Vec<u8> {
        let _open = Open::new(hwnd).unwrap();
        unsafe {
            let data = GetClipboardData(format);
            assert!(!data.is_null(), "missing format {format}");
            let pointer = GlobalLock(data) as *const u8;
            assert!(!pointer.is_null());
            let bytes = std::slice::from_raw_parts(pointer, GlobalSize(data)).to_vec();
            GlobalUnlock(data);
            bytes
        }
    }

    #[test]
    #[ignore = "run alone: creates an isolated Windows clipboard"]
    fn isolated_clipboard_roundtrip() {
        unsafe {
            let station = CreateWindowStationW(std::ptr::null(), 0, 0x000f037f, std::ptr::null());
            assert!(!station.is_null());
            assert_ne!(SetProcessWindowStation(station), 0);
            let desktop = CreateDesktopW(
                wide("VtdClipboardTest").as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                0x000f01ff,
                std::ptr::null(),
            );
            assert!(!desktop.is_null());
            assert_ne!(SetThreadDesktop(desktop), 0);
            let instance = GetModuleHandleW(std::ptr::null());
            let class = wide("VtdClipboardTest");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..std::mem::zeroed()
            };
            assert_ne!(RegisterClassW(&wc), 0);
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
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
            assert!(!hwnd.is_null());
            let html = RegisterClipboardFormatW(wide("HTML Format").as_ptr());
            let png = RegisterClipboardFormatW(wide("PNG").as_ptr());
            let mut dib = vec![0u8; 56];
            dib[..4].copy_from_slice(&40u32.to_le_bytes());
            dib[4..8].copy_from_slice(&2i32.to_le_bytes());
            dib[8..12].copy_from_slice(&2i32.to_le_bytes());
            dib[12..14].copy_from_slice(&1u16.to_le_bytes());
            dib[14..16].copy_from_slice(&32u16.to_le_bytes());
            dib[20..24].copy_from_slice(&16u32.to_le_bytes());
            dib[40..].copy_from_slice(&[
                0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, 255, 255, 255, 255,
            ]);
            let mut files = vec![0u8; 20];
            files[..4].copy_from_slice(&20u32.to_le_bytes());
            files[16..20].copy_from_slice(&1u32.to_le_bytes());
            files.extend(
                wide("C:\\clipboard-test.png\0")
                    .iter()
                    .flat_map(|v| v.to_le_bytes()),
            );
            let original = vec![
                (8, dib),
                (png, vec![137, 80, 78, 71, 13, 10, 26, 10]),
                (html, b"<b>test</b>\0".to_vec()),
                (15, files),
            ];
            write(hwnd, &original);
            let protection = RegisterClipboardFormatW(wide("EnterpriseDataProtectionId").as_ptr());
            {
                let _open = Open::new(hwnd).unwrap();
                SetClipboardData(protection, null_mut());
                assert_ne!(IsClipboardFormatAvailable(protection), 0);
            }
            let text = "Příliš žluťoučký kůň. ".repeat(100);
            let expected: Vec<u8> = wide(&text).iter().flat_map(|v| v.to_le_bytes()).collect();
            for _ in 0..10 {
                prepare(hwnd, &text, hwnd).unwrap();
                assert!(prepare(hwnd, "overlap", hwnd).is_err());
                let actual = read(hwnd, 13);
                assert_eq!(&actual[..expected.len()], expected.as_slice());
                assert!(message(hwnd, WM_TIMER, TIMER));
                assert!(PENDING.with(|p| p.borrow().is_none()));
                {
                    let _open = Open::new(hwnd).unwrap();
                    assert_ne!(IsClipboardFormatAvailable(protection), 0);
                    assert!(GetClipboardData(protection).is_null());
                }
                for (format, bytes) in &original {
                    let actual = read(hwnd, *format);
                    assert_eq!(&actual[..bytes.len()], bytes.as_slice());
                }
            }
            prepare(hwnd, "unread", hwnd).unwrap();
            assert!(message(hwnd, WM_TIMER, TIMER));
            assert_eq!(&read(hwnd, png)[..8], original[1].1.as_slice());
            prepare(hwnd, "obsolete", hwnd).unwrap();
            write(hwnd, &[(html, b"new copy\0".to_vec())]);
            restore(hwnd).unwrap();
            assert_eq!(&read(hwnd, html)[..9], b"new copy\0");
            write(hwnd, &[]);
            prepare(hwnd, "empty backup", hwnd).unwrap();
            let _ = read(hwnd, 13);
            restore(hwnd).unwrap();
            assert_eq!(CountClipboardFormats(), 0);
            let old: Vec<u8> = wide("recording one")
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect();
            write(hwnd, &[(13, old.clone())]);
            let previous_sequence = GetClipboardSequenceNumber();
            prepare(hwnd, "recording two", hwnd).unwrap();
            assert_ne!(GetClipboardSequenceNumber(), previous_sequence);
            let target = PENDING.with(|p| {
                let mut pending = p.borrow_mut();
                let paste = pending.as_mut().unwrap();
                let target = paste.target_process;
                paste.target_process = u32::MAX;
                target
            });
            {
                let _open = Open::new(hwnd).unwrap();
                assert!(GetClipboardData(13).is_null());
            }
            std::thread::sleep(std::time::Duration::from_millis(350));
            let mut msg = std::mem::zeroed();
            while PeekMessageW(&mut msg, hwnd, WM_TIMER, WM_TIMER, PM_REMOVE) != 0 {
                DispatchMessageW(&msg);
            }
            PENDING.with(|p| {
                let mut pending = p.borrow_mut();
                let paste = pending.as_mut().unwrap();
                assert!(!paste.text.1.is_null());
                paste.target_process = target;
            });
            let newer: Vec<u8> = wide("recording two")
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect();
            assert_eq!(&read(hwnd, 13)[..newer.len()], newer.as_slice());
            restore(hwnd).unwrap();
            assert_eq!(&read(hwnd, 13)[..old.len()], old.as_slice());
            let mut image = original[0].1.clone();
            image.resize(40 + 1920 * 1080 * 4, 0);
            image[4..8].copy_from_slice(&1920i32.to_le_bytes());
            image[8..12].copy_from_slice(&1080i32.to_le_bytes());
            image[20..24].copy_from_slice(&(1920u32 * 1080 * 4).to_le_bytes());
            write(hwnd, &[(8, image)]);
            let start = std::time::Instant::now();
            prepare(hwnd, "image benchmark", hwnd).unwrap();
            let bytes = PENDING.with(|p| {
                p.borrow()
                    .as_ref()
                    .unwrap()
                    .saved
                    .iter()
                    .map(|d| GlobalSize(d.1))
                    .sum::<usize>()
            });
            eprintln!(
                "1080p clipboard backup: {bytes} bytes, {:.2?}",
                start.elapsed()
            );
            let _ = read(hwnd, 13);
            restore(hwnd).unwrap();
            assert!(PENDING.with(|p| p.borrow().is_none()));
            let metadata = b"retained metadata\0".to_vec();
            write(hwnd, &[(protection, metadata.clone()), original[1].clone()]);
            prepare(hwnd, "metadata test", hwnd).unwrap();
            let _ = read(hwnd, 13);
            restore(hwnd).unwrap();
            assert_eq!(
                &read(hwnd, protection)[..metadata.len()],
                metadata.as_slice()
            );
            assert_eq!(&read(hwnd, png)[..8], original[1].1.as_slice());
            let unsupported = RegisterClipboardFormatW(wide("VtdUnavailableTest").as_ptr());
            {
                let _open = Open::new(hwnd).unwrap();
                SetClipboardData(unsupported, null_mut());
            }
            let sequence = GetClipboardSequenceNumber();
            assert!(prepare(hwnd, "must not replace clipboard", hwnd).is_err());
            assert_eq!(GetClipboardSequenceNumber(), sequence);
            assert_eq!(&read(hwnd, png)[..8], original[1].1.as_slice());
            DestroyWindow(hwnd);
        }
    }
}
