# MyNotch Voice M0 — execution prompt

Start a new Claude Code session in `C:\Projects\ContactEstablished\MyNotch`, then either paste everything below the line or say: "Read `Voice-M0-ExecutionPrompt.md` and follow it."

---

## 1. Role

You are the Coordinator for MyNotch Voice M0, the "walking skeleton" milestone of the MyNotch Voice plan.

- Repo root: `C:\Projects\ContactEstablished\MyNotch`. The expected branch is `main`. Confirm it. Do not switch branches without instruction. The user's standing workflow is to commit straight to `main`, with no feature branches.
- Platform: Windows 11. PowerShell is the primary shell; Git Bash is also available. Rust 1.98.1 (MSVC), edition 2024. Visual Studio 18 Community is installed.
- Naming: call the milestones "Voice M0–M5" in commits and docs, so they don't collide with HANDOFF.md's "Milestone 1–4" or the roadmap's "Phase 1–5".

## 2. Goal

Hold Right Ctrl in any app. MyNotch captures the microphone, transcribes locally with Whisper, and pastes the text at the cursor; M0 proves this in Notepad. The collapsed notch shows a minimal Listening / Transcribing / Inserted readout.

- Engine: whisper.cpp via `whisper-rs`, CUDA build, model `ggml-large-v3-turbo-q5_0.bin`.
- Done criteria from the plan (`docs/voice/plan.md` §10):
  - A 10 s utterance is inserted ≤ 1.0 s after key release, on the GPU.
  - The notch never takes focus.
  - The keyboard hook survives 1 hour of normal use, including sleep/resume.
  - The clipboard is restored.
- Constraints that always apply:
  - The notch never takes focus and never changes its window styles.
  - The user's clipboard is always restored.
  - Right Ctrl shortcuts keep working (chord passthrough, see section 5C).
  - Nothing listens on a network port (roadmap gate G5).

## 3. Ground yourself first

Read these before you edit anything.

- `docs/voice/plan.md`: the full Voice plan. Read all of it. Focus on §4.1 (keys), §5 (architecture: 5.1 layout, 5.3 threads, 5.4 interfaces, 5.5 state machine), §6.1 (hook), §6.2 (audio), §6.4 (STT), §6.8 (delivery), §7 (latency budget), §10 (M0), and §12 (risks).
- `docs/voice/notes.md`: the **Decisions** table, findings F1–F10, the shell's structure, and machine facts. The Decisions table was resolved on 2026-10-18 and overrides `plan.md` where they differ. The decisions are:
  - Q1: No shell rewrite. Voice is a workspace crate; the root package stays put.
  - Q2: CUDA build (user, 2026-10-18); default model large-v3-turbo q5_0.
  - Q3: Smart-mode LLM is OpenRouter (user, 2026-10-18). Not used in M0.
  - Q4: Right Ctrl (dictate) / Right Alt (smart), with chord passthrough (user, 2026-10-18).
  - Q5: Insert immediately; the review step is a setting (plan default). Not used in M0.
  - Local API transport: per-user named pipe, not localhost TCP (user, 2026-10-18). Not built in M0.
- `HANDOFF.md`: current app state, "How it works", and the verification method (scripted input plus screenshots). Use DXGI desktop duplication for screenshots, because a GDI screen copy shows the DWM blur as black.
- `docs/Features/MyNotch/roadmap.md`: the gates that apply to this work.
  - G1: run the app, don't just compile. Back up `settings.ini` and the clipboard before runtime tests, and restore them after.
  - G2: `cargo clippy --all-targets` shows no warnings, and `cargo test` is green.
  - G3: focus discipline.
  - G5: local only, no network port.
  - G7: commit to `main`. A running release exe locks `target\release`.
