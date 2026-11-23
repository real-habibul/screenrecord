# Audit tampilan dan alur 0.3.0

11 November 2026. Audit mengacu pada screenshot pengguna, source, metadata proyek yang sudah ada, dokumentasi produk pembanding, serta pengujian otomatis. Aplikasi Windows dan perangkat fisik tidak dikendalikan melalui computer-use. Ini bukan bukti pengujian kamera/driver native.

## Temuan dan tindakan

| Prioritas | Temuan yang terverifikasi | Tindakan |
| --- | --- | --- |
| P1 | Proyek 19.37.13 tidak memiliki `cameraPath`, tidak ada file kamera, `cameraEpochQpc` kosong, dan overlay disabled. Proyek 16.30.04 memiliki kamera. UI tidak membedakan kamera tak direkam dari overlay disembunyikan. | Status sumber rekaman terlihat, penjelasan camera missing, pengaturan overlay hanya aktif bila track ada, dan akses langsung ke rekaman baru. |
| P1 | Dialog rekam hanya menyediakan pilihan webcam tanpa live preview. Pengguna tidak dapat memeriksa framing atau kegagalan kamera sebelum Start. | Preview kamera native dengan tombol eksplisit, status loading/error/retry, pembatasan frame, serta cleanup sebelum recording/close/device change. |
| P2 | Header aplikasi, toolbar proyek, dan strip undo terpisah menghabiskan tinggi preview. Library selalu memakai 210 px. | Toolbar proyek ringkas dan library yang bisa ditutup/dibuka. |
| P2 | Delapan tab properti kecil membungkus menjadi dua baris; aksi musik/subtitle/mask tersembunyi. | Navigasi properti yang terbaca, aksi kontekstual dan status sumber yang dapat dipilih. |
| P2 | Timeline menampilkan semua track kosong dan memotong baris Masks pada viewport. | Tinggi timeline dibatasi dengan scroll, header track tetap terlihat, dan track kosong menjadi aksi tambah yang fungsional. |
| P2 | Klip layar hanya punya overlay trim, sementara zoom hanya bisa dipilih. | Pegangan trim layar dan move/trim zoom, dengan snap frame, batas overlap, keyboard dan undo. |
| P2 | Preview selalu fit dengan buffer maksimum 1280 px; sulit memeriksa teks/form. | Fit/100%/200% untuk kanvas 1080p, pan/scroll dan buffer detail berbatas. Tidak mengubah hasil ekspor. |
| P2 | Dialog ekspor tidak memperlihatkan dimensi akhir atau durasi trim. | Ringkasan ukuran piksel, fps dan durasi sebelum ekspor. |

## Arah tampilan

Editor Windows dengan Segoe UI, permukaan graphite netral dan aksi utama amber sesuai arahan proyek. Preview menjadi fokus tunggal. Toolbar merangkum identitas proyek, status simpan, undo/redo dan ekspor. Library adalah navigasi opsional; properti mengikuti alat atau item yang dipilih. Timeline berada tepat di bawah transport, dengan ukuran yang menjaga preview tetap berguna pada 1280 x 720.

ENERGY 1 / RHYTHM 1 / MOTION 1. Hierarki berasal dari ukuran area kerja, tipografi dan pengelompokan kontrol. Radius kecil membedakan kontrol dari preview; border seragam memisahkan permukaan. Tidak ada logo baru, titik dekoratif, aksen panel kiri, slogan, atau efek gerak pengisi ruang. Tema gelap/terang dan keyboard tetap diperlukan. Referensi digunakan untuk pola kerja dan komposisi, tanpa menyalin aset merek.

## Produk pembanding dan cara kerja

