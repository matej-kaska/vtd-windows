use anyhow::{Result, ensure};
use std::{
    io::{BufRead, BufReader, Write},
    os::windows::process::CommandExt,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

pub fn command() -> Result<Command> {
    let path = std::env::current_exe()?.with_file_name("vtd-engine.exe");
    ensure!(
        path.is_file(),
        "Missing vtd-engine.exe; extract the complete VTD package"
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
}

impl Worker {
    pub fn spawn() -> Result<Self> {
        let mut child = command()?
            .arg("__worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        Ok(Self {
            input: child.stdin.take().unwrap(),
            output: BufReader::with_capacity(1024, child.stdout.take().unwrap()),
            child,
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
