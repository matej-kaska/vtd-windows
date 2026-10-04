use super::{autostart, download::Download, runtime, tray};
use anyhow::{Context, Result, bail, ensure};
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    ptr::null_mut,
    sync::{atomic::Ordering, mpsc::TryRecvError},
};
use vtd::{
    config::Config,
    languages::LANGUAGES,
    models::{self, MODELS, Model},
};
use windows_sys::Win32::{
    Foundation::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentProcessId},
    UI::{
        Controls::{
            CheckDlgButton, CheckRadioButton, HKM_GETHOTKEY, HKM_SETHOTKEY, HKM_SETRULES,
            ICC_HOTKEY_CLASS, INITCOMMONCONTROLSEX, InitCommonControlsEx, IsDlgButtonChecked,
        },
        Input::KeyboardAndMouse::EnableWindow,
        WindowsAndMessaging::*,
    },
};

const HOLD: i32 = 1001;
const TOGGLE: i32 = 1002;
const REPLAY: i32 = 1003;
const LANGUAGE: i32 = 1004;
const AUTOSTART: i32 = 1005;
const MUTE: i32 = 1006;
const MODEL_FIRST: i32 = 1007;
const CUSTOM_MODEL: i32 = 1010;
const DOWNLOAD_STATUS: i32 = 1024;
const MODEL_STATUS_FIRST: i32 = 1030;
const MODEL_ACTION_FIRST: i32 = 1034;
const APPLY: i32 = 1040;
const STOP_DOWNLOAD: i32 = 1041;

struct Apply {
    cfg: Config,
    startup: bool,
    close: bool,
}
struct Pending {
    job: Download,
    model: &'static Model,
    apply: Option<Apply>,
}
struct DialogInit<'a> {
    cfg: &'a Config,
    path: &'a Path,
}
thread_local! { static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) }; }
thread_local! { static CONFIG_PATH: RefCell<PathBuf> = const { RefCell::new(PathBuf::new()) }; }
thread_local! { static READ_ONLY: Cell<bool> = const { Cell::new(false) }; }
thread_local! { static ACTIVE_MODEL: RefCell<PathBuf> = const { RefCell::new(PathBuf::new()) }; }
thread_local! { static WINDOW: Cell<HWND> = const { Cell::new(null_mut()) }; }
thread_local! { static PARENT: Cell<HWND> = const { Cell::new(null_mut()) }; }

fn dispatch(msg: &MSG) -> bool {
    WINDOW.with(|window| unsafe {
        let hwnd = window.get();
        !hwnd.is_null() && IsDialogMessageW(hwnd, msg) != 0
    })
}

fn close() {
    let hwnd = WINDOW.with(Cell::get);
    if !hwnd.is_null() {
        unsafe {
            DestroyWindow(hwnd);
        }
    }
}

