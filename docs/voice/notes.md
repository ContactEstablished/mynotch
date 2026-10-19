# MyNotch Voice — notes on the current shell

## Decisions (open questions in plan §13, resolved 2026-10-18)

| # | Question | Decision |
|---|---|---|
| Q1 | UI framework / rewrite? | Raw Win32 + Direct2D, already non-activating. **No rewrite.** Voice is a workspace crate; see "Verdict" below. |
| Q2 | GPU / backend | RTX 4080 SUPER, 16 GB. **CUDA** build (user). The CUDA Toolkit must be installed first, and its host-compiler check must accept VS 18's MSVC. Default model `large-v3-turbo` q5_0 as planned. |
| Q3 | Smart-mode LLM | **OpenRouter** by default (user). `local_only` still forces Ollama. |
| Q4 | Trigger keys | **Right Ctrl (dictate) / Right Alt (smart), with chord passthrough** (user): if another key goes down while a trigger is held, the session cancels and the modifier is replayed as an injected key-down, so right-hand shortcuts keep working (F6). |
| Q5 | Review step by default? | **Insert immediately** (the plan's `review_before_insert = false`); the review step is a setting. Not asked; the plan's default. |
| — | Local API transport (F1) | **Per-user named pipe**, current-user ACL, JSON messages with `"v": 1` (user). Same transport as Signal's (roadmap D4). G5 stays. §8.1's token file and Origin checks are dropped; §8.3's HTTP endpoints become pipe requests plus `voicectl transcribe <wav>`. The M3 check "a Python script can transcribe a WAV via HTTP" becomes "via the pipe". |

Written 2026-10-18 against `main` at `67b5b96` (plus uncommitted timer-style work in the tree: `gallery.rs`, `styles.rs` and edits to `app.rs`, `gfx.rs`, `focus.rs`, `time.rs`, etc.). Line numbers are from that working tree and will drift; function names are the stable reference.

## UI framework

None. MyNotch is native Rust 2024 on the `windows` crate 0.62: raw Win32 for the window and input, Direct2D + DirectWrite for drawing (`ID2D1DCRenderTarget` on a DIB, pushed with `UpdateLayeredWindow`). There is no winit, egui, iced or Slint. Dependencies today are only `windows`, `windows-numerics` and `sysinfo`: no tokio, serde or async code.

## How the notch window is created

`app::run()` (`src/app.rs:466`):

1. `desktop::claim_single_instance()` (named mutex `Local\MyNotch.SingleInstance`); a second copy exits.
2. `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)`.
3. `RegisterClassW("MyNotchWindow", wndproc)`, then `CreateWindowExW` (`src/app.rs:486`):
   - ex-styles **`WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`**, style `WS_POPUP`, size 0×0. Position, size and alpha are applied on every render by `UpdateLayeredWindow`.
   - `WM_MOUSEACTIVATE` returns **`MA_NOACTIVATE`** (`src/app.rs:2021`).
   - Click-through: the frame timer toggles `WS_EX_TRANSPARENT` so the window only takes clicks while the cursor is inside the body.
4. `App::new(hwnd)` goes into a thread-local `APP: RefCell<Option<App>>`; `register_hotkey()` (`RegisterHotKey`, default Ctrl+Alt+K); tray icon added.
5. `ShowWindow(SW_SHOWNOACTIVATE)`.

A second window, `Backdrop` (`src/backdrop.rs`), is a layered, click-through, topmost window kept just under the notch to get DWM's blur.

**So the plan's §4.2 focus requirement is already met.** Hover, expand and clicks never take the foreground. The only exceptions are explicit: `begin_typing` (`src/app.rs:720`) calls `SetForegroundWindow` for the search field, the Shortcut recorder and the Settings text field, and `end_typing` gives the foreground back. The right-click menu also takes it briefly.

## Event loop

- One UI thread, a plain `GetMessageW` / `TranslateMessage` / `DispatchMessageW` loop (`src/app.rs:511`).
- A 16 ms `SetTimer` frame tick (`TIMER_FRAME`, handled in `WM_TIMER`) runs all the time. Each tick polls the workers and the widgets' `update()`, and redraws only when something changed or an animation is on screen.
- Workers are plain `std::thread`s talking over `std::sync::mpsc`: media (MTA, WinRT; `media.rs`), weather (WinHTTP; `weather.rs`), apps enumeration (STA; `apps.rs`) and short-lived launch threads. The UI side owns a `Receiver` and drains it on the frame tick. Nothing posts window messages to wake the loop; the 16 ms tick is the wake-up.
- The clipboard listener runs on the UI thread through its own message-only window (`clipboard.rs`).

## Verdict: no rewrite needed

The shell can host Voice as it stands:

- The window is already non-activating (`WS_EX_NOACTIVATE`, `MA_NOACTIVATE`).
- A tokio runtime can live on its own thread inside the voice crate. The UI consumes `VoiceEvent`s exactly as it consumes media and weather updates: drain a channel on the frame tick (a 16 ms worst-case delay on screen, which is fine).
- The low-level keyboard hook needs its own thread with its own message loop anyway (§5.3), so it doesn't touch the UI loop.
- Voice states become a widget-like readout in the collapsed rail (the existing `Compact { priority, text, live }` mechanism) plus a voice view in the expanded notch.

Proposed layout (instead of the plan's `crates/mynotch-app`): keep the existing package at the root so no current files move (there is uncommitted work in the tree), and make the root `Cargo.toml` a workspace too, with members `crates/mynotch-voice` (no UI dependency, D5) and `tools/voicectl`. The app depends on `mynotch-voice`.

## Things in the repo that bear on the plan

| # | Finding | Effect on the plan |
|---|---|---|
| F1 | The roadmap's prime contract **G5: "Local only. No API listens on a network port."** Signal's API is planned as a per-user named pipe (roadmap D4). | §8's `127.0.0.1` WebSocket/HTTP server breaks G5. A per-user named pipe (current-user ACL) makes the token file and `Origin` checks unnecessary, since browsers can't open pipes, and matches Signal's transport. Node/Electron (Chorus) connects with `net.connect('\\\\.\\pipe\\…')`; Python can open the pipe as a file, or use `voicectl`. **Needs the user's decision.** |
| F2 | The rail (`Compact`) and the fullscreen auto-hide: over a fullscreen app the notch fades out and hides (checked every 500 ms). | Voice must still show Listening/Delivered over fullscreen apps, or say nothing. Decide at M1; M0 can ignore it. |
| F3 | Clipboard history (`clipboard.rs`) already skips copies marked `ExcludeClipboardContentFromMonitorProcessing` / `CanIncludeInClipboardHistory = 0`. | Paste delivery's exclusion formats (§6.8) keep dictations out of MyNotch's own history too. But **restoring** the user's original clipboard is a new clipboard change, so the history would re-capture it and move it to the top. Delivery should tell the clipboard widget to ignore its own restore. |
| F4 | The search field, Shortcut recorder and Weather field make the notch the foreground window (`begin_typing`). | A voice session started while one of those is focused would paste into MyNotch itself. Either end typing first or treat the notch's own field as a valid target (dictating into search is useful). |
| F5 | Global shortcuts use `RegisterHotKey` via `hotkey.rs` (parse, show, conflict reporting). | Re-paste (Ctrl+Alt+V) can reuse it. Hold-to-talk needs key-up, so it needs the `WH_KEYBOARD_LL` hook (§6.1). Probe Ctrl+Alt+V for conflicts first; on this PC Ctrl+Alt+Space belongs to Claude desktop. |
| F6 | Swallowing **Right Ctrl** while held means Right Ctrl + C / V / K stop working, including MyNotch's own Ctrl+Alt+K when pressed with the right-hand Ctrl. | Option: if another key goes down while the trigger is held (before speech starts), cancel the session and replay the modifier with an injected key-down, so chords still work. |
| F7 | Roadmap Phase 5 plans a "mic in use" dot. | Keep-warm (§6.2) keeps the mic open for 60 s, so MyNotch's own indicator would light up. Phase 5 should exclude MyNotch's own capture, or show it differently. |
| F8 | A non-elevated low-level keyboard hook gets **no** events while an elevated window has the foreground (UIPI). | The §6.8 "Target is running as admin" message can't come from a key press with an admin window in front, because the hook never fires. Detect it anyway for API- and notch-started sessions; document the limitation. |
| F9 | Naming: HANDOFF uses "Milestone 1–4", the roadmap "Phase 1–5". | The voice plan's "M0–M5" collides with HANDOFF's milestones. Refer to them as **Voice M0–M5** in commits and HANDOFF. |
| F10 | Settings are a hand-parsed `key=value` `settings.ini`; there's no serde yet. | `voice.toml` brings `serde` + `toml` into the voice crate only. The shell's settings.ini stays as it is. |

## Machine (dev PC)

- GPU: **NVIDIA GeForce RTX 4080 SUPER, 16 GB** (driver 610.88). CPU: i9-14900KF (24 cores / 32 threads). RAM: 64 GB.
- Rust 1.98.1 (MSVC). Visual Studio 18 Community is installed, with a CMake under `Common7\IDE\CommonExtensions\Microsoft\CMake` (not on PATH).
- Installed for Voice M0 (the user, 2026-10-18, with winget): CUDA Toolkit 13.4 (`nvcc` 13.4.59, `CUDA_PATH` set, its `bin` and `bin\x64` on PATH, with VS 18 integration), CMake 4.4.4 (on PATH), LLVM 23.1.3 (in `C:\Program Files\LLVM`, not on PATH).
- **Not installed:** Vulkan SDK (`VULKAN_SDK`), Ninja, Ollama.
- whisper.cpp (via `whisper-rs-sys`) builds with CMake. The `cuda` feature needs the CUDA Toolkit. `vulkan` needs the Vulkan SDK (for `glslc`); at runtime it only needs `vulkan-1.dll`, which the NVIDIA driver ships. A CUDA build has to ship the cuBLAS DLLs (several hundred MB) with the app; see the M0 build notes below.

## Voice M0 results (2026-10-18)

The walking skeleton works on the dev PC: hold Right Ctrl in any app, the microphone records, Whisper (large-v3-turbo q5_0, CUDA) transcribes on the GPU, and the text is pasted at the cursor; the collapsed notch says "● Listening", "Transcribing…", "✓ Inserted".

### Measured

| What | Result |
|---|---|
| `voicectl transcribe` on a 10.33 s TTS WAV (release), 3 runs | STT **120 / 117 / 117 ms** (flash attention on; 192 ms with it off). Model load 591–634 ms, warm-up 100–104 ms. Transcript exact each time. `nvidia-smi` listed `voicectl.exe` as a GPU compute process; whisper.cpp logged `ggml_cuda_init: found 1 CUDA devices: Device 0: NVIDIA GeForce RTX 4080 SUPER, compute capability 8.9` and `whisper_backend_init_gpu: using CUDA0 backend`. |
| In the app (release, `MYNOTCH_VOICE_FAKE_MIC` = that WAV), Right Ctrl held 2 s in Notepad | `voice.log`: key-up → audio ready **10 ms**, STT **127 ms**, paste **20 ms**, total key-up → paste sent **157 ms** (budget 1000 ms). The real microphone opened 132 ms after the press (18–22 ms on later presses). |
| App startup to "voice is on" | ~0.9 s (model load 584 ms + warm-up 156 ms, on a background thread). |
| Audio finish (downmix + resample) | 2.4 ms for 10 s of 48 kHz stereo (release, measured by the audio worker). |

The plan's budget for Whisper is 400–600 ms for a 15 s utterance; the GPU does a 10 s one in about 120 ms.

### Model

`%LOCALAPPDATA%\MyNotch\models\ggml-large-v3-turbo-q5_0.bin`, downloaded 2026-10-18 from `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin`: 574,041,195 bytes, **SHA-256 `394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2`** (matches the repo's LFS hash). Voice M1's model manager verifies against this.

### Toolchain and build notes

- Versions: Rust 1.98.1; whisper-rs 0.16.0 → whisper-rs-sys 0.15.0 → whisper.cpp 1.8.3; CUDA 13.4.59 with MSVC 19.51.36260 (VS 18, tools 14.51.36231) as nvcc's host compiler; CMake 4.4.4 with the Visual Studio generator (no Ninja needed: CUDA 13.4 ships VS 18 build customizations); LLVM 23.1.3 for bindgen; cpal 0.18.2, rubato 5.0.1, ringbuf 0.5.3, hound 3.5.1, tokio 1.53, tracing-subscriber 0.3.23.
- **bindgen** (whisper-rs-sys generates its bindings at build time) needs two environment variables on this PC, set in `.cargo/config.toml` (a value already in the environment wins): `CLANG_PATH` (libclang only finds its own `stdbool.h` by asking `clang.exe`, and winget's LLVM isn't on PATH) and `VCToolsInstallDir` (clang 23 doesn't discover VS 18, so it can't find `stdio.h`). The second names the installed MSVC version and must be updated when Visual Studio updates its tools. When bindgen fails, whisper-rs-sys only prints a hidden warning and falls back to its bundled Linux bindings, and the build then fails with `attempt to compute 12_usize - 16_usize, which would overflow` in `bindings.rs`. Changing those variables doesn't rerun its build script by itself; `cargo clean -p whisper-rs-sys` does.
- whisper-rs-sys 0.15 builds whisper.cpp with CMake's **Release** config in every profile (`cmake --build … --config Release`, seen in the log), so debug app builds aren't slow. The root `Cargo.toml` still has `[profile.dev.package.whisper-rs-sys] opt-level = 3` as asked (`OPT_LEVEL=3` reaches its build script); it doesn't change the CMake config today.
- whisper.cpp is built for this GPU only (`GGML_NATIVE` → `CUDA : ARCHS = 890`, compute 8.9). A build for other GPUs needs `CMAKE_CUDA_ARCHITECTURES` (M5). The first CUDA build takes about 4 minutes per profile.
- The CUDA runtime is linked **statically** and loads the driver (`nvcuda.dll`) itself; the exe's only CUDA import is `cublas64_13.dll` (which needs `cublasLt64_13.dll`). The root `build.rs` **delay-loads** it, so the notch starts without CUDA, and voice checks for `nvcuda.dll`, `cublas64_13.dll` and `cublasLt64_13.dll` before touching whisper.cpp. `build.rs` fails the build if `CUDA_PATH` isn't CUDA 13 (the DLL names carry the version).
- New runtime needs: the release exe is 33 MB (was 1.5 MB) and now imports `MSVCP140.dll` and `VCRUNTIME140_1.dll` (the VC++ 2015–2022 runtime, for whisper.cpp's C++); on the dev PC they're present.
- `cargo test` / `cargo clippy` at the root cover the app and the voice crate (`default-members`); add `--workspace` to include `voicectl`.

### Debug switches (environment variables, off by default, read once at startup, logged when set)

| Variable | Effect |
|---|---|
| `MYNOTCH_VOICE_MODEL=<path>` | Use another ggml model file instead of `%LOCALAPPDATA%\MyNotch\models\ggml-large-v3-turbo-q5_0.bin`. |
| `MYNOTCH_VOICE_FAKE_MIC=<wav>` | Each session still opens and records the real microphone (so capture is exercised), but its audio is replaced by this WAV (resampled to 16 kHz mono) when the key comes up. If the microphone can't open, the session carries on with the WAV. |
| `MYNOTCH_VOICE_ACCEPT_INJECTED=1` | Injected Right Ctrl events without MyNotch's own marker can start sessions, so scripted `SendInput` can drive the whole path. MyNotch's own injected keys (marker `0x4D4E5643`, "MNVC") never count. |

### Decisions made during M0

- **Exclusion formats pulled forward from Voice M1.** The dictation goes on the clipboard with `ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory` = 0 and `CanUploadToCloudClipboard` = 0, so the hour-long soak doesn't fill Win+V history or MyNotch's Clipboard tab.
- **The restore is marked too** (`CanIncludeInClipboardHistory` = 0, `CanUploadToCloudClipboard` = 0, unless the user's own clipboard carried them): it isn't a new copy, and without them Win+V would re-add the user's clipboard after every dictation. MyNotch's clipboard history also skips it by sequence number (`mynotch_voice::is_own_clipboard_restore`, read under the open clipboard) and waits while a paste is in progress (`is_clipboard_busy`), which closes a race where its 150 ms settle timer fired during the restore (F3).
- **Chord passthrough replays the key too.** SendInput from inside a low-level hook queues behind the event being processed, so letting the other key through and injecting Right Ctrl would reach the app as "A, then Ctrl". The hook swallows that key and injects Right Ctrl down followed by a copy of it (both tagged); everything after passes. Side effect: that first key-down reaches the app flagged as injected.
- **A Right Ctrl tap is replayed** (down + up, tagged), so apps that use Right Ctrl on its own (VirtualBox's Host key) still get it.
- **A lost key-up** (let go over an elevated window, F8) ends the held session at the next key: a trigger silent for 1.5 s with no auto-repeat counts as released, so the next ordinary key passes on its own (no phantom Ctrl) and the next Right Ctrl press starts afresh.
- A microphone that fails to open is reported only when the key comes up after a hold, so Right Ctrl shortcuts never flash it; letting go before the microphone opened says "The mic wasn't ready".
- The clipboard snapshot is taken at key-up, in parallel with Whisper, and reused if the clipboard hasn't changed since; formats past 64 MB in total are skipped before copying. GDI formats (bitmaps, metafiles) and private ones aren't kept: Windows remakes CF_BITMAP from the DIB, but a metafile-only clipboard loses its picture.
- If Shift/Ctrl/Alt/Win is still down 400 ms after the transcript, Ctrl+V goes out anyway and the log says so. A paste that went out but whose restore failed counts as delivered, with a warning in the log.
- If CUDA loads but finds no GPU, voice turns off rather than run Whisper on the CPU.
- The rail readout is priority 30 (above every widget); "Listening" shows once the microphone is open and the hold is past the 150 ms tap time; "Transcribing…" has no live dot; results and failures show 1.5 s / 3 s and survive a tap. Opening the notch on hover still picks a tab from the widgets alone.
- The 5-minute cap transcribes and pastes what was heard (the spec didn't say which).
- No transcript text is written to `voice.log`, only its length; window titles aren't logged either (exe name and HWND only).

### Known limitations and residual risks

- F8: with an elevated window in front, the hook gets nothing, so holding Right Ctrl there does nothing.
- Right Ctrl with the **mouse** (Ctrl+click, Ctrl+wheel) isn't a chord: Ctrl is swallowed and a session starts. A `WH_MOUSE_LL` hook would fix it (M1).
- The fixed 150 ms restore delay may be too short for slow paste targets (remote desktops, a busy IDE): they would paste the user's old clipboard (M1: per-profile delay, re-paste).
- No VAD (M1): holding Right Ctrl and saying nothing can paste a Whisper hallucination ("Thank you.").
- The microphone opens on every press (keep-warm is M1), so Windows' mic indicator flickers on Right Ctrl shortcuts.
- A press while a session is still transcribing or pasting is ignored without feedback (plan §5.5's "busy pulse" is later).
- Windows can still remove the hook silently under heavy load (the hook thread runs at time-critical priority to make it unlikely); there's no watchdog until M5. If re-installing after resume fails five times, voice logs it and stays off until restart.
- A whisper.cpp abort (CUDA error) would take the whole process down.

## Chorus

`C:\Projects\ContactEstablished\Chorus` (last commit `45d438d`, 2026-10-09): Node with `node-pty` and `@xterm/xterm` 6. That matches §9's pty-pane model. `Chorus 2.0` next to it has no git history or `package.json`.
