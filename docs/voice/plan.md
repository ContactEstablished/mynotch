> Saved 2026-10-18 from the plan the user supplied, unchanged below this note. Where the **Decisions** table in [`notes.md`](notes.md) differs from this plan (the local API is a named pipe, not localhost TCP; triggers have chord passthrough; CUDA; OpenRouter), `notes.md` wins. Milestones here are called **Voice M0–M5** to keep them apart from HANDOFF.md's Milestones 1–4 and the roadmap's Phases 1–5.

# MyNotch Voice — Implementation Plan

> A system-wide "hold a key, talk, text appears at the cursor" module for MyNotch, inspired by Lemon (heylemon.ai), built local-first, and exposed as a local service so **Chorus and any other app can use voice without implementing it themselves.**

**Platform:** Windows 11 only (v1) · **Language:** Rust · **Speech-to-text:** local Whisper (whisper.cpp) · **Status:** planning

---

## 0. Instructions for the implementing agent

1. Read the current MyNotch repo first. Record in `docs/voice/notes.md`: the UI framework (egui / iced / Slint / raw winit / other), how the notch window is created, its window styles, and how the app's event loop is structured.
2. MyNotch is very new. **A rewrite of the shell is allowed** if the current structure can't host a non-activating notch window plus a background tokio runtime cleanly. Propose it with reasons before doing it; don't rewrite silently.
3. Resolve the open questions in §13 with the user before starting M0.
4. Build milestone by milestone (§10). Each milestone ends with its acceptance checks passing. Do not start M2 features while M0/M1 latency or focus problems are open — those are the hard part.
5. Keep all Win32 `unsafe` code inside `platform::win`. Nothing else in the module calls Win32 directly.

---

## 1. Reference: what Lemon does

From Lemon's site and privacy policy (Oct 2026):

- **Push-to-talk anywhere.** Hold a key (fn on Mac) in any app with a text field; release and the result is typed where the cursor is. No per-app integrations.
- **Two outputs from the same voice input:**
  - *Voice-to-prompt* — a rambling, self-correcting thought becomes a structured prompt (goal, context, constraints, output format) for Claude, ChatGPT, Cursor, Claude Code.
  - *Dictation* — a separate key (control) gives your exact words with fillers ("um", "uh") stripped.
- **Compose in your tone.** Same key in Gmail/Slack produces a polished email or message instead of a prompt.
- **Edit by voice.** Rewrite selected text to a different tone by instruction.
- **Notch UI** at the top of the screen showing LISTENING and "✓ Typed into Gmail".
- **Extras:** opt-in screenshot context, OAuth integrations (Gmail, Calendar, Drive, Contacts), web search, opening apps/URLs.
- **Processing model:** audio is streamed to their cloud; stored audio and screenshots are kept until you email them to opt out, and may be used to train their models unless you opt out.

### What we keep, change, and drop

| Lemon feature | MyNotch Voice | Milestone |
|---|---|---|
| Hold-key push-to-talk in any app | Yes | M0 |
| Dictation with filler cleanup | Yes, fully local | M1 |
| Notch status UI ("Typed into X") | Yes | M1 |
| Voice-to-prompt, aware of the target app | Yes, via editable profiles + templates | M2 |
| Email / message compose "in your tone" | Yes, via a user-written style file | M2 |
| Edit selected text by voice | Yes | M4 |
| Cloud audio processing + retention | **Replaced:** audio never leaves the PC and is not stored by default | — |
| Screenshot context | Deferred, opt-in, local only | Later |
| OAuth integrations, web search, "instant answers" | Out of scope | — |
| macOS | Out of scope | — |
| **New:** local API so Chorus/other apps consume voice | Yes | M3 |
| **New:** custom vocabulary for dev jargon | Yes | M1 |
| **New:** spoken "send it" to submit in agent panes | Yes (API targets only) | M3 |
| **New:** transcribe-a-file endpoint | Yes | M3 |

---

## 2. Goals and non-goals

