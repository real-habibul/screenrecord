# Perbaikan audit 0.1.2

11 November 2026. Kandidat pengujian Windows 11 x64. Perubahan mengatasi P1, P2, dan optimasi yang terkait; tidak menambah fitur kreator. Pengujian otomatis tidak menggantikan validasi perangkat atau bukti rilis publik.

## P1

| Temuan | Perubahan | Bukti terarah |
| --- | --- | --- |
| Edit hilang saat recording/Auto zoom | Workspace terkunci selama operasi; save selesai sebelum proyek diganti; notifikasi Stop terlambat tidak menimpa edit lokal | UI dengan save/regenerate tertunda, save gagal, dan notifikasi terlambat |
| Kamera menunggu encoder layar saat Stop | Semua sumber menerima Stop dahulu; kamera dihentikan sebelum join native; batas tunggu cleanup 15 detik dan karantina writer | Proses kamera simulasi sudah keluar ketika finalisasi layar sengaja ditahan; recovery/start tetap ditolak selama writer aktif |
| Metadata panjang tersimpan tetapi gagal dibuka | Format proyek v2 memakai manifest kecil dan sidecar kursor immutable dengan checksum; batas save/load sama | Round-trip 3,6 juta sampel, setara delapan jam pada 125 Hz, bit-per-bit; legacy migration; korupsi sidecar; open/save bersamaan |
| Penjadwalan gerakan membesar kuadratik | Ekspor video dibagi maksimum 900 frame; daftar perintah dipakai ulang; audio dienkode sekali setelah video digabung | Perbandingan 12 bagian/143 frame, PTS, audio decode identik, pixel, pembatalan, dan benchmark |

Cleanup native yang tidak dapat dibatalkan tetap dapat hidup sampai proses aplikasi keluar. Karantina menjaga agar recovery tidak menyentuh media yang masih ditulis. Ini bukan klaim driver yang macet sudah dipaksa berhenti dengan aman.

## P2 dan optimasi

| Area | Perubahan dan pengujian |
| --- | --- |
| Webcam | Finalisasi menyalin stream ke `camera-source.mp4` tanpa encode ulang. Offset internal diterapkan sama pada preview/ekspor. Tes membandingkan 90 hash paket, PTS, dan durasinya; track legacy tetap offset nol. |
| Clock webcam | Epoch berasal dari delapan observasi timestamp paket sumber DirectShow sebelum encoding. Progress encoder hanya menandai kesiapan. Jitter pipe dan ketepatan clock perangkat masih perlu diukur secara fisik. |
| Recovery | Final video diperiksa sebelum diterima; final rusak dipertahankan dan dibangun ulang. Prefix chunk sehat dapat dipulihkan tanpa membuang sumber. WAV diperiksa terhadap format serta panjang fisik. Tes juga mencakup recovery berulang dan kegagalan publikasi jurnal. |
| Resize window | Koordinat kursor memakai canvas rekaman awal, mengikuti crop/padding video. Tes matematika mencakup window mengecil/membesar dan batas visibilitas. |
| Audio Stop | Stream dihentikan lalu paket tersisa dikuras; sampel dipotong tepat pada batas akhir. Tes impulse terakhir memastikan sampel sebelum Stop tetap ada. |
| Audio disk | Penulis terpisah memakai antrean 768.000 byte, setara dua detik stereo float32 48 kHz. Tes writer tertahan mempertahankan 200 paket yang diterima, dan overflow menghasilkan error eksplisit. |
| Trim/zoom 100 ms | Frontend dan Rust memakai toleransi 1e-9 detik. Rentang 100 ms diterima; rentang 99 ms tetap ditolak. |
| Keyboard | Shortcut global menghormati kontrol interaktif dan event yang sudah ditangani. Space mengaktifkan tombol terfokus; panah tab tidak menggeser playhead. |
| Webcam EOF | Preview dan ekspor menyembunyikan kamera setelah track berakhir, termasuk ketika offset tidak nol. Diuji dengan pixel video sintetis dan sumber VFR. |
| Portabilitas proyek | Manifest menyimpan path relatif. Proyek v1/v2 yang disalin ke library baru dibuka dengan media lokalnya, dengan pembatasan path tetap berlaku. |
| Autosave/library | IPC save hanya membawa id, nama, dan edit. Sidecar kursor dipakai ulang; library membaca ringkasan. Lima save pada fixture 3,6 juta sampel tetap menggunakan satu sidecar yang sama. Cursor frontend tidak dibungkus proxy Vue. |
| Watchdog | Waktu/frame/ukuran harus bertambah; aktivitas tulis file mendukung fase faststart. Tes child heartbeat tanpa kemajuan berhenti, sedangkan child yang masih menulis file diizinkan selesai. |
| Audio preview/ekspor | Limiter memakai unity gain dan headroom 0,95. AudioWorklet menggunakan algoritma yang sama; perbandingan PCM stereo terhadap FFmpeg memiliki selisih di bawah 1e-6. Tes browser mencakup gain sekali, seek/resume, kegagalan modul, dan disposal. |
| Fallback/probe | Fallback hanya untuk kegagalan encoder hardware yang dikenali, bukan disk/input/filter/watchdog. Probe memakai ukuran dan fps tujuan. Output akhir divalidasi terhadap jumlah frame, fps, codec dan keberadaan audio. |
| Memori capture | Antrean encoder native dibatasi dua payload melalui patch lokal windows-capture. GPU render-target view, staging texture, dan buffer readback dipakai ulang. Tes antrean memeriksa backpressure, urutan, dan EOS; pengukuran GPU fisik masih diperlukan. |

