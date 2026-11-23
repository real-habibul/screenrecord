# Kontrak proyek dan IPC

Kontrak ini mencatat keputusan kompatibilitas format proyek, perekaman, preview, dan ekspor per versi. Saat menambah field yang disimpan atau dikirim lintas proses, periksa titik masuk berikut.

| Bagian | Sumber |
| --- | --- |
| Model Rust dan default | [crates/core/src/lib.rs](../crates/core/src/lib.rs) |
| Format berkas, migrasi, dan validasi jalur | [crates/core/src/storage.rs](../crates/core/src/storage.rs) |
| DTO antarmuka Vue | [src/types.ts](../src/types.ts) |
| Perintah dan event Tauri | [src-tauri/src/app.rs](../src-tauri/src/app.rs) |

Tambahkan pemeriksaan untuk proyek lama jika perubahan memengaruhi format penyimpanan. Perbarui representasi Rust dan TypeScript dalam perubahan yang sama ketika keduanya memakai field baru.

## Catatan perubahan antarrilis

### Editor audio and vertical guides 0.7.0

Project schema 7 reads versions 1 through 6. Existing music ducking uses `audioProcessing.ducking.enabled`. The new optional `systemEnabled` flag defaults to false. A single post-volume microphone detector drives the selected computer and music buses with the shared `amountDb` reduction. The microphone feeds the mix at its own level. Both preview and export apply the final limiter after mixing.

The desktop resolves an audio source from saved project metadata. The optional IPC flag `analyze_audio.forceRefresh` requests a new scan. Open, normalization and saved timing changes request reuse. Derived data lives under `.audio-analysis` inside the project. Deleting the cache is safe when the editor is closed. Volume, gain and ducking leave the raw analysis valid.

Audition uses temporary editor state. Monitor values are `mix`, `microphone`, and `system`. The loop uses a half-open interval in edited output seconds. Playback maps it through the cut list. Solo is applied after detection and per-track processing, before the master limiter. Neither the audition range nor the solo choice is serialized.

`audioProcessing.volumeKeyframes` optionally contains `microphone`, `system`, and `music` arrays. Each point has an `id`, a `time` in original recording seconds, and a `volume` multiplier from 0 to 1. Curves are linear between points and hold the nearest endpoint outside the curve. An empty curve uses 1. The multiplier follows source gain, faders and clip fades, before speech detection and the master limiter. Music points control the combined music track. Each track accepts up to 2048 points, ordered at least 1 ms apart.

The 9:16 guide uses the final composition dimensions, including Match recording with a portrait source or crop. Its bounds follow the canvas at Fit, 100% and 200%. A pointer-transparent DOM overlay displays approximate TikTok or Instagram Reels margins. The editor stores the choice under `screenrecord-safe-area` in local preferences. Original-frame correction and the export pipeline are independent of the guide.

### Recorded audio gain 0.6.1

The Audio inspector edits microphone and computer source gain independently from 0.01x to 2.00x. Reset restores 1.00x. Each volume fader still controls balance and mute. Preview and MP4 both apply volume times source gain before mixing and limiting. Schema 6 continues to accept saved normalization gains up to 8x from earlier releases. Opening, saving unrelated edits, playing and exporting those projects preserve their gain.

New normalization recommendations target -18 dBFS RMS, respect -1 dBFS sample peak headroom and cap gain at 2.00x. Normalize reuses a finished or running analysis for the same open project, source and retained intervals. Analyze explicitly measures the source again.

### Recording setup 0.6.0

`RecordingOptions.systemAudioScope` defaults to `system`; `application` requires enabled system audio and a live canonical Apps window. A failed process-loopback request must fail visibly without capturing the whole desktop mix. `region` defaults to null. Its x, y, width and height are integer physical pixels relative to the selected source. Width and height are even and at least 2. Reject overflow, stale bounds and unavailable targets. Both preview-test and ordinary capture use the same options and crop before encoding. Cursor positions and recorded dimensions refer to the cropped media.

`get_source_thumbnail({sourceId})` is explicit, short lived and returns `{width,height,data}`, with physical source dimensions and a proportional base64 JPEG. No device capture runs just by opening setup.

`run_recording_test({id,options})` takes a fresh UUID, stops camera/mic previews and uses one normal recording part with a five-second source-clock limit. It returns `{id,duration,width,height,screenPath,cameraPath,microphonePath,systemAudioPath,cameraOffsetSeconds}` with nullable optional track paths. Camera playback uses screen time plus the existing offset. Only the selected audio channel plays. Optional `recording-test-phase` events carry `{id,phase}`, recording or finalizing. The test is separate from the project library, recovery and regular recording events. `cancel_recording_test({id})` and `discard_recording_test({id})` are idempotent and scoped to the matching test. Temporary media is retired on cancellation, discard, replacement, normal closure and after an interrupted process on the next launch.

### Recorded click indicators 0.5.2

Project schema 6 reads versions 1 through 5. Cursor sample times and coordinates remain on the original source. Optional `buttons` is a mask with left=1, right=2 and middle=4. A nonzero mask requires `pressed=true`. A legacy aggregate press with a missing or zero mask has an unknown button. New immutable 25-byte sidecars use the header `SRCURS02` with bits 0 and 1 for pressed and visible and bits 2 through 4 for the mask. A schema 6 manifest can continue referencing an existing checksummed `SRCURS01` sidecar after an edit. Recovery accepts old and new NDJSON.

The click track compares each button independently on raw cursor samples. A new typed recording starts with a hidden held-state baseline. A first typed held sample by itself is a baseline. Legacy visible press transitions, including an initial press, and middle clicks use a neutral ring. A visibility break ends any pulse at its timestamp. Holding a button never repeats a down event. A pulse lasts up to 0.4 source seconds, follows its original click position through the current crop and zoom, and only appears when its onset and the playhead are in the same continuous retained interval. The latest pulse of each kind can coexist. See `testdata/cursor-clicks.json` for the wire examples used by both composition paths.

