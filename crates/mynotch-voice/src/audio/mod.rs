//! Microphone capture and WAV loading, both ending as 16 kHz mono f32 for Whisper.
//!
//! cpal's data callback only converts samples to f32 and pushes them into a lock-free ring
//! buffer. A "voice-audio" thread owns the stream (a `cpal::Stream` isn't `Send`), moves the
//! ring's contents into memory every 10 ms, and on stop downmixes and resamples it all.

mod convert;
mod wav;

use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ErrorKind, FromSample, I24, Sample, SampleFormat, SizedSample, StreamConfig, U24};
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};

pub(crate) use convert::rms;
pub use wav::load_wav;

/// Whisper's input rate.
pub const SAMPLE_RATE: u32 = 16_000;
/// The hard cap on one session's audio.
pub const MAX_SECONDS: u32 = 300;

/// How often the audio thread empties the ring buffer.
const DRAIN_EVERY: Duration = Duration::from_millis(10);
/// How much audio the ring buffer holds.
const RING_SECONDS: usize = 1;

/// Why the microphone or a WAV file couldn't give us audio, in words fit for the log.
#[derive(Debug)]
pub struct AudioError(String);

impl AudioError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AudioError {}

/// A capture in progress from the default input device (WASAPI shared mode). Dropping it
/// without `finish` stops capture and discards the audio.
pub struct Recording {
    describe: String,
    /// A message means finish; hanging up means discard.
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<Option<Result<Captured, AudioError>>>>,
}

/// Everything one `Recording` heard.
#[derive(Debug)]
pub struct Captured {
    /// 16 kHz mono.
    pub pcm: Vec<f32>,
    /// Audio beyond `MAX_SECONDS` was dropped.
    pub truncated: bool,
    /// Frames lost because the ring buffer overflowed (should be 0).
    pub overflowed: usize,
    /// RMS of `pcm`, for the log.
    pub rms: f32,
}

impl Recording {
    /// Opens the default input device with its default config and starts capturing.
    /// Returns once the stream is playing, or the error.
    pub fn start() -> Result<Recording, AudioError> {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("voice-audio".into())
            .spawn(move || capture(&ready_tx, &stop_rx))
            .map_err(|e| AudioError::new(format!("Can't start the audio thread: {e}")))?;
        match ready_rx.recv() {
            Ok(Ok(describe)) => {
                Ok(Recording { describe, stop: Some(stop_tx), thread: Some(thread) })
            }
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err(AudioError::new("The audio thread stopped"))
            }
        }
    }

    /// One line describing the device and format for the log, e.g.
    /// "Microphone (Realtek) · 48000 Hz · 2 ch · f32".
    pub fn describe(&self) -> &str {
        &self.describe
    }

    /// Stops capture and returns everything captured as 16 kHz mono f32 (-1..1).
    /// Fails if the stream reported an error during capture (e.g. the device was unplugged).
    pub fn finish(mut self) -> Result<Captured, AudioError> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let Some(thread) = self.thread.take() else {
            return Err(AudioError::new("The audio thread stopped"));
        };
        match thread.join() {
            Ok(Some(result)) => result,
            Ok(None) | Err(_) => Err(AudioError::new("The audio thread stopped")),
        }
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        // Hanging up tells the thread to discard; joining means the microphone is released
        // by the time this returns.
        self.stop.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The "voice-audio" thread: opens the microphone, reports back to `start`, then gathers
/// audio until told to finish (`Some`) or discard (`None`).
fn capture(
    ready: &Sender<Result<String, AudioError>>,
    stop: &Receiver<()>,
) -> Option<Result<Captured, AudioError>> {
    let mic = match Mic::open() {
        Ok(mic) => mic,
        Err(e) => {
            let _ = ready.send(Err(e));
            return None;
        }
    };
    ready.send(Ok(mic.describe.clone())).ok()?;
    mic.run(stop)
}

/// What the stream's callbacks share with the audio thread.
#[derive(Default)]
struct Shared {
    /// Samples the data callback couldn't fit into the ring.
    dropped: AtomicUsize,
    /// Glitches the backend reported; capture carried on.
    xruns: AtomicUsize,
    /// The first error that ended capture.
    error: Mutex<Option<String>>,
}

