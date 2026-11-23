# Local and cloud subtitle transcription

This crate implements the editor's explicit Generate action. It does not start from a recording, project load, autosave, or application launch. Local generation is the default and does not connect to a server. The caller selects and validates one existing recording audio track. Cloud generation requires an explicit Cloud selection and Generate action, the user's session-only Deepgram API key, and disclosure that retained audio will be uploaded using their account credit. Imported background music is not mixed into the transcription source.

## Local setup and generation

```rust,ignore
local::status(directory: &Path) -> Result<LocalStatus, String>;
local::install(directory: PathBuf, cancel: Arc<AtomicBool>, progress: impl Fn(f64) + Send + 'static)
    -> impl Future<Output = Result<LocalStatus, String>> + Send;
local::transcribe(input: TranscriptionInput, directory: PathBuf, cancel: Arc<AtomicBool>, progress: impl Fn(f64) + Send + 'static)
    -> impl Future<Output = Result<Vec<SubtitleCue>, String>> + Send;
```

The caller supplies a fixed application-owned directory, never a frontend executable path. `LocalStatus` serializes camelCase fields `ready`, `runtimeReady`, `modelReady`, `runtimeVersion`, `modelName`, `downloadBytes`, `installedBytes`, and `error`. Status is read-only and verifies hashes; run it off the UI thread, not on a frequent timer. `downloadBytes` is the fresh network transfer size, 156,880,105 bytes. `installedBytes` counts verified runtime/model files, 159,555,433 bytes when complete, excluding the small license and activation metadata. A missing setup returns ready=false; damaged metadata/files return a controlled error in that status. No executable is searched on PATH.

Only an explicit Install action downloads these fixed assets:

| Asset | Immutable source | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| Windows x64 CPU runtime | whisper.cpp v1.9.5, commit `d1be6fde11ac6e0407606b4e42fe72d34add8037`, release asset `b5454/whisper-bin-x64.zip` | 8,928,640 | `6ba69e3482d7826214f90a6a9c84ca07782aec1e1d0c6a7c30c994fd5d816ccb` |
| Multilingual base model | Hugging Face `ggerganov/whisper.cpp`, revision `5359861c739e955e79d9a303bcbc70fb988958b1`, `ggml-base.bin` | 147,951,465 | `60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe` |

HTTPS downloads allow only the official source hosts and their explicitly listed GitHub/Hugging Face CDN redirects. Requests contain no audio, transcript or credential. Downloads stream to a private staging directory, enforce exact byte counts and hashes, have 20-second connection/30-second read/20-minute request deadlines, and report progress at most about seven times per second. Cancellation drops the request and removes staging files. ZIP extraction reads only thirteen named EXE/DLL entries, with fixed per-file size/hash checks; archive paths are never used as output paths. A hidden `--version` probe must start within ten seconds before first activation. Setup publishes a UUID directory and atomically replaces `active.json`; partial downloads never become active. Failed setup preserves the previously active installation. Successful repeat setup reuses verified files without downloading. Abnormal application termination can leave staging/orphan setup directories; they are never executed automatically.

Both whisper.cpp and the Whisper model are MIT licensed; installation includes the complete permission notice and both copyright notices. CPU dispatch DLLs come from the verified upstream archive, supporting the upstream x64 CPU variants without a GPU driver or Python installation. Windows may still require the Microsoft Visual C++ x64 runtime; this application does not install global dependencies automatically.

Before every local generation, the runtime/model are rehashed, unexpected backend files and reparse-point files/directories are rejected, and verified files stay open without Windows write/delete sharing through inference. The hidden subprocess has a kill-on-close Job Object, null stdin/stdout, a bounded stderr progress parser, a clean environment and a fixed working directory. It cannot inherit `GGML_BACKEND_PATH` or a user PATH executable. Local mode zeroizes and ignores any API key passed accidentally. No local transcription code constructs an HTTP request.

Local inference uses the same retained-range, native-sample-exact mono16k extraction as cloud mode. At most one prepared WAV and one bounded JSON output exist at a time. The pinned CLI's `--max-len 1 --split-on-word --output-json` supplies word segment offsets; inference is CPU-only with at most eight threads. Each chunk has a deadline of `12 * audioSeconds + 120` seconds, bounded to 120..7500 seconds. Cancellation and output-size overflow terminate and reap the owned process. A cancelled/error job never returns a partial replacement for existing subtitles.

