# Arsip checklist rilis

Checklist ini mencatat status pada waktu masing-masing versi diuji. Pekerjaan aktif dicatat di [GitHub Issues](https://github.com/real-habibul/screenrecord/issues). Untuk 0.7.4 gunakan [laporan keandalan](../RELIABILITY-0.7.4.md) dan [checklist perangkat](../CHECKLIST-RILIS-0.7.4.md).

## Keandalan rekaman 0.7.4

- [x] Tambahkan regresi untuk frame pertama, antrean encoder yang berhenti, waktu editor setelah satu jam dan kegagalan tulis karena ruang habis. [Hasil uji](../RELIABILITY-0.7.4.md)
- [ ] Pengguna mengonfirmasi uji manual 0.7.4 berhasil pada perangkatnya. Lengkapi catatan skenario yang sudah diuji, termasuk pemasangan di Windows 11 bersih, startup pada monitor yang pernah mengalami timeout, rekaman panjang, dan pemulihan. [Checklist manual](../CHECKLIST-RILIS-0.7.4.md)
- [ ] Verifikasi penandatanganan dan materi lisensi sebelum distribusi publik. [Status distribusi](../../THIRD_PARTY_NOTICES.md)

## Animasi klik 0.5.2

- [x] Uji klik singkat, tombol bersamaan, tombol ditahan saat mulai atau resume, pemulihan metadata, serta klik pada awal frame pertama. Rekaman lama yang hanya menyimpan status ditekan menghasilkan indikator netral.
- [x] Uji preview dan periksa piksel MP4 Standard 30/60 fps dan WhatsApp. Uji pusat lingkaran dalam beberapa fase animasi, bidang crop/zoom, batas cut, serta opsi menyembunyikan indikator.
- [x] Verifikasi 654 tes otomatis, format Rust, Clippy, TypeScript, dan build frontend. Periksa hasil ekspor contoh 4,6 detik dari proyek sintetis melalui pemuat proyek aplikasi.
- [x] Periksa installer NSIS Windows x64 tanpa menjalankannya. EXE di dalam paket menampilkan versi 0.5.2, checksum 47 resource cocok, dan 234 input build terverifikasi. Installer belum ditandatangani.
- [ ] Uji pemasangan serta klik kiri dan kanan dengan mouse atau touchpad di perangkat pengguna.

## Preview, riwayat edit, dan WhatsApp 0.5.1

- [x] Audit preview sesuai frame, dengan 137 unit dan 25 kasus browser terarah termasuk cut, mask, kamera, audio, dan fallback. Pengukuran awal 1080p60 memakai 204 komposisi dibandingkan 430 pada 0.5.0.
- [x] Audit transaksi riwayat edit, Undo/Redo, dan autosave. Lima regresi baru dan 15 kasus terarah lulus. Pengukuran awal drag 60 langkah turun dari 131 ke 5 serialisasi penuh.
- [x] Bandingkan performa 0.5.0 dan 0.5.1 pada fixture yang sama dalam tiga putaran. Median komposisi 1080p60 turun 864 ke 411 dan durasi drag 60 langkah turun 1,95 ke 1,06 detik.
- [x] Verifikasi ekspor WhatsApp, batas file, decode, pembatalan, dan perlindungan file tujuan. Delapan regresi dialog dan 80 tes renderer lulus.
- [x] Verifikasi 614 tes otomatis, format, Clippy, TypeScript, build frontend, dan audit npm produksi. Installer 0.5.1 x64 unsigned dibangun dan diinspeksi. Versi dan 47 resource cocok, serta 287 checksum sumber dan resource diverifikasi setelah build.
- [ ] Uji manual WebView2, perangkat rekam, pemasangan, dan kirim MP4 di WhatsApp.

## Editor vertikal 0.5.0

- [x] Tahap 1: Fit/Fill, migrasi proyek/preset, 17 kasus geometri independen, 48 tes core, 60 render +5 integrasi, 97 unit UI serta 12 regresi browser lulus; audit tidak menemukan blocker.
- [x] Tahap 2: 34 kasus geometri independen, 52 core, 62 render +5 integrasi, 118 unit UI dan 16 regresi browser lulus. Audit waktu sumber, invalidasi preview, preset dan penyimpanan tidak menemukan blocker.
- [x] Tahap 3: 8 kasus background independen, privasi piksel/tile, batas frame kamera dan warna encoder lulus audit; core 53, render 70 +5 integrasi, unit UI 130 dan browser penuh 133 lulus.
- [x] Verifikasi akhir 562 tes berbeda, format/Clippy/TypeScript/build/audit npm produksi lulus. Installer unsigned 0.5.0 x64 dibangun dan diinspeksi: versi/EXE cocok, 47 resource cocok checksum, 179 hash sumber terpilih tetap cocok; aplikasi/installer tidak dijalankan.
- [ ] Uji perangkat dan native oleh pengguna secara manual.

## Mode Apps 0.4.4

- [x] Tambahkan grup aplikasi berjalan, metadata query terbatas dan validasi PID/HWND pada startup/resume, termasuk preflight sebelum folder proyek dibuat.
- [x] Tambahkan Apps picker, pilihan jendela, refresh yang mempertahankan identitas dan keterangan audio komputer. Enter pada pencarian tidak memulai rekaman.
- [x] Verifikasi 476 tes berbeda, format/Clippy/TypeScript/build/audit npm produksi dan audit silang. Installer unsigned 0.4.4 dibangun dan diinspeksi: x64 versi 0.4.4, 47 resource cocok checksum. Satu kegagalan awal tes webcam sintetis belum terjelaskan; pengulangan terpisah/paralel/serial lulus dan buktinya tetap dicatat.
- [ ] Uji Roblox/perangkat, beberapa jendela, Pause/Resume dan instalasi secara manual.

Riset dan batas fungsi tersedia pada [mode Apps 0.4.4](APPS-0.4.4.md).

## Audit rekaman 0.4.3

- [x] Perbaiki rollback native, message queue worker free-threaded, serta apartment WinRT cleanup/static-frame/encoder.
- [x] Perbaiki terminal event sebelum ACK, pembukaan proyek lama, respons status usang, Stop tanpa sesi, dan retry startup UI.
- [x] Simpan diagnostik native dan error fase/build/identitas secara lokal tanpa pixel/audio.
- [x] Gate agregat: 455 tes berbeda lulus, format/Clippy/TypeScript/build/audit npm produksi bersih. Installer unsigned 0.4.3 dibangun dan diinspeksi tanpa memasang/menjalankan aplikasi; x64 versi 0.4.3, 47 resource cocok checksum, engine/probe hasil ekstraksi berjalan.
- [ ] Uji ulang monitor yang gagal pada perangkat pengguna; penyebab persis timeout lama belum terbukti.

Rincian ada di [audit 0.4.3](AUDIT-0.4.3.md).

## Webcam 0.4.2

- [x] Timestamp graph DirectShow untuk webcam baru, dengan validasi pemetaan ke clock layar.
- [x] Shutdown membedakan video selesai dengan teardown lambat dari output yang belum lengkap.
- [x] Regresi sintetis timestamp, cleanup, finalizing UI, offset, ekspor, dan error yang nyata; 431 tes final lulus, format/Clippy/build frontend/audit npm produksi bersih.
- [x] Build dan inspeksi installer Windows x64 0.4.2: 47 resource cocok checksum, engine/probe berjalan, tanpa memasang atau menjalankan aplikasi. Paket unsigned.
- [ ] Uji perangkat oleh pengguna: kamera virtual/fisik, Stop lalu rekam ulang, Pause/Resume, dan lip-sync awal/akhir.

Rincian tersedia di [perbaikan webcam 0.4.2](WEBCAM-0.4.2.md).

## Audit 0.4.1

- [x] Perbaiki callback awal capture dan diagnosis kegagalan per tahap.
- [x] Arsipkan penanda percobaan tanpa frame secara konservatif, pertahankan media kamera/audio dan data recovery yang ambigu.
- [x] Pastikan tindakan dialog rekam tetap terlihat; pisahkan error dan pulihkan retry preview kamera.
- [x] Tolak ekspor dengan audio tidak lengkap, format tidak sesuai, atau audio yang seharusnya tidak ada.
- [x] Tutup regresi otomatis dan audit silang: 425 tes berbeda lulus, format/Clippy/build/audit npm produksi bersih. Installer unsigned 0.4.1 dibangun dan diinspeksi: x64, 47 resource cocok checksum, engine/probe hasil ekstraksi berjalan tanpa memasang/menjalankan aplikasi.
- [ ] Uji manual monitor/window, perangkat audio/kamera, GPU, WebView2, serta instalasi bersih dan signing.

Rincian diagnosis dan batas bukti ada di [audit 0.4.1](AUDIT-0.4.1.md).

## Upgrade 0.4.0

- [x] Split/cut/ripple delete, EDL sumber dan gate privasi video/audio/caption.
- [x] Waveform, preflight meter, normalisasi RMS/peak dan ducking musik.
- [x] Countdown, preview/kontrol mengambang dan Pause/Resume sesi native.
- [x] Preset tampilan kustom, bubble/fullscreen/split dan proteksi frame kamera kosong.
- [x] Subtitle bertimestamp kata, split/merge/SRT, model lokal terverifikasi dan cloud opt-in.
- [x] Tutup gate tracking mask dengan koreksi manual setelah regresi dan audit silang final.
- [x] Bangun dan inspeksi installer unsigned 0.4.0 dari source final; x64, 47 resource cocok checksum dan FFmpeg/FFprobe hasil ekstraksi berjalan. Paket tidak dipasang/dijalankan.
- [ ] Uji perangkat/WebView2, izin audio/kamera, Pause/Resume, multi-DPI, hasil audio/lip-sync, akurasi transkripsi dan tracking pada rekaman nyata.

Rincian tiap tahap dan batas cakupan tersedia di [laporan 0.4.0](UPGRADES-0.4.0.md). Tes pengembangan tahap ini tidak menggunakan computer-use atau perangkat fisik.

## Project actions dan canvas 0.3.1

- [x] Klik kanan/keyboard/menu baris proyek, Delete dengan konfirmasi, dan penanganan cancel/error.
- [x] Backend Recycle Bin yang memvalidasi direktori/identitas, menolak operasi aktif, dan menjaga file di luar proyek.
- [x] Preview release/unmount sebelum delete, edit tertunda di-flush, dan library request lama diabaikan.
- [x] Geser/resize mask dan kamera, undo serta pembatalan gesture diverifikasi dalam integrasi akhir.
- [x] Verifikasi final: 190 tes berbeda (109 Rust/media, 32 unit frontend, 49 browser), format/Clippy/build/audit npm produksi lulus. Callback recycle diperketat, tes aplikasi dan build dijalankan ulang; installer 0.3.1 diverifikasi tanpa dijalankan.
- [ ] Uji manual Windows Recycle Bin serta interaksi canvas native. Tes tidak menghapus proyek pengguna atau mengendalikan aplikasi melalui computer-use.

## Audit tampilan dan alur 0.3.0

- [x] Riset tampilan/alur Screen Studio, Cap dan Tella, serta audit screenshot dan source. [Laporan audit](UI-AUDIT-0.3.0.md).
- [x] Preview kamera preflight native: aktivasi eksplisit, frame berbatas, timeout/retry, session ID, cleanup sebelum rekam/tutup.
- [x] Toolbar ringkas, library collapsible, properti dengan tool rail, source-presence dan track kosong yang ringkas.
- [x] Trim layar dan move/trim zoom, preview Fit/100%/200% dengan pan/scroll, serta ringkasan ekspor.
- [x] Verifikasi final: 172 tes otomatis (106 Rust/media, 27 unit frontend, 39 browser), format/Clippy/build/audit npm produksi lulus. Screenshot viewport dan kontras diperiksa. Installer unsigned 0.3.0 dibangun dan isi paket diverifikasi tanpa menjalankannya.
- [ ] Uji manual kamera fisik, izin Windows, hotplug/device busy, perpindahan preview-ke-record, dan WebView2 native. Tidak menggunakan computer-use.

## Editor 0.2.0

- [x] Track musik: impor/normalisasi lokal, posisi, source trim, volume, fade, preview dan ekspor bersama limiter.
- [x] Subtitle: cue manual dan cloud opt-in Deepgram, koreksi teks/waktu/gaya, preview libass, burn-in dan SRT.
- [x] Area blur/blok solid dengan waktu aktif dan koordinat konten sebelum crop/zoom.
- [x] Penyimpanan v3, migrasi baca v1/v2, path musik relatif, dan penolakan akses di luar proyek.
- [x] Pembatalan subtitle memakai job ID; hasil lama tidak diterapkan pada job baru. Key hanya untuk sesi pekerjaan.
- [x] Verifikasi akhir: 150 tes otomatis (102 Rust/media, 20 unit frontend, 28 browser), format, Clippy, build/TypeScript dan audit npm produksi lulus. Installer unsigned 0.2.0 dibangun dan isi paket diverifikasi tanpa menjalankannya.
- [ ] Uji manual pengguna: UI Windows, audio nyata, drag/trim/undo, cloud dengan API key, blur pada frame batas, MP4/SRT, dan close/save. Lihat [panduan editor](EDITOR-0.2.0.md).

Tidak ada computer-use pada pekerjaan editor 0.2.0. Uji otomatis memakai media sintetis, browser headless dan mock HTTP; tidak ada panggilan cloud berbayar atau perekaman perangkat baru.

Status 0.1.2: kandidat pengujian Windows 11 x64. Tanda centang menunjukkan implementasi dan bukti yang disebutkan, bukan sertifikasi produksi. Perbaikan terbaru dan batasnya ada di [laporan audit 0.1.2](AUDIT-FIXES-0.1.2.md); bukti versi sebelumnya tetap tersedia di [QA 0.1.1](QA.md).

## Perbaikan audit 0.1.2

- [x] P1: cegah kehilangan edit, hentikan sumber sebelum finalisasi, pisahkan metadata kursor, dan batasi pemrosesan gerakan ekspor.
- [x] P2: perbaiki recovery, webcam remux/offset, audio drain/queue, resize kursor, trim 100 ms, keyboard, EOF webcam, path proyek, watchdog, limiter, serta fallback encoder.
- [x] Optimasi: autosave edit saja, summary library, cursor tanpa proxy Vue, cache resource GPU, dan antrean encoder berbatas.
- [x] Tes terarah mencakup fixture metadata delapan jam, media sintetis, PCM limiter, konkurensi, kegagalan worker, dan ekspor dari salinan rekaman asli.
- [x] Verifikasi akhir: 104 tes otomatis berbeda, format, Clippy, build, dan audit npm produksi lulus. Installer unsigned 0.1.2 dibangun ulang dari source final dan isi paket diverifikasi; bukti ada di laporan audit.
- [x] Smoke test EXE 0.1.2 hasil ekstraksi: startup, membuka proyek tersimpan, frame layar/webcam, playback maju, dan pause. Tidak mengaktifkan perangkat capture atau menyimpan edit.
- [ ] Selesaikan uji perangkat baru untuk clock webcam, WASAPI drain, resize/DPI, GPU cache, dan antrean native encoder.
- [ ] Selesaikan kualitas audio/sinkronisasi, ekspor dan save/close pada WebView2 native, clean install/uninstall, signing, serta kelengkapan material distribusi sebelum klaim siap produksi. Percobaan tombol X 0.1.2 dihentikan tool karena input pengguna terdeteksi.

## Perbaikan laporan pengguna 0.1.1

- [x] Hapus label `Stored on this computer` dan verifikasi ketidakhadirannya pada UI.
- [x] Tangani tutup sejak awal startup, simpan edit dahulu, cegah permintaan ganda, tampilkan error, dan kunci editor saat menunggu save.
- [x] Cegah permintaan tutup yang kedaluwarsa setelah save terlambat; backend segera menolak tutup ketika operasi media aktif.
- [x] Temukan sumber noise Computer audio pada celah nol batas paket; ganti clock paket kontinu dengan posisi sampel WASAPI.
- [x] Tes regresi tone dengan jitter QPC menghasilkan sampel identik; loopback nyata 4,53 detik tidak lagi memiliki lubang nol searah. File asli tetap utuh.
- [ ] Verifikasi tombol X pada proses native 0.1.1 setelah proses debug lama ditutup. Kontrol aplikasi masih dihentikan lewat Escape.

## Capture

- [x] Enumerasi monitor, window, mikrofon, dan webcam yang tersedia.
- [x] Capture 30/60 fps tanpa pointer bawaan, dengan metadata posisi/tombol/visibilitas kursor.
- [x] Mikrofon dan audio komputer WASAPI, webcam FFmpeg, serta laporan kegagalan perangkat.
- [x] Stop/finalisasi, jurnal segmen, pemulihan setelah proses dihentikan, dan cleanup proses media milik aplikasi.
- [x] Uji mesin ini: layar 1080p dengan webcam/mikrofon/audio komputer; window 60 fps dua segmen; decode hasil; pemulihan setelah proses dihentikan.
- [ ] Ukur lip-sync fisik, drift durasi panjang, dan dampak penggunaan CPU/memori pada capture.
- [ ] Lengkapi matriks perangkat, DPI, multi-monitor, serta perubahan/unplug perangkat saat merekam.

## Render

- [x] Zoom otomatis/manual dengan transisi halus; pusat tetap atau mengikuti kursor.
- [x] Cursor redraw/smoothing, trim/crop, aspect, warna latar, padding, dan sudut membulat.
- [x] Overlay webcam, bentuk/posisi/mirror, serta mixing volume audio.
- [x] Ekspor MP4 H.264, progress, pembatalan, probe encoder hardware, dan fallback software.
- [x] Publikasi output atomik tanpa menimpa berkas; pengujian media decoded-pixel, dimensi, audio, trim, pembatalan, dan benturan nama.
- [x] Ekspor rekaman nyata 3 detik: H.264 1920 × 1080/30 fps + AAC, encoder AMF, decode lengkap, pemeriksaan frame zoom/kursor/webcam, hash sumber tetap.
- [ ] Ukur throughput/CPU/memori ekspor pada 1080p/1440p/2160p dan matriks GPU berbeda.

## Editor

- [x] Pemilihan sumber/perangkat, webcam, serta status perekaman dan error yang dapat ditindaklanjuti.
- [x] Daftar proyek tersimpan, preview, timeline, dan tambah/ubah/hapus zoom.
- [x] Kontrol crop, kanvas, kursor, webcam, volume, dan trim.
- [x] Autosave, ekspor, pembatalan, dan buka lokasi hasil; backend menolak input yang tidak valid.
- [x] Jalur keyboard yang diuji, layout desktop 900px, dan review screenshot tema gelap/terang.
- [x] Uji headless untuk edit/save/export-cancel, perangkat/error/stop, serta save-before-close dan block-close saat merekam.
- [x] H.264 + float32 WAV hasil rekaman asli dapat decode dan maju di Chromium melalui HTTP loopback.
- [ ] Lengkapi validasi native WebView2 setelah perbaikan terakhir: kualitas audio/lip-sync, ekspor, dan save/close. Pembukaan proyek, gambar webcam, playback maju, dan pause sudah diperiksa pada build 0.1.2.
- [ ] Audit aksesibilitas dengan screen reader dan Windows high contrast; uji DPI/scaling tambahan.

## Integrasi dan rilis

- [x] Workspace Rust/Tauri, DTO camelCase, validasi IPC, dan frontend terintegrasi.
- [x] Penyimpanan proyek atomik, validasi path, dan integrasi pemulihan saat startup.
- [x] Pemulihan fixture dengan webcam/audio sistem rusak mempertahankan layar 12,071 detik, menampilkan peringatan, dan lolos decode.
- [x] Satu instance, logging, stop shortcut bila berhasil didaftarkan, dan penjagaan shutdown saat pekerjaan aktif.
- [x] Lockfile dependency serta bundling FFmpeg/FFprobe/DLL dengan hash arsip yang diverifikasi.
- [x] Build installer NSIS 0.1.1 x64 berhasil; 47 resource beserta notice cocok checksum dan FFmpeg/FFprobe hasil ekstraksi berjalan.
- [x] Uji native awal: mulai rekam, stop dengan Ctrl+Shift+R, proyek tersimpan, frame pertama tampil.
- [x] Verifikasi lokal 0.1.1 sebelum bundling: 33 tes Rust, 8 unit frontend, 11 UI headless lulus; uji media asli opt-in lulus terpisah. Format, Clippy, build, dan audit npm produksi juga lulus.
- [x] Skrip verifikasi lokal dan workflow CI Windows tersedia. CI remote belum dijalankan.
- [x] Dokumentasi fitur, cara build/pakai, bukti QA, batas produk, dan status lisensi pihak ketiga.
- [x] Rebuild installer dari perubahan final; hash dan ukuran 54.921.263 byte tercatat di QA dan `artifacts/installer-evidence-0.1.1.json`.
- [ ] Install, launch, record, export, relaunch, dan uninstall pada mesin Windows 11 bersih.
- [ ] Uji recovery saat disk penuh/izin ditolak serta soak test durasi panjang.
- [ ] Ukur penggunaan sumber daya. Jangan mengklaim ringan/cepat hanya berdasarkan pilihan Rust/Tauri.
- [ ] Lengkapi inventaris lisensi transitif, teks lisensi, source/build material FFmpeg yang sesuai, dan tinjau paket distribusi publik.
- [ ] Tentukan lisensi aplikasi dan sediakan teks lisensinya; metadata Cargo saat ini menyebut MIT.
- [ ] Tandatangani aplikasi/installer dan verifikasi hasil penandatanganan.
- [ ] Selesaikan bukti clean-machine/hardware/native di atas sebelum menyatakan rilis publik siap produksi.

Updater otomatis belum diimplementasikan. Status ini tidak menghalangi pengujian lokal melalui paket manual, tetapi tidak boleh dipresentasikan sebagai fitur yang tersedia.