`showClicks` defaults to true for edit settings and appearance presets. Preview and MP4 draw on the visible screen foreground when cursor and click animation are enabled. At 1080 reference size the radius grows linearly from 12 to 28 px scaled by cursor size. A left click uses one amber ring, a right click uses two cyan rings, and an unknown or middle click uses one neutral ring. The inner right ring has 0.64 of the outer radius. The 2.5 px strokes have a 4.5 px dark backing and fade from 95% opacity.

Each explicitly started or resumed part queues button transitions before WGC publishes the epoch. The first WGC frame publishes its canvas dimensions before the timestamp. Events preceding that timestamp contribute to the starting held state. Later events keep their QPC time and original normalized position. A hidden baseline on resume prevents a held button from creating a new click.

### Editor performance and WhatsApp export 0.5.1

Preview keeps playback, cut and audio gates on their playback clock. Presentation callbacks and visual edits request composition, with a refresh-loop fallback for engines without frame callbacks. A pending seek, released source or removed interval must never repaint removed pixels or start its audio. Cursor, camera, subtitles, masks and framing follow their source-time boundaries during ordinary playback and after seek.

An edit gesture has one history transaction. Intermediate updates remain visible without serializing a full Undo snapshot for each event. Commit on a completed gesture or before an operation that needs current edits. Cancellation, Undo/Redo, project replacement, a pending save and autosave must retain their existing transaction and project identity guarantees.

`ExportOptions.preset` is optional on the wire and defaults to `standard`. Existing standard resolutions and frame rates still apply. `whatsapp` uses `resolution: 'auto'` and `fps: 30`. `get_whatsapp_export_plan({id})` reads the saved project and returns `{width,height,videoBitrate,audioBitrate}` or a validation error. The same planner is used for start/export. `ExportProgress.outputBytes` is optional and supplies the actual size on completion.

WhatsApp export uses the retained duration rounded to the output frame grid. Its app limit is 16,000,000 bytes. The first video rate reserves 10% and 64 KiB for variance and mux overhead, budgets 96 kbps AAC, rounds down to 1 kbps and caps at 2.4 Mbps. Rates below 320 kbps require a shorter edit or Standard. The output fits within even 720/1280, 480/854 or 360/640 short/long bounds with thresholds at 1,000 and 550 kbps. It preserves the canvas ratio subject to pixel rounding and requires both dimensions to be at least 64. H.264 and optional AAC are muxed into fast-start MP4. Retained cuts, masks, composition and original media rules apply. Check complete output and cap before atomic no-replace publication. A size miss permits one lower-rate video retry at the same dimensions while reusing mixed audio. The file is saved locally for manual sharing. Project schema stays at 5.

### Vertical editor 0.5.0

Project version becomes 5; versions 1..4 load with legacy defaults without rewriting originals until Save. `EditSettings` adds serde-default `fitMode: 'fit'|'fill' = 'fit'`, `reframeKeyframes: [{id,time,x,y,scale}] = []`, `backgroundMode: 'solid'|'videoBlur' = 'solid'`, `backgroundBlur: number = 24` (sigma in reference 1080p pixels, 4..64). Appearance presets include fit/background mode/blur, defaulting missing fields, and exclude all source-time reframe keys. Preset application must preserve keys like crop/masks/music/cuts.

Stage gates are sequential: 1 Fit/Fill, 2 Reframe keyframes, 3 Video background blur. Each stage requires root/core, preview and render agreement, focused tests and independent audit before advancing. No device or computer-use tests.

Fit preserves legacy pixel geometry. Fill fills the actual screen pane (inner canvas for bubble, remaining screen pane for split) with an aspect-correct cover viewport within the integer manual-crop bounds. Retain existing even rounding and clamp sizes to the crop bounds; dynamic zoom is applied after deriving the cover size. Fill's default center is the integer base-crop center, not cursor position. With an active legacy zoom and no reframe keys, blend that center toward the segment fixed/follow-cursor center by weight=(zoom_at-1)/(segment.scale-1); zero when segment.scale=1. Scale uses the existing 0.35-second smoothstep envelope. A scale-1 legacy zoom causes no Fill pan. Manual crop, redaction, cursor projection and camera pane semantics remain consistent across preview/export.

Reframe keys are manually authored, use ORIGINAL source seconds and normalized ORIGINAL-source center x/y (0..1), scale1..3, unique nonempty IDs <=128 chars, strictly increasing times separated by at least 0.001 seconds inside [0,duration], <=3000 keys/project. Evaluation is linear interpolation with first/last key held outside the keyed range. No keys preserves legacy behavior. With keys, the interpolated center overrides legacy zoom centers/follow-cursor; legacy zoom SCALE still multiplies key scale. All final source viewports clamp inside the manual crop. Cuts/trim retain source-time keys without retiming; export samples only retained source frames. Timeline Add/Update uses the actual paused source playhead, not a delayed preview event. Controls support seek/select/update/delete, position/scale and Undo/save/reopen. UI updates an existing key within 0.001 seconds instead of creating an invalid duplicate.

Video blur background is a second composition layer from the SAME FINAL source viewport as foreground, after ALL active redactions and before camera/caption layers. It must not reveal raw, cropped-out, zoomed-out, reframe-excluded, deleted or future pixels. Cover the canvas with aspect preserved, scale blur sigma with export resolution, render foreground normally, then camera/captions once. Camera-only uses solid background. Preview original-inspection mode disables the video background; no inspection bypass reaches export. Preview uses bounded cached scratch buffers; export retains <=900frame chunks, bounded motion command files, cancellation and temporary output cleanup. Canvas and FFmpeg blur kernels may differ; geometry/privacy/timing/output contracts must match.