// Local visual QA without connecting to a running installation or saving settings.
#[cfg(feature = "settings-preview")]
pub fn preview() -> Result<()> {
    let cfg = Config::load(&vtd::config::path()?)?;
    READ_ONLY.with(|value| value.set(true));
    show(&cfg)?;
    unsafe {
        SetWindowTextW(
            WINDOW.with(Cell::get),
            windows_sys::w!("VTD - Settings preview"),
        );
        SetDlgItemTextW(
            WINDOW.with(Cell::get),
            IDOK,
            windows_sys::w!("Preview only"),
        );
        EnableWindow(GetDlgItem(WINDOW.with(Cell::get), IDOK), 0);
        let mut msg = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if !dispatch(&msg) {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
    Ok(())
}

pub fn run(menu: Option<(bool, bool)>) -> Result<()> {
    unsafe {
        let parent = FindWindowW(windows_sys::w!("VTDWindows"), null_mut());
        ensure!(!parent.is_null(), "VTD is not running.");
        PARENT.with(|window| window.set(parent));
        if let Some((paused, busy)) = menu {
            let command = tray::show_menu(paused, busy)?;
            if command != 4 {
                if command != 0 {
                    notify(runtime::MENU_COMMAND, command as LPARAM)?;
                }
                finished(parent);
                return Ok(());
            }
        }
        // Check current recording state before opening settings from a menu.
        notify(runtime::SETTINGS_READY, 0)?;
        let cfg = Config::load(&vtd::config::path()?)?;
        show(&cfg)?;
        if let Err(e) = notify(runtime::SETTINGS_READY, WINDOW.with(Cell::get) as LPARAM) {
            close();
            return Err(e);
        }
        let mut msg = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if !dispatch(&msg) {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        finished(parent);
        Ok(())
    }
}

unsafe fn finished(parent: HWND) {
    unsafe {
        PostMessageW(
            parent,
            runtime::UI_CLOSED,
            GetCurrentProcessId() as usize,
            0,
        );
    }
}

pub(super) fn notify(message: u32, parameter: LPARAM) -> Result<()> {
    let mut result = 0;
    let sent = unsafe {
        SendMessageTimeoutW(
            PARENT.with(Cell::get),
            message,
            GetCurrentProcessId() as usize,
            parameter,
            SMTO_ABORTIFHUNG,
            3000,
            &mut result,
        )
    };
    ensure!(
        sent != 0 && result == 1,
        "VTD could not apply the settings. Close this window and restart VTD if the problem persists."
    );
    Ok(())
}

fn show(cfg: &Config) -> Result<()> {
    unsafe {
        let existing = WINDOW.with(Cell::get);
        if !existing.is_null() {
            ShowWindow(existing, SW_RESTORE);
            SetForegroundWindow(existing);
            return Ok(());
        }
        ensure!(
            InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_HOTKEY_CLASS,
            }) != 0,
            "Cannot initialize shortcut fields."
        );
        let startup = autostart::enabled(&autostart::snapshot()?)?;
        // cfg is borrowed only synchronously during WM_INITDIALOG.
        let path = vtd::config::path()?;
        let init = DialogInit { cfg, path: &path };
        let hwnd = CreateDialogParamW(
            GetModuleHandleW(null_mut()),
            std::ptr::without_provenance(2),
            null_mut(),
            Some(dialog_proc),
            &init as *const DialogInit as LPARAM,
        );
        ensure!(
            !hwnd.is_null(),
            "Cannot open settings: {}",
            std::io::Error::last_os_error()
        );
        WINDOW.with(|window| window.set(hwnd));
        // A dialog does not inherit the executable's icon. Set both sizes before
        // showing it so the taskbar, its preview and Alt+Tab use the VTD icon.
        for (kind, width, height) in [
            (ICON_SMALL, SM_CXSMICON, SM_CYSMICON),
            (ICON_BIG, SM_CXICON, SM_CYICON),
        ] {
            let icon = LoadImageW(
                GetModuleHandleW(null_mut()),
                std::ptr::without_provenance(1),
                IMAGE_ICON,
                GetSystemMetrics(width),
                GetSystemMetrics(height),
                LR_SHARED,
            );
            if !icon.is_null() {
                // Shared resource handles belong to Windows and need no DestroyIcon.
                SendMessageW(hwnd, WM_SETICON, kind as WPARAM, icon as LPARAM);
            }
        }
        CheckDlgButton(hwnd, AUTOSTART, u32::from(startup));
        CheckDlgButton(hwnd, MUTE, u32::from(cfg.mute_output));
        // Install/autostart can launch us with STARTF_USESHOWWINDOW / SW_HIDE.
        // ShowWindow's first call would honor that instead of opening settings.
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_SHOWWINDOW,
        );
        SetForegroundWindow(hwnd);
        Ok(())
    }
}