Whisper timestamps are estimates, not forced alignment. A real cut-speech fixture exposed zero-duration final words in the pinned CLI. Such a word is combined only with a directly adjacent timed token, retaining all text and that union's original interval. This rare combined token highlights as a phrase. Missing/nonadjacent zero timing, distinct overlapping words, invalid JSON, and text without resolvable timestamps fail clearly instead of silently dropping text or inventing times. Indonesian and automatic language detection use the multilingual model; recognition quality for those languages remains a manual validation item.

## Cloud API

```rust,ignore
pub async fn transcribe(
    input: TranscriptionInput,
    cancel: Arc<AtomicBool>,
    progress: impl Fn(f64) + Send + 'static,
) -> Result<Vec<screenrecord_core::SubtitleCue>, String>;
```

`TranscriptionInput` contains absolute `audio_path` and `ffmpeg_path`, source-recording `start`/`end` seconds, `ranges: Vec<(f64,f64)>`, `language` (`auto`, `id`, or `en`), and `api_key`. Empty ranges retain the legacy complete trim. Otherwise, up to 512 finite source-ordered nonoverlapping intervals must lie inside the trim. The caller passes effective retained video intervals. The future is `Send` and can run on Tauri's async runtime. Progress is monotonic from zero to one. Returned UUID cue IDs and times use the original recording timeline, including a nonzero trim start. The caller owns applying a successful result and preserving previous subtitles on cancellation or error.

## Provider and network behavior

The production endpoint is fixed to `https://api.deepgram.com/v1/listen`. Requests use `model=nova-3`, `language=multi`, punctuation/smart formatting, and mandatory `mip_opt_out=true`. Indonesian or English sets a language hint; auto omits it. The current Deepgram documentation lists both languages and word-level timestamps. Deepgram states that opted-out audio/transcripts are retained only while processing; request metadata and usage records remain. This is a provider policy, not an independently verified retention guarantee.

The HTTP client uses Windows native TLS, rejects redirects, and disables automatic retries and proxies. The API key is sent only as the Authorization header. Input and temporary header-construction strings are zeroized; the header is marked sensitive. Neither input nor provider types implement Debug or Clone. HTTP libraries necessarily hold internal credential copies, so this does not guarantee erasure of every transport-owned allocation. The key is never put into subprocess arguments, environment variables, project files, or logs.

Errors contain a controlled message and optional HTTP status, never the raw provider body, transcript, or API key. Authentication, credit, quota, upload size, service failure, and malformed timestamp errors are distinct. The crate makes no automatic retry after a possibly billable request.

OpenAI's timestamp-capable Whisper API was considered but its official documentation currently announces shutdown on February 26, 2027. The recommended replacement's documented file response does not establish a drop-in subtitle timestamp contract, so a new integration does not depend on that deprecated path.

## Audio preparation and limits

Hidden FFmpeg subprocesses extract mono 16 kHz signed 16-bit PCM WAV. The adjacent bundled `ffprobe.exe` reads the source sample rate once per job, with a 30-second deadline and bounded metadata read. Only local file/pipe protocols are permitted; video, subtitles, and other streams are excluded. Each child has kill-on-drop behavior and a Windows kill-on-close Job Object. Extraction has a 180-second deadline per chunk. No screen, camera, or microphone is opened.

Each chunk owns up to 596 seconds plus up to two seconds of context on each side, keeping the uploaded audio at or below 600 seconds. Chunks are extracted and uploaded sequentially. At most one prepared WAV exists at a time. The prepared PCM header/data length is checked, and uploads larger than 20 MiB are rejected before any connection. Response bodies are limited to 8 MiB, with at most 20,000 words per chunk and 200,000 words per job. The selected source range must fit within 24 hours.

