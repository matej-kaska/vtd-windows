//! The sleeping tray owns no audio, model, JSON, Rust runtime, or UI framework.
//! Its keyboard pump is separate from process creation and shell/UI messages.

use core::{
    cell::UnsafeCell,
    ffi::c_void,
    ptr::{null, null_mut},
    sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicUsize, Ordering},
};
use windows_sys::Win32::{
    Foundation::*,
    System::{
        Console::*,
        DataExchange::COPYDATASTRUCT,
        Environment::GetCommandLineW,
        JobObjects::*,
        LibraryLoader::*,
        Memory::*,
        SystemServices::{
            HEAP_OPTIMIZE_RESOURCES_CURRENT_VERSION, HEAP_OPTIMIZE_RESOURCES_INFORMATION,
        },
        Threading::*,
    },
    UI::{
        Input::{Ime::ImmDisableIME, KeyboardAndMouse::*},
        Shell::*,
        WindowsAndMessaging::*,
    },
};

#[path = "protocol.rs"]
mod protocol;
#[path = "transcript.rs"]
mod transcript;
use protocol::*;

const TRAY_EVENT: u32 = WM_APP + 4;
const QUEUE_SIZE: usize = 64;
static WINDOW: AtomicUsize = AtomicUsize::new(0);
static HOOK_THREAD: AtomicU32 = AtomicU32::new(0);
static TRIGGER: AtomicU32 = AtomicU32::new(119);
static TOGGLE: AtomicU32 = AtomicU32::new(120);
static REPLAY: AtomicU32 = AtomicU32::new(121);
static HELD: AtomicBool = AtomicBool::new(false);
static TOGGLE_HELD: AtomicBool = AtomicBool::new(false);
static REPLAY_HELD: AtomicBool = AtomicBool::new(false);
static REPLAY_PENDING: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static PAUSED: AtomicBool = AtomicBool::new(false);
static SETTINGS_OPEN: AtomicBool = AtomicBool::new(false);
static CLOSING: AtomicBool = AtomicBool::new(false);
static CONFIG_READY: AtomicBool = AtomicBool::new(false);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static EPOCH: AtomicUsize = AtomicUsize::new(0);
static QUEUE_READ: AtomicUsize = AtomicUsize::new(0);
static QUEUE_WRITE: AtomicUsize = AtomicUsize::new(0);
static KEYS_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static QUEUE: [AtomicUsize; QUEUE_SIZE] = [const { AtomicUsize::new(0) }; QUEUE_SIZE];
struct TranscriptSlot(UnsafeCell<transcript::Transcript>);
// Accessed only by the owning window thread. send_last() blocks incoming
// sent messages while another process reads the transcript, preventing a
// reentrant replacement from freeing or overwriting the in-flight buffer.
unsafe impl Sync for TranscriptSlot {}
static LAST: TranscriptSlot = TranscriptSlot(UnsafeCell::new(transcript::Transcript::new()));
static STATUS_TEXT: [AtomicU16; 128] = [const { AtomicU16::new(0) }; 128];
static CAPTURE_ARGUMENTS: AtomicUsize = AtomicUsize::new(0);
static CHILD_JOB: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "resident-test")]
static TEST_MODIFIERS: AtomicU32 = AtomicU32::new(0);
#[cfg(feature = "resident-test")]
static TEST_DISPATCH: AtomicBool = AtomicBool::new(false);

struct Child {
    process: AtomicUsize,
    pid: AtomicU32,
    window: AtomicUsize,
}
impl Child {
    const fn new() -> Self {
        Self {
            process: AtomicUsize::new(0),
            pid: AtomicU32::new(0),
            window: AtomicUsize::new(0),
        }
    }
    fn sender(&self, pid: usize) -> bool {
        pid != 0 && pid == self.pid.load(Ordering::Relaxed) as usize
    }
    fn close(&self) {
        self.window.store(0, Ordering::Relaxed);
        self.pid.store(0, Ordering::Relaxed);
        let process = self.process.swap(0, Ordering::Relaxed) as HANDLE;
        if !process.is_null() {
            unsafe {
                CloseHandle(process);
            }
        }
    }
    fn alive(&self) -> bool {
        let process = self.process.load(Ordering::Relaxed) as HANDLE;
        !process.is_null() && unsafe { WaitForSingleObject(process, 0) == WAIT_TIMEOUT }
    }
    fn hwnd(&self) -> HWND {
        self.window.load(Ordering::Relaxed) as HWND
    }
}
static SESSION: Child = Child::new();
static UI: Child = Child::new();
static BOOTSTRAP: Child = Child::new();

struct OwnedIcon(HICON);

