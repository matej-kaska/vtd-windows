use super::{autostart, capture::Recording, clipboard, ipc, protocol, worker::Worker};
use anyhow::{Result, bail, ensure};
#[cfg(test)]
use protocol::shortcut_matches;
use protocol::{
    COPY, IDLE, KEY, KEY_ACK, PAUSE, SESSION_CLOSED, SESSION_IDLE, SESSION_READY, SETTINGS,
};
pub(super) use protocol::{
    MENU_COMMAND, MENU_READY, OPEN_SETTINGS, SETTINGS_APPLY, SETTINGS_READY, UI_CLOSED,
};
use std::{
    cell::RefCell,
    ptr::null_mut,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use vtd::config::Config;
use windows_sys::Win32::{
    Foundation::*,
    System::{DataExchange::*, LibraryLoader::*, Memory::*, Threading::*},
    UI::{
        Input::{Ime::ImmDisableIME, KeyboardAndMouse::*},
        WindowsAndMessaging::*,
    },
};

const RESULT: u32 = WM_APP + 3;
static WINDOW: AtomicUsize = AtomicUsize::new(0);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static PAUSED: AtomicBool = AtomicBool::new(false);
static EPOCH: AtomicUsize = AtomicUsize::new(0);
thread_local! { static APP: RefCell<Option<App>> = const { RefCell::new(None) }; }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Focus {
    window: usize,
    control: usize,
    epoch: usize,
    input_epoch: usize,
}
impl Focus {
    fn current() -> Self {
        unsafe {
            let window = GetForegroundWindow();
            let mut info: GUITHREADINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of_val(&info) as u32;
            let thread = GetWindowThreadProcessId(window, null_mut());
            let ok = GetGUIThreadInfo(thread, &mut info) != 0;
            Self {
                window: window as usize,
                control: if ok { info.hwndFocus as usize } else { 0 },
                epoch: EPOCH.load(Ordering::Relaxed),
                input_epoch: ipc::epoch(),
            }
        }
    }
}

struct Job {
    samples: Vec<f32>,
    rate: u32,
    focus: Focus,
}
enum Reply {
    Loaded,
    Unloaded,
    Done(Focus, Result<String>, Duration),
}
enum Request {
    Warm,
    Unload,
    Transcribe(Job),
}
struct App {
    cfg: Config,
    hwnd: HWND,
    recording: Option<(Recording, Focus)>,
    finishing: bool,
    hold_recording: bool,
    busy: usize,
    worker_warm: bool,
    tx: mpsc::Sender<Request>,
    rx: mpsc::Receiver<Reply>,
    last: String,
    mouse: HHOOK,
}

impl Drop for App {
    fn drop(&mut self) {
        self.release_target();
    }
}

pub(super) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn control(command: &str) -> Result<()> {
    unsafe {
        let hwnd = FindWindowW(windows_sys::w!("VTDWindows"), null_mut());
        ensure!(!hwnd.is_null(), "VTD is not running");
        if command == "copy" {
            return copy_text(hwnd, &ipc::last()?);
        }
        if command == "stop" {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
        } else if command == "settings" {
            ensure!(
                PostMessageW(hwnd, SETTINGS, 0, 0) != 0,
                "Cannot open settings"
            );
        } else if command == "pause" || command == "resume" {
            ensure!(
                SendMessageTimeoutW(
                    hwnd,
                    PAUSE,
                    usize::from(command == "pause"),
                    0,
                    SMTO_ABORTIFHUNG,
                    2000,
                    null_mut()
                ) != 0,
                "Cannot change pause state"
            );
        } else {
            let mut text = [0u16; 512];
            let len = GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32);
            println!("{}", String::from_utf16_lossy(&text[..len as usize]));
        }
    }
    Ok(())
}