**Goals**
- One voice feature, hosted by MyNotch, usable everywhere: Chorus agent panes, terminals, browsers, Slack, VS Code/Cursor, email.
- Dictation that is fast enough to beat typing: text inserted ≤ 1 s after key release for a 10–20 s utterance.
- Smart rewrite that turns rambling into a usable prompt/email in ≤ 2.5 s p50.
- Privacy by default: dictation is 100% local; smart modes send transcript **text only** (never audio) to the configured LLM, and a single toggle forces local-only.
- A stable, versioned local API so Chorus stops owning any voice code.

**Non-goals (v1)**
- Live word-by-word streaming transcription into the target app.
- Wake words / always-listening.
- Agent actions (sending email, opening apps, searching the web).
- Cross-platform support.

---

## 3. Key decisions

| # | Decision | Why |
|---|---|---|
| D1 | Voice lives in MyNotch; Chorus is an API client | One implementation, every app benefits; Chorus stays focused on agents |
| D2 | Local Whisper via `whisper-rs` (whisper.cpp bindings); default model `large-v3-turbo` quantized (q5_0, ~550 MB); CPU fallback `small.en` | Accurate, private, no per-minute cost; turbo is fast on GPU |
| D3 | Batch transcription on key release (not streaming) for v1 | Push-to-talk clips are short; on GPU, batch is sub-second and far simpler |
| D4 | Delivery order: API target → clipboard paste → Unicode typing | API is cleanest for cooperating apps; paste works almost everywhere |
| D5 | Voice core has no UI dependency; UI subscribes to an event stream | Survives a MyNotch shell rewrite; same events feed the local API |
| D6 | LLM via any OpenAI-compatible endpoint (OpenRouter, Ollama, custom) | Reuse existing OpenRouter setup; Ollama for local-only |
| D7 | Prompt templates and app profiles are user-editable files | Tuning the rewrite is the main ongoing work; no rebuilds to tweak it |

---

## 4. User experience

### 4.1 Keys (all configurable)

| Action | Default | Notes |
|---|---|---|
| Dictate (hold) | **Right Ctrl** | Windows can't see the fn key; avoid Win+H (Windows' own voice typing) |
| Smart (hold) | **Right Alt** | Profile decides prompt vs email vs message vs polish |
| Hands-free latch | Double-tap either key | Tap again to finish; for long rambles |
| Cancel | Esc while active | Discards audio, nothing inserted |
| Re-paste last result | Ctrl+Alt+V | For when focus moved or paste failed |
| Edit selection (M4) | Smart key with text selected | Speech becomes an instruction applied to the selection |

Trigger keys are **swallowed** by the hook while held, so the target app never sees them (no stray Ctrl state, no Alt menu activation). This makes them dedicated to MyNotch. Mouse side buttons (XButton1/2) are an optional alternative trigger.

### 4.2 Notch states

| State | Shows |
|---|---|
| Idle | Collapsed notch |
| Listening | Live waveform, mode chip (DICTATE / SMART), target app name |
| Transcribing | Short spinner |
| Rewriting | Streaming preview of the rewrite (truncated) |
| Review (optional setting) | Full result; Enter inserts, Esc discards |
| Delivered | "✓ Inserted into {App}" for ~1.5 s, then collapse |
| Error | One-line reason + Copy button; result kept for re-paste |

**The notch must never take focus.** Create it with `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST` and answer `WM_MOUSEACTIVATE` with `MA_NOACTIVATE`. If showing the notch steals focus, paste lands nowhere — this is the #1 failure mode.

### 4.3 Modes

- **Dictate** — your words, punctuated by Whisper, cleaned locally (§7.5). Spoken commands: "new line", "new paragraph", "scratch that" (drops the previous sentence). No LLM call.
- **Smart** — the matched app profile picks a template:
  - `prompt.coding_agent` — Chorus panes, Claude Code / Codex / Gemini CLI in a terminal, Cursor, VS Code chat
  - `prompt.chat` — Claude.ai, ChatGPT, Gemini, Perplexity in the browser
  - `compose.email` — Gmail, Outlook
  - `compose.message` — Slack, Teams, Telegram, WhatsApp
  - `polish` — fallback: clean, clear prose in your voice
