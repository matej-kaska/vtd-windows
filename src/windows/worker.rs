use super::{config::Config, engine::Engine};
use anyhow::{Result, ensure};
use std::{
    io::{BufRead, BufReader, BufWriter, Read, Write},
    os::windows::process::CommandExt,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

pub struct Worker {
    child: Child,
    input: BufWriter<ChildStdin>,
    output: BufReader<ChildStdout>,
}

impl Worker {
    pub fn spawn() -> Result<Self> {
        let mut child = Command::new(std::env::current_exe()?)
            .arg("__worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
            .spawn()?;
        Ok(Self {
            input: BufWriter::new(child.stdin.take().unwrap()),
            output: BufReader::new(child.stdout.take().unwrap()),
            child,
        })
    }

    pub fn transcribe(&mut self, samples: &[f32]) -> Result<String> {
        self.input
            .write_all(&(samples.len() as u32).to_le_bytes())?;
        let bytes = unsafe {
            std::slice::from_raw_parts(
                samples.as_ptr().cast::<u8>(),
                std::mem::size_of_val(samples),
            )
        };
        self.input.write_all(bytes)?;
        self.input.flush()?;
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

pub fn run(cfg: &Config) -> Result<()> {
    let mut engine = Engine::load(cfg)?;
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    loop {
        let mut count = [0; 4];
        match input.read_exact(&mut count) {
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            result => result?,
        }
        let count = u32::from_le_bytes(count) as usize;
        ensure!(count <= 16000 * 300, "Recording exceeds 5 minutes");
        let mut samples = vec![0.0f32; count];
        let bytes =
            unsafe { std::slice::from_raw_parts_mut(samples.as_mut_ptr().cast::<u8>(), count * 4) };
        input.read_exact(bytes)?;
        let result = engine
            .transcribe(cfg, &samples)
            .map_err(|e| format!("{e:#}"));
        serde_json::to_writer(&mut output, &result)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}
