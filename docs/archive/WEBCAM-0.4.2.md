# Perbaikan webcam 0.4.2

> Arsip pengembangan. Lihat [unduhan publik](https://github.com/real-habibul/screenrecord/releases) untuk versi yang tersedia saat ini.

> Jalur `artifacts/` dan `target/` merujuk ke hasil lokal dari pengujian saat itu. Panduan untuk menjalankan pemeriksaan saat ini ada di [panduan kontribusi](../../CONTRIBUTING.md#memilih-pengujian).

Versi ini memperbaiki penghentian proses webcam dan timestamp untuk rekaman baru. Tiga agen menangani capture, audit render/media, dan audit lifecycle frontend; root menangani integrasi, kontrak, versi, serta paket. Pengujian menggunakan fixture sintetis dan browser headless. Rekaman pengguna tidak diubah dan perangkat capture tidak diaktifkan.

## Dasar diagnosis

Pada rekaman yang dilaporkan, layar dan audio berdurasi sekitar 36,266 detik. Webcam berisi 1.065 frame selama 35,500 detik; setelah offset awal 0,790257 detik, cakupannya pada timeline berakhir di 34,709743 detik. Log webcam mencatat `progress=end`, output selesai dimux, kemudian teardown driver kamera virtual. Kecepatan pengiriman yang dilaporkan sekitar 28,83 fps meskipun perangkat menyatakan 30 fps. Dua masalah ini terpisah: timestamp kamera memendekkan cakupan waktu, sedangkan batas keluar proses dua detik dapat memicu status recovery setelah media selesai ditulis.

## Kontrak perbaikan

- Webcam baru memakai timestamp graph DirectShow, bukan timestamp yang disediakan kamera. Opsi `use_video_device_timestamps=false` memilih `IReferenceClock_GetTime` pada FFmpeg yang dibundel. Pemetaan ke QPC tetap memakai pengamatan graph dan waktu penerimaan, dengan validasi clock yang sesuai. Tidak memakai perhitungan jumlah frame dibagi FPS sebagai pengganti waktu nyata.
- Penghentian membedakan proses yang selesai normal, proses yang perlu dihentikan setelah media selesai, dan proses yang belum menyelesaikan media. Selama frame, waktu output, atau ukuran output masih bertambah, proses boleh menyelesaikan flush hingga batas total 10 detik. Dua detik tanpa kemajuan atau dua detik setelah penanda akhir memicu penghentian proses yang belum keluar. Penghentian paksa hanya boleh dianggap selesai jika terdapat penanda akhir output, tidak ada error FFmpeg, proses telah direap, pembaca data selesai, serta media lulus validasi decode penuh. Kegagalan atau kepemilikan worker yang belum pasti tetap menghalangi finalisasi normal dan retry.
- Sesi tetap berada pada fase finalizing selama shutdown/validasi berlangsung. Hasil valid menjadi proyek ready. Error nyata tetap mengikuti jalur recovery. Keterlambatan teardown setelah media valid dicatat pada log tanpa salah menandai seluruh proyek sebagai interrupted.
- Validasi decode penuh dilakukan pada tahap finalisasi media, setelah cleanup perangkat. Rekaman panjang tidak ditempatkan di dalam batas tunggu cleanup native 15 detik. Proses validasi memakai pengawasan kemajuan yang sudah digunakan oleh pemrosesan media.
- File dan offset proyek lama tidak ditulis ulang. Pemutaran dan ekspor tetap memakai `cameraTime = screenTime + cameraOffsetSeconds` serta menyembunyikan kamera ketika media tidak tersedia.

Rujukan: [dokumentasi opsi DirectShow](https://ffmpeg.org/ffmpeg-devices.html#dshow), [implementasi timestamp pada FFmpeg yang dibundel](https://github.com/FFmpeg/FFmpeg/blob/a35157abb9/libavdevice/dshow_pin.c#L300-L333).

## Verifikasi dan paket

Kode backend final lulus 235 tes workspace Rust dan 14 tes media capture tambahan. Format Rust dan Clippy seluruh workspace/all-targets dengan `-D warnings` bersih. Bukti: `artifacts/webcam-0.4.2/workspace-final.log`, `artifacts/webcam-0.4.2/capture-media-final.log`, dan `artifacts/webcam-0.4.2/clippy-final.log`.

Regresi kamera menggunakan pipeline H264_MF yang dipakai aplikasi, input 28,8 fps dengan jeda 80 ms, serta penanda warna pada waktu yang diketahui. Hasilnya mempertahankan waktu kejadian; teardown yang sengaja ditahan diuji dengan child process milik tes. Output lengkap divalidasi dan dipublikasikan, sedangkan output rusak, penanda akhir yang hilang, dan error encoder tetap ditolak. Tes memeriksa keberadaan hasil decode penuh, bukan hanya metadata: `artifacts/webcam-0.4.2/camera-full-decode-proof.log`. Tes proses juga membuktikan kemajuan memperpanjang kesempatan flush, tanpa melewati batas total, dan repeated stats tidak dianggap kemajuan.

Tes render memeriksa PTS yang tidak teratur, offset 137 ms, cut pecahan detik, gap kamera, EOF, serta kesamaan audio hasil decode. Byte media sumber tetap sama. Audit silang memeriksa pemilihan clock, kepemilikan child/reader, lokasi validasi, status finalizing, dan jalur recovery. Tidak ditemukan blocker pada jalur yang diperiksa.

Frontend final lulus 108 tes browser dan 74 tes unit, termasuk penantian finalisasi lama, hasil ready/interrupted/error, Stop dari kontrol eksternal, penutupan jendela, dan EOF kamera dengan offset lama maupun timing baru. Pesan saat mencoba menutup jendela kini meminta pengguna menunggu finalisasi ketika Stop memang sedang berjalan. Tidak ada perubahan layout atau DTO. Bukti: `artifacts/webcam-0.4.2/frontend-browser-final.log`, `artifacts/webcam-0.4.2/frontend-unit.log`, dan `artifacts/webcam-0.4.2/frontend-final-focused.log`.

Total gate final: **431 tes berbeda lulus** (235 Rust + 14 media capture + 74 unit frontend + 108 browser). Build frontend/TypeScript dan audit npm produksi lulus, dengan 0 vulnerability dilaporkan. Satu run browser awal terganggu reload Vite saat perubahan teks; run tersebut disimpan sebagai bukti iterasi dan tidak dihitung lulus. Run final mengulang seluruh suite dengan kode dibekukan. Benchmark, fixture child yang dijalankan oleh tes induk, serta tes model/cloud yang tidak berubah tidak dihitung sebagai tes tambahan versi ini. Tidak ada uji computer-use atau aktivasi perangkat capture.

Installer Windows x64: `target/release/bundle/nsis/Screenrecord_0.4.2_x64-setup.exe`, 56.143.366 byte (53,54 MiB), belum ditandatangani. SHA-256: `55dc216ecbb504eb6a12dd90114a9089ad9db112e481eb4eb9fe500b5bd44d08`.

Inspeksi tanpa menjalankan installer membuktikan executable cocok dengan hasil release setelah marker NSIS Tauri, 47 resource cocok checksum, serta FFmpeg/FFprobe hasil ekstraksi berhasil menjalankan `-version`. Notice dan lisensi yang diperiksa cocok dengan sumber. Aplikasi tidak dipasang atau dijalankan. Bukti: `artifacts/installer-evidence-0.4.2.json`, log build (`artifacts/webcam-0.4.2/release-build.log`), dan checksum (`target/release/bundle/nsis/Screenrecord_0.4.2_x64-setup.exe.sha256`).

## Uji perangkat oleh pengguna

1. Rekam 60-90 detik dengan kamera virtual yang sebelumnya bermasalah, mikrofon, dan computer audio. Tepuk tangan pada awal dan akhir untuk memeriksa sinkronisasi; periksa bahwa webcam tetap tampil sampai ujung rekaman.
2. Tekan Stop dan tunggu penyimpanan. Proyek yang selesai normal harus berstatus Saved tanpa pemberitahuan interrupted/recovered. Mulai rekaman kedua untuk memeriksa pelepasan perangkat.
3. Ulangi memakai kamera fisik, lalu Pause/Resume beberapa kali. Periksa sambungan kamera/audio dan durasi setelah diekspor ke MP4.
4. Cabut atau hentikan sumber kamera saat merekam. Aplikasi harus melaporkan kegagalan yang nyata dan mempertahankan media yang tersedia, bukan menganggap sesi lengkap.

Kualitas driver, lip-sync fisik, dan perilaku hotplug tetap memerlukan uji manual. Installer belum ditandatangani dan tidak dipasang melalui pengujian otomatis.
