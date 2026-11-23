# Editor vertikal 0.5.0

Rilis ini menambahkan Fit/Fill, reframe manual dengan keyframe, dan background video blur untuk mengolah rekaman landscape menjadi video portrait. Pengujian alur aplikasi native/capture perangkat dan pemasangan installer dilakukan pengguna secara manual. Tidak ada computer-use atau capture perangkat dalam pekerjaan ini.

## Cara memakai

1. Pilih Canvas > Aspect ratio > 9:16 portrait.
2. Pilih Screen sizing > Fill untuk memenuhi area layar dengan crop, atau Fit untuk mempertahankan area sumber secara utuh. Padding 0 dan corner radius 0 menghasilkan frame tanpa margin dan sudut kosong.
3. Pada Manual framing, pause di posisi yang diinginkan, tekan Set keyframe at playhead, lalu atur fokus horizontal/vertikal dan scale. Pindah ke waktu lain dan set keyframe berikutnya. Pilihan keyframe dapat di-seek, diperbarui atau dihapus; Undo dan penyimpanan memakai alur edit yang sudah ada.
4. Pilih Background style > Blurred video dan atur Background blur strength (4..64). Efek paling terlihat dengan Fit atau margin; Fill tanpa margin menutupi latarnya dengan foreground. Fallback color dipakai pada camera-only atau inspeksi frame asli.
5. Ekspor 1080p dengan kanvas 9:16 menghasilkan 1080x1920. Periksa framing, HUD dan subtitle sebelum unggah.

## Aturan komposisi dan data

- Crop manual membatasi area sumber. Fit/Fill, keyframe dan zoom tidak dapat keluar dari batas itu.
- Fill memakai pane layar yang sebenarnya, termasuk pada split kamera. Fokus Fill saat tidak ada keyframe/zoom adalah tengah crop; kursor tidak menggerakkan framing sendiri.
- Keyframe memakai waktu rekaman asli, bukan waktu output setelah cut. Nilai diinterpolasi linear, dengan key pertama/terakhir ditahan di luar rentang key. Keyframe di luar trim atau di bagian yang dihapus tetap tersimpan; preview tidak membuka kembali frame yang sudah dipotong.
- Fokus manual menggantikan fokus zoom otomatis; scale keyframe dan scale zoom tetap dikalikan. Keyframe tanpa Fill atau pembesaran dapat tidak mengubah gambar ketika seluruh crop masih terlihat.
- Background blur harus memakai viewport akhir yang sama dengan foreground, setelah mask/redaction. Latar tidak mengambil kembali gambar di luar crop, zoom, keyframe atau cut. Kamera dan subtitle tidak diduplikasi ke latar. Mode camera-only memakai background solid.
- Proyek memakai skema 5. Proyek versi 1..4 dibaca dengan default lama tanpa ditulis ulang saat hanya dibuka. Saat disimpan, proyek diperbarui ke versi 5 agar versi lama menolaknya daripada menghapus edit baru yang tidak dikenal.
- Preset penampilan menyimpan Fit/Fill dan pengaturan background, tetapi tidak membawa keyframe waktu sumber dari rekaman lain.

## Gate per fitur

| Tahap | Bukti | Status |
| --- | --- | --- |
| Fit/Fill | 17 geometri independen, core 48, render 60 + integrasi 5, unit UI 97, regresi browser 12; format/Clippy/build | Lulus dan diaudit sebelum tahap 2 |
| Reframe keyframe | 34 geometri independen, core 52, render 62 + integrasi 5, unit UI 118, regresi browser 16; format/Clippy/build | Lulus dan diaudit sebelum tahap 3 |
| Background video blur | 8 geometri background independen, core 53, render 70 + integrasi 5, unit UI 130, browser penuh 133; format/Clippy/build | Lulus dan diaudit sebelum installer |