Context is clamped inside each retained interval. Exactly adjacent intervals are combined for transcription so splitting a video without removing footage does not fragment recognition or multiply requests. No positive gap is bridged. FFmpeg preserves source timestamps and clips native integer sample indices before resampling. Each boundary is the first native sample timestamp greater than or equal to the requested time; the end is exclusive. This avoids both nearest-sample timestamp rounding and floating-point multiplication accidentally admitting an extra sample. Provider word times are offset from the first retained native sample. Cues are built independently per retained interval and cannot span a removed gap. Trim-edge fragments shorter than 0.01 seconds are omitted from transcription because they cannot hold the minimum cue duration; other retained audio still proceeds. Cancellation and result limits apply across the complete multi-interval job.

Returned words are assigned to a chunk by their midpoint, mapped to source time, and grouped at sentence ends, pauses, approximately 84 characters, or six seconds. Overlapping repeated context words are deduplicated; distinct overlaps fail without changing existing subtitles. Generated cues retain `words` and use their exact single-space text join; line wrapping belongs to the renderer. `sourceRanges` is empty for generated cues because each already lies in one retained interval. Cues are finite, sorted, nonoverlapping and at least 0.01 seconds; a rare single prolonged word may exceed six seconds to preserve its actual timing. Results are capped at 10,000 cues, 200,000 words and 1 MiB of text, matching core validation.

Context overlap reduces boundary loss but cannot guarantee identical recognition/timing between independent requests. Timestamp disagreement near a chunk boundary may still need manual text/timing correction. Word timestamps and subtitle accuracy are model outputs, not sample-accurate ground truth. An empty speech result produces an empty cue list; text without usable timestamps fails instead of inventing timing.

Cancellation is checked at 25 ms intervals while extracting, uploading, or waiting for a response. It terminates and reaps the local extractor and drops the active HTTP request. It cannot promise to cancel work or charges already accepted by Deepgram. The caller should set the cancellation flag and await completion rather than aborting its task.

Temporary audio is removed on ordinary completion, errors, and cancellation through scoped temporary directories; successful jobs also verify directory cleanup. An abrupt process/OS termination can leave an OS temporary directory. The crate does not save partial cloud transcripts for resume. A failed multi-chunk job leaves the caller's existing subtitles unchanged; manually retrying may upload earlier chunks again.

## Verification

Run these commands in PowerShell from the workspace root after preparing the bundled FFmpeg runtime.

```powershell
cargo test -p screenrecord-transcribe --locked
$env:SCREENRECORD_FFMPEG = (Resolve-Path .\src-tauri\resources\ffmpeg.exe).Path
cargo test -p screenrecord-transcribe media_contract --locked -- --ignored --nocapture
cargo clippy -p screenrecord-transcribe --all-targets --locked -- -D warnings
```

Tests use synthetic PCM, an owned bounded child-process fixture, and HTTP servers bound to localhost. They cover request authentication/opt-out, preserved source offsets, PCM extraction, word grouping, chunk ownership, Unicode/core validation, pre-upload rejection, redirect blocking, sanitized provider errors, response size limits, cancellation before upload, cancellation while upload is stalled, and cancellation while awaiting the provider. No live API call, paid account, captured user media, or computer-use test is involved. Actual Deepgram account access, Indonesian/English recognition quality, cloud latency, and billing require the user's manual test with their own key.

Verified on November 11, 2026: 18 unit/mock tests and three explicitly selected FFmpeg/localhost tests passed. Uploads over 20 MiB were rejected without a connection; responses over 8 MiB were rejected both with and without Content-Length. The extraction integration maps a 1.0-second source trim plus a 0.2-second provider word start to a 1.2-second cue. A separate owned subprocess fixture is intentionally ignored by the ordinary runner and launched by its cancellation test.

Stage 1 cut privacy adds interval validation/context tests and `media_contract_cut_audio_never_reaches_upload_or_resampler_context`. Its 48 kHz PCM source has distinct retained values and a different value throughout removed spans. A localhost server captures both 16 kHz WAV uploads and compares every sample byte: exactly 4,000 and 4,800 retained samples, no removed-span or resampler-boundary values. Returned cues stay at original source times 0.21 and 0.81 seconds. This is synthetic local-network evidence, not a real cloud-provider test.

