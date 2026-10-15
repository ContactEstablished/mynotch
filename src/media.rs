//! Now-playing info and transport commands via Windows' Global System Media Transport
//! Controls (SMTC). Every WinRT call happens on a worker thread; the UI thread only
//! exchanges messages with it, so a slow or hung media app can never stall the notch.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as SessionManager,
    GlobalSystemMediaTransportControlsSessionMediaProperties as MediaProperties,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use windows::Storage::Streams::DataReader;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

use crate::gfx::Image;
use crate::imaging;

/// How often the worker re-reads the current session when no command arrives.
const POLL: Duration = Duration::from_millis(500);
/// Grace period after a command before re-reading, so the app has applied it.
const SETTLE: Duration = Duration::from_millis(150);
/// Album art is decoded at most this big; the panel shows it at 160 logical px.
const ART_SIZE: u32 = 320;
/// Some apps publish the thumbnail a moment after the title; keep asking this many polls.
const ART_RETRIES: u32 = 6;
/// Windows FILETIME ticks (100 ns since 1601) at the Unix epoch.
const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;

#[derive(Clone, Debug, PartialEq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    /// Friendly name of the app that owns the session, possibly empty.
    pub app: String,
    pub art: Option<Arc<Image>>,
    pub playing: bool,
    pub can_previous: bool,
    pub can_next: bool,
    pub can_toggle: bool,
    pub can_seek: bool,
    /// Track position when sampled (extrapolated past the app's last report while playing).
    pub position: Duration,
    /// Track length; zero when the app doesn't report a timeline.
    pub duration: Duration,
    pub sampled: Instant,
}

