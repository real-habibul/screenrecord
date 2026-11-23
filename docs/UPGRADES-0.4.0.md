# Upgrade editor 0.4.0

Pekerjaan berurutan sesuai permintaan pengguna. Satu tahap melewati tes yang relevan dan audit integrasi sebelum tahap selanjutnya dimulai. Native UI, kamera/mikrofon fisik, dan penilaian kualitas perangkat tetap diuji manual oleh pengguna. Tidak memakai computer-use, tidak merekam perangkat otomatis, dan tidak memanggil API berbayar dalam tes.

| Tahap | Implementasi | Audit/gate |
| --- | --- | --- |
| 1 | Split, range cut, hapus klip dan ripple timeline | Lulus tes dan audit otomatis; perangkat native menunggu uji manual |
| 2 | Waveform, meter mikrofon, normalisasi dan music ducking | Lulus tes dan audit otomatis; perangkat native menunggu uji manual |
| 3 | Preview kamera dan kontrol rekam mengambang, countdown, pause/resume | Lulus tes dan audit otomatis; perangkat native menunggu uji manual |
| 4 | Preset tampilan dan layout kamera | Lulus tes dan audit otomatis; perangkat native menunggu uji manual |
| 5 | Highlight kata, split/merge/import SRT dan transkripsi lokal | Lulus tes dan audit otomatis; kualitas bahasa/perangkat menunggu uji manual |
| 6 | Tracking mask dengan koreksi manual | Lulus tes dan audit otomatis; rekaman nyata menunggu uji manual |

Root memiliki model/penyimpanan proyek, Tauri, IPC dan integrasi. Agen editor memiliki frontend dan tes browser. Agen render memiliki ekspor dan tes media. Agen capture memiliki backend perangkat/transkripsi/tracking serta audit silang. Perubahan API disepakati lewat CONTRACT.md; setiap file hanya mempunyai satu pemilik aktif.

## Tahap 1

Format proyek naik menjadi 4, dengan pembacaan versi 1/2/3 tanpa menulis sumber sampai Save. `videoClips` berisi interval sumber yang dipertahankan, maksimal 512, berurutan dan tanpa overlap; tiap klip minimal 100 ms. Array kosong mewakili trim legacy, sehingga penghapusan klip terakhir ditolak. Track lain tetap bertumpu pada waktu sumber dan diproyeksikan ke timeline hasil yang merapat.

Video dan PCM memakai grid waktu output kumulatif untuk mencegah drift akibat pembulatan per potongan. Audio digabung sebelum satu encode AAC. Preview menahan gambar dan membisukan audio selama seek pada batas cut. Caption yang teksnya melintasi bagian terhapus disembunyikan sampai dikoreksi/digenerate ulang, karena cue kalimat tidak menunjukkan kata mana yang dihapus. Audio cloud hanya diekstrak dari interval yang masih dipertahankan, termasuk konteks resampling dan chunk.

Audit backend tahap 1: `cargo test --workspace --locked` lulus 115 tes; tiga tes FFmpeg/localhost transkripsi tambahan lulus. `cargo clippy --workspace --all-targets --locked -- -D warnings` dan pemeriksaan format lulus. Log tersedia di `artifacts/upgrade-0.4.0/stage1-rust.txt`, `stage1-core-final.txt`, dan `stage1-transcribe-media.txt`. Build frontend, 39 tes unit, dan 53 tes browser lulus. Log browser: `artifacts/ui/stage1-browser-final.log`.

Gate tahap 1 ditutup setelah dua audit silang backend dan preview. Pengukuran browser terakhir memeriksa 291.072 sampel tanpa audio terhapus (peak 0) dan 588 draw tanpa frame terhapus. Kontrol audio asli tetap terdengar dengan peak 0,17673 dan RMS 0,12304 pada gain yang ditentukan. Ini adalah bukti browser headless/sintetis, belum pengukuran speaker atau WebView2 native.

Temuan yang diperbaiki: pemilihan frame masa depan pada cut pecahan detik; satu sampel terhapus yang terbaca sebelum resampling 44,1/48 kHz; cache raster subtitle kosong setelah Undo; jadwal gate audio preview yang dapat mundur/dibuka ulang dari clock media yang terlambat. Durasi library mengikuti hasil edit, sedangkan durasi media sumber tetap disimpan terpisah.