`media_contract_fractional_cuts_exclude_native_samples_before_resampling` covers 44.1 and 48 kHz sources with fractional retained bounds 1.201234 to 1.452345 seconds. Every uploaded PCM sample must equal the retained value; samples outside the interval have a distinct loud sentinel. It also checks that word times start at the aligned native sample. A separate regression distinguishes the exact 1.1-second boundary from the immediately neighboring floating-point values without bridging a positive gap.

Primary sources checked on November 11, 2026:

- [Deepgram prerecorded audio and word timestamps](https://developers.deepgram.com/docs/pre-recorded-audio)
- [Nova-3 multilingual support and language hints](https://developers.deepgram.com/docs/nova-3-language-prompting)
- [Deepgram request schema](https://developers.deepgram.com/reference/speech-to-text/listen-pre-recorded)
- [Deepgram data handling and model-improvement opt-out](https://developers.deepgram.com/trust-security/your-data)
- [OpenAI transcription migration and deprecated timestamp models](https://developers.openai.com/api/docs/guides/transcription)
- [Tokio subprocess lifecycle](https://docs.rs/tokio/latest/tokio/process/struct.Child.html)
- [FFmpeg native sample trimming](https://github.com/FFmpeg/FFmpeg/blob/master/libavfilter/trim.c)

## Stage 5 local verification

On November 12, 2026 the public installer downloaded and verified both pinned assets in `artifacts/local-asr/installed`. Windows SAPI Microsoft David Desktop generated `testdata/local-speech.wav` directly to a file, without playback or opening a microphone. Its authored text is: "This is a local subtitle test. The video stays on this computer. We can edit every word." The 8.318322-second fixture SHA-256 is `9df03002d3e088213b03a6201df20f131baf20c49016767ff51ab23ae8641089`.

The real offline model test returned all seventeen words in three cues in 4.301 seconds in a debug build on this machine. Retaining only `[0,3.2)` and `[6.2,8.318322)` returned nine timed tokens, omitted the removed middle sentence, kept original source times, and passed core metadata validation in 5.368 seconds. Input bytes were unchanged. This is a small synthetic English acceptance test, not a general accuracy or hardware performance claim.

```powershell
# Explicit developer setup downloads the pinned runtime/model. Never part of ordinary tests.
$env:SCREENRECORD_LOCAL_ASR_DIRECTORY = Join-Path $PWD 'artifacts\local-asr\installed'
cargo run -p screenrecord-transcribe --locked --example local_smoke -- install $env:SCREENRECORD_LOCAL_ASR_DIRECTORY
$env:SCREENRECORD_LOCAL_ASR_FIXTURE = (Resolve-Path .\crates\transcribe\testdata\local-speech.wav).Path
$env:SCREENRECORD_FFMPEG = (Resolve-Path .\src-tauri\resources\ffmpeg.exe).Path
cargo test -p screenrecord-transcribe --locked local_model_contract -- --ignored --nocapture
```

Ordinary local tests cover malformed/zero-duration word offsets, exact word text and source timing, controlled errors, missing/path-traversal setup metadata, byte/hash mismatch and oversize downloads, cancellation with previous setup preserved, executable-file leases rejecting modification, unexpected backend DLL rejection, Send futures, and owned-child cancellation/output-overflow cleanup. Mock downloads use localhost GET requests and assert that neither authorization nor an audio request body is sent. The actual model test never downloads assets or calls a cloud API.

Primary local dependency sources checked November 12, 2026:

- [whisper.cpp v1.9.5 release and immutable source commit](https://github.com/ggml-org/whisper.cpp/releases/tag/v1.9.5)
- [Official b5454 asset metadata and SHA-256](https://api.github.com/repos/ggml-org/whisper.cpp/releases/tags/b5454)
- [Pinned CLI word segmentation, offsets and progress implementation](https://github.com/ggml-org/whisper.cpp/blob/d1be6fde11ac6e0407606b4e42fe72d34add8037/examples/cli/cli.cpp)
- [Pinned model metadata, LFS SHA-256 and size](https://huggingface.co/api/models/ggerganov/whisper.cpp/tree/5359861c739e955e79d9a303bcbc70fb988958b1)
- [whisper.cpp MIT license](https://github.com/ggml-org/whisper.cpp/blob/v1.9.5/LICENSE)
- [Whisper model MIT license](https://github.com/openai/whisper/blob/main/LICENSE)