impl NowPlaying {
    /// Current position, advanced since sampling while playing.
    pub fn position_now(&self) -> Duration {
        let at = if self.playing { self.position + self.sampled.elapsed() } else { self.position };
        at.min(self.duration)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    TogglePlayPause,
    Next,
    Previous,
    /// Jump to this position in the track.
    Seek(Duration),
}

/// Handle to the media worker thread. Dropping it stops the worker.
pub struct Media {
    commands: Sender<Command>,
    updates: Receiver<Option<NowPlaying>>,
}

impl Media {
    pub fn spawn() -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (update_tx, updates) = mpsc::channel();
        let _ = thread::Builder::new()
            .name("media".into())
            .spawn(move || worker(command_rx, update_tx));
        Self { commands, updates }
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    /// The newest state reported since the last call, if any. The inner `None` means
    /// nothing is playing.
    pub fn poll(&self) -> Option<Option<NowPlaying>> {
        self.updates.try_iter().last()
    }
}

/// Album art for the current track, fetched once per track (plus a few retries).
#[derive(Default)]
struct ArtCache {
    track: Option<(String, String, String)>,
    art: Option<Arc<Image>>,
    tries: u32,
}

fn worker(commands: Receiver<Command>, updates: Sender<Option<NowPlaying>>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let Ok(manager) = SessionManager::RequestAsync().and_then(|op| op.join()) else {
        return;
    };

    let mut art = ArtCache::default();
    let mut last: Option<Option<NowPlaying>> = None;
    loop {
        let now = read(&manager, &mut art);
        // Report changes, ignoring the sample time alone, and always after a command
        // (`last` was reset) so an optimistic UI update gets corrected if the app ignored it.
        let changed = match (&last, &now) {
            (Some(Some(a)), Some(b)) => *a != NowPlaying { sampled: a.sampled, ..b.clone() },
            (Some(None), None) => false,
            _ => true,
        };
        if changed {
            if updates.send(now.clone()).is_err() {
                return;
            }
            last = Some(now);
        }

        match commands.recv_timeout(POLL) {
            Ok(command) => {
                run(&manager, command);
                thread::sleep(SETTLE);
                last = None;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn run(manager: &SessionManager, command: Command) {
    let Ok(session) = manager.GetCurrentSession() else { return };
    let op = match command {
        Command::TogglePlayPause => session.TryTogglePlayPauseAsync(),
        Command::Next => session.TrySkipNextAsync(),
        Command::Previous => session.TrySkipPreviousAsync(),
        Command::Seek(to) => {
            let start = session.GetTimelineProperties().and_then(|t| t.StartTime()).map_or(0, |t| t.Duration);
            session.TryChangePlaybackPositionAsync(start + ticks(to))
        }
    };
    let _ = op.and_then(|op| op.join());
}

fn read(manager: &SessionManager, art: &mut ArtCache) -> Option<NowPlaying> {
    let session = manager.GetCurrentSession().ok()?;
    let props = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
    let info = session.GetPlaybackInfo().ok()?;
    let controls = info.Controls().ok()?;
    let text = |s: windows::core::Result<windows::core::HSTRING>| s.map(|s| s.to_string()).unwrap_or_default();
    let (title, artist, aumid) = (text(props.Title()), text(props.Artist()), text(session.SourceAppUserModelId()));
    let playing = info.PlaybackStatus().ok() == Some(PlaybackStatus::Playing);

    let track = (title.clone(), artist.clone(), aumid.clone());
    if art.track.as_ref() != Some(&track) {
        *art = ArtCache { track: Some(track), art: None, tries: 0 };
    }
    if art.art.is_none() && art.tries < ART_RETRIES {
        art.tries += 1;
        art.art = read_art(&props).map(Arc::new);
    }

    let (position, duration) = timeline(&session, playing);
    Some(NowPlaying {
        title,
        artist,
        app: app_name(&aumid),
        art: art.art.clone(),
        playing,
        can_previous: controls.IsPreviousEnabled().unwrap_or(false),
        can_next: controls.IsNextEnabled().unwrap_or(false),
        can_toggle: controls.IsPlayPauseToggleEnabled().unwrap_or(false),
        can_seek: controls.IsPlaybackPositionEnabled().unwrap_or(false) && !duration.is_zero(),
        position,
        duration,
        sampled: Instant::now(),
    })
}

fn read_art(props: &MediaProperties) -> Option<Image> {
    let stream = props.Thumbnail().ok()?.OpenReadAsync().ok()?.join().ok()?;
    let size = u32::try_from(stream.Size().ok()?).ok().filter(|&n| n > 0 && n <= 16 << 20)?;
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    reader.LoadAsync(size).ok()?.join().ok()?;
    let mut bytes = vec![0u8; size as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    imaging::decode(&bytes, ART_SIZE)
}

/// (position, length). Apps report the position only now and then, so while playing the
/// time since their last report is added on.
fn timeline(session: &Session, playing: bool) -> (Duration, Duration) {
    let Ok(t) = session.GetTimelineProperties() else { return (Duration::ZERO, Duration::ZERO) };
    let span = |r: windows::core::Result<windows::Foundation::TimeSpan>| r.map_or(0, |t| t.Duration);
    let (start, end, pos) = (span(t.StartTime()), span(t.EndTime()), span(t.Position()));
    let length = (end - start).max(0);
    let mut at = (pos - start).max(0);
    if playing && let Ok(updated) = t.LastUpdatedTime() {
        let since = now_ticks() - updated.UniversalTime;
        if updated.UniversalTime > 0 && since > 0 {
            at += since;
        }
    }
    (from_ticks(at.min(length)), from_ticks(length))
}

fn now_ticks() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| (d.as_nanos() / 100) as i64 + UNIX_EPOCH_TICKS)
}

fn ticks(d: Duration) -> i64 {
    (d.as_nanos() / 100) as i64
}

fn from_ticks(t: i64) -> Duration {
    Duration::from_nanos(t.max(0) as u64 * 100)
}

/// Best-effort friendly name from an AppUserModelID, which is either an executable name
/// (`Spotify.exe`), a packaged-app ID (`Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic`)
/// or an app-chosen string (`MSEdge`, or an opaque hash for Firefox).
fn app_name(aumid: &str) -> String {
    let s = aumid.split('!').next().unwrap_or_default(); // drop "!AppId"
    let s = s.split('_').next().unwrap_or_default(); // drop package publisher hash
    let s = s.rsplit(['\\', '/']).next().unwrap_or_default(); // drop any directory
    let s = s.strip_suffix(".exe").or_else(|| s.strip_suffix(".EXE")).unwrap_or(s);
    let s = s.rsplit('.').next().unwrap_or_default(); // "Microsoft.ZuneMusic" -> "ZuneMusic"
    match s {
        "ZuneMusic" => "Media Player",
        "SpotifyMusic" => "Spotify",
        "MSEdge" => "Edge",
        "308046B0AF4A39CB" => "Firefox",
        // Other opaque IDs are not worth showing.
        s if s.len() >= 16 && s.chars().all(|c| c.is_ascii_hexdigit()) => "",
        s => s,
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::app_name;

    #[test]
    fn friendly_app_names() {
        assert_eq!(app_name("Spotify.exe"), "Spotify");
        assert_eq!(app_name("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), "Spotify");
        assert_eq!(app_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"), "Media Player");
        assert_eq!(app_name("MSEdge"), "Edge");
        assert_eq!(app_name("Chrome"), "Chrome");
        assert_eq!(app_name("308046B0AF4A39CB"), "Firefox");
        assert_eq!(app_name("C:\\Program Files\\VLC\\vlc.exe"), "vlc");
        assert_eq!(app_name("0123456789ABCDEF0"), "");
        assert_eq!(app_name(""), "");
    }
}
