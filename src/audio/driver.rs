//! Audio output subsystem.
//!
//! Maps guest audio (Xbox One/Series X use standard PCM over HDMI/optical;
//! original Xbox used custom DAC/I2S) to the host audio API:
//! WASAPI on Windows, Core Audio on macOS, ALSA/PulseAudio on Linux.

use crate::config::AudioConfig;

/// Host audio backend (selected at runtime by `AudioConfig.backend`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioBackend {
    Wasapi,
    CoreAudio,
    Alsa,
    Pulse,
    /// Null backend for headless/CI runs.
    Null,
}

impl AudioBackend {
    pub fn from_name(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "wasapi" => Self::Wasapi,
            "coreaudio" => Self::CoreAudio,
            "alsa" => Self::Alsa,
            "pulse" | "pulseaudio" => Self::Pulse,
            _ => Self::Null,
        }
    }
}

/// An opened audio output stream.
pub struct AudioDriver {
    backend: AudioBackend,
    sample_rate: u32,
    buffer_size: u32,
    volume: f32,
    /// Frames of silence mixed so far (scaffold bookkeeping).
    pub frames_mixed: u64,
}

impl AudioDriver {
    /// Open an audio device per configuration. Falls back to [`AudioBackend::Null`]
    /// if the requested backend is unavailable.
    pub fn open(config: &AudioConfig) -> Self {
        let backend = AudioBackend::from_name(&config.backend);
        Self {
            backend,
            sample_rate: config.sample_rate,
            buffer_size: config.buffer_size,
            volume: config.volume,
            frames_mixed: 0,
        }
    }

    pub fn backend(&self) -> AudioBackend {
        self.backend
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn buffer_size(&self) -> u32 {
        self.buffer_size
    }

    /// Mix a buffer of interleaved f32 stereo samples and submit to the device.
    ///
    /// Scaffold: samples are volume-scaled and counted; real backends would
    /// hand them to WASAPI/Core Audio/ALSA ring buffers.
    pub fn submit(&mut self, samples: &mut [f32]) {
        let v = self.volume;
        for s in samples.iter_mut() {
            *s *= v;
        }
        self.frames_mixed += (samples.len() / 2) as u64;
    }
}