Background scaling first restricts to an aspect-cover subrectangle INSIDE the final viewport, before bounded scratch scaling; never allocate a full cover-scaled version of a thin viewport. FFmpeg may split the SAME redacted decoded frame before the two crops to avoid a second crop inheriting stale dynamic input dimensions. In that equivalent topology the background rectangle MUST be a subset of the final foreground viewport at each source frame. Each branch advances only its own named command targets from its own frame clock, with separate command streams; a faster branch must not mutate a slower queued sibling frame's crop. No crop/redaction rule is relaxed by this implementation choice.

### Apps capture 0.4.4

`CaptureDevices` adds `apps: CaptureApp[]` (serde default empty for older payloads). `CaptureApp = { id: string, name: string, windows: CaptureSource[] }`. Each app is one running process, with `id = process:<pid>`; its name is the queried executable basename, falling back to a real window title if metadata is inaccessible. App window sources retain `kind: window` and use `id = app:<pid>:<hwnd>`. Existing monitor/window sources remain unchanged.

Apps lists only running processes with eligible windows. Selection records exactly one explicitly selected app window through WGC, not every window/process or new popups. Start and resume must validate the selected window still belongs to the expected nonzero PID; stale app IDs never fall back to another window or screen. Metadata lookup uses limited process-query rights and closes every acquired handle. No injection, elevation, automatic capture, process-memory reads, per-app audio isolation, or saved-project schema migration.

Root runs metadata-only `screenrecord_capture::validate_source(id)` before preview shutdown, controls preparation, minimization or project creation. Already-stale targets must fail without creating an empty pending project. Recorder startup retains the same validation, plus delayed native checks. A target disappearing after preflight remains a conservative recovery case if startup evidence is incomplete; no missing-journal classifier rule is relaxed.

The Apps picker supports multiple windows, refresh, empty/error/loading states, keyboard operation, and loss of the selected app/window. Recording options carry the selected canonical app window source ID. Computer audio still captures the system mix and the Apps UI explains that scope.

### Recording audit 0.4.3

The dedicated WGC worker uses a free-threaded frame pool and an explicitly created Win32 message queue for early WM_QUIT. It does not allocate a DispatcherQueue. Partially constructed native capture owns all event tokens/session/pool through rollback, revokes registrations on every later failure, and closes the session before the pool.

Native cleanup and static-frame workers explicitly initialize WinRT and keep that thread-bound apartment alive through destruction. Both encoder constructors share an initialized transcoder worker; initialization occurs before taking the prepared payload, with the initialized parent retaining ownership if worker initialization fails. File and in-memory encoder Finish/Drop must drain all queued frames.

Startup failure writes `capture-startup-diagnostic.json` inside the part directory: schema/build, timestamp, source geometry, independent native startup/frame checkpoints, event/handler counts and HRESULT. Reading checkpoints never locks the frame handler. An epoch-zero journal alone does not distinguish absent frame events from a blocked native operation; timeout wording must not assert that distinction without diagnostic evidence.

Root additionally writes `recording-failure.json` with schema/build/recording identity, phase, bounded error and optional recovery error for start, health and stop faults. Diagnostic write failure must never replace the original recording error. Failed-start archival keeps diagnostic and media files intact.

Frontend terminal events must match recording identity, including events that arrive before a pending start ACK. Idle polling may open only the matching completed project. Stale health failures obey operation revision guards; Stop requires a known active project. Independent initialization RPC successes remain usable when another initialization RPC fails, with failed requests retryable. No IPC shape or project schema change.

### Webcam 0.4.2

New DirectShow camera recordings use `use_video_device_timestamps=false`: frame PTS comes from the graph reference clock rather than the device's nominal frame counter. Initial graph observations map the camera origin onto the screen QPC clock. Camera offsets retain their existing meaning and old projects are not retimed. Irregular frame arrivals retain elapsed time; no global stretch based on the final recording length.

Camera shutdown distinguishes a natural process exit from a deadline termination. A terminated camera process may count as a completed capture only after explicit FFmpeg output completion, child reaping, reader cleanup, and validation of the completed media. Partial output, absent completion, invalid media, nonzero natural exit, and uncertain cleanup remain errors. Fully validated media after slow driver teardown does not mark the project interrupted; diagnostic details stay in the log. Genuine missing camera coverage remains visible.

No IPC or saved-project schema changes. While stop/finalization is pending, recording state remains active with phase `finalizing`; the existing media-operation guards remain held. Completion publishes a ready project, while actual failures preserve the existing recovery/error path.

### Audit 0.4.1

`screenrecord_capture::never_started_screen(directory) -> Result<bool, String>` is read-only and requires native cleanup to be idle. It returns true only for a supported initial session/capture journal with no screen epoch, duration, segments, or final screen media. Unknown, missing, malformed, linked, resumed, or partially written screen data retains normal recovery eligibility. Camera/audio presence does not imply that screen capture started.

After a startup error, or when recovery encounters a proven no-screen attempt, root validates the entire project directory and archives `pending.json` to `failed-start.json` with a no-replace move. `startup-failure.json` records the bounded diagnostic. No media or journal is deleted, and no empty project is published. An archival error leaves the recovery marker intact. A known failed attempt is not repeatedly presented as a damaged recoverable screen recording. The startup UI guard covers preparation failures as well as acquisition failures.

Export publication requires the expected audio shape as well as video frame count: exactly one 48 kHz stereo AAC stream when audio is enabled, beginning within one sample of zero and ending within one AAC packet of edited duration. Muted output must contain no audio stream. Invalid output is not published.

