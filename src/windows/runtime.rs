use super::{autostart, clipboard, tray, worker::Worker};
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
        Input::{Ime::ImmDisableIME, KeyboardAndMouse::*},
        Shell::{NIM_ADD, NIM_DELETE, NIM_MODIFY},
        WindowsAndMessaging::*,
    },
};

const COPY: u32 = WM_APP + 1;
const KEY: u32 = WM_APP + 2;
const RESULT: u32 = WM_APP + 3;
const PAUSE: u32 = WM_APP + 5;
const SETTINGS: u32 = WM_APP + 6;
pub(super) const SETTINGS_READY: u32 = WM_APP + 7;
pub(super) const SETTINGS_APPLY: u32 = WM_APP + 8;
pub(super) const UI_CLOSED: u32 = WM_APP + 9;
pub(super) const MENU_READY: u32 = WM_APP + 10;
pub(super) const MENU_COMMAND: u32 = WM_APP + 11;
pub(super) const OPEN_SETTINGS: u32 = WM_APP + 12;
const UI_CHECK: u32 = WM_APP + 13;
static WINDOW: AtomicUsize = AtomicUsize::new(0);
static TRIGGER: AtomicU32 = AtomicU32::new(119);
static HELD: AtomicBool = AtomicBool::new(false);
static TOGGLE: AtomicU32 = AtomicU32::new(120);
static REPLAY: AtomicU32 = AtomicU32::new(121);
static TOGGLE_HELD: AtomicBool = AtomicBool::new(false);
static REPLAY_HELD: AtomicBool = AtomicBool::new(false);
static REPLAY_PENDING: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static PAUSED: AtomicBool = AtomicBool::new(false);
static SETTINGS_OPEN: AtomicBool = AtomicBool::new(false);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static EPOCH: AtomicUsize = AtomicUsize::new(0);
thread_local! { static APP: RefCell<Option<App>> = const { RefCell::new(None) }; }
thread_local! { static UI_CHILD: RefCell<Option<UiProcess>> = const { RefCell::new(None) }; }

struct UiProcess {
    child: std::process::Child,
    window: HWND,
    settings: bool,
}

struct KeyboardHook {
    id: u32,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl KeyboardHook {
    fn start() -> Result<Self> {
        let (tx, rx) = mpsc::sync_channel(0);
        #[cfg(test)]
        let desktop = unsafe {
            windows_sys::Win32::System::StationsAndDesktops::GetThreadDesktop(GetCurrentThreadId())
                as usize
        };
        let thread = std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(move || unsafe {
                #[cfg(test)]
                windows_sys::Win32::System::StationsAndDesktops::SetThreadDesktop(desktop as _);
                let mut msg = std::mem::zeroed();
                PeekMessageW(&mut msg, null_mut(), 0, 0, PM_NOREMOVE);
                let hook = SetWindowsHookExW(
                    WH_KEYBOARD_LL,
                    Some(keyboard),
                    GetModuleHandleW(null_mut()),
                    0,
                );
                let sent = tx.send((GetCurrentThreadId(), !hook.is_null()));
                if !hook.is_null() {
                    if sent.is_ok() {
                        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                            DispatchMessageW(&msg);
                        }
                    }
                    UnhookWindowsHookEx(hook);
                }
            })?;
        let (id, ready) = rx.recv()?;
        if !ready {
            let _ = thread.join();
            bail!("Cannot register keyboard hook");
        }
        Ok(Self {
            id,
            thread: Some(thread),
        })
    }
}

