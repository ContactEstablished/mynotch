//! The voice thread: a tokio runtime running the session pipeline. It loads Whisper, installs
//! the keyboard hook, then turns the pipeline's effects into work: the microphone, Whisper and
//! the paste run on the blocking pool and report back over the same channel as the hook.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self as std_mpsc, Sender};
use std::thread;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::spawn_blocking;
use tracing::{error, info, warn};

use crate::audio::{self, MAX_SECONDS, Recording};
use crate::deliver::{self, Prepared};
use crate::pipeline::{Effect, Input, Phase, Pipeline, SessionId};
use crate::platform::win::foreground::{Foreground, foreground};
use crate::platform::win::{self, gpu, hook};
use crate::stt::{SttEngine, SttOptions, WhisperEngine};
use crate::trigger::Signal;
use crate::{Config, VoiceCommand, VoiceEvent};

/// How often the microphone's level is reported while listening.
const LEVEL_EVERY: Duration = Duration::from_millis(33);

pub(crate) enum Msg {
    Command(VoiceCommand, Instant),
    Shutdown,
    MicOpened(SessionId, Result<Recording, audio::AudioError>),
    Transcribed(SessionId, Result<Heard, String>),
    Pasted(SessionId, Result<deliver::Report, String>),
}

/// What came back from the microphone and Whisper.
pub(crate) struct Heard {
    text: String,
    /// Key-up (or the cap) to the audio being ready for Whisper.
    audio: Duration,
    seconds: f32,
    rms: f32,
    stt: Duration,
    /// When Whisper finished.
    done: Instant,
}

/// A session's details, gathered as it goes, for its log line.
struct Session {
    pressed: Instant,
    target: Foreground,
    released: Option<Instant>,
    chars: usize,
    heard: Option<Heard>,
    /// The user's clipboard, being saved while Whisper works.
    prepared: Option<std_mpsc::Receiver<Result<Prepared, String>>>,
}

pub(crate) fn spawn(config: Config, commands: UnboundedSender<Msg>, inbox: UnboundedReceiver<Msg>, events: Sender<VoiceEvent>) {
    let started = thread::Builder::new().name("voice".into()).spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_time().build() {
            Ok(runtime) => runtime,
            Err(e) => return error!("voice is off: no async runtime: {e}"),
        };
        runtime.block_on(run(config, commands, inbox, events));
        runtime.shutdown_timeout(Duration::from_millis(500));
    });
    if let Err(e) = started {
        error!("voice is off: couldn't start its thread: {e}");
    }
}

async fn run(config: Config, commands: UnboundedSender<Msg>, mut inbox: UnboundedReceiver<Msg>, events: Sender<VoiceEvent>) {
    if let Some(path) = &config.fake_mic {
        info!("MYNOTCH_VOICE_FAKE_MIC is set: each session's audio is replaced by {}", path.display());
    }
    if config.accept_injected {
        info!("MYNOTCH_VOICE_ACCEPT_INJECTED is set: injected Right Ctrl presses start sessions");
    }
    let Some(engine) = load(config.model, config.flash_attn).await else { return };
    let fake_mic = match config.fake_mic {
        Some(path) => match spawn_blocking(move || audio::load_wav(&path)).await {
            Ok(Ok(pcm)) => {
                info!("fake microphone: {:.1} s of audio", pcm.len() as f32 / audio::SAMPLE_RATE as f32);
                Some(Arc::<[f32]>::from(pcm))
            }
            Ok(Err(e)) => return error!("voice is off: MYNOTCH_VOICE_FAKE_MIC: {e}"),
            Err(e) => return error!("voice is off: {e}"),
        },
        None => None,
    };
    let to_pipeline = commands.clone();
    let notify = Box::new(move |signal: Signal, at: Instant| {
        let command = match signal {
            Signal::Press => VoiceCommand::Start,
            Signal::Release => VoiceCommand::Stop,
            Signal::Cancel => VoiceCommand::Cancel,
        };
        let _ = to_pipeline.send(Msg::Command(command, at));
    });
    let _hook = match hook::install(config.accept_injected, notify) {
        Ok(hook) => hook,
        Err(e) => return error!("voice is off: couldn't install the keyboard hook: {e}"),
    };
    info!("voice is on: hold Right Ctrl to dictate");

    let mut driver = Driver {
        pipeline: Pipeline::new(),
        commands,
        events,
        engine,
        fake_mic,
        mics: HashMap::new(),
        sessions: HashMap::new(),
        pressed_at: Instant::now(),
        cap: None,
    };
    // The microphone's level, for the notch's waveform, while listening.
    let mut meter = tokio::time::interval(LEVEL_EVERY);
    meter.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            msg = inbox.recv() => match msg {
                None | Some(Msg::Shutdown) => break,
                Some(msg) => driver.on(msg),
            },
            () = tokio::time::sleep_until(driver.cap.unwrap_or_else(tokio::time::Instant::now)), if driver.cap.is_some() => {
                warn!("a session reached the {MAX_SECONDS} s limit; transcribing what was heard");
                driver.on(Msg::Command(VoiceCommand::Stop, Instant::now()));
            }
            _ = meter.tick(), if driver.cap.is_some() => driver.send_level(),
        }
    }
    info!("voice stopped");
}