Device enumeration reserves a generation before spawning background work. Camera/microphone allowlists are cleared on reservation and published together under the same mutex only for the latest generation. Late enumeration replies cannot authorize obsolete devices or revoke the newest visible selection. No IPC shape change.

### Upgrade 0.4.0 stage 6

Redaction.tracking defaults to null (static legacy mask). MaskTracking={keyframes:MaskKeyframe[],reviewed:boolean}. Keys={time,x,y,width,height,confidence,kind}, normalized ORIGINAL screen coordinates/source seconds. Kinds manual/auto/lost. First time equals region.start; keys strictly chronological inside [start,end], 1..3002/mask, <=20000/project; tracked region <=300s. Coordinates finite/inbounds, confidence0..1. Lost keys MUST be fullscreen (0,0,1,1); no auto key after lost without intervening manual correction. Unreviewed edits can save. Core validate_mask_tracking_review blocks export of any unreviewed tracked mask intersecting retained video.

Core redaction_rectangle_at is authoritative: choose last key<=time; after last hold. Interior leading to auto uses union of the two endpoint rectangles (conservative rather than interpolation). Leading to manual holds previous rect until exact correction time. Left lost or right lost yields fullscreen SOLID even for blur. Before first/nonfinite time fails closed. Pixel bounds floor left/top and ceil right/bottom. Preview and export use identical evaluated geometry before crop/zoom; existing half-open region activation remains.

Tracked blur uses a fixed patch width/height: maximum pixel rectangle dimensions across normal evaluated rectangles at each key.time, excluding lost (minimum1px). Patch origin is current actual rectangle's pixel left/top clamped to [0,sourceSize-maxPatchSize]. Blur the fixed patch then composite only the evaluated rectangle; lost uses solid fullscreen. Static blur retains existing exact-crop context. Cache maximum dimensions outside playback loops. Export command timelines remain within existing <=900frame chunks, not one process per keyframe.

start_mask_tracking({id,regionId,jobId,start?:sourceSeconds})->ACKvoid; cancel_mask_tracking({jobId}); get_mask_tracking_progress()->{jobId,projectId,regionId,state,progress,error,tracking|null}; event mask-tracking-progress same payload. UI flushes edits and cancels/awaits waveform first. Root registers UUID/cancellation/editor_busy before ACK, resolves original source and geometry from saved project only, runs local capture::tracking in blocking worker, and never modifies source/project automatically. Initial start defaults region.start. Retrack keeps keys<start and replaces tail, requires a manual key at start when tracking already exists. Completion returns full tracking reviewed=false for one undoable UI replacement. Cancel/failure discards all new results, waits for terminal state; stale job actions cannot affect current job.

Tracker uses local FFmpeg grayscale bounded stream at10fps/longest side<=640, fixed-size translation matching, original seed template, texture/score/ambiguity thresholds and latched loss, <=300s. It does not claim arbitrary scale/rotation/occlusion tracking. Manual preview drag/numeric correction inserts/replaces a manual key and resets review; effect/timing/keyframe changes also reset review. Explicit paused Draw correction can inspect the original frame with visible status, no playback/audio; leaving mode synchronously restores masks. Original inspection is never used in export. Clear tracking preserves current evaluated rectangle as static (solid if lost). Unknown extended ranges fail closed. No network/model/device acquisition.

### Upgrade 0.4.0 stage 5

SubtitleCue adds defaulted words:[{start,end,text}] and sourceRanges:[{start,end}], all in original source seconds. Empty ranges means legacy full cue coverage; explicit coverage can have holes with other cues in them. Validation checks actual coverage, positive nonoverlapping whole words contained in one range, exact single-space join(words.text)==cue.text, <=500 words/cue, <=200000 words and <=20000 coverage intervals/project. Editor metadata <=15MiB reserves 1MiB of the16MiB manifest for project fields. Schema remains4.

Core projected_subtitle intersects declared coverage with trim/EDL and keeps only entire words inside retained coverage. Preview, timeline labels, ASS and SRT consume projected safe text. Original captions remain editable/undoable. Untimed cues partially removed by EDL remain hidden. SRT import maps OUTPUT seconds to a logical cue with SOURCE ranges, no synthetic word timestamps. Root import_subtitles({id,path})->SubtitleCue[] reads only the chosen <=2MiB UTF8/UTF16 SRT; applying it is frontend one-Undo.

SubtitleStyle adds highlightWords=false and highlightColor=#E9B451. render_subtitle_preview accepts optional wordIndex; indexed misses opportunistically rasterize up to8 appearances in one FFmpeg process via render::subtitle_previews. Keys include text, ordered word segmentation, style, crop/aspect/geometry and index, not IDs/timing/EDL. Base cache is128files/64MiB, frontend8images/64MiB, perPNG16MiB and batch64MiB. Export ASS uses the same layout/colors/literal escaping and coverage/frame boundaries.

TranscriptionOptions.provider defaults local, API key optional and zeroized locally. Cloud requires explicit provider and valid key. Local install/status/progress/cancel commands use fixed AppLocalData/local-transcription paths. start_local_transcription_install registers UUID/cancel/editor_busy before ACK; frontend awaits ACK and drains matching terminal state on cancel/close/provider change. Terminal state is running/completed/cancelled/failed with optional LocalStatus. Status/hash is read on demand, not polled each second. No automatic download, generation, provider switch or upload.

