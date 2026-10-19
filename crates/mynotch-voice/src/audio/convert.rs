//! Pure conversions to what Whisper takes: one channel at 16 kHz, -1..1. Capture and
//! `load_wav` both end here.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler, WindowFunction};

use super::{AudioError, SAMPLE_RATE};

/// The resampler's input chunk in frames, as one FFT block. A long block keeps the
/// anti-aliasing cutoff near 8 kHz; its delay doesn't matter, as `process_all` trims it.
const CHUNK: usize = 1024;

/// Interleaved audio at `rate` Hz with `channels` channels, as 16 kHz mono clamped to -1..1.
pub(crate) fn to_16k_mono(
    samples: Vec<f32>,
    channels: usize,
    rate: u32,
) -> Result<Vec<f32>, AudioError> {
    let mut pcm = resample_to_16k(to_mono(samples, channels), rate)?;
    for s in &mut pcm {
        *s = s.clamp(-1.0, 1.0);
    }
    Ok(pcm)
}

/// Averages each frame of interleaved audio into one sample, in place. A trailing partial
/// frame is dropped.
pub(crate) fn to_mono(mut samples: Vec<f32>, channels: usize) -> Vec<f32> {
    match channels {
        0 => samples.clear(),
        1 => {}
        _ => {
            let frames = samples.len() / channels;
            let scale = 1.0 / channels as f32;
            // Frame `i` starts at `i * channels >= i`, so writing sample `i` never clobbers a
            // frame that's still to be read.
            for i in 0..frames {
                let sum: f32 = samples[i * channels..(i + 1) * channels].iter().sum();
                samples[i] = sum * scale;
            }
            samples.truncate(frames);
        }
    }
    samples
}

/// Resamples mono audio at `rate` Hz to 16 kHz with rubato's FFT resampler. Audio already at
/// 16 kHz is returned as is.
pub(crate) fn resample_to_16k(mono: Vec<f32>, rate: u32) -> Result<Vec<f32>, AudioError> {
    if rate == SAMPLE_RATE || mono.is_empty() {
        return Ok(mono);
    }
    let fail =
        |e: &dyn std::fmt::Display| AudioError::new(format!("Can't resample {rate} Hz audio: {e}"));
    let mut resampler = Fft::<f32>::new_custom(
        rate as usize,
        SAMPLE_RATE as usize,
        CHUNK,
        1,
        1,
        WindowFunction::BlackmanHarris2,
        FixedSync::Input,
    )
    .map_err(|e| fail(&e))?;
    let input = InterleavedSlice::new(&mono, 1, mono.len()).map_err(|e| fail(&e))?;
    let output = resampler.process_all(&input, mono.len(), None).map_err(|e| fail(&e))?;
    Ok(output.take_data())
}

/// Root mean square (0 for no samples).
pub(crate) fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    (sum / samples.len() as f64).sqrt() as f32
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// `seconds` of a sine at `freq` Hz and `amplitude`, sampled at `rate` Hz.
    pub(crate) fn sine(freq: f32, amplitude: f32, rate: u32, seconds: f32) -> Vec<f32> {
        let n = (rate as f32 * seconds) as usize;
        (0..n)
            .map(|i| amplitude * (std::f32::consts::TAU * freq * i as f32 / rate as f32).sin())
            .collect()
    }

    fn zero_crossings(samples: &[f32]) -> usize {
        samples.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count()
    }

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0, |m, s| m.max(s.abs()))
    }

    #[test]
    fn stereo_is_averaged() {
        assert_eq!(to_mono(vec![1.0, 0.0, -0.5, -0.5, 0.25, 0.75], 2), vec![0.5, -0.5, 0.5]);
    }

    #[test]
    fn more_channels_are_averaged() {
        assert_eq!(to_mono(vec![0.3, 0.6, 0.9, -0.3, -0.6, -0.9], 3), vec![0.6, -0.6]);
    }

    #[test]
    fn mono_passes_through() {
        let samples = vec![0.1, -0.2, 0.3];
        assert_eq!(to_mono(samples.clone(), 1), samples);
    }

    #[test]
    fn partial_trailing_frame_is_dropped() {
        assert_eq!(to_mono(vec![1.0, 1.0, 0.5], 2), vec![1.0]);
        assert_eq!(to_mono(vec![0.5], 2), Vec::<f32>::new());
        assert_eq!(to_mono(vec![0.5, 0.5], 0), Vec::<f32>::new());
    }

    #[test]
    fn one_second_at_48k_becomes_16k_samples() {
        let out = resample_to_16k(vec![0.0; 48_000], 48_000).unwrap();
        assert!(out.len().abs_diff(16_000) <= 160, "{} samples", out.len());
    }

    #[test]
    fn one_second_at_44k1_becomes_16k_samples() {
        let out = resample_to_16k(vec![0.0; 44_100], 44_100).unwrap();
        assert!(out.len().abs_diff(16_000) <= 160, "{} samples", out.len());
    }

    #[test]
    fn audio_at_16k_passes_through() {
        let samples = sine(440.0, 0.5, 16_000, 0.1);
        assert_eq!(resample_to_16k(samples.clone(), 16_000).unwrap(), samples);
    }

    #[test]
    fn no_audio_stays_empty() {
        assert!(resample_to_16k(Vec::new(), 48_000).unwrap().is_empty());
    }

    #[test]
    fn a_sine_keeps_its_pitch_and_level() {
        for rate in [48_000, 44_100, 22_050] {
            let input = sine(440.0, 0.5, rate, 1.0);
            let out = resample_to_16k(input.clone(), rate).unwrap();
            // A 440 Hz sine crosses zero 880 times a second.
            let crossings = zero_crossings(&out);
            assert!(crossings.abs_diff(880) <= 9, "{rate} Hz: {crossings} crossings");
            // Skip the edges, where the resampler's filter rings.
            let middle = &out[1_600..out.len() - 1_600];
            assert!((peak(middle) - 0.5).abs() < 0.02, "{rate} Hz: peak {}", peak(middle));
            assert!((rms(middle) - rms(&input)).abs() < 0.01, "{rate} Hz: rms {}", rms(middle));
        }
    }

    #[test]
    fn output_is_clamped() {
        let out = to_16k_mono(vec![1.5, -2.0, 0.25], 1, 16_000).unwrap();
        assert_eq!(out, vec![1.0, -1.0, 0.25]);
        let loud = to_16k_mono(sine(440.0, 1.0, 48_000, 0.5), 1, 48_000).unwrap();
        assert!(loud.iter().all(|s| (-1.0..=1.0).contains(s)));
    }

    #[test]
    fn rms_of_known_signals() {
        assert_eq!(rms(&[]), 0.0);
        assert!((rms(&[0.5, -0.5, 0.5, -0.5]) - 0.5).abs() < 1e-6);
        let sine = sine(440.0, 1.0, 16_000, 1.0);
        assert!((rms(&sine) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3);
    }
}