- Code. Line numbers are as of 2026-10-18, in a working tree that still contains uncommitted timer-style work. Use the names as the stable reference.
  - `src/app.rs:466` `run()`: single instance, DPI awareness, `RegisterClassW`, `CreateWindowExW` at `src/app.rs:486` with `WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`, `SetTimer` 16 ms frame tick at `src/app.rs:507`, and the `GetMessageW` loop at `src/app.rs:511`.
  - `src/app.rs:720` `begin_typing` / `src/app.rs:747` `end_typing`: the only places the notch takes the foreground (search field, Shortcut recorder, Weather field).
  - `src/app.rs:1109` `App::compact` (picks the highest-priority `Compact` from the widgets), `src/app.rs:1122` `tick`, `src/app.rs:1280` `fn animation` (how often to redraw; its rail rule is at `:1285`), `src/app.rs:1474` `render`, `src/app.rs:1642` rail drawing, `src/app.rs:1926` `WM_TIMER`, `src/app.rs:2021` `WM_MOUSEACTIVATE` → `MA_NOACTIVATE`.
  - `src/widgets/mod.rs:145` `Compact { priority: u8, text, live, progress }` ("the widget with the highest priority wins"), `:188` `Widget::compact`, `:241` `animating`, `:309` `all()` (tab order). Current priorities: focus timer running 20 / paused 5, media playing 10, stopwatch running 8 / paused 4, system 1.
  - `src/clipboard.rs`: `AddClipboardFormatListener` at `:99`, `SETTLE_MS = 150` at `:44`, `write_text` at `:149`, `WM_CLIPBOARDUPDATE` at `:219`, `is_private` at `:259` (skips copies marked `ExcludeClipboardContentFromMonitorProcessing`, `Clipboard Viewer Ignore`, or `CanIncludeInClipboardHistory` = 0).
  - `src/media.rs` and `src/weather.rs`: the worker pattern to copy. A `std::thread` plus `std::sync::mpsc` channels; the UI thread drains the `Receiver` on its 16 ms frame tick. Nothing posts window messages.
  - `src/hotkey.rs` (`RegisterHotKey` wrapper), `src/desktop.rs:23` `claim_single_instance`, and `src/theme.rs` (style tokens).
- Git checks, run first: `git branch --show-current`, `git status --porcelain`, `git log --oneline -5`.
- Scripted-input safety rules for this PC (learned 2026-10-15):
  - Before sending any shortcut chord with SendInput, probe it. `RegisterHotKey(NULL, id, mods, vk)` from the test process succeeds only if no app owns the chord; unregister it afterwards. Never send an unprobed chord. Ctrl+Alt+Space belongs to Claude desktop on this PC.
  - Before clicking or typing into a test window, verify it is the foreground window, or the window under the click point.
  - Tell the user before a scripted run moves the mouse or types. Keep runs short. If an unexpected window appears, stop and restore state.
  - The user may keep working during a run.

**Step 0 gates (do these before writing code):**

- Toolchain. Check `nvcc --version`, `$env:CUDA_PATH`, `cmake --version`, and `clang --version` (or `$env:LIBCLANG_PATH`). On 2026-10-18 none of these were on PATH. The user was given these commands, which they run themselves since each needs admin approval, followed by a restart of Claude Code:
  ```
  ! winget install -e --id Nvidia.CUDA
  ! winget install -e --id Kitware.CMake
  ! winget install -e --id LLVM.LLVM
  ```
  - The **CUDA Toolkit is required**. If `nvcc` or `CUDA_PATH` is missing, stop with status BLOCKED and give the user the first command.
  - CMake and Ninja: if they aren't installed, Visual Studio 18 ships both. Put them on PATH for the build: `C:\Program Files\Microsoft Visual Studio\18\Community\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin` (cmake.exe) and `...\CMake\Ninja` (ninja.exe). The MSVC environment script is `C:\Program Files\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat`.
  - LLVM/libclang is needed only if the `whisper-rs-sys` build fails asking for libclang (for bindgen). If it does, stop with BLOCKED and give the user the third command.
