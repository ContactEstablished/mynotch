//! Loading a WAV file as 16 kHz mono, so the pipeline can run on a recording instead of the
//! microphone.

use std::io;
use std::path::Path;

use hound::{SampleFormat, WavReader};

use super::{AudioError, convert};

/// Loads a WAV file (any rate, any channel count, 8/16/24/32-bit int or 32-bit float) as
/// 16 kHz mono f32.
pub fn load_wav(path: &Path) -> Result<Vec<f32>, AudioError> {
    let fail = |e: hound::Error| match e {
        hound::Error::IoError(e) if e.kind() == io::ErrorKind::NotFound => {
            AudioError::new(format!("No such file: {}", path.display()))
        }
        e => AudioError::new(format!("Can't read {}: {e}", path.display())),
    };
    let mut reader = WavReader::open(path).map_err(fail)?;
    let spec = reader.spec();
    let samples = match (spec.sample_format, spec.bits_per_sample) {
        (SampleFormat::Float, 32) => reader.samples::<f32>().collect::<Result<Vec<_>, _>>(),
        (SampleFormat::Int, bits @ 8..=32) => {
            let scale = 1.0 / (1u64 << (bits - 1)) as f32;
            reader.samples::<i32>().map(|s| s.map(|s| s as f32 * scale)).collect()
        }
        (format, bits) => {
            return Err(AudioError::new(format!(
                "Unsupported WAV format in {}: {bits}-bit {format:?}",
                path.display()
            )));
        }
    }
    .map_err(fail)?;
    convert::to_16k_mono(samples, usize::from(spec.channels), spec.sample_rate)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use hound::{WavSpec, WavWriter};

    use super::*;
    use crate::audio::convert::tests::sine;

    /// A temp file path no other test (or test run) uses; removed on drop.
    struct TempWav(PathBuf);

    impl TempWav {
        fn new(name: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let file = format!("mynotch-voice-{}-{n}-{name}.wav", std::process::id());
            Self(std::env::temp_dir().join(file))
        }
    }

    impl Drop for TempWav {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn spec(channels: u16, sample_rate: u32, bits: u16, format: SampleFormat) -> WavSpec {
        WavSpec { channels, sample_rate, bits_per_sample: bits, sample_format: format }
    }

    fn in_range(pcm: &[f32]) -> bool {
        pcm.iter().all(|s| (-1.0..=1.0).contains(s))
    }

    #[test]
    fn loads_44k1_stereo_i16() {
        let wav = TempWav::new("44k1-stereo-i16");
        let mut writer = WavWriter::create(&wav.0, spec(2, 44_100, 16, SampleFormat::Int)).unwrap();
        for s in sine(440.0, 0.5, 44_100, 1.0) {
            let v = (s * f32::from(i16::MAX)) as i16;
            writer.write_sample(v).unwrap();
            writer.write_sample(v).unwrap();
        }
        writer.finalize().unwrap();

        let pcm = load_wav(&wav.0).unwrap();
        assert!(pcm.len().abs_diff(16_000) <= 160, "{} samples", pcm.len());
        assert!(in_range(&pcm));
        let peak = pcm.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((peak - 0.5).abs() < 0.03, "peak {peak}");
    }

    #[test]
    fn loads_16k_mono_f32_unchanged() {
        let wav = TempWav::new("16k-mono-f32");
        let samples = sine(300.0, 0.8, 16_000, 0.5);
        let mut writer =
            WavWriter::create(&wav.0, spec(1, 16_000, 32, SampleFormat::Float)).unwrap();
        for &s in &samples {
            writer.write_sample(s).unwrap();
        }
        writer.finalize().unwrap();

        assert_eq!(load_wav(&wav.0).unwrap(), samples);
    }

    #[test]
    fn loads_48k_mono_i24() {
        let wav = TempWav::new("48k-mono-i24");
        let mut writer = WavWriter::create(&wav.0, spec(1, 48_000, 24, SampleFormat::Int)).unwrap();
        for s in sine(440.0, 0.9, 48_000, 2.0) {
            writer.write_sample((s * 8_388_607.0) as i32).unwrap();
        }
        writer.finalize().unwrap();

        let pcm = load_wav(&wav.0).unwrap();
        assert!(pcm.len().abs_diff(32_000) <= 320, "{} samples", pcm.len());
        assert!(in_range(&pcm));
        let rms = convert::rms(&pcm);
        assert!((rms - 0.9 * std::f32::consts::FRAC_1_SQRT_2).abs() < 0.02, "rms {rms}");
    }

    #[test]
    fn full_scale_i16_stays_in_range() {
        let wav = TempWav::new("full-scale-i16");
        let mut writer = WavWriter::create(&wav.0, spec(1, 16_000, 16, SampleFormat::Int)).unwrap();
        for v in [i16::MIN, i16::MAX, 0] {
            writer.write_sample(v).unwrap();
        }
        writer.finalize().unwrap();

        assert_eq!(load_wav(&wav.0).unwrap(), vec![-1.0, 32_767.0 / 32_768.0, 0.0]);
    }

    #[test]
    fn a_missing_file_is_an_error() {
        let wav = TempWav::new("missing");
        let err = load_wav(&wav.0).unwrap_err();
        assert!(err.to_string().starts_with("No such file"), "{err}");
    }

    #[test]
    fn a_file_that_isnt_a_wav_is_an_error() {
        let wav = TempWav::new("not-a-wav");
        std::fs::write(&wav.0, b"this is not a wav file").unwrap();
        assert!(load_wav(&wav.0).is_err());
    }
}