pub fn run(cfg: Config, mut capture_next: Option<std::path::PathBuf>) -> Result<()> {
    unsafe {
        let parent = ipc::parent();
        ensure!(!parent.is_null(), "VTD is not running");
        let mut owner = 0;
        GetWindowThreadProcessId(parent, &mut owner);
        // This thread only owns an invisible message window, never a text field.
        // Avoid initializing Windows text-input services when the tray gets focus.
        // Settings have their own process; other applications' IMEs are unaffected.
        ImmDisableIME(0);
        let mutex = CreateMutexW(
            null_mut(),
            0,
            wide(&format!("Local\\VTD-Session-{owner}")).as_ptr(),
        );
        ensure!(!mutex.is_null(), "Cannot create instance lock");
        if GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(mutex);
            bail!("VTD is already running");
        }
        let instance = GetModuleHandleW(null_mut());
        let class = windows_sys::w!("VTDSession");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: class,
            ..std::mem::zeroed()
        };
        ensure!(RegisterClassW(&wc) != 0, "Cannot register window");
        let hwnd = CreateWindowExW(
            0,
            class,
            class,
            0,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null_mut(),
        );
        ensure!(!hwnd.is_null(), "Cannot create message window");
        WINDOW.store(hwnd as usize, Ordering::Relaxed);
        let (tx, jobs) = mpsc::channel::<Request>();
        let (replies, rx) = mpsc::channel();
        let idle_unload_seconds = cfg.idle_unload_seconds;
        let address = hwnd as usize;
        let worker_thread = std::thread::Builder::new()
            .name("vtd-session".into())
            .stack_size(128 * 1024)
            .spawn(move || {
                let send = |r| {
                    let _ = replies.send(r);
                    PostMessageW(address as HWND, RESULT, 0, 0);
                };
                let mut engine: Option<Worker> = None;
                loop {
                    let job = if engine.is_some() && idle_unload_seconds > 0 {
                        match jobs.recv_timeout(Duration::from_secs(idle_unload_seconds)) {
                            Ok(j) => j,
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                if ACTIVE.load(Ordering::Relaxed) {
                                    continue;
                                }
                                engine = None;
                                send(Reply::Unloaded);
                                continue;
                            }
                            Err(_) => break,
                        }
                    } else {
                        match jobs.recv() {
                            Ok(j) => j,
                            Err(_) => break,
                        }
                    };
                    let job = match job {
                        Request::Warm => {
                            if engine.is_none() {
                                match Worker::spawn() {
                                    Ok(worker) => {
                                        engine = Some(worker);
                                        send(Reply::Loaded);
                                    }
                                    Err(e) => {
                                        eprintln!("VTD worker: {e:#}");
                                        send(Reply::Unloaded);
                                    }
                                }
                            }
                            continue;
                        }
                        Request::Unload => {
                            engine = None;
                            send(Reply::Unloaded);
                            continue;
                        }
                        Request::Transcribe(job) => job,
                    };
                    let start = Instant::now();
                    let result = (|| {
                        if engine.is_none() {
                            engine = Some(Worker::spawn()?);
                        }
                        let samples = vtd::audio::resample(job.samples, job.rate);
                        if let Some(path) = capture_next.take() {
                            PostMessageW(
                                ipc::parent(),
                                protocol::CAPTURE_CONSUMED,
                                GetCurrentProcessId() as usize,
                                0,
                            );
                            let spec = hound::WavSpec {
                                channels: 1,
                                sample_rate: 16000,
                                bits_per_sample: 32,
                                sample_format: hound::SampleFormat::Float,
                            };
                            let file = std::fs::OpenOptions::new()
                                .write(true)
                                .create_new(true)
                                .open(&path)?;
                            let mut wav =
                                hound::WavWriter::new(std::io::BufWriter::new(file), spec)?;
                            for &sample in &samples {
                                wav.write_sample(sample)?;
                            }
                            wav.finalize()?;
                            eprintln!("VTD diagnostic saved: {}", path.display());
                        }
                        engine.as_mut().unwrap().transcribe(samples)
                    })();
                    if result.is_err() {
                        engine = None;
                    }
                    send(Reply::Done(job.focus, result, start.elapsed()));
                }
            })?;
        APP.with(|a| {
            *a.borrow_mut() = Some(App {
                cfg,
                hwnd,
                recording: None,
                finishing: false,
                hold_recording: false,
                busy: 0,
                worker_warm: false,
                tx,
                rx,
                last: String::new(),
                mouse: null_mut(),
            })
        });
        let mut accepted = 0;
        ensure!(
            SendMessageTimeoutW(
                ipc::parent(),
                SESSION_READY,
                GetCurrentProcessId() as usize,
                hwnd as isize,
                SMTO_ABORTIFHUNG,
                3000,
                &mut accepted,
            ) != 0
                && accepted == 1,
            "Resident did not accept the recording session"
        );
        // Pending down/up messages are already ahead of this idle check.
        PostMessageW(hwnd, IDLE, 0, 0);
        let mut msg = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let join_worker = APP.with(|app| app.borrow().as_ref().is_none_or(|app| app.busy == 0));
        APP.with(|a| *a.borrow_mut() = None);
        if join_worker {
            let _ = worker_thread.join();
        }
        // On a forced close, the worker's kill-on-close job also releases its model.
        CloseHandle(mutex);
        PostMessageW(
            ipc::parent(),
            SESSION_CLOSED,
            GetCurrentProcessId() as usize,
            0,
        );
    }
    Ok(())
}