Hasil pengujian per tahap dan keterbatasan diisi setelah gate tahap tersebut selesai. Tes dan audit mengurangi risiko regresi; hasilnya tidak menyatakan aplikasi bebas dari seluruh bug pada semua perangkat.

Uji manual tahap 1 setelah build akhir:

1. Buka rekaman dengan kamera, mikrofon, dan audio komputer. Split di dua posisi, pilih klip tengah, lalu Delete. Durasi hasil dan timeline harus merapat.
2. Putar melintasi sambungan, seek cepat bolak-balik, Undo/Redo, dan ulangi dengan musik, zoom, subtitle, serta mask. Tidak boleh muncul gambar dari jeda yang dihapus atau suara singkat dari bagian terhapus.
3. Ekspor MP4 dan SRT. Periksa durasi hasil, sinkron kamera/audio, posisi efek, dan hilangnya caption yang melintasi cut. Generate ulang subtitle jika caption tersebut diperlukan.
4. Save, tutup, buka kembali, lalu pindahkan salinan folder proyek melalui Explorer. Cut harus tetap sama dan file rekaman asli tidak berubah.

## Tahap 2

Analisis waveform membaca PCM secara streaming dan menyimpan maksimal 4096 bin dalam memori. Bin mengikuti waktu file asli; statistik peak/RMS untuk normalisasi hanya menghitung interval yang dipertahankan. Target normalisasi adalah RMS -18 dBFS dengan batas peak -1 dBFS, bukan pengukuran LUFS. Gain terpisah dari volume pengguna dan tidak menulis ulang rekaman.

Music ducking memakai mikrofon sesudah volume/gain sebagai sidechain. Musik mengikuti envelope attack/release, sedangkan mikrofon dan audio komputer tetap pada levelnya. Preview membekukan envelope selama seek dan di batas cut; ekspor hanya memproses sampel output yang dipertahankan.

Meter mikrofon hanya berjalan setelah Test diklik, tanpa menyimpan atau memutar audio. Meter berhenti setelah 30 detik atau saat dialog ditutup, perangkat diganti, dan recording dimulai. Backend menghentikan/revoke sesi lama sebelum capture atau close.

Regresi backend tahap 2 lulus 138 tes workspace dan lima tes media capture tambahan, tanpa membuka perangkat fisik. Clippy seluruh workspace dengan `-D warnings` dan pemeriksaan format lulus. Log: `artifacts/upgrade-0.4.0/stage2-rust.txt` dan `stage2-capture-media.txt`. Empat tes IPC mencakup state/session, pemetaan file musik, serta cancel yang tiba sebelum reservasi job.

Ducking cocok dengan referensi PCM hingga selisih maksimum 7,45e-9; RMSE AAC 0,00034683 pada fixture 1080p. Streaming memproses audio 60 detik dalam 1,043 detik pada build debug di mesin ini, dengan buffer PCM 128 KiB dan tanpa file sementara enam kanal. Error consumer, EOF, frame count, sampel nonfinite, cancel, dan pengaruh suara pada bagian terhapus tercakup. Write yang macet di dalam OS tetap tidak menjanjikan batas waktu penghentian thread Rust.

Audit silang backend dan alur frontend tidak menyisakan blocker terkonfirmasi. Build, 46 tes unit frontend, dan 61 tes browser lulus. Setelah cache waveform dioptimalkan supaya tidak dihitung ulang setiap perpindahan playhead, build dan empat regresi terkait lulus. Log frontend: `artifacts/ui/stage2-build.log`, `stage2-unit.log`, `stage2-browser-final.log`, dan `stage2-waveform-cache.log`. Gate tahap 2 selesai.

Uji manual tahap 2 setelah build akhir:

