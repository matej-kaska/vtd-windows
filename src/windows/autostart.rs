use super::runtime::wide;
use anyhow::{Context, Result, ensure};
use windows_sys::Win32::{Foundation::*, System::Registry::*};

const RUN_KEY: *const u16 = windows_sys::w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE: *const u16 = windows_sys::w!("VTD Windows");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    kind: u32,
    data: Vec<u16>,
}

fn check(code: u32) -> Result<()> {
    if code != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(code as i32).into());
    }
    Ok(())
}

pub fn snapshot() -> Result<Option<Entry>> {
    read_at(RUN_KEY).context("Cannot read the startup setting.")
}

pub fn enabled(entry: &Option<Entry>) -> Result<bool> {
    let expected = format!(
        "\"{}\" run",
        std::env::current_exe()?.with_file_name("vtd.exe").display()
    );
    Ok(entry.as_ref().is_some_and(|entry| {
        let len = entry
            .data
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.data.len());
        String::from_utf16_lossy(&entry.data[..len])
            .trim()
            .eq_ignore_ascii_case(&expected)
    }))
}

pub fn set_enabled(enabled: bool) -> Result<()> {
    let entry = if enabled {
        Some(Entry {
            kind: REG_SZ,
            data: wide(&format!(
                "\"{}\" run",
                std::env::current_exe()?.with_file_name("vtd.exe").display()
            )),
        })
    } else {
        None
    };
    restore(&entry)
}

pub fn restore(entry: &Option<Entry>) -> Result<()> {
    write_at(RUN_KEY, entry).context("Cannot change the startup setting.")
}

fn read_at(path: *const u16) -> Result<Option<Entry>> {
    unsafe {
        let (mut kind, mut bytes) = (0, 0);
        let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
        let code = RegGetValueW(
            HKEY_CURRENT_USER,
            path,
            VALUE,
            flags,
            &mut kind,
            std::ptr::null_mut(),
            &mut bytes,
        );
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(code)?;
        ensure!(bytes <= 65536, "Unexpected autostart value size");
        let mut data = vec![0u16; (bytes as usize).div_ceil(2)];
        check(RegGetValueW(
            HKEY_CURRENT_USER,
            path,
            VALUE,
            flags,
            &mut kind,
            data.as_mut_ptr().cast(),
            &mut bytes,
        ))?;
        data.truncate((bytes as usize).div_ceil(2));
        Ok(Some(Entry { kind, data }))
    }
}

fn write_at(path: *const u16, entry: &Option<Entry>) -> Result<()> {
    unsafe {
        if let Some(entry) = entry {
            let mut key = std::ptr::null_mut();
            check(RegCreateKeyExW(
                HKEY_CURRENT_USER,
                path,
                0,
                std::ptr::null(),
                0,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut key,
                std::ptr::null_mut(),
            ))?;
            let result = RegSetValueExW(
                key,
                VALUE,
                0,
                entry.kind,
                entry.data.as_ptr().cast(),
                (entry.data.len() * 2) as u32,
            );
            RegCloseKey(key);
            check(result)
        } else {
            let code = RegDeleteKeyValueW(HKEY_CURRENT_USER, path, VALUE);
            if code == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                check(code)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enable_disable_and_rollback_preserve_registry_data() {
        // An isolated key: tests never modify the user's real Run entry.
        let path = wide(&format!(
            "Software\\VTD-Autostart-Test-{}",
            std::process::id()
        ));
        let key = path.as_ptr();
        assert_eq!(read_at(key).unwrap(), None);
        write_at(key, &None).unwrap();
        let original = Some(Entry {
            kind: REG_EXPAND_SZ,
            data: wide("\"%LOCALAPPDATA%\\Žluťoučký kůň\\vtd.exe\" run"),
        });
        write_at(key, &original).unwrap();
        assert_eq!(read_at(key).unwrap(), original);
        let changed = Some(Entry {
            kind: REG_SZ,
            data: wide("\"C:\\Program Files\\VTD\\vtd.exe\" run"),
        });
        write_at(key, &changed).unwrap();
        assert_eq!(read_at(key).unwrap(), changed);
        write_at(key, &original).unwrap();
        assert_eq!(read_at(key).unwrap(), original);
        write_at(key, &None).unwrap();
        write_at(key, &None).unwrap();
        assert_eq!(read_at(key).unwrap(), None);
        unsafe {
            assert_eq!(RegDeleteKeyW(HKEY_CURRENT_USER, key), ERROR_SUCCESS);
        }
    }
}
