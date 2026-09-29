use super::{tray, worker::Worker};
use anyhow::{Result, bail, ensure};
use std::{
    cell::RefCell,
    ptr::null_mut,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use vtd::{config::Config, recording::Recording};
use windows_sys::Win32::{
    Foundation::*,
    System::{DataExchange::*, LibraryLoader::*, Memory::*, Threading::*},
    UI::{
        Input::KeyboardAndMouse::*,
        Shell::{NIM_ADD, NIM_DELETE, NIM_MODIFY},
        WindowsAndMessaging::*,
    },
};

const COPY: u32 = WM_APP + 1;
const KEY: u32 = WM_APP + 2;
const RESULT: u32 = WM_APP + 3;
const PAUSE: u32 = WM_APP + 5;
static WINDOW: AtomicUsize = AtomicUsize::new(0);
static TRIGGER: AtomicU32 = AtomicU32::new(119);
static HELD: AtomicBool = AtomicBool::new(false);
static TOGGLE: AtomicU32 = AtomicU32::new(120);
static TOGGLE_HELD: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static PAUSED: AtomicBool = AtomicBool::new(false);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static EPOCH: AtomicUsize = AtomicUsize::new(0);
thread_local! { static APP: RefCell<Option<App>> = const { RefCell::new(None) }; }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Focus {
    window: usize,
    control: usize,
    epoch: usize,
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
        let hwnd = FindWindowW(wide("VTDWindows").as_ptr(), null_mut());
        ensure!(!hwnd.is_null(), "VTD is not running");
        if command == "stop" {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
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
        } else if command == "copy" {
            let mut result = 0;
            ensure!(
                SendMessageTimeoutW(hwnd, COPY, 0, 0, SMTO_ABORTIFHUNG, 2000, &mut result) != 0
                    && result == 1,
                "Cannot copy last transcript"
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
        let mutex = CreateMutexW(null_mut(), 0, wide("Local\\VTD-Windows").as_ptr());
        ensure!(!mutex.is_null(), "Cannot create instance lock");
        if GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(mutex);
            bail!("VTD is already running");
        }
        let instance = GetModuleHandleW(null_mut());
        let class = wide("VTDWindows");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        ensure!(RegisterClassW(&wc) != 0, "Cannot register window");
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
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
        TASKBAR_CREATED.store(
            RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
            Ordering::Relaxed,
        );
        ensure!(
            tray::update(hwnd, "VTD", NIM_ADD),
            "Cannot create tray icon"
        );
        TRIGGER.store(cfg.trigger_key, Ordering::Relaxed);
        TOGGLE.store(cfg.toggle_key, Ordering::Relaxed);
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), instance, 0);
        ensure!(!hook.is_null(), "Cannot register keyboard hook");
        let (tx, jobs) = mpsc::channel::<Request>();
        let (replies, rx) = mpsc::channel();
        let worker_cfg = cfg.clone();
        let address = hwnd as usize;
        std::thread::spawn(move || {
            let send = |r| {
                let _ = replies.send(r);
                PostMessageW(address as HWND, RESULT, 0, 0);
            };
            let mut engine: Option<Worker> = None;
            loop {
                let job = if engine.is_some() && worker_cfg.idle_unload_seconds > 0 {
                    match jobs.recv_timeout(Duration::from_secs(worker_cfg.idle_unload_seconds)) {
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
                                Ok(worker) => engine = Some(worker),
                                Err(e) => eprintln!("VTD worker: {e:#}"),
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
                        let mut wav = hound::WavWriter::new(std::io::BufWriter::new(file), spec)?;
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
        });
        APP.with(|a| {
            *a.borrow_mut() = Some(App {
                cfg,
                hwnd,
                recording: None,
                finishing: false,
                hold_recording: false,
                busy: 0,
                tx,
                rx,
                last: String::new(),
                mouse: null_mut(),
            })
        });
        status(hwnd, "VTD: připraveno");
        let mut msg = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        UnhookWindowsHookEx(hook);
        tray::update(hwnd, "", NIM_DELETE);
        APP.with(|a| *a.borrow_mut() = None);
        CloseHandle(mutex);
    }
    Ok(())
}

unsafe extern "system" fn keyboard(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if code == HC_ACTION as i32 && !PAUSED.load(Ordering::Relaxed) {
            let event = &*(l as *const KBDLLHOOKSTRUCT);
            {
                let down = w as u32 == WM_KEYDOWN || w as u32 == WM_SYSKEYDOWN;
                let up = w as u32 == WM_KEYUP || w as u32 == WM_SYSKEYUP;
                if event.vkCode == TOGGLE.load(Ordering::Relaxed) {
                    if down && !TOGGLE_HELD.swap(true, Ordering::Relaxed) {
                        PostMessageW(WINDOW.load(Ordering::Relaxed) as HWND, KEY, 3, 0);
                    }
                    if up {
                        TOGGLE_HELD.store(false, Ordering::Relaxed);
                    }
                    return 1;
                }
                if event.vkCode == TRIGGER.load(Ordering::Relaxed) {
                    if down && !HELD.swap(true, Ordering::Relaxed) {
                        PostMessageW(WINDOW.load(Ordering::Relaxed) as HWND, KEY, 1, 0);
                    }
                    if up && HELD.swap(false, Ordering::Relaxed) {
                        PostMessageW(WINDOW.load(Ordering::Relaxed) as HWND, KEY, 0, 0);
                    }
                    return 1;
                }
                if event.vkCode == VK_ESCAPE as u32 && ACTIVE.load(Ordering::Relaxed) && down {
                    PostMessageW(WINDOW.load(Ordering::Relaxed) as HWND, KEY, 2, 0);
                    return 1;
                }
                if down && event.flags & LLKHF_INJECTED == 0 {
                    EPOCH.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        CallNextHookEx(null_mut(), code, w, l)
    }
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

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if msg != 0 && msg == TASKBAR_CREATED.load(Ordering::Relaxed) {
            let mut text = [0u16; 128];
            let len = GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32);
            tray::update(
                hwnd,
                &String::from_utf16_lossy(&text[..len as usize]),
                NIM_ADD,
            );
            return 0;
        }
        match msg {
            tray::EVENT => {
                if l as u32 == WM_RBUTTONUP || l as u32 == WM_LBUTTONUP {
                    match tray::menu(hwnd, PAUSED.load(Ordering::Relaxed)) {
                        1 => APP.with(|a| {
                            if let Some(app) = a.borrow_mut().as_mut() {
                                app.pause(!PAUSED.load(Ordering::Relaxed));
                            }
                        }),
                        2 => {
                            PostMessageW(hwnd, WM_CLOSE, 0, 0);
                        }
                        _ => {}
                    }
                }
                0
            }
            KEY | RESULT | WM_TIMER | PAUSE => {
                APP.with(|a| {
                    if let Some(app) = a.borrow_mut().as_mut() {
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
                0
            }
            COPY => APP.with(|a| {
                a.borrow()
                    .as_ref()
                    .is_some_and(|app| copy_text(hwnd, &app.last).is_ok())
                    as LRESULT
            }),
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

impl App {
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
        HELD.store(false, Ordering::Relaxed);
        TOGGLE_HELD.store(false, Ordering::Relaxed);
        if paused {
            self.stop(true);
            let _ = self.tx.send(Request::Unload);
        }
        status(
            self.hwnd,
            if paused {
                "VTD: pozastaveno"
            } else if self.busy > 0 {
                "VTD: zpracovávám"
            } else {
                "VTD: připraveno"
            },
        );
    }

    fn key(&mut self, action: usize) {
        if PAUSED.load(Ordering::Relaxed) {
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
                unsafe {
                    KillTimer(self.hwnd, 2);
                }
                self.finishing = false;
                if self.busy == 0 {
                    self.release_target();
                }
            } else if action == 3 || self.cfg.toggle {
                self.finish();
            }
            return;
        }
        if self.busy >= 2 {
            status(self.hwnd, "VTD: dokončuji předchozí přepisy");
            return;
        }
        let focus = Focus::current();
        match Recording::start(&self.cfg) {
            Ok(recording) => {
                if self.busy == 0 {
                    let _ = self.tx.send(Request::Warm);
                }
                self.hold_recording = action == 1 && !self.cfg.toggle;
                self.recording = Some((recording, focus));
                ACTIVE.store(true, Ordering::Relaxed);
                unsafe {
                    SetTimer(self.hwnd, 1, self.cfg.max_recording_seconds * 1000, None);
                }
                status(self.hwnd, "VTD: nahrávám · Esc zruší");
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
            status(self.hwnd, "VTD: připraveno");
            return;
        }
        match recording.finish() {
            Ok((samples, rate)) if samples.len() >= rate as usize * 3 / 10 => {
                self.busy += 1;
                status(self.hwnd, "VTD: přepisuji");
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
                    status(self.hwnd, "Přepisovací vlákno skončilo. Restartujte VTD.");
                }
            }
            Ok(_) => {
                if self.busy == 0 {
                    self.release_target();
                }
                ACTIVE.store(self.busy > 0, Ordering::Relaxed);
                status(self.hwnd, "VTD: příliš krátký záznam");
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
                Reply::Unloaded => {
                    if self.recording.is_none() && self.busy == 0 {
                        status(self.hwnd, "VTD: model uspán");
                    }
                }
                Reply::Done(focus, result, elapsed) => {
                    self.busy -= 1;
                    if self.busy == 0 && !self.finishing {
                        self.release_target();
                    }
                    ACTIVE.store(self.recording.is_some() || self.busy > 0, Ordering::Relaxed);
                    match result {
                        Ok(text) if text.is_empty() => status(self.hwnd, "VTD: bez řeči"),
                        Ok(text) => {
                            self.last = text;
                            if let Err(e) =
                                insert(&self.last, focus, self.hwnd, self.cfg.clipboard_paste)
                            {
                                status(
                                    self.hwnd,
                                    &format!("{e}. Přepis je dostupný přes vtd copy."),
                                );
                            } else {
                                status(
                                    self.hwnd,
                                    &format!("VTD: hotovo za {:.2} s", elapsed.as_secs_f64()),
                                );
                            }
                        }
                        Err(e) => {
                            eprintln!("VTD error: {e:#}");
                            status(self.hwnd, &format!("{e:#}"));
                        }
                    }
                    if self.recording.is_some() {
                        status(self.hwnd, "VTD: nahrávám · Esc zruší");
                    } else if self.busy > 0 {
                        status(self.hwnd, "VTD: přepisuji");
                    }
                }
            }
        }
    }
}

fn insert(text: &str, target: Focus, hwnd: HWND, clipboard: bool) -> Result<()> {
    ensure!(
        target.window != 0 && target == Focus::current(),
        "Změnilo se cílové okno nebo pole"
    );
    unsafe {
        for key in [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN] {
            ensure!(
                GetAsyncKeyState(key as i32) >= 0,
                "Je stisknutá modifikační klávesa"
            );
        }
        let keys: Vec<(u16, u16, u32)> = if clipboard {
            copy_text(hwnd, text)?;
            ensure!(
                target == Focus::current(),
                "Změnilo se cílové okno nebo pole"
            );
            vec![
                (VK_CONTROL, 0, 0),
                (0x56, 0, 0),
                (0x56, 0, KEYEVENTF_KEYUP),
                (VK_CONTROL, 0, KEYEVENTF_KEYUP),
            ]
        } else {
            text.encode_utf16()
                .flat_map(|unit| {
                    [
                        (0, unit, KEYEVENTF_UNICODE),
                        (0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
                    ]
                })
                .collect()
        };
        let input: Vec<_> = keys
            .into_iter()
            .map(|(key, unit, flags)| INPUT {
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
            })
            .collect();
        let sent = SendInput(
            input.len() as u32,
            input.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        );
        ensure!(
            sent as usize == input.len(),
            "Windows nepovolil vložení celého textu"
        );
    }
    Ok(())
}

fn copy_text(hwnd: HWND, text: &str) -> Result<()> {
    ensure!(!text.is_empty(), "Zatím není žádný přepis");
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
            bail!("Schránka je používána jinou aplikací");
        }
        let ok = EmptyClipboard() != 0 && !SetClipboardData(13, memory).is_null();
        CloseClipboard();
        if !ok {
            GlobalFree(memory);
            bail!("Nelze zapsat do schránky");
        }
    }
    Ok(())
}

fn status(hwnd: HWND, text: &str) {
    let text = if PAUSED.load(Ordering::Relaxed) {
        "VTD: pozastaveno"
    } else {
        text
    };
    unsafe {
        SetWindowTextW(hwnd, wide(text).as_ptr());
    }
    tray::update(hwnd, text, NIM_MODIFY);
    eprintln!("{text}");
}

pub fn autostart(mode: Option<&str>) -> Result<()> {
    use std::os::windows::process::CommandExt;
    let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    let mut command = std::process::Command::new("reg.exe");
    match mode {
        Some("on") => {
            command
                .args(["add", key, "/v", "VTD Windows", "/t", "REG_SZ", "/d"])
                .arg(format!("\"{}\" run", std::env::current_exe()?.display()))
                .arg("/f");
        }
        Some("off") => {
            command.args(["delete", key, "/v", "VTD Windows", "/f"]);
        }
        _ => bail!("Use: vtd autostart on|off"),
    }
    ensure!(
        command.creation_flags(CREATE_NO_WINDOW).status()?.success(),
        "Cannot update autostart"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completing_one_transcription_keeps_the_next_active() {
        let (tx, _jobs) = mpsc::channel();
        let (replies, rx) = mpsc::channel();
        let mut app = App {
            cfg: Config::default(),
            hwnd: null_mut(),
            recording: None,
            finishing: false,
            hold_recording: false,
            busy: 2,
            tx,
            rx,
            last: String::new(),
            mouse: null_mut(),
        };
        let focus = Focus {
            window: 0,
            control: 0,
            epoch: 0,
        };
        for remaining in [1, 0] {
            replies
                .send(Reply::Done(focus, Ok(String::new()), Duration::ZERO))
                .unwrap();
            app.results();
            assert_eq!(app.busy, remaining);
            assert_eq!(ACTIVE.load(Ordering::Relaxed), remaining > 0);
        }
    }
}