pub fn error(hwnd: HWND, error: &anyhow::Error) {
    unsafe {
        MessageBoxW(
            hwnd,
            runtime::wide(&format!("{error:#}")).as_ptr(),
            windows_sys::w!("VTD - Settings"),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn config_path() -> PathBuf {
    CONFIG_PATH.with(|path| path.borrow().clone())
}

fn save(hwnd: HWND, close: bool) -> Result<bool> {
    let config_path = config_path();
    let mut cfg = Config::load(&config_path)?;
    unsafe {
        for (id, key) in [
            (HOLD, &mut cfg.trigger_key),
            (TOGGLE, &mut cfg.toggle_key),
            (REPLAY, &mut cfg.replay_key),
        ] {
            *key = SendDlgItemMessageW(hwnd, id, HKM_GETHOTKEY, 0, 0) as u32 & 0x7ff;
        }
        let selected = SendDlgItemMessageW(hwnd, LANGUAGE, CB_GETCURSEL, 0, 0);
        ensure!(selected >= 0, "Select a speech language.");
        if let Some(&(code, _)) = LANGUAGES.get(selected as usize) {
            cfg.language = code.into();
        } // An unlisted existing language alias remains untouched.
        cfg.mute_output = IsDlgButtonChecked(hwnd, MUTE) == 1;
        let selected = MODELS
            .iter()
            .enumerate()
            .find(|(i, _)| IsDlgButtonChecked(hwnd, MODEL_FIRST + *i as i32) == 1)
            .map(|(_, m)| m);
        if let Some(model) = selected
            && (models::from_path(&cfg.model).is_none_or(|current| current.id != model.id)
                || !cfg.model.is_file())
        {
            cfg.model = model.path(&config_path);
        }
        cfg.validate()?;
        let startup = IsDlgButtonChecked(hwnd, AUTOSTART) == 1;
        let previous = Config::load(&config_path)?;
        if let Some(model) = selected
            && (previous.model != cfg.model || !cfg.model.is_file())
        {
            start_download(
                hwnd,
                model,
                Some(Apply {
                    cfg,
                    startup,
                    close,
                }),
            );
            return Ok(false);
        }
        commit(&cfg, startup)?;
        model_changed(hwnd);
        set_status(
            hwnd,
            "Settings applied. You can now delete the previous model.",
        );
        Ok(close)
    }
}

fn commit(cfg: &Config, startup: bool) -> Result<()> {
    let config_path = config_path();
    let previous_startup = autostart::snapshot()?;
    let change_startup = autostart::enabled(&previous_startup)? != startup;
    if change_startup {
        autostart::set_enabled(startup)?;
    }
    if let Err(error) = cfg.save_preferences(&config_path) {
        if change_startup && let Err(rollback) = autostart::restore(&previous_startup) {
            bail!("{error:#}; could not restore the startup setting: {rollback:#}");
        }
        return Err(error);
    }
    super::ipc::preferences(PARENT.with(Cell::get), cfg)?;
    notify(runtime::SETTINGS_APPLY, 0)?;
    ACTIVE_MODEL.with(|active| *active.borrow_mut() = cfg.model.clone());
    Ok(())
}

fn set_status(hwnd: HWND, text: &str) {
    unsafe {
        SetDlgItemTextW(hwnd, DOWNLOAD_STATUS, runtime::wide(text).as_ptr());
    }
}

fn start_download(hwnd: HWND, model: &'static Model, apply: Option<Apply>) {
    PENDING.with(|pending| {
        *pending.borrow_mut() = Some(Pending {
            job: Download::start(model, &config_path()),
            model,
            apply,
        });
    });
    downloading(hwnd, true);
    unsafe {
        SetTimer(hwnd, 1, 150, None);
    }
}

fn downloading(hwnd: HWND, active: bool) {
    unsafe {
        for id in [
            HOLD,
            TOGGLE,
            REPLAY,
            LANGUAGE,
            AUTOSTART,
            MUTE,
            MODEL_FIRST,
            MODEL_FIRST + 1,
            MODEL_FIRST + 2,
            CUSTOM_MODEL,
            IDOK,
            APPLY,
            MODEL_ACTION_FIRST,
            MODEL_ACTION_FIRST + 1,
            MODEL_ACTION_FIRST + 2,
        ] {
            EnableWindow(GetDlgItem(hwnd, id), i32::from(!active));
        }
        ShowWindow(
            GetDlgItem(hwnd, STOP_DOWNLOAD),
            if active { SW_SHOW } else { SW_HIDE },
        );
    }
    if active {
        set_status(hwnd, "Checking the model...");
    } else {
        model_changed(hwnd);
    }
}

fn stop_download(hwnd: HWND) {
    unsafe {
        KillTimer(hwnd, 1);
    }
    // Joining the downloader kills curl and removes its incomplete file.
    PENDING.with(|pending| {
        pending.borrow_mut().take();
    });
    downloading(hwnd, false);
    set_status(hwnd, "Download stopped. Your settings have not changed.");
}

fn poll_download(hwnd: HWND) {
    let result = PENDING.with(|pending| {
        let pending = pending.borrow();
        let Pending { job, model, .. } = pending.as_ref()?;
        match job.result.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Disconnected) => {
                Some(Err("Model download stopped unexpectedly.".into()))
            }
            Err(TryRecvError::Empty) => {
                let status = match job.stage.load(Ordering::Relaxed) {
                    1 => format!(
                        "Downloading {}: {} / {} MB",
                        model.name,
                        job.bytes.load(Ordering::Relaxed) / 1_000_000,
                        model.bytes / 1_000_000
                    ),
                    2 => "Verifying the downloaded model...".into(),
                    _ => "Checking the existing model...".into(),
                };
                unsafe {
                    SetDlgItemTextW(hwnd, DOWNLOAD_STATUS, runtime::wide(&status).as_ptr());
                }
                None
            }
        }
    });
    if let Some(result) = result {
        unsafe {
            KillTimer(hwnd, 1);
        }
        let Pending { job, model, apply } =
            PENDING.with(|pending| pending.borrow_mut().take().unwrap());
        drop(job);
        downloading(hwnd, false);
        match result.map_err(anyhow::Error::msg).and_then(|_| {
            if let Some(apply) = &apply {
                commit(&apply.cfg, apply.startup)?;
            }
            Ok(())
        }) {
            Ok(()) => {
                if apply.as_ref().is_some_and(|a| a.close) {
                    unsafe {
                        DestroyWindow(hwnd);
                    }
                } else {
                    model_changed(hwnd);
                    set_status(
                        hwnd,
                        &format!(
                            "{} {}",
                            model.name,
                            if apply.is_some() {
                                "is now in use. You can delete the previous model."
                            } else {
                                "downloaded. Select it and click Apply to use it."
                            }
                        ),
                    );
                }
            }
            Err(e) => {
                set_status(hwnd, "Operation failed. Check the error and try again.");
                error(hwnd, &e);
            }
        }
    }
}

fn same_path(first: &Path, second: &Path) -> bool {
    let first = first.canonicalize().unwrap_or_else(|_| first.to_path_buf());
    let second = second
        .canonicalize()
        .unwrap_or_else(|_| second.to_path_buf());
    first.as_os_str().eq_ignore_ascii_case(second.as_os_str())
}

// Only catalog files in this installation's models directory can be removed.
// Never follow a redirected models folder or remove a file still used by VTD.
fn deletion_target(model: &Model, path: &Path, active: &Path) -> Result<PathBuf> {
    let target = model.path(path);
    ensure!(
        !same_path(&target, active),
        "Select another model and click Apply before deleting the model in use."
    );
    let resolved = target
        .canonicalize()
        .context("The model file is no longer available.")?;
    let directory = path
        .parent()
        .context("Settings folder missing")?
        .canonicalize()?
        .join("models");
    ensure!(
        resolved.parent().is_some_and(|parent| parent
            .as_os_str()
            .eq_ignore_ascii_case(directory.as_os_str())),
        "This model is stored outside the application. External model files are kept; manage them in File Explorer."
    );
    ensure!(resolved.is_file(), "The model path is not a file.");
    Ok(target)
}

fn delete_model(model: &Model, path: &Path, active: &Path) -> Result<()> {
    let target = deletion_target(model, path, active)?;
    std::fs::remove_file(&target).with_context(|| {
        format!(
            "Cannot delete {}. The file may still be in use; try again.",
            model.name
        )
    })
}

fn model_action(hwnd: HWND, index: usize) -> Result<()> {
    let model = &MODELS[index];
    let path = config_path();
    if !model.path(&path).is_file() {
        start_download(hwnd, model, None);
        return Ok(());
    }
    let active = ACTIVE_MODEL.with(|value| value.borrow().clone());
    deletion_target(model, &path, &active)?;
    // Recheck disk preferences as well, in case they changed outside this window.
    let cfg = Config::load(&path)?;
    deletion_target(model, &path, &cfg.model)?;
    let prompt = format!(
        "Delete {} from this computer?\n\nThis removes the local model file ({} MB). You can download it again later. Your selected model and settings will stay unchanged.",
        model.name,
        model.bytes / 1_000_000
    );
    let answer = unsafe {
        MessageBoxW(
            hwnd,
            runtime::wide(&prompt).as_ptr(),
            windows_sys::w!("VTD - Delete model"),
            MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
        )
    };
    if answer == IDYES {
        delete_model(model, &path, &active)?;
        model_changed(hwnd);
        set_status(
            hwnd,
            &format!(
                "{} deleted. You can download it again at any time.",
                model.name
            ),
        );
    }
    Ok(())
}

fn model_changed(hwnd: HWND) {
    let path = config_path();
    let current = match Config::load(&path) {
        Ok(cfg) => cfg,
        Err(e) => {
            set_status(hwnd, &format!("Cannot read settings: {e:#}"));
            return;
        }
    };
    unsafe {
        let read_only = READ_ONLY.with(Cell::get);
        // Apply keeps this dialog alive; an old custom-model choice must not
        // keep pointing at the model that was active when the dialog opened.
        ShowWindow(
            GetDlgItem(hwnd, CUSTOM_MODEL),
            if models::from_path(&current.model).is_none() {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
        let active = ACTIVE_MODEL.with(|value| value.borrow().clone());
        for (i, model) in MODELS.iter().enumerate() {
            let managed = model.path(&path);
            let downloaded = managed.is_file();
            let in_use = same_path(&managed, &active);
            let external = models::from_path(&active).is_some_and(|m| m.id == model.id)
                && active.is_file()
                && !in_use;
            let state = if in_use && downloaded {
                "In use"
            } else if external && downloaded {
                "External + local copy"
            } else if external {
                "In use (external)"
            } else if downloaded {
                "Downloaded"
            } else if in_use {
                "File missing"
            } else {
                "Not downloaded"
            };
            SetDlgItemTextW(
                hwnd,
                MODEL_STATUS_FIRST + i as i32,
                runtime::wide(state).as_ptr(),
            );
            SetDlgItemTextW(
                hwnd,
                MODEL_ACTION_FIRST + i as i32,
                runtime::wide(if downloaded { "Delete..." } else { "Download" }).as_ptr(),
            );
            let can_delete = deletion_target(model, &path, &active).is_ok()
                && deletion_target(model, &path, &current.model).is_ok();
            EnableWindow(
                GetDlgItem(hwnd, MODEL_ACTION_FIRST + i as i32),
                i32::from(!read_only && (!downloaded || can_delete)),
            );
        }
        let selected = MODELS
            .iter()
            .enumerate()
            .find(|(i, _)| IsDlgButtonChecked(hwnd, MODEL_FIRST + *i as i32) == 1);
        let needs_download = selected.is_some_and(|(_, m)| {
            let available = models::from_path(&current.model).is_some_and(|known| known.id == m.id)
                && current.model.is_file();
            !available && !m.path(&path).is_file()
        });
        SetDlgItemTextW(
            hwnd,
            IDOK,
            runtime::wide(if needs_download {
                "Download && save"
            } else {
                "&Save"
            })
            .as_ptr(),
        );
        SetDlgItemTextW(
            hwnd,
            APPLY,
            runtime::wide(if needs_download {
                "Download && apply"
            } else {
                "&Apply"
            })
            .as_ptr(),
        );
        EnableWindow(GetDlgItem(hwnd, IDOK), i32::from(!read_only));
        EnableWindow(GetDlgItem(hwnd, APPLY), i32::from(!read_only));
        let text = selected
                .map(|(_, m)| {
                    format!(
                        "Model download: {} MB.{}\r\nTo delete the model in use, select another one and click Apply first.",
                        m.bytes / 1_000_000,
                        if needs_download {
                            " Internet is required once."
                        } else {
                            " Available locally."
                        }
                    )
                })
                .unwrap_or_default();
        set_status(hwnd, &text);
    }
}

unsafe extern "system" fn dialog_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> isize {
    unsafe {
        match msg {
            WM_INITDIALOG => {
                let init = &*(l as *const DialogInit);
                let cfg = init.cfg;
                CONFIG_PATH.with(|path| *path.borrow_mut() = init.path.to_path_buf());
                ACTIVE_MODEL.with(|active| *active.borrow_mut() = cfg.model.clone());
                ShowWindow(GetDlgItem(hwnd, STOP_DOWNLOAD), SW_HIDE);
                let current = models::from_path(&cfg.model);
                for (i, model) in MODELS.iter().enumerate() {
                    SetDlgItemTextW(
                        hwnd,
                        MODEL_FIRST + i as i32,
                        runtime::wide(model.name).as_ptr(),
                    );
                    SetDlgItemTextW(hwnd, 1017 + i as i32, runtime::wide(model.details).as_ptr());
                    CheckDlgButton(
                        hwnd,
                        MODEL_FIRST + i as i32,
                        u32::from(current.is_some_and(|m| m.id == model.id)),
                    );
                }
                if current.is_none() {
                    CheckDlgButton(hwnd, CUSTOM_MODEL, 1);
                    SetDlgItemTextW(
                        hwnd,
                        CUSTOM_MODEL,
                        runtime::wide(&format!(
                            "Keep current custom model: {}",
                            cfg.model.file_name().unwrap_or_default().to_string_lossy()
                        ))
                        .as_ptr(),
                    );
                } else {
                    ShowWindow(GetDlgItem(hwnd, CUSTOM_MODEL), SW_HIDE);
                }
                SetDlgItemTextW(hwnd, 1021, runtime::wide(models::BENCHMARK).as_ptr());
                for (id, key) in [
                    (HOLD, cfg.trigger_key),
                    (TOGGLE, cfg.toggle_key),
                    (REPLAY, cfg.replay_key),
                ] {
                    SendDlgItemMessageW(hwnd, id, HKM_SETRULES, 0, 0);
                    SendDlgItemMessageW(hwnd, id, HKM_SETHOTKEY, key as usize, 0);
                }
                let mut selected = LANGUAGES.len();
                for (index, &(code, name)) in LANGUAGES.iter().enumerate() {
                    SendDlgItemMessageW(
                        hwnd,
                        LANGUAGE,
                        CB_ADDSTRING,
                        0,
                        runtime::wide(name).as_ptr() as LPARAM,
                    );
                    if cfg.language == code {
                        selected = index;
                    }
                }
                if selected == LANGUAGES.len() {
                    SendDlgItemMessageW(
                        hwnd,
                        LANGUAGE,
                        CB_ADDSTRING,
                        0,
                        runtime::wide(&cfg.language).as_ptr() as LPARAM,
                    );
                }
                SendDlgItemMessageW(hwnd, LANGUAGE, CB_SETCURSEL, selected, 0);
                if cfg.toggle {
                    SetDlgItemTextW(hwnd, 1011, windows_sys::w!("Start / stop (&primary):"));
                }
                model_changed(hwnd);
                1
            }
            WM_COMMAND => {
                let id = (w & 0xffff) as i32;
                let model_action =
                    (MODEL_ACTION_FIRST..MODEL_ACTION_FIRST + MODELS.len() as i32).contains(&id);
                if (READ_ONLY.with(Cell::get) && (id == IDOK || id == APPLY || model_action))
                    || (PENDING.with(|pending| pending.borrow().is_some())
                        && id != IDCANCEL
                        && id != STOP_DOWNLOAD)
                {
                    return 1;
                }
                match id {
                    IDOK | APPLY => match save(hwnd, id == IDOK) {
                        Ok(true) => {
                            DestroyWindow(hwnd);
                        }
                        Ok(false) => {}
                        Err(e) => error(hwnd, &e),
                    },
                    IDCANCEL => {
                        DestroyWindow(hwnd);
                    }
                    STOP_DOWNLOAD => stop_download(hwnd),
                    id if model_action => {
                        if let Err(e) = self::model_action(hwnd, (id - MODEL_ACTION_FIRST) as usize)
                        {
                            error(hwnd, &e);
                        }
                    }
                    id if (MODEL_FIRST..=CUSTOM_MODEL).contains(&id) => {
                        CheckRadioButton(hwnd, MODEL_FIRST, CUSTOM_MODEL, id);
                        model_changed(hwnd);
                    }
                    _ => return 0,
                }
                1
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                1
            }
            WM_TIMER if w == 1 => {
                poll_download(hwnd);
                1
            }
            WM_DESTROY => {
                KillTimer(hwnd, 1);
                PENDING.with(|pending| {
                    pending.borrow_mut().take();
                });
                PostQuitMessage(0);
                0
            }
            WM_NCDESTROY => {
                WINDOW.with(|window| {
                    if window.get() == hwnd {
                        window.set(null_mut());
                    }
                });
                0
            }
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;

    static CACHED: Model = Model {
        id: "canary-q4",
        name: "Cached test model",
        file: "canary-1b-v2-Q4_K_M.gguf",
        engine: models::EngineKind::Transcribe,
        url: "http://127.0.0.1:9/no-network-in-this-test",
        sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        bytes: 3,
        details: "",
        autodetect: false,
        languages: &["cs"],
    };

    fn finish_download(hwnd: HWND) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while PENDING.with(|pending| pending.borrow().is_some()) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
            poll_download(hwnd);
        }
    }

    fn label(hwnd: HWND, id: i32) -> String {
        let mut text = [0u16; 512];
        let len = unsafe { GetDlgItemTextW(hwnd, id, text.as_mut_ptr(), text.len() as i32) };
        String::from_utf16_lossy(&text[..len as usize])
    }

    fn fixture(name: &str) -> (PathBuf, Config) {
        let dir = std::env::temp_dir().join(format!("vtd-settings-{name}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("models")).unwrap();
        let path = dir.join("vtd.json");
        let cfg = Config {
            model: MODELS[2].path(&path),
            language: "cs".into(),
            ..Config::default()
        };
        std::fs::write(&cfg.model, b"active fixture").unwrap();
        std::fs::write(&path, serde_json::to_vec(&cfg).unwrap()).unwrap();
        (path, cfg)
    }

    fn cleanup(path: &Path) {
        // Delete only the known fixture files, never recursively remove a path.
        for model in MODELS {
            let file = model.path(path);
            if file.is_file() {
                std::fs::remove_file(file).unwrap();
            }
        }
        let dir = path.parent().unwrap();
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir.join("models")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    fn dialog(path: &Path, cfg: &Config) -> HWND {
        unsafe {
            assert_ne!(
                InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                    dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                    dwICC: ICC_HOTKEY_CLASS,
                }),
                0
            );
            let hwnd = CreateDialogParamW(
                GetModuleHandleW(null_mut()),
                std::ptr::without_provenance(2),
                null_mut(),
                Some(dialog_proc),
                &DialogInit { cfg, path } as *const DialogInit as LPARAM,
            );
            assert!(
                !hwnd.is_null(),
                "Dialog resource: {}",
                std::io::Error::last_os_error()
            );
            hwnd
        }
    }

    fn close_dialog(hwnd: HWND) {
        unsafe {
            DestroyWindow(hwnd);
            let mut msg = std::mem::zeroed();
            PeekMessageW(&mut msg, null_mut(), WM_QUIT, WM_QUIT, PM_REMOVE);
        }
    }

    #[test]
    fn deletion_protects_active_and_external_models_and_other_files() {
        let (path, cfg) = fixture("delete");
        let spare = MODELS[0].path(&path);
        std::fs::write(&spare, b"spare fixture").unwrap();
        let external = path.parent().unwrap().join(MODELS[0].file);
        std::fs::write(&external, b"external fixture").unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(delete_model(&MODELS[2], &path, &cfg.model).is_err());
        // Canonical aliases to the active path are also protected.
        assert!(
            delete_model(
                &MODELS[0],
                &path,
                &spare.parent().unwrap().join(".").join(MODELS[0].file)
            )
            .is_err()
        );
        // A same-named external file is never a target of the manager.
        delete_model(&MODELS[0], &path, &external).unwrap();
        assert!(!spare.exists());
        assert_eq!(std::fs::read(&external).unwrap(), b"external fixture");
        assert_eq!(std::fs::read(&cfg.model).unwrap(), b"active fixture");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(delete_model(&MODELS[0], &path, &external).is_err());
        std::fs::remove_file(external).unwrap();
        cleanup(&path);
    }

    #[test]
    fn downloading_another_model_does_not_apply_unsaved_preferences() {
        let (path, cfg) = fixture("download");
        std::fs::write(CACHED.path(&path), b"abc").unwrap();
        let before = std::fs::read(&path).unwrap();
        let hwnd = dialog(&path, &cfg);
        unsafe {
            SendDlgItemMessageW(hwnd, MODEL_FIRST, BM_CLICK, 0, 0);
        }
        start_download(hwnd, &CACHED, None);
        assert_eq!(unsafe { IsWindowEnabled(GetDlgItem(hwnd, APPLY)) }, 0);
        finish_download(hwnd);
        assert_ne!(unsafe { IsWindow(hwnd) }, 0);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(
            ACTIVE_MODEL.with(|active| active.borrow().clone()),
            cfg.model
        );
        assert!(label(hwnd, MODEL_STATUS_FIRST).starts_with("Downloaded"));
        assert!(label(hwnd, DOWNLOAD_STATUS).contains("downloaded"));
        assert_eq!(
            unsafe { IsWindowEnabled(GetDlgItem(hwnd, MODEL_ACTION_FIRST + 2)) },
            0
        );
        // Stopping a second job leaves preferences and a complete cache intact.
        start_download(hwnd, &CACHED, None);
        stop_download(hwnd);
        assert!(PENDING.with(|pending| pending.borrow().is_none()));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(std::fs::read(CACHED.path(&path)).unwrap(), b"abc");
        close_dialog(hwnd);
        cleanup(&path);
    }

    #[test]
    fn apply_download_keeps_dialog_open_and_unlocks_previous_model_for_deletion() {
        thread_local! { static APPLIED: Cell<bool> = const { Cell::new(false) }; }
        unsafe extern "system" fn acknowledge(
            hwnd: HWND,
            msg: u32,
            w: WPARAM,
            l: LPARAM,
        ) -> LRESULT {
            if msg == WM_COPYDATA && l != 0 {
                let packet = unsafe {
                    &*(l as *const windows_sys::Win32::System::DataExchange::COPYDATASTRUCT)
                };
                assert_eq!(packet.dwData, super::super::protocol::PREFERENCES);
                assert_eq!(
                    packet.cbData as usize,
                    std::mem::size_of::<super::super::protocol::Preferences>()
                );
                let prefs = unsafe {
                    std::ptr::read_unaligned(
                        packet.lpData.cast::<super::super::protocol::Preferences>(),
                    )
                };
                assert_eq!(prefs.trigger, 119);
                assert_eq!(prefs.toggle, 120);
                assert_eq!(prefs.replay, 121);
                1
            } else if msg == runtime::SETTINGS_APPLY {
                APPLIED.with(|applied| applied.set(true));
                1
            } else {
                unsafe { DefWindowProcW(hwnd, msg, w, l) }
            }
        }
        let (path, mut cfg) = fixture("apply");
        let hwnd = dialog(&path, &cfg);
        let parent = unsafe {
            let class = WNDCLASSW {
                lpfnWndProc: Some(acknowledge),
                hInstance: GetModuleHandleW(null_mut()),
                lpszClassName: windows_sys::w!("VTDSettingsApplyTest"),
                ..std::mem::zeroed()
            };
            assert_ne!(RegisterClassW(&class), 0);
            CreateWindowExW(
                0,
                class.lpszClassName,
                null_mut(),
                0,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                class.hInstance,
                null_mut(),
            )
        };
        assert!(!parent.is_null());
        PARENT.with(|value| value.set(parent));
        cfg.model = CACHED.path(&path);
        std::fs::write(&cfg.model, b"abc").unwrap();
        let startup_snapshot = autostart::snapshot().unwrap();
        let startup = autostart::enabled(&startup_snapshot).unwrap();
        start_download(
            hwnd,
            &CACHED,
            Some(Apply {
                cfg: cfg.clone(),
                startup,
                close: false,
            }),
        );
        finish_download(hwnd);
        assert!(APPLIED.with(Cell::get));
        assert_eq!(Config::load(&path).unwrap().model, cfg.model);
        assert_eq!(
            ACTIVE_MODEL.with(|active| active.borrow().clone()),
            cfg.model
        );
        assert_ne!(unsafe { IsWindow(hwnd) }, 0);
        assert_eq!(
            unsafe { IsWindowEnabled(GetDlgItem(hwnd, MODEL_ACTION_FIRST)) },
            0
        );
        assert_ne!(
            unsafe { IsWindowEnabled(GetDlgItem(hwnd, MODEL_ACTION_FIRST + 2)) },
            0
        );
        delete_model(&MODELS[2], &path, &cfg.model).unwrap();
        assert!(!MODELS[2].path(&path).exists());
        assert!(cfg.model.is_file());
        assert_eq!(
            autostart::enabled(&autostart::snapshot().unwrap()).unwrap(),
            startup
        );
        close_dialog(hwnd);
        unsafe {
            DestroyWindow(parent);
            UnregisterClassW(
                windows_sys::w!("VTDSettingsApplyTest"),
                GetModuleHandleW(null_mut()),
            );
        }
        PARENT.with(|value| value.set(null_mut()));
        cleanup(&path);
    }

    #[test]
    fn model_radios_are_exclusive_and_update_the_download() {
        let (path, cfg) = fixture("radios");
        std::fs::write(MODELS[0].path(&path), b"downloaded fixture").unwrap();
        let before = std::fs::read(&path).unwrap();
        let hwnd = dialog(&path, &cfg);
        unsafe {
            assert!(label(hwnd, MODEL_STATUS_FIRST + 2).starts_with("In use"));
            assert_eq!(IsWindowEnabled(GetDlgItem(hwnd, MODEL_ACTION_FIRST + 2)), 0);
            assert_eq!(label(hwnd, MODEL_ACTION_FIRST), "Delete...");
            assert_ne!(IsWindowEnabled(GetDlgItem(hwnd, MODEL_ACTION_FIRST)), 0);
            assert_eq!(label(hwnd, MODEL_ACTION_FIRST + 1), "Download");
            // Arrow-key navigation must visit only radios, never their labels
            // or Download/Delete buttons, including when the custom row is hidden.
            for i in 0..3 {
                assert_eq!(
                    GetDlgCtrlID(GetNextDlgGroupItem(
                        hwnd,
                        GetDlgItem(hwnd, MODEL_FIRST + i),
                        0
                    )),
                    MODEL_FIRST + (i + 1) % 3
                );
                assert_eq!(
                    GetDlgCtrlID(GetNextDlgGroupItem(
                        hwnd,
                        GetDlgItem(hwnd, MODEL_FIRST + i),
                        1
                    )),
                    MODEL_FIRST + (i + 2) % 3
                );
            }
            for index in [1, 2, 0, 1] {
                SendDlgItemMessageW(hwnd, MODEL_FIRST + index as i32, BM_CLICK, 0, 0);
                for i in 0..3 {
                    assert_eq!(
                        IsDlgButtonChecked(hwnd, MODEL_FIRST + i as i32),
                        u32::from(i == index)
                    );
                }
                assert!(
                    label(hwnd, DOWNLOAD_STATUS)
                        .contains(&(MODELS[index].bytes / 1_000_000).to_string())
                );
                assert_eq!(label(hwnd, APPLY).contains("Download"), index == 1);
                // Selection alone neither changes the active model nor unlocks its Delete button.
                assert_eq!(IsWindowEnabled(GetDlgItem(hwnd, MODEL_ACTION_FIRST + 2)), 0);
                assert_eq!(std::fs::read(&path).unwrap(), before);
            }
            delete_model(&MODELS[0], &path, &cfg.model).unwrap();
            model_changed(hwnd);
            assert_eq!(label(hwnd, MODEL_ACTION_FIRST), "Download");
        }
        close_dialog(hwnd);
        cleanup(&path);
    }

    #[test]
    fn native_shortcut_control_captures_function_and_media_keys() {
        unsafe {
            assert_ne!(
                InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                    dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                    dwICC: ICC_HOTKEY_CLASS,
                }),
                0
            );
            let hwnd = CreateWindowExW(
                0,
                windows_sys::w!("msctls_hotkey32"),
                null_mut(),
                0,
                0,
                0,
                180,
                24,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null_mut()),
                null_mut(),
            );
            assert!(!hwnd.is_null());
            SendMessageW(hwnd, HKM_SETRULES, 0, 0);
            for key in [120, 0xad, 0xae, 0xb3] {
                SendMessageW(hwnd, WM_KEYDOWN, key, 0);
                assert_eq!(
                    SendMessageW(hwnd, HKM_GETHOTKEY, 0, 0) & 0x7ff,
                    key as isize
                );
                SendMessageW(hwnd, WM_KEYUP, key, 0);
            }
            SendMessageW(hwnd, HKM_SETHOTKEY, 0x378, 0);
            assert_eq!(SendMessageW(hwnd, HKM_GETHOTKEY, 0, 0), 0x378);
            DestroyWindow(hwnd);
        }
    }
}