| Produk | Alur yang dipelajari | Penerapan pada Screenrecord |
| --- | --- | --- |
| [Screen Studio: webcam/microphone](https://screen.studio/guide/webcam-microphone) dan [trim](https://screen.studio/guide/trimming) | Pilih kamera dan periksa preview sebelum merekam; setelah Stop, atur komposisi kamera dan trim di editor. Dokumentasi memperlihatkan preview kamera terpisah dari hasil akhir. | Preflight kamera, perbedaan input/overlay, pegangan timeline dan preview dominan. |
| [Cap Studio](https://cap.so/docs/recording/studio-mode) dan [tampilan editor](https://cap.so/) | Pilih Studio, target layar/window, kamera/mikrofon/audio, Record, Stop, edit proyek lokal, kemudian ekspor. Kamera dan scene bergantung pada keberadaan footage. | Status source yang jujur, alat kontekstual, pengaturan kanvas dan ekspor yang dapat ditinjau. |
| [Tella: rekaman pertama](https://www.tella.tv/video/how-to-record-your-first-video-using-the-web-app-7ndy) dan [layout](https://www.tella.tv/video/customize-your-video-layout-dzyj) | Pada Windows, web app meminta izin kamera/mikrofon; pilih perangkat, lihat preview, share screen, Record, Stop, lalu Preview menuju editor. Layout layar/kamera dapat diubah sesudah rekam. | Pilihan input dan framing sebelum Start, pemisahan recorder dari editing, serta aksi kamera yang mudah ditemukan. |

Referensi diperiksa pada tanggal audit. Tidak mengunduh atau menginstal aplikasi pembanding. Perbandingan ini tidak menyatakan kesetaraan fitur.

## Kontrak preview kamera

Preview dimulai hanya dari tindakan pengguna, memakai identitas kamera DirectShow yang sama dengan recorder. Frame JPEG berada di memori dan hanya frame terakhir dipertahankan; tidak ada audio, file rekaman, atau upload. Tiap sesi memakai UUID agar Stop yang terlambat tidak mematikan sesi baru. Permintaan start yang masih antre dibatalkan saat modal ditutup atau sebelum rekam. Perangkat dilepas sebelum recorder membukanya.

Preview preflight tidak membuat kamera muncul dalam proyek yang sudah direkam tanpa kamera. Area blur tetap memerlukan penempatan dan waktu manual; tidak mengikuti form yang bergerak otomatis.

## Cara memakai alur baru

1. Buka Projects untuk memilih rekaman. Setelah terbuka, tutup panel Projects untuk memperluas preview.
2. Pilih New recording, lalu layar/window, mikrofon, Computer audio dan webcam. Pilihan kamera Off berarti hasil baru tidak mempunyai video webcam.
3. Tekan Enable preview untuk memeriksa kamera yang dipilih. Jika gagal, periksa koneksi/izin Windows dan gunakan Retry preview. Pemilihan perangkat saja tidak menyalakan preview.
4. Tekan Start recording. Preview preflight dilepas lebih dahulu, lalu recorder membuka perangkat untuk rekaman. Hentikan melalui Stop recording atau shortcut yang ditampilkan aplikasi.
5. Periksa status sumber di atas preview proyek. Camera visible/hidden berkaitan dengan footage tersimpan; No camera recorded berarti tidak ada footage webcam pada proyek itu. Preview preflight bukan jendela kamera mengambang selama merekam.
6. Gunakan Fit untuk komposisi penuh atau 100%/200% untuk detail. Scroll/pan dan Center hanya mengubah tampilan editor. Crop, zoom dan mask tetap menggunakan koordinat rekaman.
7. Geser/trim layar dan zoom di timeline. Gunakan aksi Music, Subtitles atau Mask untuk menambahkan track; pilih item untuk membuka propertinya. Undo/redo dan input numerik tetap tersedia.
8. Buka Export MP4 dan periksa dimensi piksel, fps serta durasi trim sebelum menentukan output. Proyek lama tetap tersimpan terpisah dari hasil ekspor.

## Verifikasi dan batas

Installer [Screenrecord_0.3.0_x64-setup.exe](../target/release/bundle/nsis/Screenrecord_0.3.0_x64-setup.exe) selesai: 55.517.401 byte (52,95 MiB), unsigned. SHA-256: `5055b14ba5c3b0a58a3670ca3dc6cc1de5e00dbb6e64c70d1a0325059efc6e04`. Inspeksi ekstraksi memverifikasi EXE x64 terhadap build final, checksum 47 resource, source/lisensi limiter, dan eksekusi FFmpeg/FFprobe bundel. Installer dan aplikasi tidak dijalankan. Bukti: [installer-evidence-0.3.0.json](../artifacts/installer-evidence-0.3.0.json).

Verifikasi gabungan lulus 172 tes otomatis: 106 Rust/media, 27 unit frontend dan 39 browser headless. Format, Clippy dengan warnings denied, build/TypeScript dan audit npm produksi juga lulus; audit npm melaporkan nol kerentanan pada pemeriksaan ini. Log: [verification-0.3.0.log](../artifacts/verification-0.3.0.log).

| Area | Bukti |
| --- | --- |
| Preview kamera native | Parser JPEG berbatas, FFmpeg sintetis 4 fps, EOF, timeout, Stop dan Drop mematikan proses milik tes. Empat tes komponen memeriksa aktivasi eksplisit, decode JPEG, retry, respons terlambat, pergantian perangkat, disable, unmount dan startup tertunda. |
| Registrasi sesi | Stop lama tidak membatalkan sesi baru; start yang masih antre dibatalkan. Backend memvalidasi pilihan terhadap daftar kamera native dan melepas preview sebelum recorder. |
| Layout dan aksesibilitas | Kedua tema pada 900 x 640, 1280 x 720, 1600 x 900, 1920 x 1080 dan 2560 x 1440. Tidak ada overflow dokumen; alat tetap dapat dicapai. Preview 280 px pada 900 x 640 dan 656 px pada 1920 x 1080, dengan timeline masing-masing 200 dan 260 px. |
| Kontras terukur | Teks input 16,02:1 gelap / 14,04:1 terang; batas input 4,56:1 / 3,40:1; teks aksi amber 9,06:1. Angka hanya untuk pasangan yang dihitung, bukan klaim audit screen reader lengkap. |
| Preview detail | Lima tes geometri dan tiga browser mencakup DPI 2x, Fit/100%/200%, buffer maksimal 4096 sisi panjang/16 juta piksel, pan keyboard, koordinat dan piksel mask setelah crop/zoom/scroll. |
| Editor dan media | Regresi source/zoom drag+trim, undo, autosave tertunda, webcam EOF/offset, limiter/music seek, subtitle job ID, SRT, ekspor/cancel dan close/save tetap lulus. Media rekaman yang sudah ada didekode melalui loopback; sumbernya tidak diubah. |

Metrik: [layout-contrast.json](../artifacts/ui/layout-contrast.json). Screenshot memakai media uji sintetis dan fixture IPC, bukan bukti aplikasi native atau kamera fisik:

- [Editor gelap 1920 x 1080](../artifacts/ui/editor-1920x1080-dark.png)
- [Editor terang 1920 x 1080](../artifacts/ui/editor-1920x1080-light.png)
- [Persiapan rekaman dengan frame kamera simulasi](../artifacts/ui/record-setup-1600-dark.png)

Tes otomatis memakai fake IPC, media sintetis dan subprocess milik tes. Kamera fisik tidak diaktifkan. Tidak ada computer-use, panggilan cloud berbayar, instalasi, atau peluncuran aplikasi Windows pada pekerjaan ini. Uji manual pengguna tetap meliputi izin Windows, perangkat sedang digunakan aplikasi lain, pindah kamera, preview-stop-start recording, penutupan modal, kualitas audio/webcam, DPI, dan ekspor dari UI native.

Live preview mengambang selama merekam, meter level mikrofon preflight, serta deteksi otomatis kualitas gambar belum disediakan. Preview kamera tidak menguji kualitas audio. Kernel blur browser dan FFmpeg tetap berbeda; periksa hasil ekspor untuk rekaman sensitif dan gunakan solid mask bila isi harus tertutup penuh.