impl Drop for OwnedIcon {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { DestroyIcon(self.0) };
        }
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    unsafe { ExitProcess(1) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn memset(destination: *mut c_void, value: i32, count: usize) -> *mut c_void {
    for index in 0..count {
        unsafe {
            destination
                .cast::<u8>()
                .add(index)
                .write_volatile(value as u8);
        }
    }
    destination
}
#[unsafe(no_mangle)]
unsafe extern "C" fn memcpy(
    destination: *mut c_void,
    source: *const c_void,
    count: usize,
) -> *mut c_void {
    for index in 0..count {
        unsafe {
            destination
                .cast::<u8>()
                .add(index)
                .write_volatile(source.cast::<u8>().add(index).read_volatile());
        }
    }
    destination
}
#[unsafe(no_mangle)]
unsafe extern "C" fn memmove(
    destination: *mut c_void,
    source: *const c_void,
    count: usize,
) -> *mut c_void {
    if destination as usize <= source as usize {
        unsafe {
            memcpy(destination, source, count);
        }
    } else {
        for index in (0..count).rev() {
            unsafe {
                destination
                    .cast::<u8>()
                    .add(index)
                    .write_volatile(source.cast::<u8>().add(index).read_volatile());
            }
        }
    }
    destination
}

fn reset_held() {
    HELD.store(false, Ordering::Relaxed);
    TOGGLE_HELD.store(false, Ordering::Relaxed);
    REPLAY_HELD.store(false, Ordering::Relaxed);
    REPLAY_PENDING.store(false, Ordering::Relaxed);
}

fn memory_priority(idle: bool) {
    let information = MEMORY_PRIORITY_INFORMATION {
        MemoryPriority: if idle {
            MEMORY_PRIORITY_VERY_LOW
        } else {
            MEMORY_PRIORITY_NORMAL
        },
    };
    unsafe {
        SetProcessInformation(
            GetCurrentProcess(),
            ProcessMemoryPriority,
            (&information as *const MEMORY_PRIORITY_INFORMATION).cast(),
            core::mem::size_of_val(&information) as u32,
        );
    }
}

fn release_idle_pages() {
    // Once per transition to idle, never per keystroke or on a periodic timer.
    let optimization = HEAP_OPTIMIZE_RESOURCES_INFORMATION {
        Version: HEAP_OPTIMIZE_RESOURCES_CURRENT_VERSION,
        Flags: 0,
    };
    memory_priority(true);
    unsafe {
        // Return unused heap caches to the OS before evicting resident pages.
        // NULL optimizes only heaps owned by this process. Windows can retain
        // live allocations, and failure is harmless on older implementations.
        HeapSetInformation(
            null_mut(),
            HeapOptimizeResources,
            (&optimization as *const HEAP_OPTIMIZE_RESOURCES_INFORMATION).cast(),
            core::mem::size_of_val(&optimization),
        );
        SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

unsafe fn disable_hidden_themes() -> HMODULE {
    unsafe {
        let library = LoadLibraryExW(
            windows_sys::w!("uxtheme.dll"),
            null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        );
        if !library.is_null()
            && let Some(address) = GetProcAddress(library, c"SetThemeAppProperties".as_ptr().cast())
        {
            let function: unsafe extern "system" fn(u32) = core::mem::transmute(address);
            function(0);
        }
        library
    }
}

fn tray(operation: u32) -> bool {
    unsafe {
        let mut icon = OwnedIcon(null_mut());
        let mut data: NOTIFYICONDATAW = core::mem::zeroed();
        data.cbSize = core::mem::size_of_val(&data) as u32;
        data.hWnd = WINDOW.load(Ordering::Relaxed) as HWND;
        data.uID = 1;
        data.uFlags = NIF_TIP;
        for (target, unit) in data.szTip.iter_mut().zip(&STATUS_TEXT) {
            *target = unit.load(Ordering::Relaxed);
        }
        if operation == NIM_ADD {
            data.uFlags |= NIF_ICON | NIF_MESSAGE;
            data.uCallbackMessage = TRAY_EVENT;
            icon.0 = LoadImageW(
                GetModuleHandleW(null()),
                core::ptr::without_provenance(1),
                IMAGE_ICON,
                GetSystemMetrics(SM_CXSMICON),
                GetSystemMetrics(SM_CYSMICON),
                0,
            ) as HICON;
            data.hIcon = icon.0;
            if data.hIcon.is_null() {
                return false;
            }
        }
        // Explorer copies the icon during registration. Keep the exact same
        // resource/pixels, then release our temporary USER/GDI resources.
        // No permanent import/reference to the shell implementation either.
        let library = LoadLibraryExW(
            windows_sys::w!("shell32.dll"),
            null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        );
        if library.is_null() {
            return false;
        }
        let result =
            if let Some(address) = GetProcAddress(library, c"Shell_NotifyIconW".as_ptr().cast()) {
                let function: unsafe extern "system" fn(u32, *const NOTIFYICONDATAW) -> i32 =
                    core::mem::transmute(address);
                function(operation, &data) != 0
            } else {
                false
            };
        FreeLibrary(library);
        result
    }
}

unsafe fn set_status(text: *const u16) {
    unsafe {
        let text = if PAUSED.load(Ordering::Relaxed) {
            windows_sys::w!("VTD: paused")
        } else {
            text
        };
        let mut copy = [0u16; 128];
        for (index, target) in copy.iter_mut().take(127).enumerate() {
            let unit = *text.add(index);
            *target = unit;
            if unit == 0 {
                break;
            }
        }
        if STATUS_TEXT
            .iter()
            .zip(copy)
            .all(|(previous, unit)| previous.load(Ordering::Relaxed) == unit)
        {
            return;
        }
        for (target, unit) in STATUS_TEXT.iter().zip(copy) {
            target.store(unit, Ordering::Relaxed);
        }
        SetWindowTextW(WINDOW.load(Ordering::Relaxed) as HWND, copy.as_ptr());
        tray(NIM_MODIFY);
    }
}

unsafe fn skip_word(mut text: *const u16) -> *const u16 {
    unsafe {
        let mut quoted = false;
        let mut slashes = 0;
        while *text != 0 {
            let unit = *text;
            if unit == b'"' as u16 && slashes % 2 == 0 {
                quoted = !quoted;
            }
            if !quoted && matches!(unit, 9 | 32) {
                break;
            }
            slashes = if unit == b'\\' as u16 { slashes + 1 } else { 0 };
            text = text.add(1);
        }
        while *text == b' ' as u16 || *text == b'\t' as u16 {
            text = text.add(1);
        }
        text
    }
}

unsafe fn run_arguments(arguments: *const u16) -> Option<*const u16> {
    unsafe {
        if *arguments == 0 {
            return Some(arguments);
        }
        let tail = word_matches(arguments, b"run")?;
        if *tail == 0 {
            return Some(tail);
        }
        let path = word_matches(tail, b"--capture-next")?;
        (*path != 0 && *skip_word(path) == 0).then_some(tail)
    }
}

unsafe fn word_matches(mut text: *const u16, expected: &[u8]) -> Option<*const u16> {
    unsafe {
        let quoted = *text == b'"' as u16;
        if quoted {
            text = text.add(1);
        }
        for &unit in expected {
            if *text != unit as u16 {
                return None;
            }
            text = text.add(1);
        }
        if quoted {
            if *text != b'"' as u16 {
                return None;
            }
            text = text.add(1);
        }
        if !matches!(*text, 0 | 9 | 32) {
            return None;
        }
        while matches!(*text, 9 | 32) {
            text = text.add(1);
        }
        Some(text)
    }
}

unsafe fn launch(mode: &[u8], tail: *const u16, child: &Child, inherit: bool) -> bool {
    unsafe {
        let mut path = [0u16; 1024];
        let length = GetModuleFileNameW(null_mut(), path.as_mut_ptr(), path.len() as u32) as usize;
        if length == 0 || length >= path.len() {
            return false;
        }
        let Some(index) = path[..length]
            .iter()
            .rposition(|unit| *unit == b'\\' as u16)
        else {
            return false;
        };
        let name = b"vtd-helper.exe\0";
        if index + 1 + name.len() > path.len() {
            return false;
        }
        for (target, unit) in path[index + 1..].iter_mut().zip(name) {
            *target = *unit as u16;
        }
        let mut tail_length = 0;
        if !tail.is_null() {
            while tail_length < 32767 && *tail.add(tail_length) != 0 {
                tail_length += 1;
            }
        }
        let prefix = b"vtd-helper ";
        let count = prefix.len()
            + mode.len()
            + usize::from(!mode.is_empty() && tail_length > 0)
            + tail_length
            + 1;
        if count > 32767 {
            return false;
        }
        // Common internal commands fit on the stack and never activate a
        // low-fragmentation heap bucket. Long CLI/capture arguments keep the
        // full Windows command-line limit and use a temporary allocation.
        let mut short_command = [0u16; 128];
        let heap = GetProcessHeap();
        let allocated = count > short_command.len();
        let command = if allocated {
            HeapAlloc(heap, HEAP_ZERO_MEMORY, count * 2).cast::<u16>()
        } else {
            short_command.as_mut_ptr()
        };
        if command.is_null() {
            return false;
        }
        let mut position = 0;
        for byte in prefix.iter().chain(mode) {
            *command.add(position) = *byte as u16;
            position += 1;
        }
        if !mode.is_empty() && tail_length > 0 {
            *command.add(position) = b' ' as u16;
            position += 1;
        }
        if tail_length > 0 {
            core::ptr::copy_nonoverlapping(tail, command.add(position), tail_length);
        }
        let mut startup: STARTUPINFOW = core::mem::zeroed();
        startup.cb = core::mem::size_of_val(&startup) as u32;
        if inherit {
            let output = GetStdHandle(STD_OUTPUT_HANDLE);
            if !output.is_null() && output != INVALID_HANDLE_VALUE {
                startup.dwFlags = STARTF_USESTDHANDLES;
                startup.hStdInput = GetStdHandle(STD_INPUT_HANDLE);
                startup.hStdOutput = output;
                startup.hStdError = GetStdHandle(STD_ERROR_HANDLE);
            }
        }
        let job = CHILD_JOB.load(Ordering::Relaxed) as HANDLE;
        let mut process: PROCESS_INFORMATION = core::mem::zeroed();
        let result = CreateProcessW(
            path.as_ptr(),
            command,
            null(),
            null(),
            i32::from(inherit),
            CREATE_NO_WINDOW | if job.is_null() { 0 } else { CREATE_SUSPENDED },
            null(),
            null(),
            &startup,
            &mut process,
        );
        if allocated {
            HeapFree(heap, 0, command.cast());
        }
        if result == 0 {
            return false;
        }
        if !job.is_null() {
            if AssignProcessToJobObject(job, process.hProcess) == 0 {
                TerminateProcess(process.hProcess, 1);
                WaitForSingleObject(process.hProcess, 5000);
                CloseHandle(process.hThread);
                CloseHandle(process.hProcess);
                return false;
            }
            if ResumeThread(process.hThread) == u32::MAX {
                TerminateProcess(process.hProcess, 1);
                WaitForSingleObject(process.hProcess, 5000);
                CloseHandle(process.hThread);
                CloseHandle(process.hProcess);
                return false;
            }
        }
        child.close();
        CloseHandle(process.hThread);
        child
            .process
            .store(process.hProcess as usize, Ordering::Relaxed);
        child.pid.store(process.dwProcessId, Ordering::Relaxed);
        true
    }
}

fn recover_children() {
    if SESSION.process.load(Ordering::Relaxed) != 0 && !SESSION.alive() {
        SESSION.close();
        KEYS_IN_FLIGHT.store(0, Ordering::Relaxed);
        ACTIVE.store(false, Ordering::Relaxed);
        QUEUE_READ.store(QUEUE_WRITE.load(Ordering::Relaxed), Ordering::Relaxed);
        // The hook may already have seen the next down event. Its matching up
        // must survive recovery of the previous crashed recording process.
    }
    if UI.process.load(Ordering::Relaxed) != 0 && !UI.alive() {
        UI.close();
        if SETTINGS_OPEN.load(Ordering::Acquire) {
            reset_held();
        }
        SETTINGS_OPEN.store(false, Ordering::Relaxed);
    }
}

fn queue_key(action: usize) {
    let write = QUEUE_WRITE.load(Ordering::Relaxed);
    if write.wrapping_sub(QUEUE_READ.load(Ordering::Relaxed)) >= QUEUE_SIZE {
        unsafe {
            set_status(windows_sys::w!("VTD: too many pending shortcuts"));
        }
        return;
    }
    QUEUE[write % QUEUE_SIZE].store(action, Ordering::Relaxed);
    QUEUE_WRITE.store(write.wrapping_add(1), Ordering::Relaxed);
}

fn key(action: usize) {
    recover_children();
    if PAUSED.load(Ordering::Relaxed)
        || SETTINGS_OPEN.load(Ordering::Acquire)
        || CLOSING.load(Ordering::Relaxed)
    {
        return;
    }
    if !SESSION.hwnd().is_null() {
        unsafe {
            if PostMessageW(SESSION.hwnd(), KEY, action, 0) != 0 {
                KEYS_IN_FLIGHT.fetch_add(1, Ordering::Relaxed);
            }
        }
        return;
    }
    if !SESSION.alive() {
        // A release/cancel by itself must not launch a new microphone session.
        if action == 0 || action == 2 {
            return;
        }
        memory_priority(false);
        if !unsafe {
            launch(
                b"__session",
                CAPTURE_ARGUMENTS.load(Ordering::Relaxed) as _,
                &SESSION,
                true,
            )
        } {
            unsafe {
                set_status(windows_sys::w!("VTD: cannot start vtd-helper.exe"));
            }
            release_idle_pages();
            return;
        }
    }
    if action == 1 || action == 3 {
        ACTIVE.store(true, Ordering::Relaxed);
    }
    queue_key(action);
}

fn settings_allowed() -> bool {
    !ACTIVE.load(Ordering::Relaxed)
        && KEYS_IN_FLIGHT.load(Ordering::Relaxed) == 0
        && QUEUE_READ.load(Ordering::Relaxed) == QUEUE_WRITE.load(Ordering::Relaxed)
        && !CLOSING.load(Ordering::Relaxed)
}

fn open_ui(settings: bool) {
    recover_children();
    if settings && !settings_allowed() {
        unsafe {
            set_status(windows_sys::w!(
                "Settings are available after recording and transcription finish."
            ));
        }
        return;
    }
    if UI.alive() {
        if settings {
            let was_settings = SETTINGS_OPEN.swap(true, Ordering::AcqRel);
            if !was_settings {
                EPOCH.fetch_add(1, Ordering::Relaxed);
                reset_held();
            }
            let hwnd = UI.hwnd();
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
        }
        return;
    }
    let mode: &[u8] = if settings {
        b"__settings"
    } else if PAUSED.load(Ordering::Relaxed) {
        b"__menu 1 0"
    } else if ACTIVE.load(Ordering::Relaxed) {
        b"__menu 0 1"
    } else {
        b"__menu 0 0"
    };
    if unsafe { launch(mode, null(), &UI, false) } {
        if settings {
            SETTINGS_OPEN.store(true, Ordering::Release);
            EPOCH.fetch_add(1, Ordering::Relaxed);
            reset_held();
        }
    } else {
        unsafe {
            set_status(windows_sys::w!("VTD: cannot open settings"));
        }
    }
}

fn pause(paused: bool) {
    if paused {
        PAUSED.store(true, Ordering::Release);
    }
    EPOCH.fetch_add(1, Ordering::Relaxed);
    reset_held();
    if !paused {
        PAUSED.store(false, Ordering::Release);
    }
    if SESSION.alive() && !SESSION.hwnd().is_null() {
        unsafe {
            PostMessageW(SESSION.hwnd(), PAUSE, usize::from(paused), 0);
        }
    } else {
        ACTIVE.store(false, Ordering::Relaxed);
        unsafe {
            set_status(windows_sys::w!("VTD: ready"));
        }
        release_idle_pages();
    }
}

unsafe fn send_last(hwnd: HWND) -> bool {
    unsafe {
        let bytes = (&*LAST.0.get()).bytes();
        if bytes.is_empty() {
            return false;
        }
        let packet = COPYDATASTRUCT {
            dwData: TRANSCRIPT,
            cbData: bytes.len() as u32,
            lpData: bytes.as_ptr().cast_mut().cast(),
        };
        let mut accepted = 0;
        SendMessageTimeoutW(
            hwnd,
            WM_COPYDATA,
            GetCurrentProcessId() as usize,
            &packet as *const _ as isize,
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            3000,
            &mut accepted,
        ) != 0
            && accepted == 1
    }
}

unsafe fn packet(pid: usize, l: LPARAM) -> LRESULT {
    unsafe {
        if l == 0 {
            return 0;
        }
        let data = &*(l as *const COPYDATASTRUCT);
        if data.lpData.is_null() {
            return 0;
        }
        match data.dwData {
            PREFERENCES
                if (BOOTSTRAP.sender(pid) || (UI.sender(pid) && settings_allowed()))
                    && data.cbData as usize == core::mem::size_of::<Preferences>() =>
            {
                let preferences = core::ptr::read_unaligned(data.lpData.cast::<Preferences>());
                TRIGGER.store(preferences.trigger, Ordering::Relaxed);
                TOGGLE.store(preferences.toggle, Ordering::Relaxed);
                REPLAY.store(preferences.replay, Ordering::Relaxed);
                EPOCH.fetch_add(1, Ordering::Relaxed);
                reset_held();
                CONFIG_READY.store(true, Ordering::Release);
                1
            }
            STATUS
                if SESSION.sender(pid)
                    && data.cbData as usize == core::mem::size_of::<Status>() =>
            {
                let mut status = core::ptr::read_unaligned(data.lpData.cast::<Status>());
                status.text[127] = 0;
                ACTIVE.store(status.active != 0, Ordering::Relaxed);
                // The resident owns pause state; a delayed session status cannot resume it.
                set_status(status.text.as_ptr());
                1
            }
            TRANSCRIPT
                if SESSION.sender(pid) && data.cbData > 0 && data.cbData <= 32 * 1024 * 1024 =>
            {
                let bytes =
                    core::slice::from_raw_parts(data.lpData.cast::<u8>(), data.cbData as usize);
                isize::from((&mut *LAST.0.get()).replace(bytes))
            }
            _ => 0,
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if msg != 0 && msg == TASKBAR_CREATED.load(Ordering::Relaxed) {
            tray(NIM_ADD);
            if !SESSION.alive() {
                release_idle_pages();
            }
            return 0;
        }
        match msg {
            WM_COPYDATA => packet(w, l),
            TRAY_EVENT if matches!(l as u32, WM_LBUTTONUP | WM_RBUTTONUP) => {
                open_ui(false);
                0
            }
            KEY => {
                key(w);
                0
            }
            PAUSE => {
                pause(w != 0);
                0
            }
            SETTINGS => {
                open_ui(true);
                0
            }
            UI_CHECK => {
                recover_children();
                0
            }
            GET_EPOCH => EPOCH.load(Ordering::Relaxed) as isize,
            CAPTURE_CONSUMED if SESSION.sender(w) => {
                CAPTURE_ARGUMENTS.store(0, Ordering::Relaxed);
                0
            }
            LAST_REQUEST => {
                let mut pid = 0;
                let target = w as HWND;
                if GetWindowThreadProcessId(target, &mut pid) == 0 {
                    return 0;
                }
                isize::from(send_last(target))
            }
            BOOTSTRAP_CLOSED if BOOTSTRAP.sender(w) => {
                BOOTSTRAP.close();
                if CONFIG_READY.load(Ordering::Acquire) {
                    set_status(windows_sys::w!("VTD: ready"));
                } else {
                    set_status(windows_sys::w!("VTD: configuration could not be loaded"));
                }
                if !SESSION.alive() {
                    release_idle_pages();
                }
                0
            }
            SESSION_READY if SESSION.sender(w) => {
                let mut pid = 0;
                let target = l as HWND;
                if target.is_null()
                    || GetWindowThreadProcessId(target, &mut pid) == 0
                    || pid as usize != w
                {
                    return 0;
                }
                SESSION.window.store(target as usize, Ordering::Relaxed);
                send_last(target);
                if PAUSED.load(Ordering::Relaxed) {
                    PostMessageW(target, PAUSE, 1, 0);
                }
                let mut read = QUEUE_READ.load(Ordering::Relaxed);
                let write = QUEUE_WRITE.load(Ordering::Relaxed);
                while read != write {
                    if PostMessageW(
                        target,
                        KEY,
                        QUEUE[read % QUEUE_SIZE].load(Ordering::Relaxed),
                        0,
                    ) != 0
                    {
                        KEYS_IN_FLIGHT.fetch_add(1, Ordering::Relaxed);
                    }
                    read = read.wrapping_add(1);
                }
                QUEUE_READ.store(read, Ordering::Relaxed);
                if CLOSING.load(Ordering::Relaxed) {
                    PostMessageW(target, WM_CLOSE, 0, 0);
                }
                1
            }
            SESSION_CLOSED if SESSION.sender(w) => {
                SESSION.close();
                KEYS_IN_FLIGHT.store(0, Ordering::Relaxed);
                ACTIVE.store(false, Ordering::Relaxed);
                if CLOSING.load(Ordering::Relaxed) {
                    DestroyWindow(hwnd);
                } else {
                    // Inputs arriving after the idle handshake belong to a new session.
                    let mut read = QUEUE_READ.load(Ordering::Relaxed);
                    let write = QUEUE_WRITE.load(Ordering::Relaxed);
                    QUEUE_READ.store(write, Ordering::Relaxed);
                    while read != write {
                        let action = QUEUE[read % QUEUE_SIZE].load(Ordering::Relaxed);
                        key(action);
                        read = read.wrapping_add(1);
                    }
                    if !SESSION.alive() {
                        release_idle_pages();
                    }
                }
                0
            }
            KEY_ACK if SESSION.sender(w) => {
                let pending = KEYS_IN_FLIGHT.load(Ordering::Relaxed);
                if pending > 0 {
                    KEYS_IN_FLIGHT.store(pending - 1, Ordering::Relaxed);
                }
                1
            }
            SESSION_IDLE if SESSION.sender(w) => {
                if ACTIVE.load(Ordering::Relaxed)
                    || KEYS_IN_FLIGHT.load(Ordering::Relaxed) != 0
                    || QUEUE_READ.load(Ordering::Relaxed) != QUEUE_WRITE.load(Ordering::Relaxed)
                {
                    return 0;
                }
                // Subsequent keys queue until SESSION_CLOSED, even if this window still exists.
                SESSION.window.store(0, Ordering::Relaxed);
                1
            }
            SETTINGS_READY | MENU_READY if UI.sender(w) => {
                if msg == SETTINGS_READY && l == 0 {
                    if !settings_allowed() {
                        return 0;
                    }
                    SETTINGS_OPEN.store(true, Ordering::Release);
                    EPOCH.fetch_add(1, Ordering::Relaxed);
                    reset_held();
                    UI.window.store(0, Ordering::Relaxed);
                    return 1;
                }
                let target = l as HWND;
                let mut pid = 0;
                GetWindowThreadProcessId(target, &mut pid);
                if target.is_null() || pid as usize != w {
                    return 0;
                }
                UI.window.store(target as usize, Ordering::Relaxed);
                if msg == MENU_READY && SETTINGS_OPEN.load(Ordering::Acquire) {
                    PostMessageW(target, OPEN_SETTINGS, 0, 0);
                }
                1
            }
            MENU_COMMAND if UI.sender(w) => {
                match l {
                    1 => pause(!PAUSED.load(Ordering::Relaxed)),
                    2 => {
                        PostMessageW(hwnd, WM_CLOSE, 0, 0);
                    }
                    _ => return 0,
                }
                1
            }
            SETTINGS_APPLY if UI.sender(w) => {
                if !settings_allowed() {
                    return 0;
                }
                if SESSION.alive()
                    && !SESSION.hwnd().is_null()
                    && PostMessageW(
                        SESSION.hwnd(),
                        SETTINGS_APPLY,
                        GetCurrentProcessId() as usize,
                        0,
                    ) == 0
                {
                    return 0;
                }
                set_status(windows_sys::w!("VTD: settings saved"));
                1
            }
            UI_CLOSED if UI.sender(w) => {
                UI.close();
                if SETTINGS_OPEN.load(Ordering::Acquire) {
                    reset_held();
                }
                SETTINGS_OPEN.store(false, Ordering::Release);
                if !SESSION.alive() {
                    release_idle_pages();
                }
                0
            }
            WM_CLOSE => {
                recover_children();
                CLOSING.store(true, Ordering::Relaxed);
                reset_held();
                if !UI.hwnd().is_null() {
                    PostMessageW(UI.hwnd(), WM_CLOSE, 0, 0);
                }
                if SESSION.alive() {
                    if !SESSION.hwnd().is_null() {
                        PostMessageW(SESSION.hwnd(), WM_CLOSE, 0, 0);
                    }
                } else {
                    DestroyWindow(hwnd);
                }
                0
            }
            WM_DESTROY => {
                tray(NIM_DELETE);
                PostQuitMessage(0);
                0
            }
            #[cfg(feature = "resident-test")]
            TEST_KEY => {
                PostThreadMessageW(HOOK_THREAD.load(Ordering::Relaxed), TEST_KEY, w, l);
                1
            }
            #[cfg(feature = "resident-test")]
            TEST_HOOK_THREAD => HOOK_THREAD.load(Ordering::Relaxed) as isize,
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

unsafe fn modifiers_held(released: u32) -> bool {
    #[cfg(feature = "resident-test")]
    if TEST_DISPATCH.load(Ordering::Relaxed) {
        return TEST_MODIFIERS.load(Ordering::Relaxed) != 0;
    }
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

unsafe fn shortcut_modifiers() -> u32 {
    #[cfg(feature = "resident-test")]
    if TEST_DISPATCH.load(Ordering::Relaxed) {
        return TEST_MODIFIERS.load(Ordering::Relaxed);
    }
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

unsafe fn replay_when_released(released: u32) {
    if !unsafe { modifiers_held(released) } && REPLAY_PENDING.swap(false, Ordering::Relaxed) {
        EPOCH.fetch_add(1, Ordering::Relaxed);
        unsafe {
            PostMessageW(WINDOW.load(Ordering::Relaxed) as HWND, KEY, 4, 0);
        }
    }
}

unsafe extern "system" fn keyboard(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if code == HC_ACTION as i32
            && CONFIG_READY.load(Ordering::Acquire)
            && !PAUSED.load(Ordering::Relaxed)
        {
            let event = &*(l as *const KBDLLHOOKSTRUCT);
            #[cfg(feature = "resident-test")]
            let fixture_input = event.dwExtraInfo == TEST_INPUT_MARKER;
            #[cfg(not(feature = "resident-test"))]
            let fixture_input = false;
            if event.flags & LLKHF_INJECTED != 0 && !fixture_input {
                return CallNextHookEx(null_mut(), code, w, l);
            }
            if SETTINGS_OPEN.load(Ordering::Acquire) {
                if [
                    TRIGGER.load(Ordering::Relaxed),
                    TOGGLE.load(Ordering::Relaxed),
                    REPLAY.load(Ordering::Relaxed),
                ]
                .iter()
                .any(|key| key & 0xff == event.vkCode)
                {
                    PostMessageW(WINDOW.load(Ordering::Relaxed) as HWND, UI_CHECK, 0, 0);
                }
                return CallNextHookEx(null_mut(), code, w, l);
            }
            let down = matches!(w as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            let up = matches!(w as u32, WM_KEYUP | WM_SYSKEYUP);
            if up && REPLAY_PENDING.load(Ordering::Relaxed) {
                replay_when_released(event.vkCode);
            }
            let bindings = [
                (REPLAY.load(Ordering::Relaxed), &REPLAY_HELD, 4),
                (TOGGLE.load(Ordering::Relaxed), &TOGGLE_HELD, 3),
                (TRIGGER.load(Ordering::Relaxed), &HELD, 1),
            ];
            let already_held = bindings
                .iter()
                .any(|(key, held, _)| key & 0xff == event.vkCode && held.load(Ordering::Relaxed));
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
            if down {
                EPOCH.fetch_add(1, Ordering::Relaxed);
            }
        }
        CallNextHookEx(null_mut(), code, w, l)
    }
}

unsafe extern "system" fn keyboard_thread(ready: *mut c_void) -> u32 {
    unsafe {
        ImmDisableIME(0);
        let mut msg = core::mem::zeroed();
        PeekMessageW(&mut msg, null_mut(), 0, 0, PM_NOREMOVE);
        let hook = SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard),
            GetModuleHandleW(null_mut()),
            0,
        );
        if !hook.is_null() {
            HOOK_THREAD.store(GetCurrentThreadId(), Ordering::Relaxed);
        }
        SetEvent(ready as HANDLE);
        if hook.is_null() {
            return 1;
        }
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            #[cfg(feature = "resident-test")]
            if msg.message == TEST_KEY {
                TEST_DISPATCH.store(true, Ordering::Relaxed);
                TEST_MODIFIERS.store((msg.lParam as u32) >> 1, Ordering::Relaxed);
                let event = KBDLLHOOKSTRUCT {
                    vkCode: msg.wParam as u32,
                    ..core::mem::zeroed()
                };
                keyboard(
                    HC_ACTION as i32,
                    if msg.lParam & 1 != 0 {
                        WM_KEYDOWN
                    } else {
                        WM_KEYUP
                    } as usize,
                    &event as *const _ as isize,
                );
                TEST_DISPATCH.store(false, Ordering::Relaxed);
                continue;
            }
            DispatchMessageW(&msg);
        }
        UnhookWindowsHookEx(hook);
        0
    }
}

unsafe fn run(arguments: *const u16) -> u32 {
    unsafe {
        #[cfg(feature = "resident-test")]
        let name = windows_sys::w!("Local\\VTD-Test-Windows");
        #[cfg(not(feature = "resident-test"))]
        let name = windows_sys::w!("Local\\VTD-Windows");
        let mutex = CreateMutexW(null(), 0, name);
        if mutex.is_null() {
            return 1;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(mutex);
            return 0;
        }
        let job = CreateJobObjectW(null(), null());
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = core::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if job.is_null()
            || SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                core::mem::size_of_val(&limits) as u32,
            ) == 0
        {
            if !job.is_null() {
                CloseHandle(job);
            }
            CloseHandle(mutex);
            return 1;
        }
        // The OS closes this handle on every exit path, including a resident crash.
        // All descendants belong to it before their first instruction can run.
        CHILD_JOB.store(job as usize, Ordering::Relaxed);
        ImmDisableIME(0);
        let theme = disable_hidden_themes();
        let instance = GetModuleHandleW(null());
        let class = windows_sys::w!("VTDWindows");
        let registration = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class,
            ..core::mem::zeroed()
        };
        let registered = RegisterClassW(&registration);
        let hwnd = if registered != 0 {
            CreateWindowExW(
                0,
                class,
                windows_sys::w!("VTD: ready"),
                0,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                null(),
            )
        } else {
            null_mut()
        };
        if !theme.is_null() {
            FreeLibrary(theme);
        }
        if hwnd.is_null() {
            CloseHandle(mutex);
            return 1;
        }
        WINDOW.store(hwnd as usize, Ordering::Relaxed);
        TASKBAR_CREATED.store(
            RegisterWindowMessageW(windows_sys::w!("TaskbarCreated")),
            Ordering::Relaxed,
        );
        for (target, unit) in STATUS_TEXT
            .iter()
            .zip([86u16, 84, 68, 58, 32, 114, 101, 97, 100, 121, 0])
        {
            target.store(unit, Ordering::Relaxed);
        }
        let added = tray(NIM_ADD);
        // The isolated integration-test desktop deliberately has no Explorer.
        if !added && !cfg!(feature = "resident-test") {
            DestroyWindow(hwnd);
            CloseHandle(mutex);
            return 1;
        }
        let ready = CreateEventW(null(), 1, 0, null());
        let thread = if !ready.is_null() {
            CreateThread(
                null(),
                64 * 1024,
                Some(keyboard_thread),
                ready.cast(),
                STACK_SIZE_PARAM_IS_A_RESERVATION,
                null_mut(),
            )
        } else {
            null_mut()
        };
        if thread.is_null()
            || WaitForSingleObject(ready, 5000) != WAIT_OBJECT_0
            || HOOK_THREAD.load(Ordering::Relaxed) == 0
        {
            if !thread.is_null() {
                CloseHandle(thread);
            }
            if !ready.is_null() {
                CloseHandle(ready);
            }
            DestroyWindow(hwnd);
            CloseHandle(mutex);
            return 1;
        }
        CloseHandle(ready);
        CAPTURE_ARGUMENTS.store(arguments as usize, Ordering::Relaxed);
        if !launch(b"__bootstrap", null(), &BOOTSTRAP, false) {
            MessageBoxW(
                null_mut(),
                windows_sys::w!("Missing vtd-helper.exe. Extract the complete VTD package."),
                windows_sys::w!("VTD"),
                MB_OK | MB_ICONERROR,
            );
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
        }
        // Bootstrap still needs to initialize this window. Trim only after
        // BOOTSTRAP_CLOSED so those first messages do not immediately fault
        // all the just-evicted startup pages back in.
        let mut msg = core::mem::zeroed();
        let result = loop {
            let status = GetMessageW(&mut msg, null_mut(), 0, 0);
            if status <= 0 {
                break u32::from(status < 0);
            }
            DispatchMessageW(&msg);
        };
        PostThreadMessageW(HOOK_THREAD.load(Ordering::Relaxed), WM_QUIT, 0, 0);
        WaitForSingleObject(thread, 5000);
        CloseHandle(thread);
        SESSION.close();
        UI.close();
        BOOTSTRAP.close();
        (&mut *LAST.0.get()).clear();
        CloseHandle(mutex);
        result
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn mainCRTStartup() -> ! {
    unsafe {
        let arguments = skip_word(GetCommandLineW());
        let result = if let Some(tail) = run_arguments(arguments) {
            run(tail)
        } else {
            AttachConsole(ATTACH_PARENT_PROCESS);
            let child = Child::new();
            if launch(b"", arguments, &child, true) {
                let process = child.process.load(Ordering::Relaxed) as HANDLE;
                WaitForSingleObject(process, INFINITE);
                let mut result = 1;
                GetExitCodeProcess(process, &mut result);
                child.close();
                result
            } else {
                1
            }
        };
        ExitProcess(result);
    }
}