impl Drop for KeyboardHook {
    fn drop(&mut self) {
        unsafe {
            PostThreadMessageW(self.id, WM_QUIT, 0, 0);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn ui_active() -> bool {
    UI_CHILD.with(|slot| {
        let mut state = slot.borrow_mut();
        let Some(settings) = state.as_mut() else {
            return false;
        };
        // Event-driven crash recovery, only on input/menu activity. No polling timer.
        if matches!(settings.child.try_wait(), Ok(None)) {
            true
        } else {
            *state = None;
            SETTINGS_OPEN.store(false, Ordering::Release);
            reset_held_keys();
            false
        }
    })
}

fn settings_active() -> bool {
    ui_active() && UI_CHILD.with(|state| state.borrow().as_ref().unwrap().settings)
}

fn ui_sender(pid: WPARAM) -> bool {
    UI_CHILD.with(|state| {
        state
            .borrow()
            .as_ref()
            .is_some_and(|settings| settings.child.id() as usize == pid)
    })
}

fn open_ui(settings: bool) -> Result<()> {
    use std::os::windows::process::CommandExt;
    if ui_active() {
        if !settings {
            return Ok(());
        }
        configuration()?;
        let (hwnd, was_settings) = UI_CHILD.with(|state| {
            let mut state = state.borrow_mut();
            let child = state.as_mut().unwrap();
            let previous = child.settings;
            child.settings = true;
            (child.window, previous)
        });
        SETTINGS_OPEN.store(true, Ordering::Release);
        if !was_settings {
            EPOCH.fetch_add(1, Ordering::Relaxed);
            reset_held_keys();
        }
        if !hwnd.is_null() {
            unsafe {
                if was_settings {
                    ShowWindow(hwnd, SW_RESTORE);
                    SetForegroundWindow(hwnd);
                } else {
                    PostMessageW(hwnd, OPEN_SETTINGS, 0, 0);
                }
            }
        }
        return Ok(());
    }
    let mut command = std::process::Command::new(std::env::current_exe()?);
    if settings {
        configuration()?;
        command.arg("__settings");
    } else {
        let busy = APP.with(|a| {
            a.borrow()
                .as_ref()
                .is_some_and(|app| app.recording.is_some() || app.busy > 0)
        });
        command.args([
            "__menu",
            if PAUSED.load(Ordering::Relaxed) {
                "1"
            } else {
                "0"
            },
            if busy { "1" } else { "0" },
        ]);
    }
    let child = command.creation_flags(CREATE_NO_WINDOW).spawn()?;
    UI_CHILD.with(|state| {
        *state.borrow_mut() = Some(UiProcess {
            child,
            window: null_mut(),
            settings,
        })
    });
    if settings {
        SETTINGS_OPEN.store(true, Ordering::Release);
        EPOCH.fetch_add(1, Ordering::Relaxed);
        reset_held_keys();
    }
    Ok(())
}

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
        let hwnd = FindWindowW(windows_sys::w!("VTDWindows"), null_mut());
        ensure!(!hwnd.is_null(), "VTD is not running");
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
        // This thread only owns an invisible message window, never a text field.
        // Avoid initializing Windows text-input services when the tray gets focus.
        // Settings have their own process; other applications' IMEs are unaffected.
        ImmDisableIME(0);
        let mutex = CreateMutexW(null_mut(), 0, windows_sys::w!("Local\\VTD-Windows"));
        ensure!(!mutex.is_null(), "Cannot create instance lock");
        if GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(mutex);
            bail!("VTD is already running");
        }
        let instance = GetModuleHandleW(null_mut());
        let class = windows_sys::w!("VTDWindows");
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
        TASKBAR_CREATED.store(
            RegisterWindowMessageW(windows_sys::w!("TaskbarCreated")),
            Ordering::Relaxed,
        );
        ensure!(
            tray::update(hwnd, "VTD", NIM_ADD),
            "Cannot create tray icon"
        );
        TRIGGER.store(cfg.trigger_key, Ordering::Relaxed);
        TOGGLE.store(cfg.toggle_key, Ordering::Relaxed);
        REPLAY.store(cfg.replay_key, Ordering::Relaxed);
        let hook = KeyboardHook::start()?;
        let (tx, jobs) = mpsc::channel::<Request>();
        let (replies, rx) = mpsc::channel();
        let idle_unload_seconds = cfg.idle_unload_seconds;
        let address = hwnd as usize;
        std::thread::spawn(move || {
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
        status(hwnd, "VTD: ready");
        let mut msg = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        drop(hook);
        tray::update(hwnd, "", NIM_DELETE);
        APP.with(|a| *a.borrow_mut() = None);
        CloseHandle(mutex);
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

unsafe fn replay_when_released(released: u32) {
    unsafe {
        if !modifiers_held(released) && REPLAY_PENDING.swap(false, Ordering::Relaxed) {
            EPOCH.fetch_add(1, Ordering::Relaxed);
            PostMessageW(WINDOW.load(Ordering::Relaxed) as HWND, KEY, 4, 0);
        }
    }
}

#[inline(never)]
unsafe fn shortcut_modifiers() -> u32 {
    let mut modifiers = 0;
    for (key, flag) in [
        (VK_SHIFT, 0x100),
        (VK_CONTROL, 0x200),
        (VK_MENU, 0x400),
        (VK_LWIN, 0x1000),
        (VK_RWIN, 0x1000),
    ] {
        if unsafe { GetAsyncKeyState(key as i32) } < 0 {
            modifiers |= flag;
        }
    }
    modifiers
}

fn shortcut_matches(shortcut: u32, key: u32, modifiers: u32, held: bool, up: bool) -> bool {
    shortcut & 0xff == key && (held || (!up && shortcut & 0x700 == modifiers))
}

unsafe extern "system" fn keyboard(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if code == HC_ACTION as i32 && !PAUSED.load(Ordering::Relaxed) {
            let event = &*(l as *const KBDLLHOOKSTRUCT);
            if event.flags & LLKHF_INJECTED != 0 {
                return CallNextHookEx(null_mut(), code, w, l);
            }
            if SETTINGS_OPEN.load(Ordering::Acquire) {
                if event.vkCode == (TRIGGER.load(Ordering::Relaxed) & 0xff)
                    || event.vkCode == (TOGGLE.load(Ordering::Relaxed) & 0xff)
                    || event.vkCode == (REPLAY.load(Ordering::Relaxed) & 0xff)
                {
                    PostMessageW(WINDOW.load(Ordering::Relaxed) as HWND, UI_CHECK, 0, 0);
                }
                return CallNextHookEx(null_mut(), code, w, l);
            }
            {
                let down = w as u32 == WM_KEYDOWN || w as u32 == WM_SYSKEYDOWN;
                let up = w as u32 == WM_KEYUP || w as u32 == WM_SYSKEYUP;
                if up && REPLAY_PENDING.load(Ordering::Relaxed) {
                    replay_when_released(event.vkCode);
                }
                let bindings = [
                    (REPLAY.load(Ordering::Relaxed), &REPLAY_HELD, 4),
                    (TOGGLE.load(Ordering::Relaxed), &TOGGLE_HELD, 3),
                    (TRIGGER.load(Ordering::Relaxed), &HELD, 1),
                ];
                let already_held = bindings.iter().any(|(key, held, _)| {
                    key & 0xff == event.vkCode && held.load(Ordering::Relaxed)
                });
                let modifiers = if already_held {
                    0x10000
                } else if down
                    && bindings
                        .iter()
                        .any(|(key, _, _)| key & 0xff == event.vkCode)
                {
                    shortcut_modifiers()
                } else {
                    0
                };
                for (key, held, action) in bindings {
                    if !shortcut_matches(
                        key,
                        event.vkCode,
                        modifiers,
                        held.load(Ordering::Relaxed),
                        up,
                    ) {
                        continue;
                    }
                    if down && !held.swap(true, Ordering::Relaxed) && action != 4 {
                        PostMessageW(WINDOW.load(Ordering::Relaxed) as HWND, KEY, action, 0);
                    }
                    if up && held.swap(false, Ordering::Relaxed) {
                        match action {
                            1 => {
                                PostMessageW(WINDOW.load(Ordering::Relaxed) as HWND, KEY, 0, 0);
                            }
                            4 => {
                                REPLAY_PENDING.store(true, Ordering::Relaxed);
                                replay_when_released(event.vkCode);
                            }
                            _ => {}
                        }
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
        if clipboard::message(hwnd, msg, w) {
            return 0;
        }
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
                if (l as u32 == WM_RBUTTONUP || l as u32 == WM_LBUTTONUP)
                    && let Err(e) = open_ui(false)
                {
                    status(hwnd, &format!("Cannot open menu: {e:#}"));
                }
                0
            }
            MENU_COMMAND if ui_sender(w) => {
                match l {
                    1 => APP.with(|a| {
                        if let Some(app) = a.borrow_mut().as_mut() {
                            app.pause(!PAUSED.load(Ordering::Relaxed));
                        }
                    }),
                    2 => {
                        PostMessageW(hwnd, WM_CLOSE, 0, 0);
                    }
                    _ => return 0,
                }
                1
            }
            SETTINGS => {
                if let Err(e) = open_ui(true) {
                    status(hwnd, &format!("Cannot open settings: {e:#}"));
                }
                0
            }
            UI_CHECK => {
                ui_active();
                0
            }
            SETTINGS_READY | MENU_READY if ui_sender(w) => {
                let dialog = l as HWND;
                if msg == SETTINGS_READY && dialog.is_null() {
                    if configuration().is_err() {
                        return 0;
                    }
                    UI_CHILD.with(|state| {
                        let mut state = state.borrow_mut();
                        let child = state.as_mut().unwrap();
                        child.settings = true;
                        child.window = null_mut();
                    });
                    SETTINGS_OPEN.store(true, Ordering::Release);
                    EPOCH.fetch_add(1, Ordering::Relaxed);
                    reset_held_keys();
                    return 1;
                }
                let mut pid = 0;
                GetWindowThreadProcessId(dialog, &mut pid);
                if pid as usize != w {
                    return 0;
                }
                UI_CHILD.with(|state| {
                    if let Some(settings) = state.borrow_mut().as_mut() {
                        settings.window = dialog;
                        if msg == MENU_READY && settings.settings {
                            PostMessageW(dialog, OPEN_SETTINGS, 0, 0);
                        }
                    }
                });
                1
            }
            SETTINGS_APPLY if ui_sender(w) => {
                let result = configuration().and_then(|_| Config::load(&vtd::config::path()?));
                match result {
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
            UI_CLOSED if ui_sender(w) => {
                UI_CHILD.with(|state| *state.borrow_mut() = None);
                SETTINGS_OPEN.store(false, Ordering::Release);
                reset_held_keys();
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
            WM_CLOSE => {
                if let Err(e) = clipboard::restore(hwnd) {
                    status(hwnd, &e.to_string());
                    return 0;
                }
                let dialog =
                    UI_CHILD.with(|state| state.borrow().as_ref().map(|settings| settings.window));
                if let Some(dialog) = dialog
                    && !dialog.is_null()
                {
                    PostMessageW(dialog, WM_CLOSE, 0, 0);
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
        TRIGGER.store(cfg.trigger_key, Ordering::Relaxed);
        TOGGLE.store(cfg.toggle_key, Ordering::Relaxed);
        REPLAY.store(cfg.replay_key, Ordering::Relaxed);
        reset_held_keys();
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
        reset_held_keys();
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
        if PAUSED.load(Ordering::Relaxed) || settings_active() {
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
                    let _ = self.tx.send(Request::Warm);
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
                Reply::Unloaded => {
                    if self.recording.is_none() && self.busy == 0 {
                        status(self.hwnd, "VTD: model unloaded");
                    }
                }
                Reply::Done(focus, result, elapsed) => {
                    self.busy -= 1;
                    if self.busy == 0 && !self.finishing {
                        self.release_target();
                    }
                    ACTIVE.store(self.recording.is_some() || self.busy > 0, Ordering::Relaxed);
                    match result {
                        Ok(text) if text.is_empty() => status(self.hwnd, "VTD: no speech"),
                        Ok(text) => {
                            self.last = text;
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
        target.window != 0 && target == Focus::current(),
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
            let sent = SendInput(4, keys.as_ptr(), std::mem::size_of::<INPUT>() as i32);
            if sent != 4 {
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
    tray::update(hwnd, text, NIM_MODIFY);
    eprintln!("{text}");
}

pub fn autostart(mode: Option<&str>) -> Result<()> {
    match mode {
        Some("on") => autostart::set_enabled(true),
        Some("off") => autostart::set_enabled(false),
        _ => bail!("Use: vtd autostart on|off"),
    }
}

fn reset_held_keys() {
    HELD.store(false, Ordering::Relaxed);
    TOGGLE_HELD.store(false, Ordering::Relaxed);
    REPLAY_HELD.store(false, Ordering::Relaxed);
    REPLAY_PENDING.store(false, Ordering::Relaxed);
}

fn configuration() -> Result<()> {
    APP.with(|app| {
        let app = app.borrow();
        let app = app
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("VTD is not running"))?;
        ensure!(
            app.recording.is_none() && app.busy == 0,
            "Settings are available after recording and transcription finish."
        );
        Ok(())
    })
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
    fn exited_settings_process_does_not_leave_shortcuts_paused() {
        use std::{
            os::windows::process::CommandExt,
            process::{Command, Stdio},
        };
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--list")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .unwrap();
        child.wait().unwrap();
        SETTINGS_OPEN.store(true, Ordering::Release);
        UI_CHILD.with(|state| {
            *state.borrow_mut() = Some(UiProcess {
                child,
                window: null_mut(),
                settings: true,
            })
        });
        assert!(!settings_active());
        assert!(!SETTINGS_OPEN.load(Ordering::Acquire));
        assert!(UI_CHILD.with(|state| state.borrow().is_none()));
    }

    #[test]
    #[ignore = "run alone: uses an isolated Windows desktop"]
    fn keyboard_pump_survives_blocked_tray_thread() {
        use windows_sys::Win32::System::StationsAndDesktops::*;
        static DELIVERED: AtomicUsize = AtomicUsize::new(0);
        unsafe extern "system" fn deliver(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
            let msg = unsafe { &*(l as *const MSG) };
            if code != HC_ACTION as i32 || w != PM_REMOVE as usize || msg.message != WM_APP + 42 {
                return unsafe { CallNextHookEx(null_mut(), code, w, l) };
            }
            let (vk, message) = [
                (119, WM_KEYDOWN),
                (119, WM_KEYUP),
                (120, WM_KEYDOWN),
                (120, WM_KEYDOWN),
                (120, WM_KEYUP),
                (121, WM_KEYDOWN),
                (121, WM_KEYUP),
                (VK_ESCAPE as u32, WM_KEYDOWN),
            ][msg.wParam];
            let event = KBDLLHOOKSTRUCT {
                vkCode: vk,
                ..unsafe { std::mem::zeroed() }
            };
            unsafe {
                keyboard(
                    HC_ACTION as i32,
                    message as usize,
                    &event as *const _ as isize,
                );
            }
            DELIVERED.fetch_add(1, Ordering::Release);
            unsafe { CallNextHookEx(null_mut(), code, w, l) }
        }
        unsafe {
            let station = CreateWindowStationW(null_mut(), 0, 0x000f037f, null_mut());
            assert!(!station.is_null());
            assert_ne!(SetProcessWindowStation(station), 0);
            let desktop = CreateDesktopW(
                windows_sys::w!("VtdKeyboardTest"),
                null_mut(),
                null_mut(),
                0,
                0x000f01ff,
                null_mut(),
            );
            assert!(!desktop.is_null());
            assert_ne!(SetThreadDesktop(desktop), 0);
            let hwnd = CreateWindowExW(
                0,
                windows_sys::w!("STATIC"),
                windows_sys::w!("VtdKeyboardTest"),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                null_mut(),
                null_mut(),
                null_mut(),
            );
            assert!(!hwnd.is_null());
            WINDOW.store(hwnd as usize, Ordering::Relaxed);
            TRIGGER.store(119, Ordering::Relaxed);
            TOGGLE.store(120, Ordering::Relaxed);
            REPLAY.store(121, Ordering::Relaxed);
            ACTIVE.store(true, Ordering::Relaxed);
            let hook = KeyboardHook::start().unwrap();
            let monitor = SetWindowsHookExW(WH_GETMESSAGE, Some(deliver), null_mut(), hook.id);
            assert!(!monitor.is_null());
            let mut msg = std::mem::zeroed();
            for blocked in [None, Some(&SETTINGS_OPEN), Some(&PAUSED), None] {
                PAUSED.store(false, Ordering::Relaxed);
                SETTINGS_OPEN.store(false, Ordering::Release);
                if let Some(flag) = blocked {
                    flag.store(true, Ordering::Relaxed);
                }
                reset_held_keys();
                DELIVERED.store(0, Ordering::Relaxed);
                for id in 0..8 {
                    assert_ne!(PostThreadMessageW(hook.id, WM_APP + 42, id, 0), 0);
                }
                std::thread::sleep(Duration::from_millis(1200));
                assert_eq!(DELIVERED.load(Ordering::Acquire), 8);
                let mut actions = Vec::new();
                while PeekMessageW(&mut msg, hwnd, KEY, KEY, PM_REMOVE) != 0 {
                    actions.push(msg.wParam);
                }
                assert_eq!(
                    actions,
                    if blocked.is_none() {
                        vec![1, 0, 3, 4, 2]
                    } else {
                        vec![]
                    }
                );
            }
            UnhookWindowsHookEx(monitor);
            drop(hook);
            DestroyWindow(hwnd);
        }
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
        assert_eq!(REPLAY.load(Ordering::Relaxed), 122);
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