- Known CUDA risk. `nvcc` may reject Visual Studio 18's MSVC as a host compiler, or CMake's Visual Studio generator may lack CUDA's MSBuild integration for VS 18. If the build fails that way, first try the Ninja generator (`CMAKE_GENERATOR=Ninja`) from an environment set up by `vcvars64.bat`. Do not switch to the Vulkan backend without asking the user; they chose CUDA on 2026-10-18. The first CUDA build of whisper.cpp is slow, so run it in the background.
- Debug builds and latency. The `cmake` crate builds whisper.cpp in CMake's Debug configuration when the Rust profile has `opt-level = 0`, which makes inference far slower. Add a per-package override to the root `Cargo.toml` so debug app builds stay usable, for example `[profile.dev.package.whisper-rs-sys] opt-level = 3`, and confirm in the build output that the override took effect. Take every latency number from a `--release` build.
- Release build. A release build of MyNotch is usually running from `target\release\mynotch.exe`. Check with `Get-Process mynotch`. The single-instance guard makes a second copy exit at once, and the running exe locks `target\release`. Before any runtime test, ask the user to quit it (right-click the notch → Quit) or approve stopping it.
- Model. Download `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin` (~550 MB) to `%LOCALAPPDATA%\MyNotch\models\`. Record its SHA-256 in `docs/voice/notes.md`. Voice M1's model manager will verify against that hash.

## 4. Pre-existing changes warning

The working tree already contains finished but uncommitted timer-style work from before Voice M0. Do not revert, stage, or commit these as part of Voice M0.

- Modified: `HANDOFF.md`, `src/app.rs`, `src/gfx.rs`, `src/settings.rs`, `src/theme.rs`, `src/widgets/focus.rs`, `src/widgets/mod.rs`, `src/widgets/time.rs`.
- Untracked: `src/widgets/gallery.rs`, `src/widgets/styles.rs`.
- That work is the eight Timer card styles and a style gallery, described in HANDOFF.md.
- Also untracked, but part of this effort: `docs/voice/notes.md` and `docs/voice/plan.md`. These are this effort's own docs, and they ARE committed with Voice M0.
- `Voice-M0-ExecutionPrompt.md` (this prompt, in the repo root) is untracked. Don't commit it unless the user asks; at commit time, ask whether to keep, move or delete it.

Before you edit `src/app.rs` or `HANDOFF.md` (both already carry timer-style changes, and both change in M0), ask the user whether to commit the timer-style work first, as its own commit. Recommended: run `cargo build`, `cargo clippy --all-targets`, and `cargo test`. If all three are green, commit it with a plain-English message about the eight timer styles and the style gallery. Never mix voice changes into that commit.

If the user declines, stop before you touch `src/app.rs` or `HANDOFF.md` and ask how they want it handled. Interactive `git add -p` is not available in this environment.

## 5. Implementation scope

**A. Workspace.** Make the root `Cargo.toml` both the existing package and a `[workspace]` with members `crates/mynotch-voice` and `tools/voicectl`. Move no existing files. The app depends on `mynotch-voice` by path. Edition 2024.

**B. `crates/mynotch-voice`.** This crate has no UI dependency (plan D5). Use plan §5.1's module names, but only the ones M0 needs.

- `lib.rs`: `VoiceService`, which starts and returns a handle with a command sender and an event receiver.
- `pipeline.rs`: the subset of plan §5.5's state machine that M0 needs: Idle → Arming → Listening → Transcribing → Delivering → Done → Idle, plus Cancelled and Failed. One session at a time. A key-down during Transcribing or Delivering is ignored. Unit-test the transitions.
- `audio/`: `cpal` default input (WASAPI shared), opened on key-down and closed when the session ends. Keep-warm and pre-roll are Voice M1. Downmix to mono f32, then resample to 16 kHz with `rubato`. The cpal callback only pushes into a lock-free `ringbuf` ring buffer: no allocation, no locks. Hard cap of 5 minutes per session.
- `stt/`: an `SttEngine` trait (plan §5.4) and a whisper implementation using `whisper-rs` with its `cuda` feature. Use the current published version, and check its README for build requirements. Load the model once at startup on a background thread, then run a 1 s warm-up inference. Parameters: language `en`, no timestamps, temperature 0. Run inference off the async threads (`spawn_blocking`).
- `deliver/`: clipboard paste only (see D).
- `platform/win/`: ALL Win32 `unsafe` code (hook, foreground window, SendInput, clipboard). Nothing else in the crate calls Win32 or uses `unsafe` (plan §0 rule 5).
- A `tokio` runtime on its own thread runs the pipeline (plan §5.3).
- Events: the M0 subset of plan §5.4's `VoiceEvent`: SessionStarted, Transcribing, Transcript, Delivered, Cancelled, Failed. Level is optional. Send them on a channel that the UI drains on its 16 ms frame tick, using the same pattern as `media.rs` and `weather.rs`.
- Logging: `tracing` to `%LOCALAPPDATA%\MyNotch\voice.log`. The app is GUI-subsystem with no console. For every session, log per-stage milliseconds: key-up → audio finalized, STT, paste, and total key-up → paste sent. Also log hook install and re-install lines, and the backend whisper reports (CUDA).
- Model path: `%LOCALAPPDATA%\MyNotch\models\ggml-large-v3-turbo-q5_0.bin`, overridable with the env var `MYNOTCH_VOICE_MODEL`. No `voice.toml` in M0.
- Graceful failure: if the model or CUDA fails to load, voice is disabled, the error is logged, and the rest of the notch keeps working.

**C. Hook (`platform::win::hook`), per plan §6.1.**

- `SetWindowsHookExW(WH_KEYBOARD_LL)` on a dedicated thread with its own `GetMessageW` loop. The callback only posts to a channel and returns in microseconds.
- Trigger: `VK_RCONTROL` (0xA3) only. Right Alt / Smart mode is Voice M2.
- Swallow the trigger's key-down and key-up. Ignore auto-repeat key-downs while the key is held.
- A hold shorter than 150 ms is a tap: no session, and the audio is discarded. The double-tap latch is Voice M1.
- Tag every `SendInput` that MyNotch makes with a unique `dwExtraInfo` marker. Never treat events carrying that marker as triggers. In normal mode, also ignore `LLKHF_INJECTED` events.
- **Chord passthrough** (user decision, 2026-10-18): if any other key goes down while Right Ctrl is held, cancel the session (discard the audio, paste nothing). Then inject a tagged Right Ctrl key-down, and let that other key through. The app then sees a normal chord: Right Ctrl+C copies, and Right Ctrl+Alt+K still opens MyNotch's search. The later Right Ctrl key-up passes through, not swallowed.
- Write the decision logic as a pure state machine with unit tests covering: tap, hold, chord, auto-repeat, and own-injected events.
- Hook survival: re-install the hook after resume from sleep (`PBT_APMRESUMEAUTOMATIC`), and log each install. Message-only windows don't receive broadcast messages such as `WM_POWERBROADCAST`, so use `RegisterSuspendResumeNotification` or a hidden top-level window on the hook thread. The full watchdog with a synthetic self-check is Voice M5.
- Known limitation (notes.md F8): a non-elevated low-level hook gets no events while an elevated window is in front. Document it. Do not work around it.

**D. Delivery, per plan §6.8 step 2.**

- Capture the foreground HWND at key-down and log it. The foreground-changed guard itself is Voice M1; M0 only logs whether the foreground changed.
- Snapshot the clipboard: every format whose data is an `HGLOBAL` (at minimum `CF_UNICODETEXT`). An empty clipboard is restored as empty.
- Set the text, plus `ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory` = 0, and `CanUploadToCloudClipboard` = 0. The plan lists these formats under Voice M1. They were pulled into M0 on 2026-10-18, because the 1-hour soak would otherwise fill Windows' Win+V history and MyNotch's own Clipboard tab with dictations. Report this in the final report.
- Wait, with a short timeout, until no Shift, Ctrl, Alt, or Win key is physically held. Then `SendInput` Ctrl+V (tagged), wait 150 ms, and restore the snapshot.
- Restoring the clipboard is itself a clipboard change (notes.md F3). MyNotch's Clipboard history (`src/clipboard.rs`) would capture it and move it to the top. The restore must not show up as a new history item. The voice crate must not depend on the app, so expose something the app can check, for example the clipboard sequence number after the restore, via an event or a shared atomic. Have `clipboard.rs` skip that update.

**E. Notch readout (`src/app.rs`, shell-level; Voice is not a tab in M0).**

- Merge a voice readout into `App::compact` (`src/app.rs:1109`) with a priority above every current one (the current maximum is 20): "● Listening" (live) while the key is held; "Transcribing…"; "✓ Inserted" for about 1.5 s; and a short one-line message on failure.
- Watch out: `App::compact` returns `(widget index, Compact)`, and its callers look the widget up by that index (`self.widgets[i].animating()` in `fn animation` at `src/app.rs:1285`; the rail drawing at `src/app.rs:1642`). A voice readout has no widget behind it, so change the return type and its callers; don't fake an index.
- It must not expand or open the notch, call `SetForegroundWindow`, or change any window style. Keep frames coming while listening (`fn animation`, `src/app.rs:1280`). Use `src/theme.rs` tokens and the rail's existing text style.
- While the notch is hidden over a fullscreen app, nothing shows in M0 (notes.md F2; decided in Voice M1).
- Start `VoiceService` from `run()` / `App::new`.

**F. `tools/voicectl` (console binary).** `voicectl transcribe <file.wav> [--model <path>]` prints the text and timings (model load ms, warm-up ms, STT ms), using `mynotch-voice`'s STT directly. This is how GPU latency is measured without a microphone.

**G. Debug test switches.** These are environment variables. They are off by default, read once at startup, logged when active, and documented in `docs/voice/notes.md`.

- `MYNOTCH_VOICE_FAKE_MIC=<wav path>`: when a session stops, the captured audio is replaced by that WAV's audio (resampled to 16 kHz mono).
- `MYNOTCH_VOICE_ACCEPT_INJECTED=1`: injected Right Ctrl events that don't carry MyNotch's own marker can start sessions, so scripted `SendInput` can drive the whole path.

**H. Docs.**

- Update `HANDOFF.md`: a Voice M0 entry in "Decisions made", the current state, new rows in the file table, and a "Verified" entry listing what was and wasn't verified.
- Update `docs/voice/notes.md`: M0 results (measured latencies, model SHA-256, toolchain versions, build notes, and the debug switches).

## 6. Strict non-goals

Voice M0 must not include any of the following.

- VAD / Silero; cleanup (fillers, repeats); spoken commands; vocabulary and replacements.
- Right Alt, Smart mode, any LLM or OpenRouter call, profiles, templates.
- Double-tap latch, Esc cancel, re-paste (Ctrl+Alt+V).
- History / SQLite, `voice.toml`, any Settings-tab UI for voice.
- Model manager (download UI, SHA verification), keep-warm and pre-roll, device picker or device-change handling.
- Unicode-typing fallback; foreground-changed and elevated-target guardrails (log only).
- The local API / named-pipe server, the TypeScript client, any change to the Chorus or Signal repos.
- An expanded-notch voice view or waveform; showing voice over fullscreen apps.
- Any change to the notch's window styles, focus rules (G3), or existing widgets' behaviour; the Phase 5 mic indicator; rewriting the shell.
- Anything that opens a network listener (G5).

## 7. Required workflow

- You are the Coordinator. You plan, integrate, and verify. You may dispatch worker subagents for self-contained pieces, for example the hook state machine and its tests, audio capture, or `voicectl`. You own the integration in `src/app.rs` and `src/clipboard.rs` yourself.
- After each piece: a spec-compliance review against this prompt and `docs/voice/plan.md`, then a code-quality review by a fresh subagent. Resolve every finding (fix it, or record why not) before you move on.
- Then run all of section 8.
- Commit: one intentional commit for Voice M0 on `main`, including `docs/voice/`. Give it a plain-English title and a description that a non-technical person can read first, with technical detail second. End the commit message with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Do not push or open a PR unless the user says so.

## 8. Verification

Run everything from the repo root.

1. Build and lint: `cargo build --workspace` (the first CUDA build is slow; run it in the background), `cargo clippy --workspace --all-targets` with no warnings, and `cargo test --workspace` green. Record the test count before and after.
2. Grep gates (expected: 0 matches each):
   - `rg -n "unsafe" crates/mynotch-voice/src --glob '!**/platform/win/**'`
   - `rg -n "windows::" crates/mynotch-voice/src --glob '!**/platform/win/**'`
   - `rg -n "TcpListener|UdpSocket|TcpStream" src crates tools`
3. GPU STT latency (no microphone needed). Make a ~10 s speech WAV with Windows' built-in text-to-speech, in a scratch folder outside the repo (set `$scratch` to that folder first):

   ```powershell
   Add-Type -AssemblyName System.Speech
   $s = New-Object System.Speech.Synthesis.SpeechSynthesizer
   $fmt = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo 16000, ([System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen), ([System.Speech.AudioFormat.AudioChannel]::Mono)
   $s.SetOutputToWaveFile("$scratch\m0-10s.wav", $fmt)
   $s.Speak("Open the settings file, add a section for the voice model, then rebuild the whole workspace and check that every warning is gone before you commit the change to the main branch.")
   $s.Dispose()
   ```

   Run `cargo run --release -p voicectl -- transcribe "$scratch\m0-10s.wav"` three times. Report the WAV's duration, the warm STT ms of each run, the transcript, and evidence that CUDA was actually used (whisper.cpp's CUDA init log lines, and the process visible in `nvidia-smi`). The plan's budget for Whisper is 400–600 ms for a 15 s utterance.
4. Runtime in the real app (G1). Run this only after you have told the user what will happen and received their go-ahead.
   - Back up `%APPDATA%\MyNotch\settings.ini` and the user's clipboard first. Restore both at the end.
   - Launch the new **release** build (`target\release\mynotch.exe`) with `MYNOTCH_VOICE_FAKE_MIC` set to the WAV from step 3, and with `MYNOTCH_VOICE_ACCEPT_INJECTED=1`.
   - Open a fresh Notepad window and confirm it is the foreground window (`GetForegroundWindow`, plus its title).
   - Put a known sentinel string on the clipboard. Send Right Ctrl down, hold for about 2 s, then send Right Ctrl up.
   - Observe and report each of these:
     - The text appears in Notepad. Read it back with UI Automation or `WM_GETTEXT`, not via the clipboard.
     - The foreground window stays Notepad for the whole session. Poll `GetForegroundWindow` every ~10 ms. It must never be MyNotch's window.
     - The clipboard afterwards equals the sentinel.
     - `voice.log`'s total key-up → paste is ≤ 1000 ms.
     - The rail shows Listening, then Transcribing, then Inserted (DXGI desktop-duplication screenshots).
     - MyNotch's Clipboard tab has no new item for the dictation or the restore.
     - If Windows clipboard history is on, it gained no dictation entry. Mark this "not verified" if you can't check it.
   - Chord passthrough: with Notepad focused, send Right Ctrl down, A down, A up, Right Ctrl up. Confirm Select All happened (UI Automation selection) and nothing was pasted.
   - Tap: Right Ctrl down and up within 100 ms pastes nothing.
   - Probe every chord before you send it (section 3 rules).
5. User-run checks. You can't speak into the mic or put the PC to sleep, so give the user a short checklist and read `voice.log` afterwards.
   - Live mic: in Notepad, hold Right Ctrl, speak for about 10 s, and release. Report the text and the log's stage timings.
   - Soak: an hour of normal use, including one sleep/resume. Afterwards, check the log for hook re-install lines, and confirm sessions still work.
   - If the user can't do these during the session, report them as pending and use status DONE_WITH_CONCERNS.
6. At the end, restore `settings.ini` and the clipboard. Then ask the user whether to relaunch the release build of the new code, or the one they were running before.

## 9. Failure honesty

- If a verification command fails for an environment reason unrelated to your change, capture the exact output, explain it, and do not claim success.
- Never report a check as passed unless you observed it. Otherwise say "not verified" and say why.

## 10. Final report

Your final report must include:

- Status: one of DONE / DONE_WITH_CONCERNS / NEEDS_CONTEXT / BLOCKED.
- Files changed (new and modified).
- Build, clippy, and test results, with counts. Grep gate results.
- Measured latencies (voicectl runs and the in-app session), and what was actually observed at runtime.
- Review outcomes: each finding and how it was resolved.
- Confirmation that each non-goal was left alone.
- Decisions made during the session, including the exclusion formats pulled forward from Voice M1.
- Residual risks and the user-run checks still pending.
- Final `git status`, and the commit hash if a commit was made.