1. Pilih mikrofon, klik Test, dan bicara. Periksa meter/indikasi peak, Stop, pergantian perangkat, batas sesi 30 detik, lalu mulai rekaman. Perangkat harus tersedia kembali.
2. Buka rekaman dengan bagian sangat keras dan bagian pelan. Cut bagian keras, jalankan Analyze dan Normalize, lalu periksa preview, Undo/Redo, Save, dan buka ulang.
3. Tambahkan musik, aktifkan ducking, lalu bicara dan berhenti. Bandingkan preview dengan MP4 hasil ekspor, termasuk sambungan cut dan seek cepat.
4. Saat waveform dihitung, coba pindah proyek, ekspor, dan tutup aplikasi. Analisis harus dibatalkan dan tidak meninggalkan status sibuk.

## Tahap 3

Pause menghentikan akuisisi native, Resume membuka bagian baru, dan Stop menggabungkan bagian pada grid frame/sampel output kumulatif. Satu bagian langsung menjadi sumber proyek; beberapa bagian menghasilkan direktori assembled unik. File bagian asli tetap dipertahankan. Thumbnail 4 fps memakai proses kamera yang sama dengan rekaman H264. Countdown 0/3/5 detik tidak memulai capture atau membuat proyek sebelum selesai.

Watchdog backend memeriksa sesi setiap detik, termasuk saat frontend tidak polling. Batas rekam delapan jam memiliki toleransi recovery/finalisasi 60 detik untuk waktu penghentian; tidak memotong sampel secara diam-diam. Status finalizing tetap aktif setelah slot native diambil. Perintah, frame, hasil, dan error mencantumkan identitas sesi supaya respons lama tidak mengganggu rekaman baru.

Root meminta pengecualian capture untuk kontrol dan jendela utama sebelum capture dimulai. Keberhasilan SetWindowDisplayAffinity wajib; readback yang tersedia harus cocok. GetWindowDisplayAffinity yang tidak tersedia tidak menghalangi rekam, sesuai [kontrak API Microsoft untuk layered windows](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowdisplayaffinity). Hasil visual pengecualian belum diuji pada WebView2/WGC native.

Gate tahap 3: 159 tes workspace, tujuh tes media capture, Clippy dan format lulus. Frontend lulus build, 48 tes unit, dan 69 tes browser. Perbaikan error lintas sesi sesudah baseline browser lulus 12 tes khusus, build, dan 48 tes unit. Log: `artifacts/upgrade-0.4.0/stage3-rust.txt`, `stage3-capture-media.txt`, serta `artifacts/ui/stage3-browser-final.log` dan `stage3-error-session.log`. Audit silang root/capture/render tidak menyisakan blocker terkonfirmasi pada lingkup otomatis.

Benchmark media sintetis 60 detik, layar 1080p60, kamera 720p, mikrofon dan system audio, encoder NVIDIA di mesin ini: jalur salin 7,48 detik, jalur normalisasi dengan durasi pecahan 13,61 detik. Rekaman native dapat memerlukan normalisasi karena timestamp/durasinya. Angka ini tidak menyatakan waktu finalisasi rekaman satu jam atau performa perangkat lain.

Uji manual tahap 3 setelah build akhir:

1. Batalkan countdown lewat Cancel/Escape, kemudian rekam singkat pada countdown 0/3/5. Periksa frame awal agar aplikasi/kontrol tidak ikut terekam.
2. Coba Pause/Resume berulang dengan kamera, mikrofon, dan system audio. Periksa pelepasan perangkat saat Pause, elapsed yang berhenti, serta hilangnya waktu jeda dari hasil.
3. Pindahkan kontrol dengan drag dan keyboard pada beberapa monitor/DPI. Uji Pause, Resume, Stop, Show app, dan Ctrl+Shift+R. Periksa posisi serta hilangnya kursor/klik kontrol dari output.
4. Pada rekaman uji, tutup window sumber atau putuskan perangkat; kemudian periksa penyimpanan/recovery. Coba juga restart setelah aplikasi dihentikan paksa pada rekaman uji pendek.
5. Periksa sinkron fisik kamera/audio, camera EOF/gap tanpa filler, kecepatan finalisasi sesi panjang, dan kondisi ruang disk rendah.

## Tahap 4