#[inline(never)]
unsafe fn modifiers_held(released: u32) -> bool {
    [
        VK_LSHIFT,
        VK_RSHIFT,
        VK_LCONTROL,
        VK_RCONTROL,
        VK_LMENU,
        VK_RMENU,
        VK_LWIN,
        VK_RWIN,
    ]
    .iter()
    .any(|&key| key as u32 != released && unsafe { GetAsyncKeyState(key as i32) } < 0)
}

unsafe extern "system" fn mouse(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if code == HC_ACTION as i32
            && matches!(
                w as u32,
                WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_MOUSEWHEEL
            )
        {
            EPOCH.fetch_add(1, Ordering::Relaxed);
        }
        CallNextHookEx(null_mut(), code, w, l)
    }
}

fn idle_if_finished(hwnd: HWND) {
    let finished = APP.with(|app| {
        app.borrow()
            .as_ref()
            .is_some_and(|app| app.recording.is_none() && app.busy == 0 && !app.worker_warm)
    });
    if finished && !clipboard::pending() {
        unsafe {
            let parent = ipc::parent();
            if parent.is_null() {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
                return;
            }
            let mut accepted = 0;
            if SendMessageTimeoutW(
                parent,
                SESSION_IDLE,
                GetCurrentProcessId() as usize,
                0,
                SMTO_ABORTIFHUNG,
                3000,
                &mut accepted,
            ) != 0
                && accepted == 1
            {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
            }
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if clipboard::message(hwnd, msg, w) {
            // WM_RENDERFORMAT can arrive inside SendInput while insert() still
            // holds APP's mutable borrow. Finish the clipboard request immediately,
            // but inspect the application only after that outer call returns.
            PostMessageW(hwnd, IDLE, 0, 0);
            return 0;
        }
        match msg {
            WM_COPYDATA if l != 0 => {
                let mut resident_pid = 0;
                GetWindowThreadProcessId(ipc::parent(), &mut resident_pid);
                let packet = &*(l as *const COPYDATASTRUCT);
                if w == resident_pid as usize
                    && packet.dwData == protocol::TRANSCRIPT
                    && !packet.lpData.is_null()
                {
                    let bytes = std::slice::from_raw_parts(
                        packet.lpData.cast::<u8>(),
                        packet.cbData as usize,
                    );
                    if let Ok(text) = std::str::from_utf8(bytes) {
                        APP.with(|app| {
                            if let Some(app) = app.borrow_mut().as_mut() {
                                app.last = text.to_owned();
                            }
                        });
                        return 1;
                    }
                }
                0
            }
            SETTINGS_APPLY => {
                let mut resident_pid = 0;
                GetWindowThreadProcessId(ipc::parent(), &mut resident_pid);
                if w != resident_pid as usize {
                    return 0;
                }
                match vtd::config::path().and_then(|path| Config::load(&path)) {
                    Ok(cfg) => {
                        APP.with(|app| {
                            if let Some(app) = app.borrow_mut().as_mut() {
                                app.update_preferences(cfg);
                            }
                        });
                        1
                    }
                    Err(e) => {
                        status(hwnd, &format!("Cannot apply settings: {e:#}"));
                        0
                    }
                }
            }
            KEY | RESULT | WM_TIMER | PAUSE => {
                APP.with(|app| {
                    if let Some(app) = app.borrow_mut().as_mut() {
                        if msg == PAUSE {
                            app.pause(w != 0);
                        } else if msg == RESULT {
                            app.results();
                        } else if msg == WM_TIMER && (w == 1 || app.finishing) {
                            app.stop(false);
                        } else if msg == KEY {
                            app.key(w);
                        }
                    }
                });
                if msg == KEY {
                    SendMessageTimeoutW(
                        ipc::parent(),
                        KEY_ACK,
                        GetCurrentProcessId() as usize,
                        0,
                        SMTO_ABORTIFHUNG,
                        3000,
                        null_mut(),
                    );
                }
                idle_if_finished(hwnd);
                0
            }
            IDLE => {
                idle_if_finished(hwnd);
                0
            }
            COPY => APP.with(|app| {
                app.borrow()
                    .as_ref()
                    .is_some_and(|app| copy_text(hwnd, &app.last).is_ok())
                    as LRESULT
            }),
            WM_CLOSE => {
                if let Err(e) = clipboard::restore(hwnd) {
                    status(hwnd, &e.to_string());
                    return 0;
                }
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

impl App {
    fn update_preferences(&mut self, cfg: Config) {
        if self.cfg.language != cfg.language || self.cfg.model != cfg.model {
            // The next worker reads the committed model and language from disk.
            let _ = self.tx.send(Request::Unload);
        }
        self.cfg.language = cfg.language;
        self.cfg.model = cfg.model;
        self.cfg.trigger_key = cfg.trigger_key;
        self.cfg.toggle_key = cfg.toggle_key;
        self.cfg.replay_key = cfg.replay_key;
        self.cfg.mute_output = cfg.mute_output;
        status(self.hwnd, "VTD: settings saved");
    }

    fn track_target(&mut self) -> Result<Focus> {
        if self.mouse.is_null() {
            self.mouse = unsafe {
                SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), GetModuleHandleW(null_mut()), 0)
            };
            ensure!(!self.mouse.is_null(), "Cannot track target field");
        }
        Ok(Focus::current())
    }

    fn release_target(&mut self) {
        if !self.mouse.is_null() {
            unsafe {
                UnhookWindowsHookEx(self.mouse);
            }
            self.mouse = null_mut();
        }
    }

    fn pause(&mut self, paused: bool) {
        PAUSED.store(paused, Ordering::Relaxed);
        if paused {
            self.stop(true);
            let _ = self.tx.send(Request::Unload);
        }
        status(
            self.hwnd,
            if paused {
                "VTD: paused"
            } else if self.busy > 0 {
                "VTD: processing"
            } else {
                "VTD: ready"
            },
        );
    }

    fn key(&mut self, action: usize) {
        if PAUSED.load(Ordering::Relaxed) {
            return;
        }
        if action == 4 {
            let result = insert(&self.last, Focus::current(), self.hwnd, true);
            match result {
                Ok(()) => status(self.hwnd, "VTD: last transcript inserted"),
                Err(e) => status(self.hwnd, &e.to_string()),
            }
            return;
        }
        if action == 2 {
            self.stop(true);
            return;
        }
        if action == 0 {
            if self.hold_recording {
                self.finish();
            }
            return;
        }
        if self.recording.is_some() {
            if self.finishing {
                self.stop(false);
            } else {
                if action == 3 || self.cfg.toggle {
                    self.finish();
                }
                return;
            }
        }
        if self.busy >= 2 {
            status(self.hwnd, "VTD: finishing previous transcriptions");
            return;
        }
        let focus = Focus::current();
        match Recording::start(&self.cfg) {
            Ok(recording) => {
                if self.busy == 0 {
                    self.worker_warm = self.tx.send(Request::Warm).is_ok();
                }
                self.hold_recording = action == 1 && !self.cfg.toggle;
                self.recording = Some((recording, focus));
                ACTIVE.store(true, Ordering::Relaxed);
                unsafe {
                    SetTimer(self.hwnd, 1, self.cfg.max_recording_seconds * 1000, None);
                }
                status(self.hwnd, "VTD: recording - Esc to cancel");
            }
            Err(e) => status(self.hwnd, &e.to_string()),
        }
    }

    fn finish(&mut self) {
        if self.recording.is_some() && !self.finishing {
            match self.track_target() {
                Ok(focus) => self.recording.as_mut().unwrap().1 = focus,
                Err(e) => {
                    self.stop(true);
                    status(self.hwnd, &e.to_string());
                    return;
                }
            }
            self.finishing = true;
            unsafe {
                if SetTimer(self.hwnd, 2, 250, None) == 0 {
                    self.stop(false);
                }
            }
        }
    }

    fn stop(&mut self, cancel: bool) {
        if cancel {
            EPOCH.fetch_add(1, Ordering::Relaxed);
            self.release_target();
        }
        let Some((recording, mut focus)) = self.recording.take() else {
            return;
        };
        if !self.finishing && !cancel {
            match self.track_target() {
                Ok(target) => focus = target,
                Err(e) => {
                    ACTIVE.store(self.busy > 0, Ordering::Relaxed);
                    unsafe {
                        KillTimer(self.hwnd, 1);
                    }
                    status(self.hwnd, &e.to_string());
                    return;
                }
            }
        }
        unsafe {
            KillTimer(self.hwnd, 1);
            KillTimer(self.hwnd, 2);
        }
        self.finishing = false;
        if cancel {
            ACTIVE.store(self.busy > 0, Ordering::Relaxed);
            drop(recording);
            status(self.hwnd, "VTD: ready");
            return;
        }
        match recording.finish() {
            Ok((samples, rate)) if samples.len() >= rate as usize * 3 / 10 => {
                self.busy += 1;
                self.worker_warm = true;
                status(self.hwnd, "VTD: transcribing");
                if self
                    .tx
                    .send(Request::Transcribe(Job {
                        samples,
                        rate,
                        focus,
                    }))
                    .is_err()
                {
                    self.busy -= 1;
                    if self.busy == 0 {
                        self.release_target();
                    }
                    ACTIVE.store(self.busy > 0, Ordering::Relaxed);
                    status(self.hwnd, "Transcription thread stopped. Restart VTD.");
                }
            }
            Ok(_) => {
                if self.busy == 0 {
                    self.release_target();
                }
                ACTIVE.store(self.busy > 0, Ordering::Relaxed);
                status(self.hwnd, "VTD: recording too short");
            }
            Err(e) => {
                if self.busy == 0 {
                    self.release_target();
                }
                ACTIVE.store(self.busy > 0, Ordering::Relaxed);
                status(self.hwnd, &e.to_string());
            }
        }
    }

    fn results(&mut self) {
        while let Ok(reply) = self.rx.try_recv() {
            match reply {
                Reply::Loaded => self.worker_warm = true,
                Reply::Unloaded => {
                    self.worker_warm = false;
                    if self.recording.is_none() && self.busy == 0 {
                        status(self.hwnd, "VTD: model unloaded");
                    }
                }
                Reply::Done(focus, result, elapsed) => {
                    self.worker_warm = result.is_ok();
                    self.busy -= 1;
                    if self.busy == 0 && !self.finishing {
                        self.release_target();
                    }
                    ACTIVE.store(self.recording.is_some() || self.busy > 0, Ordering::Relaxed);
                    match result {
                        Ok(text) if text.is_empty() => status(self.hwnd, "VTD: no speech"),
                        Ok(text) => {
                            self.last = text;
                            let _ = ipc::send(
                                ipc::parent(),
                                protocol::TRANSCRIPT,
                                self.last.as_bytes(),
                            );
                            if let Err(e) =
                                insert(&self.last, focus, self.hwnd, self.cfg.clipboard_paste)
                            {
                                status(
                                    self.hwnd,
                                    &format!(
                                        "{e}. Click the target field and use the insert-last-transcript shortcut."
                                    ),
                                );
                            } else {
                                status(
                                    self.hwnd,
                                    &format!("VTD: completed in {:.2} s", elapsed.as_secs_f64()),
                                );
                            }
                        }
                        Err(e) => {
                            eprintln!("VTD error: {e:#}");
                            status(self.hwnd, &format!("{e:#}"));
                        }
                    }
                    if self.recording.is_some() {
                        status(self.hwnd, "VTD: recording - Esc to cancel");
                    } else if self.busy > 0 {
                        status(self.hwnd, "VTD: transcribing");
                    }
                }
            }
        }
    }
}

fn insert(text: &str, target: Focus, hwnd: HWND, clipboard: bool) -> Result<()> {
    ensure!(!text.is_empty(), "No transcript available yet");
    ensure!(
        target.window != 0 && target.input_epoch != usize::MAX && target == Focus::current(),
        "The target window or field changed"
    );
    unsafe {
        ensure!(!modifiers_held(0), "A modifier key is held down");
        let input = |key, unit, flags| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key,
                    wScan: unit,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let sent_all = if clipboard {
            clipboard::prepare(hwnd, text, target.window as HWND)?;
            if target != Focus::current() {
                clipboard::restore(hwnd)?;
                bail!("The target window or field changed");
            }
            let keys = [
                input(VK_CONTROL, 0, 0),
                input(0x56, 0, 0),
                input(0x56, 0, KEYEVENTF_KEYUP),
                input(VK_CONTROL, 0, KEYEVENTF_KEYUP),
            ];
            #[cfg(feature = "resident-test")]
            let count = std::env::var("VTD_TEST_PASTE_LIMIT")
                .ok()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(4)
                .min(4);
            #[cfg(not(feature = "resident-test"))]
            let count = 4;
            let sent = SendInput(count, keys.as_ptr(), std::mem::size_of::<INPUT>() as i32);
            #[cfg(feature = "resident-test")]
            eprintln!("VTD test paste: requested={count} sent={sent}");
            if sent != 4 {
                // A partial send can leave our Ctrl or V down. Release only keys
                // pressed by that prefix before restoring the original clipboard.
                let releases = match sent {
                    1 | 3 => &keys[3..],
                    2 => &keys[2..],
                    _ => &[],
                };
                if !releases.is_empty() {
                    SendInput(
                        releases.len() as u32,
                        releases.as_ptr(),
                        std::mem::size_of::<INPUT>() as i32,
                    );
                }
                clipboard::restore(hwnd)?;
            }
            sent == 4
        } else {
            let keys: Vec<_> = text
                .encode_utf16()
                .flat_map(|unit| {
                    [
                        input(0, unit, KEYEVENTF_UNICODE),
                        input(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
                    ]
                })
                .collect();
            SendInput(
                keys.len() as u32,
                keys.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            ) as usize
                == keys.len()
        };
        ensure!(sent_all, "Windows did not allow all text to be inserted");
    }
    Ok(())
}

fn copy_text(hwnd: HWND, text: &str) -> Result<()> {
    ensure!(!text.is_empty(), "No transcript available yet");
    unsafe {
        let data = wide(text);
        let memory = GlobalAlloc(GMEM_MOVEABLE, data.len() * 2);
        ensure!(!memory.is_null(), "Clipboard allocation failed");
        let dest = GlobalLock(memory) as *mut u16;
        if dest.is_null() {
            GlobalFree(memory);
            bail!("Clipboard lock failed");
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), dest, data.len());
        GlobalUnlock(memory);
        if OpenClipboard(hwnd) == 0 {
            GlobalFree(memory);
            bail!("The clipboard is in use by another application");
        }
        let ok = EmptyClipboard() != 0 && !SetClipboardData(13, memory).is_null();
        CloseClipboard();
        if !ok {
            GlobalFree(memory);
            bail!("Cannot write to the clipboard");
        }
    }
    Ok(())
}

fn status(hwnd: HWND, text: &str) {
    let text = if PAUSED.load(Ordering::Relaxed) {
        "VTD: paused"
    } else {
        text
    };
    unsafe {
        SetWindowTextW(hwnd, wide(text).as_ptr());
    }
    ipc::status(
        ipc::parent(),
        text,
        ACTIVE.load(Ordering::Relaxed),
        PAUSED.load(Ordering::Relaxed),
    );
    eprintln!("{text}");
}

pub fn autostart(mode: Option<&str>) -> Result<()> {
    match mode {
        Some("on") => autostart::set_enabled(true),
        Some("off") => autostart::set_enabled(false),
        _ => bail!("Use: vtd autostart on|off"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_match_exact_modifiers_and_release_after_modifier_changes() {
        assert!(shortcut_matches(120, 120, 0, false, false));
        assert!(!shortcut_matches(120, 120, 0x100, false, false));
        assert!(shortcut_matches(0x320, 0x20, 0x300, false, false));
        assert!(!shortcut_matches(0x320, 0x20, 0x200, false, false));
        assert!(!shortcut_matches(0x320, 0x20, 0x1300, false, false));
        assert!(shortcut_matches(0x320, 0x20, 0, true, true));
        assert!(!shortcut_matches(0x320, 0x20, 0, false, true));
        assert!(!shortcut_matches(0x220, 0x20, 0x10000, false, false));
        assert!(shortcut_matches(0x320, 0x20, 0x10000, true, false));
        assert!(shortcut_matches(0xad, 0xad, 0, false, false));
    }

    #[test]
    fn completing_one_transcription_keeps_the_next_active() {
        let (tx, jobs) = mpsc::channel();
        let (replies, rx) = mpsc::channel();
        let mut app = App {
            cfg: Config::default(),
            hwnd: null_mut(),
            recording: None,
            finishing: false,
            hold_recording: false,
            busy: 2,
            worker_warm: false,
            tx,
            rx,
            last: String::new(),
            mouse: null_mut(),
        };
        let focus = Focus {
            window: 0,
            control: 0,
            epoch: 0,
            input_epoch: 0,
        };
        for remaining in [1, 0] {
            replies
                .send(Reply::Done(focus, Ok(String::new()), Duration::ZERO))
                .unwrap();
            app.results();
            assert_eq!(app.busy, remaining);
            assert_eq!(ACTIVE.load(Ordering::Relaxed), remaining > 0);
        }
        for result in [
            Ok("Příliš žluťoučký kůň.".into()),
            Ok(String::new()),
            Err(anyhow::anyhow!("decode failed")),
        ] {
            app.busy = 1;
            replies
                .send(Reply::Done(focus, result, Duration::ZERO))
                .unwrap();
            app.results();
            assert_eq!(app.last, "Příliš žluťoučký kůň.");
        }
        replies.send(Reply::Unloaded).unwrap();
        app.results();
        assert_eq!(app.last, "Příliš žluťoučký kůň.");
        assert!(insert("", focus, null_mut(), false).is_err());
        let mut preferences = app.cfg.clone();
        preferences.replay_key = 122;
        app.update_preferences(preferences.clone());
        assert_eq!(app.cfg.replay_key, 122);
        assert!(jobs.try_recv().is_err()); // A shortcut edit retains the warm engine.
        preferences.language = if app.cfg.language == "de" { "cs" } else { "de" }.into();
        app.update_preferences(preferences);
        assert!(matches!(jobs.try_recv().unwrap(), Request::Unload));
        let mut preferences = app.cfg.clone();
        preferences.model = std::path::Path::new("models").join(vtd::models::MODELS[1].file);
        app.update_preferences(preferences.clone());
        assert_eq!(app.cfg.model, preferences.model);
        assert!(matches!(jobs.try_recv().unwrap(), Request::Unload));
        assert_eq!(app.last, "Příliš žluťoučký kůň.");
    }
}