- **Edit (M4)** — with a selection, "make this more direct and cut it in half" rewrites the selection and replaces it.

---

## 5. Architecture

```
  Hotkey hook ──┐                                 ┌──> Notch UI   (renders states)
  Notch UI    ──┼──> VoiceCommand (mpsc) ──┐      ├──> Local API  (pushes to clients)
  Local API   ──┘                          v      ├──> History    (SQLite)
                         ┌──────────────────────────────────────┐
                         │ Pipeline (one session at a time)     │
                         │ Capture → VAD trim → STT → Cleanup   │
                         │        → [Rewrite] → Deliver         │
                         └───────────── VoiceEvent (broadcast) ─┘
```

### 5.1 Module layout

Start as one crate with clear modules; split into crates only if build times hurt.

```
mynotch/
  Cargo.toml                 # workspace
  crates/
    mynotch-app/             # existing notch shell/UI (or its rewrite)
    mynotch-voice/
      src/
        lib.rs               # VoiceService: start(), commands(), events()
        pipeline.rs          # session state machine
        audio/               # cpal capture, resample, ring buffer, level meter
        vad.rs               # Silero VAD gate + trimming
        stt/                 # SttEngine trait, whisper impl, model manager
        cleanup.rs           # fillers, stutters, spoken commands, vocabulary fixes
        rewrite/             # Rewriter trait, OpenAI-compatible client, templates
        profiles.rs          # app matching rules
        deliver/             # API delivery, paste injector, unicode typer
        api/                 # local WebSocket + HTTP server
        store.rs             # SQLite history, settings
        platform/win/        # ALL Win32: hook, foreground info, SendInput, clipboard, UIA
  clients/
    ts/                      # @mynotch/voice-client (for Chorus / Electron / Node)
  tools/
    voicectl/                # CLI: transcribe a wav, replay fixtures, bench latency
  docs/voice/
```

### 5.2 Suggested crates

