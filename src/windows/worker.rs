use anyhow::{Result, ensure};
use std::{
    io::{BufRead, BufReader, Write},
    os::windows::io::AsRawHandle,
    os::windows::process::CommandExt,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::JobObjects::*,
};

struct ProcessJob(HANDLE);

impl ProcessJob {
    fn new() -> Result<Self> {
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            ensure!(!handle.is_null(), "Cannot create transcription process job");
            let job = Self(handle);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            ensure!(
                SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of_val(&limits) as u32,
                ) != 0,
                "Cannot protect transcription process cleanup"
            );
            Ok(job)
        }
    }
}

impl Drop for ProcessJob {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub fn command() -> Result<Command> {
    let cfg = vtd::config::Config::load(&vtd::config::path()?)?;
    let name = match vtd::models::engine(&cfg.model) {
        vtd::models::EngineKind::Whisper => "vtd-engine.exe",
        vtd::models::EngineKind::Transcribe => "vtd-transcribe.exe",
    };
    let path = std::env::current_exe()?.with_file_name(name);
    ensure!(
        path.is_file(),
        "Missing {name}; extract the complete VTD package"
    );
    let mut command = Command::new(path);
    command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    command.env("DISABLE_VULKAN_OBS_CAPTURE", "1");
    Ok(command)
}

pub struct Worker {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    _job: ProcessJob,
}

impl Worker {
    pub fn spawn() -> Result<Self> {
        let job = ProcessJob::new()?;
        let mut child = command()?
            .arg("__worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        if unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle().cast()) } == 0 {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("Cannot attach the transcription process to its cleanup job");
        }
        Ok(Self {
            input: child.stdin.take().unwrap(),
            output: BufReader::with_capacity(1024, child.stdout.take().unwrap()),
            child,
            _job: job,
        })
    }

    pub fn transcribe(&mut self, samples: Vec<f32>) -> Result<String> {
        self.input
            .write_all(&(samples.len() as u32).to_le_bytes())?;
        let bytes = unsafe {
            std::slice::from_raw_parts(
                samples.as_ptr().cast::<u8>(),
                std::mem::size_of_val(samples.as_slice()),
            )
        };
        self.input.write_all(bytes)?;
        drop(samples);
        let mut reply = String::new();
        ensure!(
            self.output.read_line(&mut reply)? > 0,
            "Transcription worker stopped"
        );
        serde_json::from_str::<Result<String, String>>(&reply)?.map_err(anyhow::Error::msg)
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
