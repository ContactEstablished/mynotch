//! MyNotch Voice: hold Right Ctrl in any app, talk, let go, and what you said is pasted at the
//! cursor. Speech is transcribed on this PC by Whisper (whisper.cpp on the GPU); nothing leaves
//! it. This crate has no UI: the notch shows what it's doing from the events it sends.
//!
//! `VoiceService::start` runs everything on background threads (plan §5.3): a keyboard hook
//! thread, a tokio runtime for the session pipeline, and its blocking pool for the microphone,
//! Whisper and the paste. All Win32 code is in `platform::win`.

pub mod audio;
mod deliver;
mod logging;
mod pipeline;
mod platform;
mod service;
pub mod stt;
mod trigger;

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub use logging::log_to_stderr;
pub use pipeline::SessionId;
/// Tags every key MyNotch injects (in `dwExtraInfo`), so the dictation hook lets it by. Other
/// parts of MyNotch that inject keys (keep awake's F15 tap) use it too.
pub use platform::win::input::MARKER as INPUT_MARKER;

/// A hold shorter than this is a tap, which starts nothing. A UI can wait this long before
/// showing a session, so taps and chords (Right Ctrl+C) don't flash it.
pub const TAP: Duration = Duration::from_millis(trigger::TAP_MS as u64);

/// The default model's file name, in `models_dir()`.
pub const MODEL_FILE: &str = "ggml-large-v3-turbo-q5_0.bin";

/// What a voice session is doing (plan §5.4, the events M0 has).
#[derive(Clone, Debug, PartialEq)]
pub enum VoiceEvent {
    /// The microphone is open and listening. `app` is where the text will go ("Notepad"), or
    /// empty if unknown.
    SessionStarted { id: SessionId, app: String },
    /// How loud the microphone is (RMS, 0..1), about 30 times a second while listening.
    Level { id: SessionId, rms: f32 },
    /// The key came up; Whisper is working.
    Transcribing { id: SessionId },
    Transcript { id: SessionId, text: String },
    /// Pasted at the cursor, into `app`.
    Delivered { id: SessionId, app: String },
    /// A tap or a chord; nothing was pasted.
    Cancelled { id: SessionId },
    /// One line on what went wrong; nothing was pasted.
    Failed { id: SessionId, error: String },
}

/// What another part of MyNotch can ask of voice (the keyboard hook sends the same).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceCommand {
    Start,
    Stop,
    Cancel,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub model: PathBuf,
    /// Test switch: replace each session's audio with this WAV.
    pub fake_mic: Option<PathBuf>,
    /// Test switch: let injected Right Ctrl presses (without MyNotch's marker) start sessions,
    /// so scripted input can drive the whole path.
    pub accept_injected: bool,
    pub flash_attn: bool,
}

impl Config {
    /// The defaults, with the environment's overrides: `MYNOTCH_VOICE_MODEL` (a model file),
    /// and the test switches `MYNOTCH_VOICE_FAKE_MIC` (a WAV) and
    /// `MYNOTCH_VOICE_ACCEPT_INJECTED=1`.
    pub fn from_env() -> Self {
        let var = |name| std::env::var_os(name).filter(|v| !v.is_empty());
        Self {
            model: var("MYNOTCH_VOICE_MODEL").map(PathBuf::from).unwrap_or_else(|| models_dir().join(MODEL_FILE)),
            fake_mic: var("MYNOTCH_VOICE_FAKE_MIC").map(PathBuf::from),
            accept_injected: var("MYNOTCH_VOICE_ACCEPT_INJECTED").is_some_and(|v| v == "1"),
            flash_attn: true,
        }
    }
}

/// `%LOCALAPPDATA%\MyNotch`: the log and the models.
pub fn data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("MyNotch")
}

pub fn models_dir() -> PathBuf {
    data_dir().join("models")
}

/// Whether a clipboard change (by `GetClipboardSequenceNumber`) was voice putting back the
/// user's clipboard after a paste. Clipboard history should skip it: it isn't a new copy.
pub fn is_own_clipboard_restore(sequence: u32) -> bool {
    deliver::is_own_restore(sequence)
}

/// Whether a paste is using the clipboard right now: what's on it is a dictation about to be
/// replaced by the user's clipboard again. Clipboard history should look again later.
pub fn is_clipboard_busy() -> bool {
    deliver::is_busy()
}

pub struct VoiceService;

impl VoiceService {
    /// Starts voice in the background and logs to `%LOCALAPPDATA%\MyNotch\voice.log`. If the
    /// model or CUDA can't be loaded, the log says why and voice stays off (Right Ctrl keeps its
    /// usual meaning); the handle then simply never has events.
    pub fn start(config: Config) -> VoiceHandle {
        logging::log_to_file(&data_dir());
        let (commands, inbox) = tokio::sync::mpsc::unbounded_channel();
        let (events_tx, events) = mpsc::channel();
        service::spawn(config, commands.clone(), inbox, events_tx);
        VoiceHandle { commands, events }
    }
}

/// Talks to the running voice service. Dropping it stops the service.
pub struct VoiceHandle {
    commands: tokio::sync::mpsc::UnboundedSender<service::Msg>,
    events: mpsc::Receiver<VoiceEvent>,
}

impl VoiceHandle {
    pub fn send(&self, command: VoiceCommand) {
        let _ = self.commands.send(service::Msg::Command(command, Instant::now()));
    }

    /// Events since the last call, oldest first; never blocks.
    pub fn events(&self) -> mpsc::TryIter<'_, VoiceEvent> {
        self.events.try_iter()
    }
}

impl Drop for VoiceHandle {
    fn drop(&mut self) {
        let _ = self.commands.send(service::Msg::Shutdown);
    }
}