`tokio`, `cpal` (WASAPI capture), `rubato` (resample to 16 kHz), `ringbuf`, `whisper-rs` (features `cuda` or `vulkan`), Silero VAD (whisper.cpp's built-in VAD or the `voice_activity_detector` crate), `windows` (Win32), `uiautomation` (M4), `axum` (WebSocket + HTTP), `reqwest` + SSE parsing (LLM streaming), `rusqlite`, `keyring` (API keys in Windows Credential Manager), `serde`/`toml`, `regex`, `tracing`, `hound` (WAV fixtures).

### 5.3 Threads

| Thread | Job | Rule |
|---|---|---|
| Hook thread | `WH_KEYBOARD_LL` + message loop | Callback returns in microseconds; only posts to a channel |
| Audio callback (cpal) | Push samples into a lock-free ring buffer | No allocation, no locks |
| Tokio runtime | Pipeline, API server, LLM calls | — |
| Blocking pool | Whisper inference (`spawn_blocking`) | Model loaded once, kept resident |
| UI thread | Whatever the MyNotch shell uses | Subscribes to `VoiceEvent` |

### 5.4 Core interfaces (sketch)

```rust
pub enum VoiceCommand {
    Start { mode: Mode, origin: Origin, target_hint: Option<TargetHint> },
    Stop,          // key released / client says done
    Cancel,
    RepasteLast,
}

pub enum VoiceEvent {
    SessionStarted { id: SessionId, mode: Mode, target: TargetInfo },
    Level { id: SessionId, rms: f32 },                 // ~30 Hz for the waveform
    Transcribing { id: SessionId },
    Transcript { id: SessionId, raw: String, cleaned: String },
    RewriteDelta { id: SessionId, delta: String },
    Delivered { id: SessionId, via: DeliveryVia, app: String },
    Cancelled { id: SessionId },
    Failed { id: SessionId, error: VoiceError, text: Option<String> },
}

pub trait SttEngine: Send + Sync {
    fn transcribe(&self, pcm_16k_mono: &[f32], opts: &SttOptions) -> Result<Transcript>;
}

#[async_trait]
pub trait Rewriter: Send + Sync {
    async fn rewrite(&self, req: RewriteRequest, on_delta: DeltaSink) -> Result<String>;
}

#[async_trait]
pub trait Deliverer: Send + Sync {
    async fn deliver(&self, text: &str, target: &TargetInfo) -> Result<DeliveryVia>;
}

pub trait ContextProvider: Send + Sync {
    fn foreground(&self) -> ForegroundInfo;            // hwnd, pid, exe, title, integrity level
    fn selection(&self) -> Option<String>;             // M4
}
```

### 5.5 Session state machine

```
Idle ─key down─> Arming ─mic ready─> Listening ─key up─> Finalizing (VAD trim)
  ─speech?─ no ──> Idle ("Didn't catch that")
          └ yes ─> Transcribing ─> Cleanup ─┬─ Dictate ─> Delivering ─> Done ─> Idle
                                            └─ Smart ───> Rewriting ─> Delivering
Any state ─Esc─> Cancelled ─> Idle
Any state ─error─> Failed (keep text for Copy / re-paste) ─> Idle
```

One active session at a time. A new key-down during Transcribing/Rewriting is ignored (show a subtle "busy" pulse).

---

## 6. Component details

### 6.1 Hotkey hook (`platform::win::hook`)

- `SetWindowsHookExW(WH_KEYBOARD_LL)` on a dedicated thread running `GetMessageW`.
- Match trigger keys by virtual key (`VK_RCONTROL` 0xA3, `VK_RMENU` 0xA5). Ignore events with `LLKHF_INJECTED` so MyNotch's own `SendInput` never retriggers it.
- Swallow trigger key down **and** up (return non-zero). Ignore auto-repeat key-downs while held.
- Hold < 150 ms counts as a tap (for double-tap latch), not a session.
- Windows silently unhooks callbacks that exceed `LowLevelHooksTimeout`. Keep the callback trivial and add a watchdog that re-installs the hook if no events arrive after a synthetic self-check.
- Optional `WH_MOUSE_LL` for side-button triggers.

### 6.2 Audio capture (`audio`)

- `cpal` default input device (WASAPI shared mode); device picker in settings; re-open on device change (headset unplug, Bluetooth reconnect, resume from sleep).
- Open the stream on key-down. Keep it **warm** for 60 s after the last session (configurable, 0 = close immediately) and maintain a 300 ms pre-roll buffer while warm so the first syllable isn't clipped. Note: Windows shows the mic-in-use indicator while warm — that's expected and should be explained in settings.
- Downmix to mono f32, resample to 16 kHz.
- Hard cap 5 minutes per session; emit RMS level events for the waveform.
- Write WAV only when "debug capture" is on.

### 6.3 VAD gate (`vad`)

- Run Silero VAD over the captured buffer; trim leading/trailing silence.
- If total speech < 300 ms, abort quietly. Whisper hallucinates phrases like "Thank you for watching" on near-silence; this gate prevents that from being typed into a terminal.

### 6.4 Speech-to-text (`stt`)

- `whisper-rs` built with `cuda` (NVIDIA) or `vulkan` (any GPU); CPU build as fallback.
- **Model manager:** download on first run to `%LOCALAPPDATA%\MyNotch\models\`, verify SHA-256, show size and progress in settings. Default `ggml-large-v3-turbo-q5_0.bin`; offer `ggml-large-v3-turbo.bin` (~1.6 GB, slightly better) and `ggml-small.en.bin` (CPU).
- Load the model at startup (or first use) and run a 1 s warm-up inference so the first real session isn't slow.
- Params: language `en` (configurable), no timestamps, temperature 0 with fallback, `initial_prompt` from the vocabulary list.
- **Vocabulary:** Whisper's prompt is capped at ~224 tokens, so keep the glossary short and ordered by priority (global list + per-profile list). Example: `Chorus, MyNotch, MediatR, EF Core, Vue, Playwright, OpenRouter, Ollama`.
- Future engines behind the same trait (not v1): NVIDIA Parakeet via sherpa-onnx, cloud Groq/Deepgram.

### 6.5 Cleanup (`cleanup`) — local and deterministic

- Remove hesitation fillers: um, uh, er, erm, hmm. Leave "like" / "you know" / "I mean" in dictation (too risky to remove by rule); the LLM handles them in Smart mode.
- Collapse immediate repeats ("the the", "I I").
- Apply spoken commands ("new line", "new paragraph", "scratch that").
- Apply replacement rules after STT (regex map in config), e.g. `see sharp → C#`, `dot net → .NET`, `view js → Vue.js`.
- Leading-space policy at insertion (add a space if the previous char isn't whitespace — requires caret context, so v1 just uses a per-profile setting; M4 uses UIA).

### 6.6 Rewriter (`rewrite`)

- OpenAI-compatible `/chat/completions` with streaming. Providers: OpenRouter, Ollama (`http://localhost:11434/v1`), custom base URL. Keys in Windows Credential Manager.
- Per-mode model setting. Pick fast models; output is short, so latency matters more than raw capability.
- **Timeout fallback:** if the rewrite doesn't finish in 6 s (configurable), insert the cleaned transcript and show "Rewrite timed out — inserted dictation".
- `local_only = true` forces Ollama and refuses to call any remote endpoint.
- Templates live in `%APPDATA%\MyNotch\voice\modes\*.md` with variables `{{transcript}}`, `{{app}}`, `{{style}}`, `{{selection}}`. Ship good defaults; hot-reload on change.

**Rules every template includes**
- Output only the final text. No preamble, no quotes, no "Here's your prompt".
- The speaker backtracks ("no wait", "actually", "scratch that"). Keep only the final intent.
- Preserve every concrete detail: names, numbers, file paths, symbols, error text, versions.
- Never add requirements, technology choices, or facts the speaker didn't say.
- Short input stays short. Only add structure when it helps.

**Default `prompt.coding_agent` template (sketch)**

```
You turn a developer's spoken, rambling request into a clear prompt for an AI coding agent running in {{app}}.
Apply the rules above. When the request has several parts, use these sections and skip any that are empty:
Goal — Context — Constraints — Done when.
Write in plain imperative sentences. Keep code identifiers and paths exactly as spoken.

Transcript:
{{transcript}}
```

**Style file** (`style.md`) — written by the user: how they open and sign off emails, length preferences, words they never use. Injected as `{{style}}` into compose modes.

### 6.7 Context and profiles (`profiles`)

- Capture at **key-down** (not release): foreground `HWND`, PID, exe name, window title, process integrity level.
- Profiles in config, first match wins, `default` last. Match on exe name and/or title regex. Browsers are matched by title because the tab title is in the window title.
- Electron apps in dev run as `electron.exe`; cooperating apps should register by PID through the API (§8) rather than rely on exe name.
- **Privacy:** only the profile's app label (e.g. "Gmail") goes to the LLM by default. Sending the full window title is a per-profile opt-in.

### 6.8 Delivery (`deliver`)

Routing per session:

1. **API target** — if the foreground PID belongs to a registered API client (or the session was started by a client with a target hint), send a `deliver` message and wait up to 500 ms for `deliver.ack`. No ack → fall through to 2.
2. **Clipboard paste** (default for everything else):
   1. Snapshot the clipboard (all formats where possible; at minimum `CF_UNICODETEXT`).
   2. Set the text plus the formats `ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory` = 0 and `CanUploadToCloudClipboard` = 0, so dictations don't pile up in Win+V history or sync to other devices.
   3. Make sure no modifiers are physically held, then `SendInput` the profile's paste chord (Ctrl+V; Ctrl+Shift+V for terminals that need it).
   4. Wait ~150 ms (configurable per profile) and restore the original clipboard.
3. **Unicode typing** (`SendInput` + `KEYEVENTF_UNICODE`) for apps that block paste. **Never** for multi-line text into a terminal — every newline becomes Enter and submits the prompt early.

Guardrails
- If the foreground `HWND` changed since key-down, don't inject. Show the result in the notch with Copy, keep it for re-paste.
- If the target runs elevated and MyNotch doesn't, Windows (UIPI) silently drops injected input. Detect the integrity level and show "Target is running as admin — use Copy".

### 6.9 Storage (`store`)

- SQLite at `%LOCALAPPDATA%\MyNotch\voice.db`: `sessions(id, started_at, mode, profile_id, app_label, raw, cleaned, final, delivered_via, capture_ms, stt_ms, llm_ms, deliver_ms, error)`.
- Retention default 30 days; "never store text" option. Audio is not stored unless debug capture is on.
- Settings in `%APPDATA%\MyNotch\voice.toml` (example in Appendix A).
- History view in the notch's expanded panel: last 20 results, click to copy.

---

## 7. Latency budget (GPU, 15 s utterance)

| Stage | Dictate target | Smart target |
|---|---|---|
| Key up → VAD trim done | 30 ms | 30 ms |
| Whisper | 400–600 ms | 400–600 ms |
| Cleanup | < 5 ms | < 5 ms |
| LLM rewrite (first token → done) | — | 800–1500 ms |
| Paste + restore | 150 ms | 150 ms |
| **Total** | **≤ 1.0 s** | **≤ 2.5 s p50** |

Record every stage in `sessions`. `voicectl stats` prints p50/p95 per stage.

---

## 8. Local API (the abstraction other apps use)

### 8.1 Transport and security

- Bind `127.0.0.1` only. Port chosen at startup; written with a random token to `%APPDATA%\MyNotch\voice-api.json` (`{ "v": 1, "port": 47820, "token": "…" }`), file ACL'd to the current user.
- Every connection must present the token (`Authorization: Bearer` header or in `hello`).
- **Reject any request carrying an `Origin` header** that isn't explicitly allow-listed. Browser pages can open WebSockets to localhost; this stops a random website from listening to your voice sessions.
- JSON messages carry `"v": 1`. Changes within v1 are additive only.

### 8.2 WebSocket `/v1/ws`

```jsonc
// client → MyNotch
{"type":"hello","v":1,"client":"chorus","version":"0.9.0","token":"…"}
{"type":"register_target","target_id":"chorus","pid":12345,"smart_mode":"prompt.coding_agent"}
{"type":"subscribe","events":["session.state","level","transcript"]}   // watch without being a target
{"type":"session.start","mode":"smart","target_id":"chorus","hint":{"pane":"agent-2"}}
{"type":"session.stop"}
{"type":"session.cancel"}
{"type":"deliver.ack","session_id":"…","ok":true}

// MyNotch → client
{"type":"session.state","session_id":"…","state":"listening","mode":"smart","app":"Chorus"}
{"type":"level","session_id":"…","rms":0.42}
{"type":"transcript","session_id":"…","raw":"…","cleaned":"…"}
{"type":"rewrite.delta","session_id":"…","delta":"…"}
{"type":"deliver","session_id":"…","target_id":"chorus","hint":{"pane":"agent-2"},"text":"…","submit":false}
{"type":"session.state","session_id":"…","state":"done"}
```

### 8.3 HTTP (for scripts and simple clients)

| Endpoint | Body | Returns |
|---|---|---|
| `GET /v1/health` | — | `{ ok, model, backend }` |
| `POST /v1/transcribe?mode=dictate\|smart&profile=…` | audio (`audio/wav` in v1; OGG/Opus later) | `{ text, raw, ms }` |
| `POST /v1/rewrite` | `{ text, mode, profile? }` | `{ text }` |

### 8.4 TypeScript client (`clients/ts`)

`@mynotch/voice-client`: reads the discovery file, connects, reconnects with backoff, exposes typed events (`onState`, `onLevel`, `onDeliver`) and calls (`registerTarget`, `start`, `stop`, `cancel`, `transcribe`). Used by Chorus; usable by any Electron/Node app.

---

## 9. Chorus integration

- **Drop native voice from the Chorus roadmap.** Add `@mynotch/voice-client` instead.
- On startup, Chorus connects and calls `register_target` with its main-process PID. When Chorus is focused and you hold a voice key, MyNotch delivers over the API instead of pasting.
- On `deliver`: write the text into the active pane's pty **without** a trailing newline. If `submit: true`, then write `\r`.
- **Spoken submit:** ending an utterance with "send it" (configurable phrase) strips the phrase and sets `submit: true`. API targets only — never for generic paste.
- **Pane mic button:** click to call `session.start` with `hint.pane`, so the text lands in that pane even if you click elsewhere while talking. Show a small listening indicator in the pane from `session.state` / `level` events.
- **Telegram voice notes** (for the planned Telegram bridge): forward the audio to `POST /v1/transcribe?mode=smart&profile=chorus` and treat the result as the prompt. Needs OGG/Opus decode in MyNotch (add in M3 or right after).
- **Graceful absence:** if MyNotch isn't running, hide the mic button. Generic paste still works into the xterm.js panes if MyNotch starts later.

Other apps get voice for free via paste, or richer behavior by using the TS client or HTTP endpoints.

---

## 10. Milestones

### M0 — Walking skeleton (prove the hard parts)
Hold Right Ctrl → capture → Whisper → paste into Notepad. Minimal notch state (listening / done). No VAD, no profiles, no LLM.
**Done when:** a 10 s utterance is inserted ≤ 1.0 s after release on GPU; the notch never takes focus; the hook survives 1 hour of normal use including sleep/resume; clipboard is restored.

### M1 — Dictation you'd actually use
VAD gate, cleanup, vocabulary + replacements, spoken commands, all notch states, history + re-paste, settings file, device switching, model manager, keep-warm + pre-roll, clipboard-history exclusion, foreground-changed and elevated-target guardrails.
**Done when:** works in Notepad, Chrome/Edge textarea, VS Code, Cursor, Windows Terminal (incl. Claude Code), Slack desktop, and Chorus; no Win+V history entries; silence produces nothing.

### M2 — Smart mode
Profiles, rewriter (OpenRouter + Ollama), templates + style file with hot reload, streaming preview, timeout fallback, optional review step, `local_only` toggle.
**Done when:** the golden test set (§11) passes; rambles with corrections resolve to the final intent; p50 ≤ 2.5 s end to end.

### M3 — Local API + Chorus
WebSocket + HTTP server, discovery file, token + Origin checks, TS client, Chorus registration, pane routing, spoken submit, `/v1/transcribe`.
**Done when:** holding a key with Chorus focused delivers via API (not paste) to the active pane; the pane mic button works; a web page in Chrome cannot connect; a Python script can transcribe a WAV via HTTP.

### M4 — Edit mode and richer context
Selection capture (UI Automation `TextPattern`; fallback Ctrl+C with clipboard restore), edit mode, caret-context spacing/capitalization, per-profile vocabulary.
**Done when:** select a paragraph in Gmail/Slack/VS Code, hold Smart, say an instruction, selection is replaced correctly.

### M5 — Hardening and shipping
Start with Windows, single instance, hook watchdog, crash-safe clipboard restore, notch on the active monitor + per-monitor DPI, installer, code signing, settings UI polish.

### Later (not committed)
Live partial transcription in the notch · opt-in local screenshot context · voice actions (open app/URL) · Parakeet engine · cloud STT provider · laptop-side MyNotch for Chorus remote sessions.

---

## 11. Testing

- **Audio fixtures:** record ~30 real utterances (plain dictation, rambles with backtracking, heavy dev jargon, quiet/noisy room, silence). Store WAV + expected text. `voicectl bench` reports word error rate and per-stage latency; run on every STT change.
- **Golden rewrite tests:** transcript → output, checked by assertions rather than exact match: no preamble; every file path/symbol from the transcript is present; corrected-away content is absent; length within bounds.
- **Injection matrix:** manual checklist per target app (paste chord, restore delay, multi-line behavior) kept in `docs/voice/injection-matrix.md`.
- **Unit tests:** cleanup rules, profile matching, state machine transitions, API message parsing.

---

## 12. Risks

| Risk | Mitigation |
|---|---|
| Notch steals focus → paste goes nowhere | `WS_EX_NOACTIVATE`, `MA_NOACTIVATE`, verify foreground before injecting |
| Windows silently removes a slow hook | Trivial callback, watchdog re-install, self-check |
| Whisper hallucinates on silence | VAD gate + minimum speech duration |
| Multi-line text submits early in terminals | Paste, never Unicode-type multi-line into terminals; API delivery for Chorus |
| Clipboard manager races on restore | Configurable restore delay; exclusion formats; re-paste last |
| GPU busy (Unreal Editor, games) spikes latency | Show latency in history; option to switch to CPU `small.en` |
| Antivirus flags keyboard hook + SendInput | Code-sign the binary; no network on the hook thread; document behavior |
| Elevated targets ignore injected input | Detect integrity level; show Copy |
| LLM adds things you didn't say | Strict template rules + golden tests; review step available |

---

## 13. Open questions (resolve before M0)

1. What UI framework does MyNotch use today, and is the notch window already non-activating? Rewrite needed?
2. Which GPU and how much VRAM? Decides `cuda` vs `vulkan` build and default model.
3. Default LLM for Smart mode: OpenRouter (fast model) or local Ollama? Running Whisper and a local LLM on the same GPU adds contention.
4. Are Right Ctrl / Right Alt acceptable as dedicated trigger keys?
5. Should Smart mode insert immediately (Lemon's behavior) or show the review step by default?

---

## Appendix A — `voice.toml` example

```toml
[hotkeys]
dictate = "RightCtrl"
smart = "RightAlt"
repaste = "Ctrl+Alt+V"
double_tap_latch_ms = 300

[audio]
device = "default"
keep_warm_secs = 60
preroll_ms = 300
max_secs = 300

[stt]
engine = "whisper"
model = "ggml-large-v3-turbo-q5_0.bin"
backend = "cuda"          # cuda | vulkan | cpu
language = "en"
vocabulary = ["Chorus", "MyNotch", "MediatR", "EF Core", "Vue", "Playwright", "OpenRouter", "Ollama"]

[[stt.replacements]]
from = "(?i)\\bsee sharp\\b"
to = "C#"

[llm]
provider = "openrouter"   # openrouter | ollama | custom
model = ""                # pick a fast model in settings
timeout_ms = 6000
local_only = false

[delivery]
paste_restore_delay_ms = 150
review_before_insert = false

[[profiles]]
id = "chorus"
match = { process = ["Chorus.exe"] }   # Chorus also registers by PID via the API
smart_mode = "prompt.coding_agent"
deliver = "api"

[[profiles]]
id = "terminal-agent"
match = { process = ["WindowsTerminal.exe"], title_regex = "(?i)claude|codex|gemini|aider|opencode" }
smart_mode = "prompt.coding_agent"
paste_chord = "Ctrl+V"

[[profiles]]
id = "editor"
match = { process = ["Code.exe", "Cursor.exe"] }
smart_mode = "prompt.coding_agent"

[[profiles]]
id = "ai-chat-web"
match = { process = ["chrome.exe", "msedge.exe"], title_regex = "Claude|ChatGPT|Gemini|Perplexity" }
smart_mode = "prompt.chat"

[[profiles]]
id = "email"
match = { process = ["chrome.exe", "msedge.exe", "olk.exe", "OUTLOOK.EXE"], title_regex = "Gmail|Outlook|Inbox" }
smart_mode = "compose.email"

[[profiles]]
id = "chat-apps"
match = { process = ["slack.exe", "ms-teams.exe", "Telegram.exe", "WhatsApp.exe"] }
smart_mode = "compose.message"

[[profiles]]
id = "default"
smart_mode = "polish"
```
