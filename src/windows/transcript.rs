//! Main-window-owned transcript storage without low-fragmentation heap caches.

use core::ptr::null_mut;
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc, VirtualFree,
};

const INLINE_CAPACITY: usize = 1024;

pub struct Transcript {
    inline: [u8; INLINE_CAPACITY],
    external: *mut u8,
    length: usize,
}

impl Transcript {
    pub const fn new() -> Self {
        Self {
            inline: [0; INLINE_CAPACITY],
            external: null_mut(),
            length: 0,
        }
    }

    pub fn bytes(&self) -> &[u8] {
        if self.external.is_null() {
            &self.inline[..self.length]
        } else {
            // Only replace() can set length or external; it commits this range.
            unsafe { core::slice::from_raw_parts(self.external, self.length) }
        }
    }

    pub fn replace(&mut self, bytes: &[u8]) -> bool {
        if bytes.is_empty() || core::str::from_utf8(bytes).is_err() {
            return false;
        }
        let replacement = if bytes.len() <= INLINE_CAPACITY {
            null_mut()
        } else {
            unsafe {
                VirtualAlloc(
                    null_mut(),
                    bytes.len(),
                    MEM_RESERVE | MEM_COMMIT,
                    PAGE_READWRITE,
                )
                .cast::<u8>()
            }
        };
        if replacement.is_null() && bytes.len() > INLINE_CAPACITY {
            return false;
        }
        let destination = if replacement.is_null() {
            self.inline.as_mut_ptr()
        } else {
            replacement
        };
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len());
        }
        self.release_external();
        self.external = replacement;
        self.length = bytes.len();
        true
    }

    fn release_external(&mut self) {
        if !self.external.is_null() {
            unsafe {
                // The entire previous reservation is returned, including when
                // a long dictation is replaced by a short one.
                VirtualFree(self.external.cast(), 0, MEM_RELEASE);
            }
            self.external = null_mut();
        }
    }

    pub fn clear(&mut self) {
        self.release_external();
        self.length = 0;
    }
}

impl Drop for Transcript {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_inline_and_boundary_transcripts_remain_exact() {
        let mut transcript = Transcript::new();
        for text in ["", "Dobrý den, příliš žluťoučký kůň 🦊"] {
            let accepted = transcript.replace(text.as_bytes());
            assert_eq!(accepted, !text.is_empty());
            if accepted {
                assert_eq!(transcript.bytes(), text.as_bytes());
                assert!(transcript.external.is_null());
            }
        }
        let text = "ě".repeat(INLINE_CAPACITY / 2);
        assert!(transcript.replace(text.as_bytes()));
        assert_eq!(transcript.bytes(), text.as_bytes());
        assert!(transcript.external.is_null());
        let large = format!("{text} 🦊");
        assert!(transcript.replace(large.as_bytes()));
        assert_eq!(transcript.bytes(), large.as_bytes());
        assert!(!transcript.external.is_null());
    }

    #[test]
    fn large_replacements_and_rejected_input_preserve_replay() {
        let mut transcript = Transcript::new();
        let long = "Přepis 🦊 ".repeat(32_000);
        assert!(transcript.replace(long.as_bytes()));
        assert_eq!(transcript.bytes(), long.as_bytes());
        assert!(!transcript.replace(&[0xc3, 0x28]));
        assert!(!transcript.replace(&[]));
        assert_eq!(transcript.bytes(), long.as_bytes());
        assert!(transcript.replace("Krátký přepis".as_bytes()));
        assert_eq!(transcript.bytes(), "Krátký přepis".as_bytes());
        assert!(transcript.external.is_null());
        assert!(transcript.replace(long.as_bytes()));
        transcript.clear();
        assert!(transcript.bytes().is_empty());
        assert!(transcript.external.is_null());
    }

    #[test]
    fn replacing_long_text_returns_its_virtual_memory_to_windows() {
        use windows_sys::Win32::System::Memory::{
            MEM_FREE, MEMORY_BASIC_INFORMATION, VirtualQuery,
        };

        let mut transcript = Transcript::new();
        let text = "a".repeat(512 * 1024);
        assert!(transcript.replace(text.as_bytes()));
        let allocation = transcript.external;
        assert!(transcript.replace(b"Short replacement"));
        let mut information: MEMORY_BASIC_INFORMATION = unsafe { core::mem::zeroed() };
        assert_ne!(
            unsafe {
                VirtualQuery(
                    allocation.cast(),
                    &mut information,
                    core::mem::size_of_val(&information),
                )
            },
            0
        );
        assert_eq!(information.State, MEM_FREE);
        assert_eq!(transcript.bytes(), b"Short replacement");
    }
}