Preset hanya menyimpan aspect ratio, background, padding, corner radius, gaya kursor, kamera, dan gaya subtitle. Timeline, crop, zoom, musik, audio, teks, mask, serta referensi media tetap utuh. Custom preset dibatasi 50 dan file 128 KiB, disimpan atomik di settings lokal; file rusak/future version tidak ditimpa.

Layout kamera mendukung bubble, fullscreen, dan split. Fullscreen selalu menyembunyikan layar asli, termasuk saat kamera disembunyikan/tidak tersedia. Split mempertahankan area kamera kosong pada gap. Preset camera-only tanpa rekaman kamera ditolak agar hasilnya tidak membuka layar secara implisit.

Gate tahap 4 selesai: 169 tes Rust, Clippy, format, 53 tes unit frontend, 75 tes browser, dua pemeriksaan aksesibilitas terkait, dan build lulus. Audit memperbaiki EOF kamera dengan alpha mask serta invalidasi frame privasi saat layout diganti selama seek. File preset read-only diuji tanpa kehilangan isi atau file sementara. Log: `artifacts/upgrade-0.4.0/stage4-rust.txt` dan `artifacts/ui/stage4-*`.

Uji manual tahap 4: terapkan preset, Undo/Redo, simpan dan buka ulang; coba bubble/fullscreen/split pada rekaman kamera yang memiliki gap dan EOF. Periksa layout portrait, mirror, tema gelap/terang, serta pastikan fullscreen tidak menampilkan layar saat kamera dimatikan.

## Tahap 5

Subtitle menyimpan waktu kata dan rentang sumber terpisah. Cut membuang seluruh kata yang tidak lagi utuh, lalu membangun teks preview/ekspor dari kata yang tersisa. Impor SRT membaca UTF-8/UTF-16 dan memetakan waktu hasil edit kembali ke sumber tanpa mengarang timestamp kata. Split/merge menjaga metadata; penggabungan rentang yang saling menyela mengurutkan kata secara kronologis. Impor dan perubahan editor dapat di-Undo.

Transkripsi lokal menjadi default. Unduhan runtime whisper.cpp 1.9.5 dan model multilingual base hanya dimulai dengan tindakan pengguna, dengan pin SHA-256 untuk arsip, executable/DLL, dan model. Audio lokal tidak dikirim ke jaringan. Cloud tetap pilihan eksplisit dengan API key. Pembatalan menunggu ACK dan penghentian pekerjaan, termasuk ketika dialog ditutup atau provider diganti.

Model diuji dengan ucapan sintetis milik tes sepanjang 8,318 detik: 17 kata bertimestamp dalam 4,301 detik; versi cut hanya mempertahankan dua rentang yang diizinkan. Ini bukan pengukuran akurasi bahasa Indonesia. File model untuk tes berada di `artifacts/local-asr/installed`, tidak dibundel ke installer. Instalasi rusak/sebelumnya dipertahankan; pembersihan direktori yatim belum otomatis.

Preview highlight memakai batch delapan tampilan dalam satu proses FFmpeg. Ukuran cache dibatasi delapan gambar/64 MiB di frontend dan 128 file/64 MiB di backend. Benchmark delapan tampilan: 0,285 detik pada mesin ini. Raster memakai teks/segmentasi/style sebagai identitas, sehingga perubahan timing tidak memicu render ulang yang sama.

Audit memperbaiki batas metadata lama 1,9 MB menjadi 15 MiB, dengan ruang satu MiB bagi manifest 16 MiB. Regresi transkrip 4.000 cue/40.000 kata melewati batas lama dan tetap dapat divalidasi/ekspor SRT. Uji frontend terakhir: 64 unit, baseline 85 browser, tiga regresi merge, serta regresi pembatalan/penutupan setelah baseline; build lulus. Log akhir backend dan lokal: `artifacts/upgrade-0.4.0/stage5-*`; frontend: `artifacts/ui/stage5-*`.

Gate tahap 5 ditutup dengan 193 tes workspace Rust, tiga tes ekstraksi FFmpeg/localhost, satu kontrak model lokal aktual, Clippy seluruh target dan format lulus. Audit silang root/capture/render/editor tidak menyisakan blocker terkonfirmasi dalam cakupan otomatis.