Local runtime whisper.cpp1.9.5 (commit d1be6fde11ac6e0407606b4e42fe72d34add8037), official x64 asset b5454 SHA256 6ba69e3482d7826214f90a6a9c84ca07782aec1e1d0c6a7c30c994fd5d816ccb. Multilingual ggml-base.bin pinned revision5359861c739e955e79d9a303bcbc70fb988958b1 SHA256 60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe. Download/install verifies hashes and official HTTPS redirect hosts; inference holds verified runtime/model read-only, rejects extra DLLs and does not use PATH binaries. Retained-range extraction is shared with cloud. Grouped zero-duration neighboring CLI tokens retain original union timing; no invented timestamp.

### Upgrade 0.4.0 stage 4

CameraSettings.layout defaults to bubble and accepts bubble/fullscreen/split. Fullscreen ALWAYS hides screen/cursor/source effects regardless camera.enabled/path/frame availability; camera unavailable or hidden yields background only. Split reserves static screen/camera panes even when camera is hidden/missing. Landscape uses screen left/camera right; portrait uses screen top/camera bottom. Camera.width is split share0.08..0.6; gap=min(padding,24)*outputscale rounded to even pixels. Bubble retains existing geometry/shape; split/full use rounded rectangles and cover for camera, contain for screen. Mirror works in all modes. Preview must synchronously invalidate old screen pixels when switching fullscreen, including while decoded seek is pending.

AppearanceSettings contains ONLY aspectRatio/background/padding/cornerRadius/cursorSize/cursorSmoothing/showCursor/camera/subtitleStyle. Preset application is one undoable edit and preserves trim/crop/EDL/zooms/music/volumes/audioProcessing/subtitle text/redactions/media/cameraIntervals. No-camera application forces camera.enabled=false while preserving layout; UI rejects camera-only presets without a recording camera. Direct screen-mask/camera manipulation is unavailable when the relevant geometry is not visible; UI offers a layout switch.

`list_appearance_presets()->AppearancePreset[]` returns custom presets only. Built-ins are frontend readonly. `save_appearance_preset({id?:UUID,name,settings})->AppearancePreset` creates or updates existing; `delete_appearance_preset({id})->void`. Names use Project limits120characters, trim and case-insensitive uniqueness. AppLocalData/settings/appearance-presets.json format1 holds max50/128KiB, rejects symlink/reparse file targets, validates before mutation, and uses synced temporary atomic replacement. Corrupt/oversized/future files remain unchanged. All API file operations run on blocking workers under one preset_write mutex; no source/project writes.

### Upgrade 0.4.0 stage 3

`RecordingState` adds defaulted `paused`, `cameraAvailable`, and `phase` (idle/recording/paused/finalizing). `active` remains true during Pause and finalization; elapsed excludes Pause and stays frozen while finalizing. `pause_recording({projectId})` and `resume_recording({projectId})` return updated state. `stop_recording({projectId?})` returns Project; current clients always pass the UUID. Stale UUIDs cannot mutate or take the active slot. Global shortcut binds the ID at keypress.

Capture `session::RecordingSession` fully stops native acquisition on Pause and starts a new `part-00000` directory on Resume. `session-journal.json` is written before acquisition, with at most512 parts. Failures preserve partial parts and recovery emits explicit warnings for unusable/incompatible parts. The independent backend health watchdog runs every second while the matching session exists; UI polling is not required. Stop trigger is `MAX_RECORDING_SECONDS=28800`; recovery/assembly/export accept `MAX_RECORDING_RECOVERY_SECONDS=28860` to preserve polling/stop overshoot.

One part is published directly using confined nested media paths. Multiple parts pass immutable inputs to `render::assemble_recording`; outputs are uniquely named assembled directories published atomically without replacing existing data. Frame/48k sample grids are cumulative; part offsets use unrounded source durations. Cursor tails at/after each part end are discarded, and an invisible/unpressed cursor sample at Resume resets smoothing. Original parts remain available for recovery.

`Project.cameraIntervals:Option<CameraInterval{start,end}[]>` defaults to None (legacy full camera timeline); Some([]) hides all. Intervals are chronological, nonoverlapping, at most512, in assembled original source time. Joined camera files contain filler outside these intervals; preview/export must not display it. Existing camera EOF and offset checks still apply.

The `recording-controls` webview loads local `index.html?overlay=recording`, is always on top and requests capture exclusion; main requests exclusion before minimization and native capture startup. Root requires successful native SetWindowDisplayAffinity; a successful Get readback must match, but unavailable readback is diagnostic because Microsoft documents Get only for layered windows. Actual pixel exclusion still requires manual native verification. References: [Set](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity), [Get](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowdisplayaffinity). Window movement uses a dedicated drag handle and keyboard; size/position clamp to monitor work area. Position persists within the application session.

`get_recording_camera_frame({projectId})->{generation,sequence,mimeType,data,active,error}` reads the bounded latest JPEG from the SAME FFmpeg camera process that records H264, not a second camera acquisition. Generation changes per Resume. UI polls serially250ms only while recording with camera; it discards old generations/action responses and hides paused frames. `restore_main_window()->void` shows the protected main window. Countdown0/3/5 seconds is frontend only and cancellable before start IPC.

Root broadcasts `recording-state-changed` for active state changes, `recording-finalization-progress{projectId,progress}`, and terminal `recording-stopped(Project)`/`recording-error{projectId,error}`. Terminal events occur after the backend operation/busy guard is released; clients reject older session IDs, including errors arriving during a new start acknowledgement. Finalizing cached state survives taking the native slot so clients cannot mistake it for idle. Capture exclusion/physical Pause/Resume/sync remain manual test items.

### Upgrade 0.4.0 stage 2

Schema stays 4 during this unreleased upgrade. Default `EditSettings.audioProcessing` is `{microphoneGain:1,systemGain:1,ducking:{enabled:false,amountDb:12}}`. Gains are finite 0.01..8 and ducking amount 0..24 dB. Recording gain multiplies the existing user volume. Ducking only changes music, driven by microphone after volume and normalization; the existing final limiter still runs once.