Fixture independen: [reframe-reference.json](../testdata/reframe-reference.json). Log dan screenshot berada di [artifacts/vertical-0.5.0](../artifacts/vertical-0.5.0). Jumlah gate per tahap tidak dijumlahkan sebagai jumlah tes berbeda pada verifikasi akhir.

Tahap 2 sempat menemukan ketidakcocokan warna pada fixture gradien MPEG4 yang lossy. Fixture diganti menjadi FFV1 RGB lossless tanpa menaikkan toleransi; koordinat tetap dihitung independen dan hasil source-time/cut/chunk lulus. Bukti awal serta [diagnostik warna](../artifacts/vertical-0.5.0/render-stage2-gradient-diagnostic.json) tetap disimpan.

Tes tahap 3 menemukan dua bug lama yang relevan dengan fitur baru. Chromium dapat mengambil piksel tetangga saat memperbesar subrectangle sumber yang sangat tipis, termasuk pada foreground. Jalur preview kini mengisolasi piksel sumber sebelum resampling dengan tile native berbatas, bukan menambah buffer sebesar rekaman. Tes sentinel memakai warna tetangga terlarang dan memeriksa nilai RGBA. Tes renderer juga menemukan satu frame kamera melewati batas akhir interval saat ekspor memakai chunk kecil; gate kamera kini memakai batas frame integer yang kausal, dengan akhir interval tetap eksklusif.

Background memakai subrectangle yang berada sepenuhnya di dalam viewport akhir. Area itu dipotong sebelum scaling berbatas agar crop tipis tidak membuat gambar perantara sangat besar. Di FFmpeg, foreground/background mempunyai command stream sendiri agar frame pada satu cabang tidak mengubah crop frame yang masih mengantre di cabang lain. Fixture tambahan: [background-reference.json](../testdata/background-reference.json).

Tes warna menemukan bahwa konversi YUV default dan penanda warna encoder tidak selalu sama, sehingga biru dapat menjadi [0,13,255] pada hasil decode AMF. Ekspor kini mengonversi komposisi RGB akhir ke limited BT.709 dan memakai penanda warna yang sesuai pada jalur encode. Stream yang hanya disalin, capture dan vendor tidak diubah. Regresi warna sintetis untuk encoder NVIDIA, AMF dan software Media Foundation yang tersedia memeriksa primary/grayscale serta metadata. [Diagnostik awal](../artifacts/vertical-0.5.0/render-stage3-color-diagnostic.json) disimpan.

Tes reframe memisahkan koordinat geometri dari loss codec: komposisi RGB sebelum subsampling harus memenuhi ekspektasi koordinat independen, sedangkan H.264 dibandingkan terhadap komposisi itu dengan batas error yang dicatat. Gambar YUV420/H.264 tidak diklaim identik piksel dengan komposisi RGB mentah.

## Verifikasi akhir dan installer

Sebanyak 562 tes berbeda lulus pada kandidat final. Jumlah ini memakai suite final, bukan penjumlahan tes yang diulang di setiap gate.

| Cakupan | Lulus | Bukti |
| --- | ---: | --- |
| Tauri, core, capture, transcribe | 198 | [integration-final.log](../artifacts/vertical-0.5.0/integration-final.log) |
| Renderer | 70 | [render-stage3-tests-complete.log](../artifacts/vertical-0.5.0/render-stage3-tests-complete.log) |
| Integrasi ekspor | 5 | [log renderer yang sama](../artifacts/vertical-0.5.0/render-stage3-tests-complete.log) |
| Media capture sintetis tambahan | 14 | [capture-media-final.log](../artifacts/vertical-0.5.0/capture-media-final.log) |
| Vendor native tanpa sumber capture | 12 | [vendor-final.log](../artifacts/vertical-0.5.0/vendor-final.log) |
| Unit frontend | 130 | [stage3-unit.log](../artifacts/vertical-0.5.0/stage3-unit.log) |
| Browser headless penuh | 133 | [frontend-browser-final.log](../artifacts/vertical-0.5.0/frontend-browser-final.log) |