Uji manual tahap 5: unduh model dari UI lalu generate subtitle Indonesia/campuran pada rekaman sendiri; nilai akurasi dan koreksi timing. Uji highlight pada WebView2, split/merge, cut, SRT dari editor lain, Undo/Redo, Save/reopen, dan bandingkan MP4/SRT. Uji Cancel, Escape, provider switch, serta native close saat download/generate. Cloud membutuhkan pengujian dengan key dan izin upload pengguna; tidak dipanggil dalam tes otomatis.

## Tahap 6

Tracking mask memproses sumber lokal sebelum crop/zoom pada 10 fps, downscale sisi panjang maksimal 640 piksel, dan durasi maksimal 300 detik per region. Template tetap melacak translasi; tekstur lemah, ambiguitas, objek tertutup, rotasi, perubahan skala atau gerakan terlalu cepat dapat menghentikan tracking. Ini bukan jaminan otomatis semua informasi sensitif tertutup.

Keyframe tetap memakai waktu/koordinat sumber. Di antara posisi otomatis, mask mencakup gabungan kedua kotak ujung untuk memberi cakupan konservatif. Saat Lost, seluruh layar sumber menjadi solid hitam, termasuk jika mode region adalah blur; perlindungan bertahan sampai koreksi manual. Hasil baru tidak mengubah rekaman atau proyek sampai diterapkan oleh editor sebagai satu langkah Undo. Cancel/error mempertahankan tracking sebelumnya.

Panel Masks menyediakan Track region, progress/cancel, daftar keyframe, koreksi angka/drag, retrack dari koreksi, hapus keyframe, Clear tracking, dan Mark tracking reviewed. Edit posisi/waktu/efek menghapus status review. Export menolak tracking aktif yang belum ditinjau, baik dari UI maupun backend. Clear tracking mempertahankan kotak saat ini sebagai mask statis, solid jika sedang Lost.

Draw correction on original membuka frame asli secara eksplisit ketika playback berhenti, dengan status yang terlihat dan audio dibisukan. Menyelesaikan gambar, Escape, atau keluar dari mode ini memulihkan preview tertutup secara sinkron. Mode inspeksi tidak digunakan oleh ekspor. Koreksi memakai timestamp frame yang benar-benar ditampilkan, termasuk saat pause terjadi di antara pembaruan playhead.

Tracking backend lulus 53 tes biasa dan 11 kontrak media capture, termasuk empat kontrak tracking. Fixture 10 detik 1080p menghasilkan 100 posisi dengan satu buffer frame 230.400 byte dan tanpa antrean frame; pengukuran debug sekitar 0,52 detik pada mesin ini. Sentinel RGB pada start pecahan/nonzero dan video sparse/VFR membuktikan decoder memilih frame aktif sebelumnya, bukan frame masa depan. Log: `artifacts/upgrade-0.4.0/stage6-capture-media.txt`.

Audit UI memperbaiki properti sementara `lost` yang ikut masuk ke rollback gesture statis, tanpa mengendurkan tes kesamaan metadata/riwayat Undo. Fixture bersama 13 kasus memastikan core dan frontend memakai kotak yang sama pada batas pecahan detik, Lost dan koreksi manual.

Gate frontend tahap 6 lulus 97 tes browser dan 74 tes unit, serta build produksi. Dua tes timing/keyframe tambahan lulus setelah confidence keyframe batas diwariskan dari titik sumber, tanpa mengarang nilai 100%. Log: `artifacts/ui/stage6-browser-final.log`, `stage6-unit.log`, `stage6-build.log`, dan `stage6-confidence.log`. Screenshot headless tema gelap/terang, Lost dan koreksi frame asli berada di `artifacts/ui/stage6-*`. Tidak menjalankan UI native atau computer-use.

Audit renderer menemukan perlambatan teardown grafik FFmpeg ketika banyak blur memakai sumber alpha dari cabang gambar yang saling bersarang. Sumber alpha independen berbatas frame mempertahankan urutan blur dan mengurangi workload satu frame 1080p/16 mask dari 99,984 detik menjadi 0,353 detik pada mesin ini. Hambatan diamati sesudah frame terakhir/encoder EOF, sehingga angka tersebut mengukur kasus teardown, bukan rasio percepatan ekspor umum. Blur satu baris/kolom juga mendapat penanganan khusus setelah FFmpeg terbukti crash pada patch selebar satu piksel dan memberi hasil tidak valid pada tinggi satu piksel; baris/kolom milik patch sendiri diduplikasi sementara, lalu dicrop kembali tanpa membaca tetangga.

