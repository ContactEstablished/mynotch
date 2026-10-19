//! Voice's log, with whisper.cpp's own messages (model load, the CUDA device) brought into it.

use std::fs::{self, OpenOptions};
use std::path::Path;
use std::sync::Mutex;

use tracing::Level;

/// The log starts afresh past this size (the old one is kept as `voice.old.log`).
const MAX_LOG: u64 = 5 << 20;

/// Logs to `dir\voice.log`, appending. Does nothing if logging is already set up.
pub(crate) fn log_to_file(dir: &Path) {
    let path = dir.join("voice.log");
    let opened = fs::create_dir_all(dir).and_then(|()| {
        if fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG) {
            let _ = fs::rename(&path, dir.join("voice.old.log"));
        }
        OpenOptions::new().create(true).append(true).open(&path)
    });
    if let Ok(file) = opened {
        install(Mutex::new(file));
    }
}

/// Logs to stderr (for `voicectl`).
pub fn log_to_stderr() {
    install(std::io::stderr);
}

fn install<W>(writer: W)
where
    W: for<'a> tracing_subscriber::fmt::MakeWriter<'a> + Send + Sync + 'static,
{
    let installed =
        tracing_subscriber::fmt().with_ansi(false).with_max_level(Level::INFO).with_writer(writer).try_init().is_ok();
    if installed {
        whisper_rs::install_logging_hooks();
    }
}
