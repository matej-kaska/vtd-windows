use windows::Win32::{
    Foundation::RPC_E_CHANGED_MODE,
    Media::Audio::{
        Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator, MMDeviceEnumerator, eMultimedia,
        eRender,
    },
    System::Com::{
        CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
    },
};

pub struct OutputMute {
    volume: Option<IAudioEndpointVolume>,
    initialized: bool,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl OutputMute {
    pub fn new() -> windows::core::Result<Self> {
        unsafe {
            let result = CoInitializeEx(None, COINIT_MULTITHREADED);
            if result != RPC_E_CHANGED_MODE {
                result.ok()?;
            }
            let mut guard = Self {
                volume: None,
                initialized: result.is_ok(),
                _thread: std::marker::PhantomData,
            };
            let devices: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let volume: IAudioEndpointVolume = devices
                .GetDefaultAudioEndpoint(eRender, eMultimedia)?
                .Activate(CLSCTX_ALL, None)?;
            if !volume.GetMute()?.as_bool() {
                volume.SetMute(true, std::ptr::null())?;
                guard.volume = Some(volume);
            }
            Ok(guard)
        }
    }
}

impl Drop for OutputMute {
    fn drop(&mut self) {
        unsafe {
            if let Some(volume) = self.volume.take()
                && let Err(e) = volume.SetMute(false, std::ptr::null())
            {
                eprintln!("VTD restore output: {e}");
            }
            if self.initialized {
                CoUninitialize();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, recording::Recording};

    #[test]
    #[ignore = "uses local playback and microphone devices"]
    fn recording_restores_mute_on_finish_and_cancel() -> anyhow::Result<()> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            struct Restore(IAudioEndpointVolume, bool);
            impl Drop for Restore {
                fn drop(&mut self) {
                    unsafe {
                        let _ = self.0.SetMute(self.1, std::ptr::null());
                    }
                }
            }
            let devices: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let volume: IAudioEndpointVolume = devices
                .GetDefaultAudioEndpoint(eRender, eMultimedia)?
                .Activate(CLSCTX_ALL, None)?;
            let original = volume.GetMute()?.as_bool();
            let restore = Restore(volume.clone(), original);
            let level = volume.GetMasterVolumeLevelScalar()?;
            let cfg = Config {
                mute_output: true,
                ..Config::default()
            };
            for initially_muted in [original, true] {
                volume.SetMute(initially_muted, std::ptr::null())?;
                for finish in [true, false] {
                    let recording = Recording::start(&cfg)?;
                    assert!(volume.GetMute()?.as_bool());
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    if finish {
                        recording.finish()?;
                    } else {
                        drop(recording);
                    }
                    assert_eq!(volume.GetMute()?.as_bool(), initially_muted);
                    assert_eq!(volume.GetMasterVolumeLevelScalar()?, level);
                }
            }
            drop(restore);
            assert_eq!(volume.GetMute()?.as_bool(), original);
            drop(volume);
            drop(devices);
            CoUninitialize();
        }
        Ok(())
    }
}
