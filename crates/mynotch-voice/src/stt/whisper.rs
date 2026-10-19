//! Whisper via whisper.cpp (`whisper-rs`), built with CUDA. The model and one decoding state
//! are loaded once and kept resident; sessions take turns on the state.

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState};

use super::{SttEngine, SttError, SttOptions, Transcript};
use crate::audio::SAMPLE_RATE;

/// Whisper is given at least this much audio; shorter clips are padded with silence.
const MIN_SAMPLES: usize = SAMPLE_RATE as usize;
/// Decoder threads. The GPU does the heavy work; these only drive it.
const THREADS: i32 = 4;

pub struct WhisperEngine {
    /// Holds on to the model itself.
    state: Mutex<WhisperState>,
}

impl WhisperEngine {
    /// Loads a ggml model file onto the GPU. whisper.cpp falls back to the CPU if CUDA finds no
    /// device; `platform::win::gpu::devices` tells.
    pub fn load(model: &Path, flash_attn: bool) -> Result<Self, SttError> {
        let mut params = WhisperContextParameters::default();
        params.use_gpu(true).flash_attn(flash_attn);
        let context = WhisperContext::new_with_params(model, params)
            .map_err(|e| SttError(format!("Couldn't load the model {}: {e}", model.display())))?;
        let state = context.create_state().map_err(|e| SttError(format!("Couldn't set up Whisper: {e}")))?;
        Ok(Self { state: Mutex::new(state) })
    }

    /// Runs one second of silence through, so the first real session isn't the slow one.
    pub fn warm_up(&self) -> Result<Duration, SttError> {
        let start = Instant::now();
        self.transcribe(&[0.0; MIN_SAMPLES], &SttOptions::default())?;
        Ok(start.elapsed())
    }

    /// whisper.cpp's build features ("CUDA : ARCHS = 890 | ...").
    pub fn system_info() -> &'static str {
        whisper_rs::print_system_info()
    }
}

impl SttEngine for WhisperEngine {
    fn transcribe(&self, pcm: &[f32], opts: &SttOptions) -> Result<Transcript, SttError> {
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(&opts.language));
        params.set_n_threads(THREADS);
        params.set_no_timestamps(true);
        params.set_no_context(true);
        // 0 to start with; whisper.cpp still falls back to higher temperatures on a bad decode.
        params.set_temperature(0.0);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        let mut padded = Vec::new();
        let pcm = if pcm.len() < MIN_SAMPLES {
            padded.extend_from_slice(pcm);
            padded.resize(MIN_SAMPLES, 0.0);
            &padded[..]
        } else {
            pcm
        };
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.full(params, pcm).map_err(|e| SttError(format!("Whisper failed: {e}")))?;
        let text: String = state.as_iter().filter_map(|s| s.to_str_lossy().ok().map(|t| t.into_owned())).collect();
        Ok(Transcript { text: tidy(&text) })
    }
}

/// Whisper's text, trimmed, or empty when all it heard were tags like "[BLANK_AUDIO]" or
/// "(silence)" (with any punctuation around them).
fn tidy(text: &str) -> String {
    let text = text.trim();
    let mut rest = text;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c.is_ascii_punctuation() && c != '[' && c != '(');
        let close = match rest.chars().next() {
            Some('[') => ']',
            Some('(') => ')',
            Some(_) => return text.to_owned(),
            None => return String::new(),
        };
        match rest.find(close) {
            Some(end) => rest = &rest[end + 1..],
            None => return text.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tidy;

    #[test]
    fn speech_is_trimmed_and_tags_alone_are_nothing() {
        assert_eq!(tidy(" Open the settings file."), "Open the settings file.");
        assert_eq!(tidy(" [BLANK_AUDIO]"), "");
        assert_eq!(tidy("(silence) [BLANK_AUDIO] "), "");
        assert_eq!(tidy("  "), "");
        assert_eq!(tidy("[Music] Hello there"), "[Music] Hello there", "speech with a tag stays as it is");
        assert_eq!(tidy("(unclosed"), "(unclosed");
        assert_eq!(tidy("[BLANK_AUDIO]."), "", "a tag with punctuation is still nothing");
        assert_eq!(tidy("..."), "");
        assert_eq!(tidy("Done."), "Done.");
    }
}