Sampel kursor tetap rapat. Menghapus sampel diam dapat mengubah kurva smoothing, sehingga optimasi ini tidak diterapkan tanpa bukti kesetaraan visual.

## Pengukuran ekspor

Pengukuran memakai video sintetis pada mesin pengembangan, runtime FFmpeg bundel, dan orchestration Rust debug. Setiap hasil ekspor didekode penuh.

| Durasi 1080p60 | Waktu ekspor final | Interval gerakan maksimum | Ukuran perintah maksimum |
| --- | --- | --- | --- |
| 3 detik | 3,7324 detik | 179 | 9.119 byte |
| 15 detik | 13,7267 detik | 897 | 42.108 byte |
| 60 detik | 53,6567 detik | 897 | 42.108 byte |

Benchmark penjadwalan terisolasi 38.400 frame berubah dari 59,6915 menjadi 4,7013 detik, sekitar 12,70 kali. Rasio ini bukan percepatan seluruh ekspor. Benchmark ekspor penuh memakai pemilihan encoder otomatis dan tidak menyimpan tag encoder setiap hasil. Angka tersebut tidak membuktikan kecepatan GPU/CPU lain. Bukti: [render-hardening-benchmark.json](../artifacts/render-hardening-benchmark.json).

Video sementara per bagian dan MP4 final berada di disk bersamaan saat muxing, sehingga perlu sekitar dua kali ukuran video keluaran. Komposisi masih memakai CPU.

## Bukti pengujian

Total 104 tes otomatis berbeda lulus: 72 Rust/media, 11 unit frontend, dan 21 browser. Format, Clippy dengan warnings denied, build frontend, serta audit npm produksi juga lulus; audit npm melaporkan nol kerentanan pada pemeriksaan ini. Dua benchmark dijalankan terpisah dari hitungan tersebut.