/// Checks for CUDA, loads the model and warms it up, off the async thread. `None` (logged)
/// turns voice off.
async fn load(model: PathBuf, flash_attn: bool) -> Option<Arc<WhisperEngine>> {
    let loaded = spawn_blocking(move || {
        let missing = win::missing_dlls(&win::CUDA_DLLS);
        if !missing.is_empty() {
            return Err(format!("CUDA isn't available ({} not found)", missing.join(", ")));
        }
        info!("loading {} (flash attention {})", model.display(), if flash_attn { "on" } else { "off" });
        let start = Instant::now();
        let engine = WhisperEngine::load(&model, flash_attn).map_err(|e| e.to_string())?;
        let loaded = start.elapsed();
        let gpus = gpu::devices();
        if gpus.is_empty() {
            return Err("CUDA found no GPU, and Whisper on the CPU would be too slow".into());
        }
        let warm = engine.warm_up().map_err(|e| e.to_string())?;
        info!(
            "model loaded in {} ms on {}, warm-up {} ms; whisper.cpp: {}",
            loaded.as_millis(),
            gpus.join(", "),
            warm.as_millis(),
            WhisperEngine::system_info().trim()
        );
        Ok(Arc::new(engine))
    })
    .await;
    match loaded {
        Ok(Ok(engine)) => Some(engine),
        Ok(Err(e)) => {
            error!("voice is off: {e}");
            None
        }
        Err(e) => {
            error!("voice is off: loading the model crashed: {e}");
            None
        }
    }
}

struct Driver {
    pipeline: Pipeline,
    /// For work on the blocking pool to report back.
    commands: UnboundedSender<Msg>,
    events: Sender<VoiceEvent>,
    engine: Arc<WhisperEngine>,
    fake_mic: Option<Arc<[f32]>>,
    /// Open microphones by session, including ones still closing for a session that's gone.
    /// `None`: the fake microphone stands in for one that couldn't open.
    mics: HashMap<SessionId, Option<Recording>>,
    sessions: HashMap<SessionId, Session>,
    /// When the trigger last went down, for the session it opens.
    pressed_at: Instant,
    /// The listening session's length limit.
    cap: Option<tokio::time::Instant>,
}