Deterministic 48 kHz stereo ducking: sidechain power=(L^2+R^2)/2; power envelope exponential attack10ms/release150ms. Activity clamps `(sqrt(envelope)-0.01)/(10^(-30/20)-0.01)` to0..1. Target music gain=1-activity*(1-10^(-amountDb/20)); music-gain envelope attack10ms/release250ms. Export state continues over retained output samples across cuts/chunks. Explicit preview seek/restart resets it; automatic cut jumps preserve it.

Preview duck DSP must freeze at the latched input cut deadline and during decoded seeks, including when raw media continues briefly upstream of the final gate. Inactive samples output silence without changing the sidechain or gain envelope; deleted source audio must not influence the next kept span.

`analyze_audio({id,source,jobId})->AudioAnalysis` waits for a cancellable background job. Source is microphone/system or `music:<clipId>`, resolved exclusively from saved project media. `cancel_audio_analysis({jobId})->void` cannot cancel other generations. Event `audio-analysis-progress` supplies `{jobId,projectId,source,progress}`. Only one analysis runs; it participates in backend busy checks. UI cancels and awaits it before recording/export/transcription/import/delete/close. Local edits can proceed and invalidate normalization statistics.

Jobs use fresh UUIDs. The backend retains up to128 pending cancellation UUIDs so a cancellation received before the async Analyze handler is polled cannot be lost. Reservation consumes its pending cancellation without opening a media file or changing the busy state.

`AudioAnalysis{sourceDuration,bins:[{min,max}],peak,rms,analyzedSeconds,recommendedGain}` has at most4096 source-anchored uniform waveform bins. Raw peak/RMS use only retained source intervals, including music file offsets. Recommended gain targets -18 dBFS RMS with -1 dBFS peak headroom, bounded0.01..8; silence returns1 and UI disables Normalize. No files are rewritten. Waveform display projects original source bins through the EDL.

`start_microphone_meter({microphoneId,sessionId})->void`, `get_microphone_meter({sessionId})->{sequence,peak,rms,active,ready,error}`, `stop_microphone_meter({sessionId})->void`. Meter starts only from explicit Test action, opens only a validated enumerated microphone, stores/plays no audio, has100ms windows,10s startup and30s session limits. UI polls serially at100ms and stops on dialog close/device change/recording/unmount. Backend revokes pending generations and stops resources before capture/close; old Stop cannot affect a new session.

### Upgrade 0.4.0 stage 1

Project version is 4; versions 1/2/3 load and migrate on Save. `EditSettings.videoClips:VideoClip[]` defaults to[]; `VideoClip{id,start,end}` uses original source seconds. Explicit clips are ordered, nonoverlapping, >=100ms, <=512. Empty means legacy trim, never delete-all. Effective clips intersect source trim and must retain >=100ms total.

`Project.duration` remains the original source duration; library `ProjectSummary.duration` reports the saved edited output duration. Summary reads skip cursor data and do not open cursor sidecars.

Core `kept_intervals(&EditSettings,duration)->Vec<KeptInterval{start,end,output_start}>` and source/output/project_interval helpers define half-open mapping. Timeline displays contiguous output time; edits other than EDL stay source anchored. Global video frame indices use ceil(cumulative output seconds*fps), sampling source start+(frame/fps-output_start). PCM uses the equivalent48000Hz grid, with one final AAC encode. Adjacent splits must be visually/audibly identical to unsplit content.

Video samples retain the source frame whose presentation interval contains the sampled source time. Its PTS can precede a fractional kept start, matching HTMLVideo; future frames whose presentation begins at or beyond the kept end must never be selected. Audio uses individual native sample timestamps in [start,end), clipped with integer native-sample boundaries before resampling to prevent deleted samples ringing into retained audio.

Core retained_subtitle rejects cues partly removed by EDL, so deleted speech cannot leak through unchanged sentence text. Legacy trim behavior stays intact. SRT projects surviving cues to output time. Cloud TranscriptionInput.ranges contains kept source ranges; chunk context and resampling must never read/upload gaps. Empty ranges preserves legacy trim.

Preview must not draw decoded frames from gaps or excluded endpoints. At cut transitions pause/mute, await source+auxiliary seek completion, invalidate stale generations and resume with precise span audio gating. Native devices and cloud accounts are not exercised during automated tests.

### Project actions and canvas editing 0.3.1

`delete_project({id})->void` moves the validated project folder to the Windows Recycle Bin, using `IFileOperation` with `FOFX_RECYCLEONDELETE`; no permanent-delete fallback. Exports outside the project folder stay in place. The backend refuses recording/recovery/export/import/transcription and outstanding capture cleanup, serializes project writes and subtitle rasters, checks UUID and manifest identity, confines the direct child to the canonical library, and rejects nested symlinks/junctions/reparse points before dispatch. Filesystem path input is never accepted from the frontend.

UI requires a named confirmation, pauses and flushes saves, then calls Preview.release() and unmounts it before deleting the displayed project. Failure remounts the current in-memory edits with an actionable error. Success invalidates in-flight library requests and clears/open-replaces the deleted project without autosave resurrection. A fresh explicit Refresh can list a project restored through Windows Recycle Bin.

Preview manipulation changes the existing redaction/camera edit settings. Mask geometry remains normalized to the original screen; camera geometry remains normalized to final output canvas. Move and resize use the same inverse view transform at Fit/100%/200%, crop, zoom, and scroll. One gesture is one history step; Escape/pointer cancellation restores the original item. Numeric property controls remain available.

### Recorder/editor 0.3.0

