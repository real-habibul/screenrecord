# Screenrecord

Build a local Windows 11 x64 recorder and editor with Rust and Tauri 2. The first release includes screen/window capture, microphone, system audio, webcam, editable cursor and auto zoom, trim/crop, backgrounds, saved projects, and MP4 export.

Three subagents are authorized. Ownership: capture agent owns crates/capture; render agent owns crates/render; UI agent owns src plus frontend package/tooling; root owns crates/core, src-tauri, workspace configuration, packaging, and integration. Do not overwrite another agent's changes. Coordinate API changes through CONTRACT.md and the root agent.

No decorative brand dots, left-edge accents on panels, em dashes, marketing filler, fake project data, or nonfunctional controls. Segoe UI, restrained neutral surfaces, amber action emphasis. Preview and timeline lead the composition. ENERGY 1 / RHYTHM 1 / MOTION 1. Dark and light themes must work. Use antislop with saved during mode and relevant modules.

Every feature exposed to the user must work. Tests must cover meaningful state, validation, timing, and output contracts. Never label the product production ready without release evidence; signing and hardware coverage must be reported accurately. No telemetry or remote upload. Capture starts only with an explicit user action. Do not record application debugging runs indefinitely.

Editor 0.2.0: the user explicitly authorized cloud auto-subtitles, so selected recorded audio may be uploaded only after the user presses Generate with their own session-only API key. No other upload or telemetry. Root owns crates/core, src-tauri, integration and packaging; render agent owns crates/render; UI agent owns src and frontend tests/tooling; former capture agent owns new crates/transcribe. The user will manually test the native app. Do not run computer-use tests; automated unit/browser/media tests remain authorized.
