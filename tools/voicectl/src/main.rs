//! Command-line tools for MyNotch Voice.
//!
//! `voicectl transcribe <file.wav> [--model <path>] [--no-flash-attn]` transcribes a WAV with
//! the same Whisper setup as the app and prints the text and how long each step took. It's how
//! GPU latency is measured without a microphone. whisper.cpp's own log (the CUDA device it
//! found) goes to stderr.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use mynotch_voice::audio::{SAMPLE_RATE, load_wav};
use mynotch_voice::stt::{SttEngine, SttOptions, WhisperEngine};
use mynotch_voice::{Config, log_to_stderr};

const USAGE: &str = "usage: voicectl transcribe <file.wav> [--model <path>] [--no-flash-attn]";

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("voicectl: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let mut args = args.into_iter();
    if args.next().as_deref() != Some("transcribe") {
        return Err(USAGE.into());
    }
    let (mut wav, mut model, mut flash_attn) = (None, Config::from_env().model, true);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--model" => model = args.next().map(PathBuf::from).ok_or(USAGE)?,
            "--no-flash-attn" => flash_attn = false,
            _ if wav.is_none() && !arg.starts_with("--") => wav = Some(PathBuf::from(arg)),
            _ => return Err(USAGE.into()),
        }
    }
    let wav = wav.ok_or(USAGE)?;
    log_to_stderr();

    let pcm = load_wav(&wav).map_err(|e| e.to_string())?;
    let start = Instant::now();
    let engine = WhisperEngine::load(&model, flash_attn).map_err(|e| e.to_string())?;
    let load = start.elapsed();
    let warm = engine.warm_up().map_err(|e| e.to_string())?;
    let start = Instant::now();
    let transcript = engine.transcribe(&pcm, &SttOptions::default()).map_err(|e| e.to_string())?;
    let stt = start.elapsed();

    println!("{}", transcript.text);
    println!(
        "audio {:.2} s · model load {} ms · warm-up {} ms · STT {} ms · flash attention {} · {}",
        pcm.len() as f32 / SAMPLE_RATE as f32,
        load.as_millis(),
        warm.as_millis(),
        stt.as_millis(),
        if flash_attn { "on" } else { "off" },
        model.display(),
    );
    Ok(())
}