Project schema remains 3. Existing source media and edit DTOs are unchanged. Camera preview is a local preflight operation, not a project track. Start only after explicit Enable preview. Source selection by itself does not open devices.

`start_camera_preview({cameraId,sessionId})->void` validates the selected ID against the last native device enumeration, reserves the session before background work, and opens the DirectShow device. Start acknowledges process creation; frames may still be loading.
`get_camera_preview_frame({sessionId})->{sequence:u64,mimeType:'image/jpeg',data:string|null,active:bool,error:string|null}` returns only the latest frame, at most 512 KiB before base64. UI polls serially at 250 ms, never concurrently.
`stop_camera_preview({sessionId})->void` revokes an in-flight request immediately, then releases matching native resources. An old session cannot stop a newer one. UI stops preview on dialog close/Escape, camera change, recording start, and component unmount. Backend also stops/revokes all preview before Recorder::start and close_app. Drop/process cleanup handles abnormal application exit.

Preview has no audio, no disk media, no upload, and a 15-second first-frame deadline plus a five-second stalled-frame watchdog. Native stop has a two-second caller deadline with retained cleanup ownership; uncertain cleanup prevents the next recording until writers exit. Tests never open physical cameras.

Editor viewport scaling is independent from export edits: Fit follows available viewport size; 100% and 200% use the canonical 1080p export canvas in CSS pixels. Scroll/pan changes only the view. Source-coordinate masks must retain their position under all view scales, scroll positions, crop and zoom settings. Export summary uses the same short-edge sizing rule as Rust render math.

### Editor 0.2.0 contract

Project schema is now version 3. Versions 1 and 2 load without writes and migrate on save. Version 3 prevents older applications from silently dropping editor tracks. `EditSettings` adds defaulted `music`, `subtitles`, `subtitleStyle`, and `redactions`. Imported audio paths are absolute over IPC and relative on disk, confined to the project directory. Existing original recording paths and cursor sidecars are preserved.

- MusicClip { id,name,path,sourceDuration,start,sourceStart,duration,volume,fadeIn,fadeOut }. Maximum 32. Source recording time `t` maps to music time `sourceStart+t-start`. Linear fades fit inside duration; volume 0..2. All sources mix before the existing limiter. Imports normalize a user-selected audio file into project-owned 48 kHz stereo float32 WAV; source files remain unchanged. Supported imports: MP3/WAV/M4A/AAC/FLAC/OGG/Opus, at most 512 MiB and one hour.
- SubtitleCue { id,start,end,text }. Maximum 10000, no overlap, each 1..500 characters and at most three explicit lines; total text <=1 MiB. SubtitleStyle { enabled,fontSize,color,background,position }, Segoe UI, fontSize 12..96 relative to the output's 1080px short edge, opaque hex colors, position top/bottom. Preview uses the same libass raster as export.
- Redaction { id,start,end,x,y,width,height,mode,strength }. Rectangles are normalized to the original captured screen before crop/zoom, floor left/top and ceil right/bottom. Mode blur uses Gaussian sigma 1..50 original source pixels, solid uses opaque black. Maximum 100 regions and 16 simultaneously active.

All track times use original recording seconds and half-open intervals [start,end). Export intersects the saved trim. Undo/redo changes only edit data; imported media remains available after undo. No automatic deletion of user media.

Editor commands:
`import_music({id,path,start})->MusicClip`
`export_subtitles({id,outputPath})->void` (SRT timecodes rebased to selected trim, no overwrite)
`render_subtitle_preview({id,cue,style,aspectRatio,crop})->string` (absolute cached PNG path for unsaved inputs)
`start_transcription({id,jobId,options:{apiKey,source:'microphone'|'system',language:'id'|'en'|'auto'}})->void`
`cancel_transcription({jobId})->void`
`get_transcription_progress()->TranscriptionProgress|null`
Event `transcription-progress` uses {jobId,state:'running'|'completed'|'cancelled'|'failed',projectId,progress,error,cues:SubtitleCue[]|null}. Frontend creates a fresh UUID per job and ignores other job IDs. Backend registers cancellation and publishes running before asynchronous preparation, then acknowledges start. Cancel is enabled after this acknowledgement and matches only that job. Completion returns cues for an undoable UI transaction; backend does not replace saved subtitles. UI flushes edits before generation and blocks project switching until completion/cancellation. Empty speech/failure preserves existing cues.

Cloud transcription is the user-authorized exception to local-only media processing: explicit Generate sends only the selected recorded audio and trim to Deepgram Nova-3, with model-improvement opt-out. Keys are session-only, excluded from logs, manifests and settings. The cloud provider can charge for accepted requests, including cancellation after upload. No actual paid API calls in automated tests; use local mock HTTP. Native UI testing is manual by the user, no computer-use automation.

Shared Rust types are in `screenrecord-core` (`crates/core`). All serialized field names use camelCase; enums use lowercase. Timestamps for edit/cursor data are seconds from the recording start. Normalized coordinates range 0..1 within the captured content. IPC media paths are absolute; version 3 manifests store project-relative paths. Legacy paths are rebased within their recorded project directory when a project is relocated. The root owns command implementations.

Project storage version 3 extends editor tracks and retains the cursor sidecar format introduced in version 2. On disk, `project.json` has a `cursorFile` reference instead of an inline `cursor` array. The immutable `cursor-<uuid>.bin` sidecar has an 8-byte `SRCURS01` header, little-endian u64 count, then 25-byte records (f64 time/x/y and pressed/visible bits). The reference records the sample count and SHA-256. Writers publish the durable sidecar before atomically replacing the manifest. Save and load both limit cursor data to 16 million samples and new manifests to 2 MiB. Legacy version 1 JSON is read without modifying the source, with a 1 GiB input limit, and migrates on save. Existing media and recovery `cursor.ndjson` are preserved.