impl Driver {
    fn on(&mut self, msg: Msg) {
        let input = match msg {
            Msg::Command(command, at) => match command {
                VoiceCommand::Start => {
                    self.pressed_at = at;
                    Input::Start
                }
                VoiceCommand::Stop => {
                    if let Phase::Listening(id) = self.pipeline.phase()
                        && let Some(s) = self.sessions.get_mut(&id)
                    {
                        s.released = Some(at);
                    }
                    Input::Stop
                }
                VoiceCommand::Cancel => Input::Cancel,
            },
            // The loop handles it.
            Msg::Shutdown => return,
            Msg::MicOpened(id, Ok(recording)) => {
                self.mics.insert(id, Some(recording));
                Input::MicReady(id)
            }
            Msg::MicOpened(id, Err(e)) if self.fake_mic.is_some() => {
                warn!("session {id}: no microphone ({e}); carrying on with the fake one");
                self.mics.insert(id, None);
                Input::MicReady(id)
            }
            Msg::MicOpened(id, Err(e)) => {
                warn!("session {id}: the microphone didn't open: {e}");
                Input::MicFailed(id, e.to_string())
            }
            Msg::Transcribed(id, Ok(heard)) => {
                let text = heard.text.clone();
                if let Some(s) = self.sessions.get_mut(&id) {
                    s.chars = text.chars().count();
                    s.heard = Some(heard);
                }
                Input::Transcribed(id, text)
            }
            Msg::Transcribed(id, Err(e)) => Input::TranscribeFailed(id, e),
            Msg::Pasted(id, Ok(report)) => {
                self.log_session(id, &report);
                Input::Delivered(id)
            }
            Msg::Pasted(id, Err(e)) => Input::DeliverFailed(id, e),
        };
        for effect in self.pipeline.handle(input) {
            self.apply(effect);
        }
        self.cap = match self.pipeline.phase() {
            Phase::Listening(_) => {
                Some(self.cap.unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(u64::from(MAX_SECONDS))))
            }
            _ => None,
        };
    }

    /// Reports the listening session's microphone level.
    fn send_level(&self) {
        if let Phase::Listening(id) = self.pipeline.phase()
            && let Some(Some(mic)) = self.mics.get(&id)
        {
            let _ = self.events.send(VoiceEvent::Level { id, rms: mic.level() });
        }
    }

    /// Runs `work` on the blocking pool and hands its result back to the pipeline, or
    /// `if_panicked` if it panicked, so a session never waits forever.
    fn blocking(&self, work: impl FnOnce() -> Msg + Send + 'static, if_panicked: Msg) {
        let commands = self.commands.clone();
        let task = spawn_blocking(work);
        tokio::spawn(async move {
            let msg = task.await.unwrap_or_else(|e| {
                error!("voice: a task panicked: {e}");
                if_panicked
            });
            let _ = commands.send(msg);
        });
    }

    fn apply(&mut self, effect: Effect) {
        match effect {
            Effect::OpenMic(id) => {
                let target = foreground();
                info!("session {id}: Right Ctrl down in {} (window {:#x})", or_unknown(&target.exe), target.hwnd);
                let session = Session { pressed: self.pressed_at, target, released: None, chars: 0, heard: None, prepared: None };
                self.sessions.insert(id, session);
                self.blocking(
                    move || Msg::MicOpened(id, Recording::start()),
                    Msg::MicOpened(id, Err(audio::AudioError::new("The microphone crashed"))),
                );
            }
            Effect::DropMic(id) => {
                self.sessions.remove(&id);
                if let Some(Some(recording)) = self.mics.remove(&id) {
                    spawn_blocking(move || drop(recording));
                }
            }
            Effect::Transcribe(id) => {
                let recording = self.mics.remove(&id).flatten();
                let released = self.sessions.get(&id).and_then(|s| s.released).unwrap_or_else(Instant::now);
                let (engine, fake_mic) = (self.engine.clone(), self.fake_mic.clone());
                self.blocking(
                    move || Msg::Transcribed(id, listen_and_transcribe(recording, fake_mic, &*engine, released)),
                    Msg::Transcribed(id, Err("Transcription failed".into())),
                );
                // Save the user's clipboard meanwhile, so that isn't on the way to the paste.
                let (tx, rx) = std_mpsc::channel();
                spawn_blocking(move || {
                    let _ = tx.send(deliver::prepare());
                });
                if let Some(s) = self.sessions.get_mut(&id) {
                    s.prepared = Some(rx);
                }
            }
            Effect::Deliver(id, text) => {
                let session = self.sessions.get_mut(&id);
                let target = session.as_ref().map(|s| s.target.clone());
                let prepared = session.and_then(|s| s.prepared.take());
                self.blocking(
                    move || {
                        let now = foreground();
                        if let Some(target) = target
                            && now.hwnd != target.hwnd
                        {
                            // M0 only notes it; the guard that keeps the text back is Voice M1.
                            warn!("session {id}: the foreground changed to {} (window {:#x}) since the press", or_unknown(&now.exe), now.hwnd);
                        }
                        let prepared = prepared.and_then(|rx| rx.recv().ok()).and_then(|saved| {
                            saved.map_err(|e| warn!("session {id}: couldn't save the clipboard early: {e}")).ok()
                        });
                        Msg::Pasted(id, deliver::paste(&text, prepared))
                    },
                    Msg::Pasted(id, Err("Couldn't paste".into())),
                );
            }
            Effect::Emit(mut event) => {
                if let VoiceEvent::SessionStarted { id, app } | VoiceEvent::Delivered { id, app } = &mut event
                    && let Some(s) = self.sessions.get(id)
                {
                    app.clone_from(&s.target.app);
                }
                match &event {
                    VoiceEvent::SessionStarted { id, .. } => {
                        if let (Some(s), Some(mic)) = (self.sessions.get(id), self.mics.get(id)) {
                            let device = mic.as_ref().map_or("the fake microphone", |r| r.describe());
                            info!("session {id}: listening on {device} ({} ms after the press)", s.pressed.elapsed().as_millis());
                        }
                    }
                    VoiceEvent::Cancelled { id } => {
                        info!("session {id}: cancelled");
                        self.sessions.remove(id);
                    }
                    VoiceEvent::Failed { id, error } => {
                        warn!("session {id}: failed: {error}");
                        self.sessions.remove(id);
                    }
                    VoiceEvent::Delivered { id, .. } => {
                        self.sessions.remove(id);
                    }
                    VoiceEvent::Level { .. } | VoiceEvent::Transcribing { .. } | VoiceEvent::Transcript { .. } => {}
                }
                let _ = self.events.send(event);
            }
        }
    }

    fn log_session(&self, id: SessionId, paste: &deliver::Report) {
        let Some(Session { target, released: Some(released), chars, heard: Some(heard), .. }) = self.sessions.get(&id)
        else {
            return;
        };
        let ms = |d: Duration| d.as_millis();
        info!(
            "session {id}: inserted {chars} chars into {}; {:.1} s of audio (rms {:.3}); key-up → audio ready {} ms, STT {} ms, paste {} ms, total key-up → paste sent {} ms; clipboard: {} formats ({} bytes) restored, {} skipped{}",
            or_unknown(&target.exe),
            heard.seconds,
            heard.rms,
            ms(heard.audio),
            ms(heard.stt),
            ms(paste.sent_at.saturating_duration_since(heard.done)),
            ms(paste.sent_at.saturating_duration_since(*released)),
            paste.formats,
            paste.bytes,
            paste.skipped,
            if paste.modifiers_held { "; modifier keys were still down" } else { "" },
        );
        if let Some(e) = &paste.restore_failed {
            warn!("session {id}: the text was pasted, but the clipboard couldn't be put back: {e}");
        }
    }
}

