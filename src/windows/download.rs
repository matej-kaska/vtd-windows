use anyhow::{Context, Result, ensure};
use std::{
    fs::File,
    io::Read,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use vtd::models::Model;
use windows_sys::Win32::{Security::Cryptography::*, System::Threading::CREATE_NO_WINDOW};

pub struct Download {
    pub bytes: Arc<AtomicU64>,
    pub stage: Arc<AtomicU8>, // 0 checking cache, 1 downloading, 2 verifying
    pub result: Receiver<Result<PathBuf, String>>,
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Download {
    pub fn start(model: &'static Model, config: &Path) -> Self {
        let bytes = Arc::new(AtomicU64::new(0));
        let stage = Arc::new(AtomicU8::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, result) = mpsc::channel();
        let (progress, state, stop) = (bytes.clone(), stage.clone(), cancel.clone());
        let target = model.path(config);
        let thread = thread::spawn(move || {
            let result = fetch(model, &target, &stop, &progress, &state)
                .map(|()| target)
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
        });
        Self {
            bytes,
            stage,
            result,
            cancel,
            thread: Some(thread),
        }
    }
}
impl Drop for Download {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn digest(path: &Path, cancel: &AtomicBool) -> Result<String> {
    struct Hash {
        algorithm: BCRYPT_ALG_HANDLE,
        hash: BCRYPT_HASH_HANDLE,
    }
    impl Drop for Hash {
        fn drop(&mut self) {
            unsafe {
                BCryptDestroyHash(self.hash);
                BCryptCloseAlgorithmProvider(self.algorithm, 0);
            }
        }
    }
    let mut state = Hash {
        algorithm: std::ptr::null_mut(),
        hash: std::ptr::null_mut(),
    };
    unsafe {
        ensure!(
            BCryptOpenAlgorithmProvider(
                &mut state.algorithm,
                BCRYPT_SHA256_ALGORITHM,
                std::ptr::null(),
                0
            ) >= 0,
            "Cannot initialize SHA-256"
        );
        ensure!(
            BCryptCreateHash(
                state.algorithm,
                &mut state.hash,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
                0,
                0
            ) >= 0,
            "Cannot create SHA-256"
        );
    }
    let mut file = File::open(path)?;
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        ensure!(!cancel.load(Ordering::Relaxed), "Download cancelled");
        let len = file.read(&mut buffer)?;
        if len == 0 {
            break;
        }
        ensure!(
            unsafe { BCryptHashData(state.hash, buffer.as_ptr(), len as u32, 0) } >= 0,
            "SHA-256 failed"
        );
    }
    let mut hash = [0u8; 32];
    ensure!(
        unsafe { BCryptFinishHash(state.hash, hash.as_mut_ptr(), 32, 0) } >= 0,
        "SHA-256 failed"
    );
    Ok(hash.iter().map(|b| format!("{b:02x}")).collect())
}

fn fetch(
    model: &Model,
    path: &Path,
    cancel: &AtomicBool,
    progress: &AtomicU64,
    stage: &AtomicU8,
) -> Result<()> {
    if path.metadata().is_ok_and(|m| m.len() == model.bytes)
        && digest(path, cancel)? == model.sha256
    {
        return Ok(());
    }
    ensure!(!cancel.load(Ordering::Relaxed), "Download cancelled");
    std::fs::create_dir_all(path.parent().context("Model folder missing")?)?;
    let temporary = path.with_extension(format!("part-{}", std::process::id()));
    struct Partial(PathBuf);
    impl Drop for Partial {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let partial = Partial(temporary);
    let system = PathBuf::from(std::env::var_os("SystemRoot").context("Windows folder missing")?);
    struct Downloader(std::process::Child);
    impl Drop for Downloader {
        fn drop(&mut self) {
            if !matches!(self.0.try_wait(), Ok(Some(_))) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }
    let mut child = Downloader(
        Command::new(system.join("System32/curl.exe"))
            .args([
                "--location",
                "--fail",
                "--silent",
                "--show-error",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--connect-timeout",
                "30",
                "--retry",
                "2",
                "--output",
            ])
            .arg(&partial.0)
            .arg(model.url)
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Cannot start the Windows model downloader (curl.exe)")?,
    );
    stage.store(1, Ordering::Relaxed);
    loop {
        ensure!(!cancel.load(Ordering::Relaxed), "Download cancelled");
        if let Some(status) = child.0.try_wait()? {
            ensure!(
                status.success(),
                "Model download failed. Check your connection and try again (curl exit {}).",
                status.code().unwrap_or(-1)
            );
            break;
        }
        if let Ok(meta) = partial.0.metadata() {
            progress.store(meta.len(), Ordering::Relaxed);
        }
        thread::sleep(Duration::from_millis(100));
    }
    stage.store(2, Ordering::Relaxed);
    ensure!(
        partial.0.metadata()?.len() == model.bytes && digest(&partial.0, cancel)? == model.sha256,
        "Model verification failed. The previous model has been kept."
    );
    ensure!(!cancel.load(Ordering::Relaxed), "Download cancelled");
    std::fs::rename(&partial.0, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_and_failed_download_preserve_existing_model() {
        let dir = std::env::temp_dir().join(format!("vtd-download-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("model.gguf");
        let model = Model {
            id: "fixture",
            name: "Fixture",
            file: "model.gguf",
            engine: vtd::models::EngineKind::Transcribe,
            url: "http://127.0.0.1:9/rejected-non-https",
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            bytes: 3,
            details: "",
            autodetect: true,
            languages: &[],
        };
        let stop = AtomicBool::new(false);
        let progress = AtomicU64::new(0);
        let stage = AtomicU8::new(0);
        std::fs::write(&path, b"abc").unwrap();
        fetch(&model, &path, &stop, &progress, &stage).unwrap();
        assert_eq!(stage.load(Ordering::Relaxed), 0); // Verified cache needs no network.
        std::fs::write(&path, b"old").unwrap();
        assert!(fetch(&model, &path, &stop, &progress, &stage).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        stop.store(true, Ordering::Relaxed);
        assert!(fetch(&model, &path, &stop, &progress, &stage).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn verifies_hash_and_obeys_cancellation() {
        let path = std::env::temp_dir().join(format!("vtd-hash-{}.txt", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        let stop = AtomicBool::new(false);
        assert_eq!(
            digest(&path, &stop).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        stop.store(true, Ordering::Relaxed);
        assert!(digest(&path, &stop).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