- Capture: 25 unit test dan 3 tes media sintetis; log di [capture-p2](../artifacts/capture-p2/). Tidak ada perangkat yang diaktifkan.
- Core: 21 tes, termasuk metadata delapan jam, konkurensi Windows, relocasi, integritas cursor, dan batas durasi.
- Integrasi aplikasi: dua tes memastikan publikasi proyek/offset dan pemeliharaan marker recovery saat validasi gagal.
- Render: 14 tes library, 5 tes integrasi, serta tes loader contoh; dua benchmark dijalankan eksplisit. Software Media Foundation juga menghasilkan MP4 multi-bagian yang lolos decode.
- Frontend: build, 11 unit test, 20 alur editor headless, dan satu uji media rekaman asli melalui HTTP loopback/AudioWorklet. Uji browser memakai fixture IPC, bukan backend Tauri.
- Patch encoder: satu tes antrean/EOS melalui manifest vendored dependency.
- Ekspor rekaman yang sudah ada: H.264/AAC 1920 x 1080, 90 frame, 3,0 detik, decode penuh; hash semua sumber tetap sama. Bukti di [evidence.json](../artifacts/recorded-media-check-9e62e8764b0844fd866922366f408828/evidence.json).
- Smoke test native dari EXE hasil ekstraksi installer final: versi 0.1.2 terlihat, proyek tersimpan berhasil dibuka, frame layar dan webcam tampil, serta playhead maju dari 0:00,0 ke 0:12,2 saat playback. Pause kembali menampilkan tombol Play. Tidak ada pesan error yang terlihat. Tidak ada capture atau penyimpanan edit pada uji ini. Kualitas suara, lip-sync, dan latensi perangkat tidak diukur. Tool menghentikan percobaan klik X karena mendeteksi input pengguna; hasil close belum terverifikasi. Bukti: [native-smoke-0.1.2.json](../artifacts/native-smoke-0.1.2.json).

Verifikasi gabungan dan build installer dicatat dalam [verification-0.1.2.log](../artifacts/verification-0.1.2.log). Pemeriksaan integrasi terakhir juga meneruskan peringatan webcam pendek pada Stop yang berhasil; hasil tes aplikasi, loader contoh, dan rebuild sesudah perubahan itu ada di [finalization-0.1.2.log](../artifacts/finalization-0.1.2.log). Pemeriksaan paket memeriksa EXE x64, runtime, source limiter yang terbaca, dan lisensi terhadap checksum sumber; pemeriksaan ekstraksi bukan pengujian install/uninstall.

## Batas yang masih terbuka

Installer final: `target/release/bundle/nsis/Screenrecord_0.1.2_x64-setup.exe`, 55.029.259 byte (52,48 MiB), unsigned. SHA-256: `40936beb52a1d979812d03926bae3288d18fe1d5d4fc460352dd24fbf08366bd`. Ekstraksi membuktikan EXE x64 cocok dengan build setelah marker bundel Tauri, 47 resource cocok checksum, source/lisensi limiter dan lisensi windows-capture tersedia, serta FFmpeg/FFprobe hasil ekstraksi berjalan. Installer belum dieksekusi. Bukti: [installer-evidence-0.1.2.json](../artifacts/installer-evidence-0.1.2.json).

- Sinkronisasi fisik webcam/mic, drift durasi panjang, RF64 di atas 4 GiB, dan soak test.
- Final WASAPI drain pada driver berbeda; pencabutan perangkat; disk penuh/izin ditolak pada aplikasi native.
- Window resize/DPI/multi-monitor; Intel/AMD/NVIDIA; penggunaan GPU cache dan backpressure encoder pada capture nyata.
- Kualitas audio, sinkronisasi, dan latensi perangkat pada WebView2 native; ekspor dari UI, save-before-close, serta tombol X pada build final. Startup, membuka proyek, playback maju, dan pause sudah diperiksa pada EXE hasil ekstraksi.
- Install/uninstall/relaunch pada Windows 11 bersih, CI remote, penandatanganan, dan inventaris/material distribusi pihak ketiga yang lengkap.

HDR tone mapping tetap tidak tersedia. Crash sebelum chunk pertama selesai dapat kehilangan rekaman awal; setelah itu bagian terakhir yang belum selesai, sekitar 12 detik, tetap merupakan batas recovery.

Source limiter dan lisensi LGPL 2.1 disertakan secara terpisah; lisensi MIT windows-capture juga disertakan. Ini tidak menyatakan keseluruhan audit lisensi atau kesiapan distribusi publik sudah selesai.