impl Shared {
    /// cpal's error callback. It isn't the real-time data path, so it may lock and allocate.
    fn report(&self, err: &cpal::Error) {
        match err.kind() {
            ErrorKind::Xrun => {
                self.xruns.fetch_add(1, Ordering::Relaxed);
            }
            // The stream carries on in both cases.
            ErrorKind::RealtimeDenied | ErrorKind::DeviceChanged => {}
            _ => {
                if let Ok(mut error) = self.error.lock() {
                    error.get_or_insert_with(|| stream_error(err));
                }
            }
        }
    }
}

/// An open, playing input stream and the ring buffer it fills.
struct Mic {
    stream: cpal::Stream,
    ring: HeapCons<f32>,
    rate: u32,
    channels: usize,
    shared: Arc<Shared>,
    describe: String,
}

impl Mic {
    fn open() -> Result<Mic, AudioError> {
        let device = cpal::default_host()
            .default_input_device()
            .ok_or_else(|| AudioError::new("No microphone found"))?;
        let supported = device.default_input_config().map_err(|e| open_error(&e))?;
        let format = supported.sample_format();
        let config = supported.config();
        let rate = config.sample_rate;
        let channels = usize::from(config.channels);
        if channels == 0 || rate == 0 {
            return Err(AudioError::new(format!(
                "The microphone reports {channels} channels at {rate} Hz"
            )));
        }
        let name = device
            .description()
            .map(|d| d.name().to_owned())
            .unwrap_or_else(|_| "Microphone".to_owned());
        let describe = format!("{name} · {rate} Hz · {channels} ch · {format}");

        let shared = Arc::new(Shared::default());
        let (prod, ring) = HeapRb::<f32>::new(rate as usize * channels * RING_SECONDS).split();
        let s = Arc::clone(&shared);
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, config, prod, s),
            SampleFormat::F64 => build::<f64>(&device, config, prod, s),
            SampleFormat::I8 => build::<i8>(&device, config, prod, s),
            SampleFormat::I16 => build::<i16>(&device, config, prod, s),
            SampleFormat::I24 => build::<I24>(&device, config, prod, s),
            SampleFormat::I32 => build::<i32>(&device, config, prod, s),
            SampleFormat::U8 => build::<u8>(&device, config, prod, s),
            SampleFormat::U16 => build::<u16>(&device, config, prod, s),
            SampleFormat::U24 => build::<U24>(&device, config, prod, s),
            SampleFormat::U32 => build::<u32>(&device, config, prod, s),
            other => {
                return Err(AudioError::new(format!("Unsupported microphone format: {other}")));
            }
        }
        .map_err(|e| open_error(&e))?;
        stream.play().map_err(|e| open_error(&e))?;
        Ok(Mic { stream, ring, rate, channels, shared, describe })
    }

    /// Gathers audio until `stop` says finish (`Some`) or hangs up (`None`, discarded).
    fn run(self, stop: &Receiver<()>) -> Option<Result<Captured, AudioError>> {
        let Mic { stream, mut ring, rate, channels, shared, .. } = self;
        let mut audio = Accumulator::for_device(rate, channels);
        loop {
            match stop.recv_timeout(DRAIN_EVERY) {
                Err(RecvTimeoutError::Timeout) => audio.drain(&mut ring),
                Ok(()) => break,
                Err(RecvTimeoutError::Disconnected) => return None,
            }
        }
        // Dropping the stream joins cpal's thread, so nothing is pushed after this drain.
        drop(stream);
        audio.drain(&mut ring);

        if let Some(error) = shared.error.lock().ok().and_then(|mut e| e.take()) {
            return Some(Err(AudioError::new(error)));
        }
        let xruns = shared.xruns.load(Ordering::Relaxed);
        if xruns > 0 {
            tracing::warn!("the microphone glitched {xruns} times during capture");
        }
        let overflowed = shared.dropped.load(Ordering::Relaxed).div_ceil(channels);
        let truncated = audio.truncated;
        let started = Instant::now();
        let frames = audio.samples.len() / channels;
        let pcm = match convert::to_16k_mono(audio.samples, channels, rate) {
            Ok(pcm) => pcm,
            Err(e) => return Some(Err(e)),
        };
        tracing::debug!(
            "converted {frames} frames at {rate} Hz × {channels} ch to {} samples in {:.1} ms",
            pcm.len(),
            started.elapsed().as_secs_f64() * 1000.0
        );
        let rms = convert::rms(&pcm);
        Some(Ok(Captured { pcm, truncated, overflowed, rms }))
    }
}

