# Preview, Undo, dan ekspor WhatsApp 0.5.1

Versi ini mengurangi komposisi preview yang berulang dan pekerjaan Undo saat menggeser kontrol. Preset WhatsApp menghasilkan salinan MP4 kecil dari timeline yang sama. Proyek memakai skema versi 5.

## Cara ekspor untuk WhatsApp

1. Buka **Export MP4** dan pilih preset **WhatsApp**.
2. Periksa dimensi dan durasi yang ditampilkan. Preset mempertahankan rasio kanvas, memakai 30 fps, dan menyesuaikan resolusi hingga 720p berdasarkan durasi setelah cut.
3. Pilih **Browse** untuk menentukan lokasi, lalu **Export video**. Cek ukuran hasil dan gunakan **Show in folder** untuk menemukan MP4 yang akan dikirim lewat WhatsApp.

Batas preset adalah **16 MB** atau 16.000.000 byte. Aplikasi menghitung bitrate dari durasi yang tersisa, menyediakan ruang untuk audio dan overhead MP4, lalu memeriksa ukuran dan kelengkapan hasil sebelum menerbitkan file. Jika hasil terlalu besar, satu percobaan ulang boleh menurunkan bitrate video. Jika durasi atau hasil melampaui anggaran, pendekkan edit atau pilih **Standard**. Gunakan nama baru untuk melindungi file tujuan yang sudah ada.