Renderer lulus 57 tes regresi, lalu tiga tes tracking akhir setelah penanganan tiny patch diperluas ke 1xN, Nx1 dan 1x1. Clippy bersih. Ekspor sintetis 10 detik 1080p30 dengan satu blur bergerak, 91 keyframe, selesai 11,776 detik memakai encoder NVIDIA. Stress 16 blur/60 frame/1080p ke null output memerlukan 11,606 detik pada satu thread filter. Performa mask bertumpuk tetap bergantung CPU dan tidak dijanjikan realtime. Perintah dan hasil ada di `crates/render/README.md`.

Uji manual tahap 6: track form atau objek yang bergeser, putar seluruh rentangnya, periksa setiap bagian Lost, gambar koreksi, lalu retrack dan tandai Reviewed. Coba crop/zoom/cut, drag/resize saat playback dipause, Undo/Redo, Save/reopen, Cancel/native close dan bandingkan MP4 dengan preview. Gunakan solid untuk kebutuhan menutup seluruh piksel; blur bersifat pengaburan visual dan perlu penilaian pengguna. Periksa kinerja mask bertumpuk pada mesin sasaran.

## Integrasi akhir

Gate keenam ditutup setelah audit silang ketiga agen dan root tidak menyisakan blocker terkonfirmasi pada cakupan otomatis. Regresi source final lulus 211 tes workspace Rust, 11 kontrak media capture, tiga kontrak ekstraksi transkripsi, satu tes model lokal aktual, satu tes antrean vendor, 74 unit frontend dan 97 tes browser. Satu tes contoh CLI juga lulus setelah fixture metadata kamera diperbarui. Clippy seluruh target, format, build frontend dan audit dependency produksi npm lulus (0 vulnerabilities).

Log integrasi Rust: `artifacts/upgrade-0.4.0/stage6-final-rust.txt`. Log capture, model, vendor, npm dan contoh CLI berada di direktori yang sama. Hasil browser/build/unit terbaru berada di `artifacts/ui/stage6-*`. Tes fokus yang dijalankan ulang tidak ditambahkan ke jumlah unik suite utama.

Pemeriksaan ini memakai media sintetis, HTTP localhost dan browser headless. Tidak ada computer-use, rekaman perangkat baru, cloud transcription berbayar, penghapusan proyek pengguna atau instalasi aplikasi dalam pengujian ini. Uji native/WebView2, kualitas audio nyata, lip-sync, multi-DPI, camera hotplug/Pause/Resume, clean install/uninstall dan penandatanganan tetap terbuka. Kelengkapan distribusi publik dicatat di `THIRD_PARTY_NOTICES.md` dan `TODO.md`; status ini bukan pernyataan siap produksi pada semua perangkat.

## Paket pengujian Windows

Installer `target/release/bundle/nsis/Screenrecord_0.4.0_x64-setup.exe` selesai dibangun dan diinspeksi pada 12 November 2026. Ukuran 56.101.773 byte (53,50 MiB), x64, status tanda tangan `NotSigned`. SHA-256: `0849cb56f5ef955b377c9e2a91b229289b55fa0df561560d32708ff13dbf5ef1`. File `.sha256` tersedia di samping installer.

Inspeksi mengekstrak paket tanpa memasang atau membuka aplikasi. Binary cocok dengan release sesudah normalisasi marker bundle Tauri; 47 resource, notice dan sumber/lisensi limiter cocok checksum. FFmpeg/FFprobe hasil ekstraksi dapat dijalankan. Runtime/model transkripsi lokal tidak ikut installer, melainkan diunduh lewat tindakan eksplisit di UI. Bukti: `artifacts/installer-evidence-0.4.0.json` dan `artifacts/upgrade-0.4.0/final-installer-inspection.txt`.
