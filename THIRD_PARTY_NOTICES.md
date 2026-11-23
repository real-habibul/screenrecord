# Komponen pihak ketiga

Kode aplikasi Screenrecord menggunakan [lisensi MIT](LICENSE). Komponen berikut mempertahankan lisensi, hak cipta, dan syarat distribusi masing-masing.

## FFmpeg dan FFprobe

Screenrecord 0.7.4 menggunakan FFmpeg dan FFprobe untuk Windows x64. Build `n8.1.3-34-ga35157abb9` melaporkan LGPL versi 3 atau yang lebih baru. Installer menyertakan dua executable serta tujuh DLL FFmpeg. Screenrecord menjalankan FFmpeg sebagai proses terpisah. Pengguna dapat mengganti executable dan DLL di direktori `resources` dengan satu set build yang kompatibel. [Sumber FFmpeg](https://github.com/FFmpeg/FFmpeg/tree/a35157abb96167fd7308edc92b55408d5bb084b0) dan [informasi lisensi FFmpeg](https://ffmpeg.org/legal.html).

| Materi | Identitas |
| --- | --- |
| Arsip biner | [win64-lgpl-shared, FFmpeg 8.1](https://github.com/FFmpeg-Builds/FFmpeg-Builds/releases/download/autobuild-2026-11-10-12-39/ffmpeg-n8.1.3-34-ga35157abb9-win64-lgpl-shared-8.1.zip) |
| SHA-256 arsip biner | `3045c2213ed8cbb05edf0d5a631b4490061de3b7567ec145644156e7c98da2c6` |
| Resep build | [FFmpeg-Builds c9fe5114](https://github.com/FFmpeg-Builds/FFmpeg-Builds/tree/c9fe5114060e507409f7ee07edfd82efaf25d767) |
| Proses build upstream | [GitHub Actions run 42813262313](https://github.com/FFmpeg-Builds/FFmpeg-Builds/actions/runs/42813262313) |

[scripts/prepare-media.ps1](scripts/prepare-media.ps1) memeriksa SHA-256 sebelum menyalin runtime. Konfigurasi biner dapat diperiksa dengan `resources/ffmpeg.exe -buildconf` dan lisensinya dengan `resources/ffmpeg.exe -L`. DLL ini juga berisi kode pustaka eksternal, termasuk libass, libopenh264, libopus, dan libvpx. Unduh kedua arsip sumber dari rilis installer. Arsip pertama berisi sumber FFmpeg, resep, sumber aplikasi pihak ketiga dan materi rav1e. Arsip kedua berisi 84 masukan build untuk target Windows x64. Rincian versi serta petunjuk kompilasi tersedia dalam [panduan sumber](licenses/THIRD-PARTY-SOURCES.md).

## Limiter audio

Modul AudioWorklet mengadaptasi filter `af_alimiter.c`. Hak cipta Krzysztof Foltman, Markus Schmidt, Thor Harald Johansen dan kontributor lain, serta Paul B Mahol. Lisensi LGPL versi 2.1 atau yang lebih baru. Adaptasi mengolah audio stereo dengan attack 5 ms, release 50 ms dan batas 0,95. Installer menyertakan kode JavaScript yang dapat dibaca dan teks lisensinya di direktori `licenses`. [Sumber adaptasi](public/audio-limiter.js), [filter asli](https://github.com/FFmpeg/FFmpeg/blob/a35157abb96167fd7308edc92b55408d5bb084b0/libavfilter/af_alimiter.c), dan [lisensi LGPL 2.1](licenses/audio-limiter-LGPL-2.1.txt).

## Aplikasi Windows dan editor

Editor menggunakan Vue dan Tauri. Installer menyertakan teks lisensi Rust, JavaScript, SDK WebView2 dan NSIS dalam `licenses/runtime-notices.zip`. Atribusi pustaka build media terdapat dalam `licenses/ffmpeg-notices.zip`. [Unduhan rilis](https://github.com/real-habibul/screenrecord/releases). Versi dependensi JavaScript dan Rust tercatat dalam [package-lock.json](package-lock.json) dan [Cargo.lock](Cargo.lock). Pohon sumber memuat varian Windows Capture yang digunakan aplikasi beserta [lisensi MIT upstream](vendor/windows-capture/LICENCE). Installer menggunakan NSIS dan memakai bootstrapper WebView2 Evergreen dari Microsoft bila runtime belum tersedia. [Ketentuan WebView2](https://developer.microsoft.com/en-us/microsoft-edge/webview2/).

## Transkripsi opsional

Jika pengguna mengaktifkan transkripsi lokal, aplikasi mengunduh whisper.cpp build b5454 dari commit `d1be6fde11ac6e0407606b4e42fe72d34add8037` dan model Whisper multilingual base dari revisi `5359861c739e955e79d9a303bcbc70fb988958b1`. Keduanya menggunakan lisensi MIT. Aplikasi memverifikasi SHA-256 dan menyimpan atribusi pada direktori pemasangan lokal. Pilihan transkripsi cloud menggunakan API Deepgram dengan kunci pengguna. [Sumber whisper.cpp](https://github.com/ggml-org/whisper.cpp/tree/d1be6fde11ac6e0407606b4e42fe72d34add8037), [revisi model](https://huggingface.co/ggerganov/whisper.cpp/tree/5359861c739e955e79d9a303bcbc70fb988958b1), dan [lisensi Whisper](https://github.com/openai/whisper/blob/main/LICENSE).
