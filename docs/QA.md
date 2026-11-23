# Bukti QA 0.1.1

Catatan 11 November 2026. Status kandidat rilis untuk pengujian. Tabel ini membedakan pengujian otomatis, uji media nyata, dan observasi native. Tidak ada klaim semua perangkat atau alur rilis telah lulus.

## Hasil yang tersedia

| Area | Bukti | Batas bukti |
| --- | --- | --- |
| Frontend build | `npm run build` lulus Vue/TypeScript dan Vite production build | Tidak menguji runtime WebView2 |
| Unit frontend | `npm test`: 8 lulus | Matematika edit: zoom/overlap, trim/crop, interpolasi/smoothing, gap capture, time label |
| UI headless | `npm run test:ui`: 11 lulus; 1 uji media opt-in dilewati bila env tidak diisi | IPC Tauri memakai fixture khusus tes; bukan backend native |
| Media asli pada browser | Uji opt-in: 1 lulus untuk H.264 `screen.mp4` dan float32 WAV 48 kHz stereo `system.wav` | Decode dan clock maju melalui HTTP loopback; tidak memvalidasi protokol aset Tauri |
| Rust aggregate | 33 lulus: 12 core, 13 capture, 8 render | Hardware/perangkat di luar mesin uji belum tercakup |
| Verifikasi lokal | `scripts/verify.ps1 -BuildInstaller`: format, Clippy, Rust tests, frontend build/tests, UI, audit npm, dan build NSIS lulus | Log lengkap: `artifacts/verification-0.1.1.log` |
| Capture nyata | Capture smoke dan pemulihan crash dilakukan di mesin ini | Matriks perangkat dan sinkronisasi fisik belum lengkap |
| Render nyata | Tes render mencakup ekspor FFmpeg dan pixel hasil decode | Hardware encoder pada seluruh GPU belum diuji |
| Ekspor rekaman nyata | MP4 3,000 detik, H.264 Main 1920 × 1080/30 fps/90 frame + AAC 48 kHz stereo, AMF; decode lengkap berhasil | Satu klip pendek, bukan benchmark lintas perangkat |
| Recovery media rusak | Fixture salinan dengan webcam/audio sistem rusak mempertahankan layar 12,071 detik dan mengeluarkan peringatan; decode lengkap lulus | Bukan simulasi disk penuh atau pencabutan perangkat |
| Computer audio 0.1.1 | Tone loopback 997 Hz pada -30 dBFS, 4,5276 detik; nol lubang nol searah atau multi-sampel; signal/residual 60,88 dB | Satu endpoint, durasi pendek; bukan pengukuran noise seluruh perangkat |
| Dependency frontend | `npm audit --omit=dev`: 0 kerentanan pada pemeriksaan ini | Bukan audit keamanan lengkap atau inventaris lisensi |
| Native awal | Rekam layar, Ctrl+Shift+R untuk stop, proyek tersimpan dan frame pertama terlihat | Snapshot segera setelah Play belum membuktikan playback berjalan; build terakhir belum diuji ulang native |
| Packaging 0.1.1 | NSIS x64, 54.921.263 byte; aplikasi cocok setelah marker bundel Tauri; 47 resource dan notice cocok checksum; FFmpeg/FFprobe hasil ekstraksi berjalan | Installer tidak dieksekusi; signature `NotSigned`; bukan uji install/uninstall |
| CI | Workflow Windows dan `scripts/verify.ps1` tersedia | Workflow remote belum dijalankan |

Installer: `target/release/bundle/nsis/Screenrecord_0.1.1_x64-setup.exe`, ukuran 52,38 MiB. SHA-256: `508ee593ce7ac8c8c5af8bca35a82e479c4a83d1bffa391d3d71b24b5ba0a9f7`. Bukti ekstraksi dan checksum ada di `artifacts/installer-evidence-0.1.1.json`. Tauri mengganti marker internal jenis bundel dari `UNK` ke `NSS`; pemeriksaan executable mengizinkan perubahan itu saja. Instruksi reproduksi ada di [README](../README.md).

## Capture dan render

Smoke capture mencakup layar 1080p dengan webcam, mikrofon, dan audio komputer; window 60 fps hingga dua segmen; decode keseluruhan hasil gabungan; serta penghentian paksa host dan pemulihan layar/kursor/webcam yang telah tersimpan. Pemeriksaan tidak menemukan proses FFmpeg milik uji yang tertinggal. Detail perilaku dan perintah ada di [capture README](../crates/capture/README.md).

Pengujian render memeriksa H.264, audio, trim/durasi, dimensi portrait/square, pixel webcam, perubahan area sumber karena zoom, gambar ulang kursor, pembatalan tanpa output parsial, dan perlindungan saat berkas tujuan dibuat oleh proses lain. Lihat [render README](../crates/render/README.md). Komposisi tetap menggunakan CPU, termasuk saat encoder hardware dipilih.

