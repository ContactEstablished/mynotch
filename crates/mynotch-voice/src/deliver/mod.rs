//! Getting the text to the cursor. M0 has one way: the clipboard and Ctrl+V (plan §6.8 step 2).
//! The user's clipboard is put back afterwards, whatever happens.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use tracing::warn;

use crate::platform::win::clipboard::{self, Owner, Snapshot};
use crate::platform::win::{input, pump_for};

/// How long to wait for the user to let go of Shift/Ctrl/Alt/Win before pasting anyway.
const MODIFIER_WAIT: Duration = Duration::from_millis(400);
/// How long the target app gets to read the clipboard before it's put back.
const RESTORE_DELAY: Duration = Duration::from_millis(150);

/// The clipboard sequence number right after MyNotch last put the user's clipboard back.
static RESTORED: AtomicU32 = AtomicU32::new(0);
/// A paste is between setting the clipboard and putting it back.
static BUSY: AtomicBool = AtomicBool::new(false);

/// Whether the clipboard change numbered `sequence` (`GetClipboardSequenceNumber`) was voice
/// putting the user's own clipboard back, which clipboard history shouldn't record as new.
pub fn is_own_restore(sequence: u32) -> bool {
    sequence != 0 && sequence == RESTORED.load(Ordering::Acquire)
}

/// Whether a paste is using the clipboard right now (what's on it isn't the user's).
pub fn is_busy() -> bool {
    BUSY.load(Ordering::Acquire)
}

/// The user's clipboard, saved ahead of time (at key-up, while Whisper works), so saving it
/// isn't on the way to the paste. Used only if the clipboard hasn't changed since.
pub struct Prepared {
    snapshot: Snapshot,
    sequence: u32,
}

pub fn prepare() -> Result<Prepared, String> {
    let owner = Owner::new()?;
    let sequence = clipboard::sequence();
    let snapshot = owner.snapshot()?;
    Ok(Prepared { snapshot, sequence })
}

pub struct Report {
    /// When Ctrl+V went out.
    pub sent_at: Instant,
    /// Shift/Ctrl/Alt/Win were still down when the wait ran out.
    pub modifiers_held: bool,
    /// Formats saved and put back, and those that couldn't be.
    pub formats: usize,
    pub skipped: usize,
    pub bytes: usize,
    /// The text was pasted, but the user's clipboard couldn't be put back.
    pub restore_failed: Option<String>,
}

/// Pastes `text` at the cursor of the foreground window and restores the clipboard. Blocks for
/// about 150 ms after the paste; run it off the async threads. Errors are short, for the notch;
/// the details are logged.
pub fn paste(text: &str, prepared: Option<Prepared>) -> Result<Report, String> {
    let owner = Owner::new().map_err(|e| failed("Couldn't paste", &e))?;
    let snapshot = match prepared {
        Some(p) if p.sequence == clipboard::sequence() => p.snapshot,
        _ => owner.snapshot().map_err(|e| failed("Clipboard busy", &e))?,
    };
    BUSY.store(true, Ordering::Release);
    let result = owner.set_private_text(text).map_err(|e| failed("Clipboard busy", &e)).and_then(|()| {
        let wait_until = Instant::now() + MODIFIER_WAIT;
        while input::modifiers_held() && Instant::now() < wait_until {
            pump_for(Duration::from_millis(10));
        }
        let modifiers_held = input::modifiers_held();
        // Blocked when the window runs as administrator and MyNotch doesn't (UIPI).
        input::send_paste().map_err(|e| failed("Couldn't paste here", &e))?;
        Ok((Instant::now(), modifiers_held))
    });
    if result.is_ok() {
        pump_for(RESTORE_DELAY);
    }
    // Put the user's clipboard back even if the paste failed.
    let restored = restore(&owner, &snapshot);
    BUSY.store(false, Ordering::Release);
    if let Err(e) = &restored {
        warn!("couldn't put the clipboard back: {e}");
    }
    let (sent_at, modifiers_held) = result?;
    Ok(Report {
        sent_at,
        modifiers_held,
        formats: snapshot.formats(),
        skipped: snapshot.skipped,
        bytes: snapshot.bytes(),
        restore_failed: restored.err(),
    })
}

fn failed(short: &str, detail: &str) -> String {
    warn!("paste: {short}: {detail}");
    short.to_owned()
}

fn restore(owner: &Owner, snapshot: &Snapshot) -> Result<(), String> {
    let mut last = Err(String::new());
    for _ in 0..3 {
        last = owner.restore(snapshot);
        if let Ok(sequence) = last {
            RESTORED.store(sequence, Ordering::Release);
            return Ok(());
        }
        pump_for(Duration::from_millis(50));
    }
    last.map(|_| ())
}
