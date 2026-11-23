# Screenrecord

Screenrecord is a Windows 11 x64 desktop recorder and editor built with Rust, Tauri, and Vue. Use [the contributor guide](CONTRIBUTING.md) for build and test commands and update [the project contract](docs/project-contract.md) when changing persisted data or IPC.

## Privacy and data

- Capture starts after an explicit user action. Keep development captures short and use disposable projects.
- Keep recordings and exports on the device. Cloud transcription may upload the selected recorded audio after the user explicitly starts generation with their own session-only key. Screenrecord collects no telemetry.
- Preserve existing projects and recordings when changing storage, recovery, preview, or export.

## Interface and validation

- Use Segoe UI, neutral surfaces, and amber for primary actions. Keep the preview and timeline prominent in both light and dark themes. Avoid decorative brand dots, left-edge panel accents, em dashes, and placeholder controls.
- Use the antislop skill in the user's saved mode when available, with the modules relevant to the change.
- Use automated unit, headless browser, and synthetic media tests. Reserve native app and device testing for manual verification.
- Report signing, clean-install checks, and hardware coverage according to the evidence for the exact release.