MP4 memakai H.264, dengan AAC jika audio disertakan. Batas 16 MB adalah pilihan untuk menjaga file kecil. Panduan WhatsApp Windows menyebut batas video bawaan yang berbeda menurut kondisi koneksi. [Panduan WhatsApp](https://faq.whatsapp.com/453914586839706/?cms_platform=windows-desktop&locale=en_US)

Pada klip sintetis 2,4 detik yang sama, Standard menghasilkan 2,39 MB dan preset WhatsApp menghasilkan 0,88 MB pada 1280 × 720, 30 fps, dengan AAC stereo. Tes juga mendekode MP4 portrait dan persegi. Uji dengan batas yang sengaja diperkecil berhasil menurunkan hasil dari 866.097 ke 778.039 byte, dengan hasil decode audio yang identik. Uji pembatalan, benturan nama file, dan kegagalan memenuhi batas memastikan file tujuan terlindungi. [Bukti tes ekspor](../artifacts/perf-0.5.1/render-full-final.log)

## Perbandingan dan audit

Pengukuran memakai snapshot 0.5.0 dan kandidat baru, fixture video sintetis yang sama, Chromium headless, serta IPC tiruan. Seluruh 68 berkas yang dapat dibandingkan dari snapshot awal memiliki SHA-256 yang sama dengan catatan rilis 0.5.0. Komposisi canvas, serialisasi, respons seek, waktu tugas main thread, dan heap JavaScript diukur dari browser. Penggunaan RAM total aplikasi Windows, biaya GPU, dan kelancaran perekaman perangkat perlu pengukuran tersendiri. [Manifest baseline](../artifacts/perf-0.5.1/baseline-manifest.json)

### Preview

Audit menemukan bahwa gambar video bisa maju ke bagian yang dipotong sebelum jam video dan callback menyusul. Pada batas cut, preview memeriksa timestamp pada salinan frame yang langsung digambar lalu dilepas. Jika runtime tidak bisa membuat salinan tersebut, preview dapat menampilkan latar sesaat sampai seek terverifikasi. Tes cut dengan seek sengaja dilambatkan tidak menemukan piksel merah atau sampel audio dari bagian terhapus, dan memastikan frame serta audio yang dipertahankan tetap tampil. Cakupan juga meliputi mask aktif, interval kamera, reframe, dan latar blur. Tes CORS awal perlu koreksi karena Playwright otomatis menambahkan header izin ketika header itu kosong. Uji origin yang diizinkan dan ditolak kemudian lulus. [Bukti piksel dan audio](../artifacts/perf-0.5.1/stage1-cut-final.json), [tes CORS](../artifacts/perf-0.5.1/stage1-cors-final.log)

Tes penuh pertama menemukan dua kegagalan yang berbeda. Preview kadang menggambar frame lama sesudah seek. Permintaan seek ulang dengan waktu yang sama di dekat akhir klip webcam juga membuat Chromium melompat ke akhir file. Frame jeda yang sudah diverifikasi kini dipakai kembali saat sumber dan waktunya cocok. Preview memeriksa kesiapan decode, lalu memakai callback frame atau bukti langsung dari timestamp dan durasi frame. Tes khusus memicu callback yang membawa frame lama untuk seek baru. Setelah perbaikan, preview menunggu frame yang sesuai sebelum menggambar. Sebanyak 24 pengulangan tes stres lulus, termasuk CFR, VFR, akhir klip webcam, serta pemeriksaan piksel dan PCM pada cut. [Hasil awal](../artifacts/perf-0.5.1/frontend-browser-paused-seek-failure.log), [bukti callback lama](../artifacts/perf-0.5.1/new-seek-stale-frame-failure-evidence.json), [regresi akhir](../artifacts/perf-0.5.1/paused-seek-stress-v4-final.log), [bukti piksel dan PCM final](../artifacts/perf-0.5.1/cut-privacy-final-v4.json)

### Riwayat edit

Editor menandai perubahan selama drag dan membuat snapshot Undo saat gesture selesai. Preview tetap merespons tiap input. Autosave menunggu commit, dan Undo/Redo menunggu pelepasan handle. Audit menambahkan regresi untuk drag 60 langkah dengan 1.000 subtitle, Escape, rollback timeline saat tutup, respons simpan tertunda, status bersih sesudah Undo ke keadaan tersimpan, dan pergantian proyek. Lima tes khusus lulus setelah locator nama proyek diperbaiki. [Bukti tes riwayat edit](../artifacts/perf-0.5.1/stage2-focused-final.log)

### Dialog ekspor

Delapan tes terarah meliputi pengaturan Standard, dimensi dan ukuran aktual WhatsApp, pemilih file, penutupan dialog, percobaan ulang ke path yang sama, status tertunda, pembatalan, dan penguncian hapus proyek. Status dari percobaan sebelumnya diabaikan. Tombol Cancel kembali aktif setelah backend menerima ekspor, termasuk saat pembacaan status pertama masih tertunda. Tampilan dialog diperiksa pada 900 × 640 di kedua tema. [Log tes dialog](../artifacts/perf-0.5.1/stage3-eight-final.log), [tema gelap](../artifacts/perf-0.5.1/whatsapp-dark-900.png), [tema terang](../artifacts/perf-0.5.1/whatsapp-light-900.png)

### Tolok ukur akhir

Pengukuran membandingkan build produksi frontend 0.5.0 dan 0.5.1 dalam tiga putaran dengan urutan versi bergantian. Skenario playback berjalan sekitar enam detik per putaran. Fixture 1080p60 memakai kanvas portrait, reframe, blur, mask, cut, kursor, dan webcam sintetis 30 fps. Fixture 1080p30 memakai kanvas landscape dan batas 60 Hz untuk callback animasi. Gestur terdiri dari 60 langkah slider pada proyek dengan 1.000 subtitle.

| Ukuran | 0.5.0 | 0.5.1 |
| --- | ---: | ---: |
| Komposisi preview 1080p60 | 864 | 411 |
| Frame video tampil pada 1080p60 | 361 | 360 |
| Waktu main thread playback 1080p60 | 1.264,3 ms | 956,8 ms |
| Komposisi preview 1080p30 | 364 | 199 |
| Frame video tampil pada 1080p30 | 180 | 180 |
| Waktu main thread playback 1080p30 | 711,2 ms | 635,4 ms |
| Serialisasi penuh selama drag dan siklus simpan | 131 | 5 |
| Volume serialisasi UTF-16 | 78,56 MB | 3,00 MB |
| Durasi drag 60 langkah | 1,95 detik | 1,06 detik |
| Latensi seek P50 / P95 | 41,7 / 57,6 ms | 44,7 / 60,8 ms |

Angka selain persentil seek adalah median tiga putaran. Persentil seek dihitung dari 18 tindakan per versi dan mencakup perpindahan melintasi cut. Jumlah frame terlewat pada fixture 60 fps tercatat 1, 1, 1 sebelum perubahan dan 0, 1, 1 sesudahnya. Ketiga putaran 30 fps mencatat nol. Heap JavaScript di akhir drag turun dari 52,50 MB ke 39,90 MB pada sampel median, sementara sampel akhir playback sedikit naik. Snapshot heap dipengaruhi waktu pengumpulan sampah. [Data 24 sampel pengukuran](../artifacts/perf-0.5.1/final-comparison.json)

### Tes otomatis

Sebanyak 614 tes berbeda lulus. Jumlah ini mencakup 311 tes Rust dan media, 147 tes unit frontend, serta 156 tes browser. Tes browser termasuk decode H.264 1080p60 dan audio PCM float sintetis. Pemeriksaan format, Clippy, TypeScript, build frontend, dan audit dependency npm produksi juga lulus. [Audit independen](../artifacts/perf-0.5.1/INDEPENDENT-AUDIT.md), [tes browser](../artifacts/perf-0.5.1/frontend-browser-final-v4.log), [tes unit frontend](../artifacts/perf-0.5.1/frontend-unit-final-v4.log)

### Installer

Installer NSIS Windows x64 0.5.1 sebesar 53,59 MiB selesai dibangun dan belum ditandatangani. Inspeksi mengonfirmasi versi executable, arsitektur x64, dan checksum 47 resource. FFmpeg dan FFprobe hasil ekstraksi dapat dijalankan. Sebanyak 287 checksum sumber dan resource cocok sesudah build. Seluruh 79 berkas snapshot benchmark cocok dengan sumber sebelum build. [Installer](../target/release/bundle/nsis/Screenrecord_0.5.1_x64-setup.exe), [SHA-256](../target/release/bundle/nsis/Screenrecord_0.5.1_x64-setup.exe.sha256), [bukti verifikasi](../artifacts/perf-0.5.1/verification-evidence.json), [inspeksi paket](../artifacts/installer-evidence-0.5.1.json)

## Uji manual

- Uji pemasangan dan penghapusan di Windows serta buka proyek di WebView2 native.
- Putar dan seek proyek 30 fps serta 60 fps dengan crop, blur, kamera, mask, keyframe, dan cut. Perhatikan respons preview, perpindahan cut, dan sinkronisasi audio.
- Geser slider dan handle timeline, lalu tekan Undo dan Redo. Periksa satu langkah per drag dan perubahan yang kembali setelah proyek dibuka ulang.
- Ekspor klip pendek landscape dan portrait dengan preset WhatsApp. Periksa gambar, subtitle, suara, ukuran akhir, lalu coba kirim sebagai video di WhatsApp.
- Uji klip panjang. Dialog seharusnya meminta trim atau Standard bila anggaran bitrate minimum tidak mencukupi.
