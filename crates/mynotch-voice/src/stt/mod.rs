//! Speech to text. One engine today, Whisper (`whisper`); others would sit behind the same trait.

pub mod whisper;

use std::fmt;

pub use whisper::WhisperEngine;

#[derive(Clone, Debug)]
pub struct SttOptions {
    /// Spoken language, e.g. "en".
    pub language: String,
}

impl Default for SttOptions {
    fn default() -> Self {
        Self { language: "en".into() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transcript {
    /// What was said, trimmed; empty if nothing was recognised.
    pub text: String,
}

#[derive(Debug)]
pub struct SttError(pub String);

impl fmt::Display for SttError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SttError {}

pub trait SttEngine: Send + Sync {
    /// Transcribes 16 kHz mono audio.
    fn transcribe(&self, pcm_16k_mono: &[f32], opts: &SttOptions) -> Result<Transcript, SttError>;
}