### Initial DTO reference

- CaptureSource { id: string, title: string, kind: 'monitor'|'window', width: u32, height: u32 }
- Device { id: string, name: string }
- CaptureDevices { sources: CaptureSource[], microphones: Device[], cameras: Device[] }
- RecordingOptions { sourceId: string, fps: u32 (30|60), microphoneId: string|null, systemAudio: bool, cameraId: string|null }
- CursorSample { time: f64, x: f64, y: f64, pressed: bool, buttons?: u8, visible: bool }
- ZoomSegment { id: string, start: f64, end: f64, scale: f64 (1..3), x: f64, y: f64, followCursor: bool }
- EditSettings { trimStart: f64, trimEnd: f64, aspectRatio: 'source'|'16:9'|'9:16'|'1:1', background: string (hex), padding: u32, cornerRadius: u32, cursorSize: f64, cursorSmoothing: f64, showCursor: bool, zooms: ZoomSegment[], camera: CameraSettings, crop: CropSettings|null, systemVolume: f64, microphoneVolume: f64 }
- CameraSettings { enabled: bool, x: f64, y: f64, width: f64, shape: 'rectangle'|'circle', mirror: bool }, normalized canvas position/width.
- CropSettings { x:f64,y:f64,width:f64,height:f64 }, normalized source rectangle.
- Project { version: u32, id: string, name: string, createdAt: string, directory: string, screenPath: string, microphonePath: string|null, systemAudioPath: string|null, cameraPath: string|null, cameraOffsetSeconds: f64, width: u32, height: u32, fps: u32, duration: f64, cursor: CursorSample[], edits: EditSettings, status: 'ready'|'interrupted' }
- ProjectSummary { id,name,createdAt,duration,directory,status }
- RecordingState { active: bool, elapsed: f64, projectId: string|null }
- ExportOptions { outputPath: string, preset?: 'standard'|'whatsapp', resolution: '1080p'|'1440p'|'2160p'|'auto', fps: u32 }
- ExportProgress { state: 'running'|'completed'|'cancelled'|'failed', progress: f64 (0..1), outputPath: string, error: string|null, outputBytes?: u64 }
- AppInfo { version:string, projectsDirectory:string, ffmpegAvailable:bool, ffmpegPath:string|null }

### Initial capture API

`pub fn list_devices(ffmpeg_path: &Path) -> Result<CaptureDevices, String>`
`pub struct Recorder` implements Send. `Recorder::start(directory: &Path, options: &RecordingOptions, ffmpeg_path: &Path) -> Result<Recorder,String>`, `elapsed(&self)->f64`, `stop(self)->Result<CaptureResult,String>`.
`CaptureResult` in capture crate: screen_path PathBuf, microphone_path Option<PathBuf>, system_audio_path Option<PathBuf>, camera_path Option<PathBuf>, camera_offset_seconds f64, width u32,height u32,fps u32,duration f64,cursor Vec<CursorSample>.
Store durable recording metadata so app crashes leave recoverable files. Camera optional but implemented. Capture actual pointer-free frames and separate global cursor position/button samples. Webcam and audio errors must be visible, never silently accepted. No mock device results. Drop must clean worker/process lifetime.

### Initial render API

`pub fn auto_zoom(cursor:&[CursorSample], duration:f64)->Vec<ZoomSegment>`
`pub fn export(project:&Project, options:&ExportOptions, ffmpeg_path:&Path, cancel:Arc<AtomicBool>, progress:impl Fn(ExportProgress)+Send+'static)->Result<(),String>` blocking, invoked on a background thread.
Use hardware encoding when viable with working software fallback. Keep original media intact. Write temporary output then atomic publication, never overwrite an existing user file. Add probe helpers as needed. Export must implement editable zoom, cursor smoothing/redraw, aspect/crop/padding/background, trim, webcam overlay/mirror/shape, audio volumes. Validate inputs and clean only owned temporary artifacts. Include real generated-media export integration tests if ffmpeg exists.

### Initial Tauri command reference

`get_app_info() -> AppInfo`
`list_capture_devices() -> CaptureDevices`
`list_projects() -> ProjectSummary[]`
`open_project({id}) -> Project`
`start_recording({options}) -> RecordingState`
`get_recording_state() -> RecordingState`
`stop_recording() -> Project`
`save_project({project}) -> Project` (backend validates and canonicalizes paths)
`save_project_edits({id,name,edits}) -> {name,edits}` is the frontend save route. It changes only the manifest, reusing the immutable cursor sidecar. Existing version 1/2 projects migrate on the first save. Library listings read summaries without decoding cursor data.
`rename_project({id,name}) -> Project`
`export_project({id, options}) -> void`
`cancel_export() -> void`
`get_export_progress() -> ExportProgress|null`
`reveal_file({path}) -> void`
Events: `export-progress` ExportProgress. Poll recording status while active; poll export as fallback. Use Tauri dialog plugin save dialog for output path. Use convertFileSrc for local video. No fake data in browser mode: show desktop runtime required with working explanatory state. English UI, functional labels. Name Screenrecord.

Use the same timing/normalized coordinate math for preview as export; consult render agent when implementing preview.

`Project.cameraOffsetSeconds` and `CaptureResult.camera_offset_seconds` default to zero for existing aligned camera files. New remuxed camera tracks retain their frames; screen time `t` maps to camera time `t + cameraOffsetSeconds`. Offset must be finite and between 0 and 3600 seconds. Hide the camera after its media ends, consistently in preview and export. The 100 ms minimum trim/zoom duration uses a 1e-9 second tolerance on both sides of IPC.