/// Builds the input stream for device samples of type `T`.
fn build<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mut prod: HeapProd<f32>,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels);
    let data_shared = Arc::clone(&shared);
    device.build_input_stream::<T, _, _>(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            push_frames(&mut prod, data, channels, &data_shared.dropped);
        },
        move |err| shared.report(&err),
        None,
    )
}

/// The data callback's whole job: convert to f32 and push whole frames into the ring, without
/// allocating, locking or waiting. Samples that don't fit are counted in `dropped`.
fn push_frames<T, P>(prod: &mut P, data: &[T], channels: usize, dropped: &AtomicUsize)
where
    T: Sample,
    f32: FromSample<T>,
    P: Producer<Item = f32>,
{
    // Whole frames only, so the ring never holds half a frame and channels stay in step.
    let room = prod.vacant_len() / channels * channels;
    let n = data.len().min(room);
    prod.push_iter(data[..n].iter().map(|&s| f32::from_sample(s)));
    if n < data.len() {
        dropped.fetch_add(data.len() - n, Ordering::Relaxed);
    }
}

/// Interleaved device-rate audio gathered so far, capped at `cap` samples.
struct Accumulator {
    samples: Vec<f32>,
    cap: usize,
    /// Audio arrived after the cap was reached and was dropped.
    truncated: bool,
}

impl Accumulator {
    /// Room for `MAX_SECONDS` at `rate` Hz × `channels`, with memory for the first 10 s up front.
    fn for_device(rate: u32, channels: usize) -> Self {
        let per_second = rate as usize * channels;
        Self::new(MAX_SECONDS as usize * per_second, 10 * per_second)
    }

    fn new(cap: usize, reserve: usize) -> Self {
        Self { samples: Vec::with_capacity(reserve.min(cap)), cap, truncated: false }
    }

    fn push(&mut self, chunk: &[f32]) {
        let room = self.cap - self.samples.len();
        if chunk.len() > room {
            self.truncated = true;
        }
        self.samples.extend_from_slice(&chunk[..chunk.len().min(room)]);
    }

    /// Moves everything in the ring into `samples`.
    fn drain<C: Consumer<Item = f32>>(&mut self, ring: &mut C) {
        let (a, b) = ring.as_slices();
        let n = a.len() + b.len();
        self.push(a);
        self.push(b);
        ring.skip(n);
    }
}

/// An error opening the microphone.
fn open_error(err: &cpal::Error) -> AudioError {
    AudioError::new(match err.kind() {
        ErrorKind::DeviceNotAvailable => "The microphone isn't available".to_owned(),
        ErrorKind::DeviceBusy => "The microphone is busy".to_owned(),
        ErrorKind::PermissionDenied => "Microphone access is denied".to_owned(),
        _ => format!("Can't open the microphone: {err}"),
    })
}

