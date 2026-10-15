//! Now-playing info and transport commands via Windows' Global System Media Transport
//! Controls (SMTC). Every WinRT call happens on a worker thread; the UI thread only
//! exchanges messages with it, so a slow or hung media app can never stall the notch.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSessionManager as SessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

/// How often the worker re-reads the current session when no command arrives.
const POLL: Duration = Duration::from_millis(500);
/// Grace period after a command before re-reading, so the app has applied it.
const SETTLE: Duration = Duration::from_millis(150);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    /// Friendly name of the app that owns the session, possibly empty.
    pub app: String,
    pub playing: bool,
    pub can_previous: bool,
    pub can_next: bool,
    pub can_toggle: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    TogglePlayPause,
    Next,
    Previous,
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

fn worker(commands: Receiver<Command>, updates: Sender<Option<NowPlaying>>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let Ok(manager) = SessionManager::RequestAsync().and_then(|op| op.join()) else {
        return;
    };

    let mut last = None;
    loop {
        let now = read(&manager);
        // Always report after a command (`last` was reset), so an optimistic UI update
        // gets corrected if the app ignored the command.
        if last.as_ref() != Some(&now) {
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
    };
    let _ = op.and_then(|op| op.join());
}

fn read(manager: &SessionManager) -> Option<NowPlaying> {
    let session = manager.GetCurrentSession().ok()?;
    let props = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
    let info = session.GetPlaybackInfo().ok()?;
    let controls = info.Controls().ok()?;
    let text = |s: windows::core::Result<windows::core::HSTRING>| s.map(|s| s.to_string()).unwrap_or_default();
    Some(NowPlaying {
        title: text(props.Title()),
        artist: text(props.Artist()),
        app: app_name(&text(session.SourceAppUserModelId())),
        playing: info.PlaybackStatus().ok() == Some(PlaybackStatus::Playing),
        can_previous: controls.IsPreviousEnabled().unwrap_or(false),
        can_next: controls.IsNextEnabled().unwrap_or(false),
        can_toggle: controls.IsPlayPauseToggleEnabled().unwrap_or(false),
    })
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