Format Rust, Clippy workspace/all-targets dengan `-D warnings`, TypeScript/build frontend, dan audit npm produksi lulus. Audit npm melaporkan 0 vulnerability. Benchmark khusus, fixture child dan pengujian lain yang diabaikan tidak dihitung sebagai lulus. Tes media capture dijalankan serial, tanpa aktivasi layar/kamera/mikrofon.

Geometri preview/ekspor cocok pada 34 kasus framing dan 8 kasus background independen. Tes piksel foreground memperoleh error 0 untuk warna konstan, transform dan stale overwrite; perbedaan maksimum gradien 1. Permukaan canvas sementara tambahan dibatasi di bawah 20 MiB; ini batas buffer fitur, bukan pengukuran RAM total aplikasi. Screenshot tema [gelap](../artifacts/vertical-0.5.0/stage3-dark-900x640.png) dan [terang](../artifacts/vertical-0.5.0/stage3-light-900x640.png) diperiksa pada 900x640, dengan fokus native dan panel yang dapat digulir. Tidak ada logo/aset merek atau motion UI tambahan; arah netral/amber, Segoe UI dan ENERGY 1 / RHYTHM 1 / MOTION 1 dipertahankan.

Sepuluh [pasangan warna teks/fokus](../artifacts/vertical-0.5.0/contrast-pairs.json) pada kontrol baru memenuhi ambang 4,5:1 untuk teks normal dan 3:1 untuk fokus. Pemeriksaan tampilan ini tidak menggantikan uji screen reader, Windows high contrast atau seluruh matriks DPI.

Build rilis berhasil. [Installer Windows x64 0.5.0](../target/release/bundle/nsis/Screenrecord_0.5.0_x64-setup.exe) berukuran 56.184.968 byte (53,58 MiB). SHA-256: `a93d137f9e4dde7d08a0e0056582b8902f759df1a9cd266237b4cd7841a006d6`.

Inspeksi tanpa memasang atau menjalankan aplikasi membuktikan executable x64 versi 0.5.0 cocok dengan hasil build setelah penanda bundle NSIS. Sebanyak 47 resource serta notice/lisensi cocok checksum; FFmpeg/FFprobe hasil ekstraksi berhasil menjalankan `-version`. Paket masih **unsigned**. Seluruh 179 hash sumber terpilih cocok dengan snapshot sebelum bundling. Bukti: [build](../artifacts/vertical-0.5.0/release-build.log), [inspeksi paket](../artifacts/installer-evidence-0.5.0.json), dan [ringkasan verifikasi](../artifacts/vertical-0.5.0/verification-evidence.json).

## Uji manual

- Buka salinan proyek landscape, pilih 9:16 dan bandingkan Fit/Fill, crop dan layout kamera.
- Set dua keyframe pada waktu berbeda, ubah fokus/scale, lalu periksa playback, Undo, simpan/buka ulang dan hasil ekspor.
- Potong bagian tengah video dan periksa bahwa framing tetap mengikuti waktu sumber serta bagian terhapus tidak terlihat atau terdengar.
- Aktifkan background blur dengan mask solid pada tulisan sensitif. Periksa foreground dan background pada preview maupun MP4; blur sendiri bukan pengganti mask solid.
- Uji background dengan split/fullscreen kamera, gap kamera, seek cepat serta perubahan mask saat pause.
- Periksa hasil 1080p dan resolusi lain, kualitas teks, kesesuaian gambar/suara, dan pemasangan pada mesin Windows bersih. Installer belum dinyatakan signed atau teruji perangkat sampai ada bukti tersendiri.

Capture layar/window, Roblox, webcam, lip-sync dan keandalan sesi panjang masih memerlukan uji manual perangkat. Status timeout capture sebelumnya tercatat pada [audit 0.4.3](AUDIT-0.4.3.md); tes editor/ekspor tidak membuktikan kasus perangkat tersebut telah hilang.