/// An error that ended capture.
fn stream_error(err: &cpal::Error) -> String {
    match err.kind() {
        ErrorKind::DeviceNotAvailable => "The microphone was disconnected".to_owned(),
        ErrorKind::StreamInvalidated => "The microphone changed".to_owned(),
        ErrorKind::PermissionDenied => "Microphone access is denied".to_owned(),
        _ => format!("Microphone error: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use ringbuf::traits::Observer;

    use super::*;

    #[test]
    fn recording_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Recording>();
    }

    #[test]
    fn device_samples_become_f32() {
        let dropped = AtomicUsize::new(0);
        let (mut prod, mut cons) = HeapRb::<f32>::new(16).split();
        push_frames(&mut prod, &[i16::MIN, 0, i16::MAX / 2], 1, &dropped);
        push_frames(&mut prod, &[0u16, 32_768, u16::MAX], 1, &dropped);
        push_frames(&mut prod, &[i32::MIN, 0], 1, &dropped);
        push_frames(&mut prod, &[0.25f32, -0.75], 1, &dropped);
        let got: Vec<f32> = cons.pop_iter().collect();
        let want = [-1.0, 0.0, 0.5, -1.0, 0.0, 1.0, -1.0, 0.0, 0.25, -0.75];
        assert_eq!(got.len(), want.len());
        for (g, w) in got.iter().zip(want) {
            assert!((g - w).abs() < 1e-3, "{got:?}");
        }
        assert_eq!(dropped.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_full_ring_drops_whole_frames_and_counts_them() {
        let dropped = AtomicUsize::new(0);
        let (mut prod, mut cons) = HeapRb::<f32>::new(5).split();
        // Room for 5 samples is room for 2 stereo frames; the third frame is dropped whole.
        push_frames(&mut prod, &[0.1f32, 0.2, 0.3, 0.4, 0.5, 0.6], 2, &dropped);
        assert_eq!(dropped.load(Ordering::Relaxed), 2);
        push_frames(&mut prod, &[0.7f32, 0.8], 2, &dropped);
        assert_eq!(dropped.load(Ordering::Relaxed), 4);
        assert_eq!(cons.pop_iter().collect::<Vec<_>>(), vec![0.1, 0.2, 0.3, 0.4]);
        push_frames(&mut prod, &[0.7f32, 0.8], 2, &dropped);
        assert_eq!(cons.pop_iter().collect::<Vec<_>>(), vec![0.7, 0.8]);
    }

    #[test]
    fn the_cap_truncates() {
        let mut audio = Accumulator::new(6, 100);
        audio.push(&[0.1, 0.2, 0.3, 0.4]);
        assert!(!audio.truncated);
        audio.push(&[0.5, 0.6]);
        assert!(!audio.truncated, "exactly at the cap isn't truncated");
        audio.push(&[0.7, 0.8]);
        assert!(audio.truncated);
        audio.push(&[]);
        assert_eq!(audio.samples, vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6]);
    }

    #[test]
    fn draining_keeps_order_across_the_ring_wrap() {
        let dropped = AtomicUsize::new(0);
        let (mut prod, mut ring) = HeapRb::<f32>::new(4).split();
        let mut audio = Accumulator::new(100, 0);
        push_frames(&mut prod, &[1.0f32, 2.0, 3.0], 1, &dropped);
        audio.drain(&mut ring);
        // These wrap around the end of the ring's storage.
        push_frames(&mut prod, &[4.0f32, 5.0, 6.0], 1, &dropped);
        audio.drain(&mut ring);
        audio.drain(&mut ring);
        assert_eq!(audio.samples, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert!(ring.is_empty());
    }

    #[test]
    fn the_cap_is_max_seconds_of_audio() {
        // A slow device keeps the test small: MAX_SECONDS + 1 s of 100 Hz stereo, a second at
        // a time; exactly MAX_SECONDS is kept.
        let mut audio = Accumulator::for_device(100, 2);
        let second = vec![0.5f32; 200];
        for _ in 0..MAX_SECONDS {
            audio.push(&second);
        }
        assert!(!audio.truncated);
        audio.push(&second);
        assert!(audio.truncated);
        assert_eq!(audio.samples.len(), MAX_SECONDS as usize * 200);
    }

    /// Captures a second from the real default microphone. Run it by hand:
    /// `cargo test -p mynotch-voice audio::tests::live -- --ignored --nocapture`
    #[test]
    #[ignore = "needs a microphone"]
    fn live_capture_for_a_second() {
        let recording = Recording::start().unwrap();
        println!("device: {}", recording.describe());
        thread::sleep(Duration::from_secs(1));
        let captured = recording.finish().unwrap();
        println!(
            "samples: {}, rms: {:.5}, overflowed: {}, truncated: {}",
            captured.pcm.len(),
            captured.rms,
            captured.overflowed,
            captured.truncated
        );
        assert!(captured.pcm.len().abs_diff(16_000) < 3_200, "{} samples", captured.pcm.len());
        assert_eq!(captured.overflowed, 0);
        assert!(!captured.truncated);

        // Dropping a recording discards it and frees the microphone for the next one.
        drop(Recording::start().unwrap());
        assert!(Recording::start().unwrap().finish().is_ok());
    }
}