Uji integrasi memakai sumber `artifacts/capture-audio-final` dan menghasilkan `artifacts/integration-export/recorded-media-1080p.mp4`. Ekspor klip 3 detik selesai dalam 2,989 detik pada mesin ini memakai encoder AMF. Decode penuh keluar dengan kode 0, frame hasil diperiksa untuk zoom/kursor/webcam, dan hash sumber tidak berubah. Angka ini hanya pengamatan satu klip pendek; belum menjadi bukti konsumsi sumber daya atau kinerja pada rekaman panjang.

Audio sumber memakai WAV float32, dengan header RIFF untuk ukuran di bawah 4 GiB dan RF64 untuk rekaman lebih besar. Keberhasilan decode klip pendek tidak membuktikan perekaman >4 GiB atau drift jangka panjang sudah teruji.

## Frontend dan desain

Sebelas alur headless mencakup keadaan browser yang memerlukan desktop, pergantian tema, playback, perubahan edit dan save, export-cancel, pemilihan perangkat, stop setelah error perangkat, keyboard Tab, ukuran 900px, serta tutup aplikasi. Tes tutup meliputi proyek sudah tersimpan, save tertunda/gagal, retry, backend command tidak tersedia, request ganda, operasi rekam/ekspor aktif, timeout save tanpa tutup terlambat, dan larangan mengedit selama proses tutup. Media fixture berupa pola uji; aplikasi produksi tidak memuat proyek palsu.

Screenshot `artifacts/ui/editor-dark.png`, `editor-light.png`, dan `editor-narrow.png` ditinjau secara visual. Preview dan timeline memimpin layout, Segoe UI dipakai tanpa font unduhan, dan amber menandai tindakan/zoom terpilih. Tinjauan memeriksa tema, pemotongan teks, overflow, dan keterbacaan kontrol. Ini bukan audit aksesibilitas penuh.

| Pasangan warna yang dihitung | Rasio |
| --- | --- |
| Teks sekunder gelap `#b5bdca` pada `#1d2026` | 8,62:1 |
| Teks sekunder terang `#565e6c` pada `#f2f3f5` | 5,89:1 |
| Teks tombol `#201b11` pada amber `#e9b451` | 9,06:1 |
| Batas input gelap `#78818f` pada `#15171b` | 4,56:1 |
| Batas input terang `#7c8491` pada putih | 3,77:1 |

Rasio batas input dibandingkan dengan ambang non-text 3:1, bukan ambang teks normal 4,5:1. Tidak semua kombinasi hover/disabled/error telah diaudit. Review lingkup ini tidak menemukan dekorasi titik merek, aksen garis kiri panel, em dash, atau filler pemasaran pada UI yang dibuat. Tidak dilakukan uji screen reader atau Windows high contrast.

## Batas observasi native

Hotfix audio memakai posisi frame perangkat untuk mempertahankan sampel kontinu. Tes sintetis membuktikan tone identik bit demi bit dengan jitter QPC dan timestamp-error pada paket kontinu. Pada WAV yang dilaporkan pengguna terdapat 1.969 celah nol pendek; 1.656 di antaranya memiliki gelombang searah pada kedua tepi. Setelah perbaikan, uji loopback nyata tidak memiliki celah seperti itu. Sampel nol tunggal yang tersisa merupakan zero crossing sine yang normal. Bukti analisis ada di `artifacts/audio-noise-audit/audio-analysis.json` dan `artifacts/audio-hotfix/loopback-analysis.json`. Hash audio sumber tidak berubah.

Pada debug app awal, rekaman nyata berhasil dihentikan lewat Ctrl+Shift+R dan dimuat ke editor. Snapshot segera setelah Play masih menunjukkan waktu 0; tree aksesibilitas juga memuat teks error pada elemen media tersembunyi. Observasi ini belum memastikan apakah playback gagal atau belum sempat berjalan. Perbaikan frontend kemudian memisahkan kegagalan audio/webcam dari playback layar, menampilkan error per track, menghindari playback track nonaktif, dan memperbaiki race async playback. Media yang sama berhasil decode di Chrome headless; perilaku WebView2/protokol aset pada build terbaru masih perlu diperiksa.

Pengguna menghentikan kontrol komputer dengan Escape. Sesudah itu tidak ada interaksi native otomatis tambahan. Karena itu, playback, audio/webcam, export, dan close pada build terbaru masih memerlukan pemeriksaan native. Uji headless tidak menggantikan pemeriksaan tersebut.

## Gate yang masih terbuka

- Install/uninstall dan alur record-edit-export-reopen pada mesin Windows 11 bersih dengan WebView2.
- Native playback dan save/close setelah perubahan terakhir.
- Sinkronisasi fisik bibir/suara, drift durasi panjang, soak test, konsumsi CPU/RAM/disk, dan throughput ekspor.
- Webcam/mikrofon lain, GPU Intel/AMD/NVIDIA, DPI/scaling, multi-monitor, serta unplug/pergantian perangkat.
- Perilaku izin perangkat, disk penuh, dan path tujuan yang tidak dapat ditulis.
- Penandatanganan aplikasi/installer; inventaris serta kelengkapan lisensi dan source/build material untuk distribusi publik.

HDR tone mapping, bentuk kursor Windows selain pointer, dan auto updater belum diimplementasikan. Gate di atas tetap terbuka walaupun installer dapat dibangun.