/// Stops the microphone, swaps in the fake one if set, and runs Whisper. Errors are short, for
/// the notch; the details are logged.
fn listen_and_transcribe(
    recording: Option<Recording>,
    fake_mic: Option<Arc<[f32]>>,
    engine: &dyn SttEngine,
    released: Instant,
) -> Result<Heard, String> {
    let captured = match recording {
        Some(recording) => Some(recording.finish().map_err(|e| e.to_string())?),
        None => None,
    };
    if let Some(c) = &captured {
        if c.truncated {
            warn!("the recording was cut at {MAX_SECONDS} s");
        }
        if c.overflowed > 0 {
            warn!("{} audio frames were lost", c.overflowed);
        }
    }
    let (pcm, rms): (Arc<[f32]>, f32) = match (fake_mic, captured) {
        (Some(fake), captured) => {
            if let Some(c) = &captured {
                info!("captured {:.1} s (rms {:.3}), replaced by the fake microphone", c.pcm.len() as f32 / audio::SAMPLE_RATE as f32, c.rms);
            }
            let rms = audio::rms(&fake);
            (fake, rms)
        }
        (None, Some(c)) => (Arc::from(c.pcm), c.rms),
        (None, None) => return Err("No audio".into()),
    };
    let audio = released.elapsed();
    let start = Instant::now();
    let transcript = engine.transcribe(&pcm, &SttOptions::default()).map_err(|e| {
        warn!("{e}");
        "Transcription failed".to_owned()
    })?;
    Ok(Heard {
        text: transcript.text,
        audio,
        seconds: pcm.len() as f32 / audio::SAMPLE_RATE as f32,
        rms,
        stt: start.elapsed(),
        done: Instant::now(),
    })
}

fn or_unknown(exe: &str) -> &str {
    if exe.is_empty() { "an unknown app" } else { exe }
}
