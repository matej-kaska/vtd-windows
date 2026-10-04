// Native integration tests can feed a known WAV through the same recording lifecycle.
// Production builds use the microphone directly, without a wrapper or environment lookup.
#[cfg(not(feature = "resident-test"))]
pub use vtd::recording::Recording;

#[cfg(feature = "resident-test")]
pub struct Recording {
    live: Option<vtd::recording::Recording>,
    fixture: Option<Vec<f32>>,
}

#[cfg(feature = "resident-test")]
impl Recording {
    pub fn start(cfg: &vtd::config::Config) -> anyhow::Result<Self> {
        if let Some(path) = std::env::var_os("VTD_TEST_WAV") {
            Ok(Self {
                live: None,
                fixture: Some(vtd::audio::read_wav(std::path::Path::new(&path))?),
            })
        } else {
            Ok(Self {
                live: Some(vtd::recording::Recording::start(cfg)?),
                fixture: None,
            })
        }
    }

    pub fn finish(mut self) -> anyhow::Result<(Vec<f32>, u32)> {
        if let Some(samples) = self.fixture.take() {
            Ok((samples, 16000))
        } else {
            self.live.take().unwrap().finish()
        }
    }
}
