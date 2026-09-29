use super::{audio::Recording, config::Config, engine::Engine};
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
use windows_sys::Win32::{
    Foundation::*,
    System::{DataExchange::*, LibraryLoader::*, Memory::*, Threading::*},
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

const COPY: u32 = WM_APP + 1;
const KEY: u32 = WM_APP + 2;
const RESULT: u32 = WM_APP + 3;
static WINDOW: AtomicUsize = AtomicUsize::new(0);
static TRIGGER: AtomicU32 = AtomicU32::new(119);
static HELD: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicBool = AtomicBool::new(false);
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
    Ready,
    Unloaded,
    Done(Focus, Result<String>, Duration),
}
struct App {
    cfg: Config,
    hwnd: HWND,
    recording: Option<(Recording, Focus)>,
    finishing: bool,
    busy: bool,
    tx: mpsc::Sender<Job>,
    rx: mpsc::Receiver<Reply>,
    last: String,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn control(command: &str) -> Result<()> {
    unsafe {
        let hwnd = FindWindowW(wide("VTDWindows").as_ptr(), null_mut());
        ensure!(!hwnd.is_null(), "VTD is not running");
        if command == "stop" {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
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

pub fn run(cfg: Config) -> Result<()> {
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
        TRIGGER.store(cfg.trigger_key, Ordering::Relaxed);
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), instance, 0);
        ensure!(!hook.is_null(), "Cannot register keyboard hook");
        let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), instance, 0);
        ensure!(!mouse.is_null(), "Cannot register mouse hook");
        let (tx, jobs) = mpsc::channel::<Job>();
        let (replies, rx) = mpsc::channel();
        let worker_cfg = cfg.clone();
        let address = hwnd as usize;
        std::thread::spawn(move || {
            let send = |r| {
                let _ = replies.send(r);
                PostMessageW(address as HWND, RESULT, 0, 0);
            };
            let mut engine = match Engine::load(&worker_cfg) {
                Ok(e) => {
                    send(Reply::Ready);
                    Some(e)
                }
                Err(e) => {
                    send(Reply::Done(
                        Focus {
                            window: 0,
                            control: 0,
                            epoch: 0,
                        },
                        Err(e),
                        Duration::ZERO,
                    ));
                    None
                }
            };
            loop {
                let job = if engine.is_some() && worker_cfg.idle_unload_seconds > 0 {
                    match jobs.recv_timeout(Duration::from_secs(worker_cfg.idle_unload_seconds)) {
                        Ok(j) => j,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
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
                let start = Instant::now();
                let result = (|| {
                    if engine.is_none() {
                        engine = Some(Engine::load(&worker_cfg)?);
                    }
                    let samples = super::audio::resample(&job.samples, job.rate);
                    engine.as_mut().unwrap().transcribe(&worker_cfg, &samples)
                })();
                send(Reply::Done(job.focus, result, start.elapsed()));
            }
        });
        APP.with(|a| {
            *a.borrow_mut() = Some(App {
                cfg,
                hwnd,
                recording: None,
                finishing: false,
                busy: true,
                tx,
                rx,
                last: String::new(),
            })
        });
        status(hwnd, "VTD: načítání modelu");
        let mut msg = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        UnhookWindowsHookEx(hook);
        UnhookWindowsHookEx(mouse);
        APP.with(|a| *a.borrow_mut() = None);
        CloseHandle(mutex);
    }
    Ok(())
}

unsafe extern "system" fn keyboard(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if code == HC_ACTION as i32 {
            let event = &*(l as *const KBDLLHOOKSTRUCT);
            {
                let down = w as u32 == WM_KEYDOWN || w as u32 == WM_SYSKEYDOWN;
                let up = w as u32 == WM_KEYUP || w as u32 == WM_SYSKEYUP;
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
        match msg {
            KEY | RESULT | WM_TIMER => {
                APP.with(|a| {
                    if let Some(app) = a.borrow_mut().as_mut() {
                        if msg == RESULT {
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
    fn key(&mut self, action: usize) {
        if action == 2 {
            self.stop(true);
            return;
        }
        if self.busy {
            return;
        }
        if action == 0 {
            if !self.cfg.toggle {
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
            } else if self.cfg.toggle {
                self.finish();
            }
            return;
        }
        let focus = Focus::current();
        match Recording::start(&self.cfg) {
            Ok(recording) => {
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
        }
        let Some((recording, focus)) = self.recording.take() else {
            return;
        };
        unsafe {
            KillTimer(self.hwnd, 1);
            KillTimer(self.hwnd, 2);
        }
        self.finishing = false;
        if cancel {
            ACTIVE.store(false, Ordering::Relaxed);
            drop(recording);
            status(self.hwnd, "VTD: připraveno");
            return;
        }
        match recording.finish() {
            Ok((samples, rate)) if samples.len() >= rate as usize * 3 / 10 => {
                self.busy = true;
                status(self.hwnd, "VTD: přepisuji");
                if self
                    .tx
                    .send(Job {
                        samples,
                        rate,
                        focus,
                    })
                    .is_err()
                {
                    self.busy = false;
                    ACTIVE.store(false, Ordering::Relaxed);
                    status(self.hwnd, "Přepisovací vlákno skončilo. Restartujte VTD.");
                }
            }
            Ok(_) => {
                ACTIVE.store(false, Ordering::Relaxed);
                status(self.hwnd, "VTD: příliš krátký záznam");
            }
            Err(e) => {
                ACTIVE.store(false, Ordering::Relaxed);
                status(self.hwnd, &e.to_string());
            }
        }
    }

    fn results(&mut self) {
        while let Ok(reply) = self.rx.try_recv() {
            match reply {
                Reply::Ready => {
                    self.busy = false;
                    status(self.hwnd, "VTD: připraveno");
                }
                Reply::Unloaded => {
                    if self.recording.is_none() {
                        status(self.hwnd, "VTD: model uspán");
                    }
                }
                Reply::Done(focus, result, elapsed) => {
                    self.busy = false;
                    ACTIVE.store(false, Ordering::Relaxed);
                    match result {
                        Ok(text) if text.is_empty() => status(self.hwnd, "VTD: bez řeči"),
                        Ok(text) => {
                            self.last = text;
                            if let Err(e) = insert(&self.last, focus) {
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
                }
            }
        }
    }
}

fn insert(text: &str, target: Focus) -> Result<()> {
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
        let mut input = Vec::with_capacity(text.len() * 2);
        for unit in text.encode_utf16() {
            for flags in [KEYEVENTF_UNICODE, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP] {
                input.push(INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: 0,
                            wScan: unit,
                            dwFlags: flags,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                });
            }
        }
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
    unsafe {
        SetWindowTextW(hwnd, wide(text).as_ptr());
    }
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
