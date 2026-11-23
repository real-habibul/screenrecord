# Audit 0.4.1

Audit ini menindaklanjuti screenshot kegagalan mulai rekam pada 0.4.0. Tiga agen menangani capture, editor, dan render; root menangani lifecycle Tauri, recovery, integrasi dan paket. Pengujian tidak memakai computer-use atau perekaman perangkat. Folder proyek pengguna hanya dibaca untuk metadata diagnosis, tidak diubah oleh alat pengembangan.

## Temuan dan perbaikan

1. **P1 - Awal capture layar.** Dua jurnal sesi gagal menunjukkan `epochQpc: 0`, tanpa segmen layar, sementara kamera menghasilkan file. Ini mempersempit kegagalan ke sebelum pemrosesan frame layar. Pool capture sekarang menggunakan API free-threaded agar callback tidak bergantung pada dispatcher thread pekerja. Penantian frame pertama memeriksa worker yang berakhir lebih awal dan membedakan belum ada frame, encoder, serta snapshot GPU. Perbaikan ini masih memerlukan uji monitor/window pada perangkat pengguna.
2. **P1 - Penanda recovery dari rekaman yang tidak pernah dimulai.** Startup sebelumnya meninggalkan `pending.json` setelah error. Aplikasi menganggapnya rekaman terputus dan terus menampilkan error recovery. Sekarang hanya percobaan yang terbukti tidak pernah menghasilkan frame layar yang diarsipkan menjadi `failed-start.json`; media kamera/audio dan jurnal tetap utuh. Data ambigu, versi tidak dikenal, link/junction, dan segmen layar parsial tetap melalui recovery biasa. Guard pemulihan jendela juga mencakup kegagalan penyiapan kontrol mengambang.
3. **P2 - Dialog rekam.** Area isi bergulir terpisah dari tindakan utama, sehingga Start/Cancel tetap terjangkau pada jendela pendek. Error dialog dan preview kamera dipisahkan dari banner utama. Status pilihan kamera dan status preview dibedakan; cleanup yang gagal dapat dicoba ulang.
4. **P2 - Audio ekspor tidak lengkap.** Regresi media nyata mereproduksi MP4 dengan video 2 detik tetapi AAC hanya 0,1 detik yang sebelumnya diterima. Validasi sebelum publikasi sekarang memeriksa jumlah stream, stereo 48 kHz AAC, awal audio, dan durasi audio. Ekspor tanpa audio juga menolak stream audio yang tidak diharapkan.
5. **P1 - Kepemilikan frame dan worker native.** Encoder sebelumnya dapat menahan surface milik pool capture setelah callback berakhir, sehingga isinya bisa dipakai ulang saat encode masih berjalan. Setiap frame kini disalin ke surface GPU tersendiri; antrean tetap dibatasi dua payload. Jalur gagal mengirim pesan Stop tetap memegang worker sampai selesai. Startup native juga mempunyai deadline dan memindahkan worker yang terlambat ke registry cleanup, sehingga retry tidak membuka sesi baru ketika worker lama masih hidup. Ini bukan penghentian paksa driver; cleanup yang macet tetap memblokir akuisisi berikutnya.
6. **P2 - Refresh perangkat terlambat.** Dua enumerasi yang tumpang tindih sebelumnya dapat membuat jawaban lama mengganti daftar kamera/mikrofon backend sesudah jawaban baru. Generasi permintaan kini dipesan sebelum pekerjaan dimulai, kedua daftar diperbarui atomik, dan hasil kedaluwarsa ditolak. Refresh gagal tidak menyisakan daftar perangkat lama sebagai izin membuka preview/meter.
7. **P1 - Antrean encoder setelah kegagalan.** Kegagalan asynchronous encode sebelumnya bisa meninggalkan producer menunggu antrean penuh walau consumer sudah berakhir. Pengiriman kini memeriksa penyelesaian worker dan mengembalikan error. Finish/Drop menutup sender agar payload yang sudah diterima tetap dikuras, lalu consumer menerima end-of-stream tanpa pengiriman sentinel yang bisa macet.

## Verifikasi

Snapshot kode final lulus 425 tes otomatis berbeda:

| Cakupan | Lulus | Bukti |
| --- | ---: | --- |
| Workspace Rust, termasuk model, lifecycle Tauri, capture, render dan transkripsi | 227 | [workspace-tests-final.log](../artifacts/audit-0.4.1/workspace-tests-final.log) |
| Kontrak media capture tambahan | 12 | [capture-media-final.log](../artifacts/audit-0.4.1/capture-media-final.log) |
| Vendor capture/encoder | 7 | [vendor-capture-tests.log](../artifacts/audit-0.4.1/vendor-capture-tests.log) |
| Contoh CLI render dan format proyek legacy | 1 | [render-example.log](../artifacts/audit-0.4.1/render-example.log) |
| Unit frontend | 74 | [recording-041-unit.log](../artifacts/ui/recording-041-unit.log) |
| Browser headless, termasuk fixture media | 104 | [recording-041-browser-final.log](../artifacts/ui/recording-041-browser-final.log) |

Pemeriksaan format Rust, Clippy seluruh workspace/all-targets dengan `-D warnings`, TypeScript, build frontend, dan audit dependensi npm produksi lulus; audit npm melaporkan 0 vulnerability. Suite workspace memuat tes yang sengaja diabaikan: benchmark, fixture child yang dijalankan oleh tes induknya, dan tes media/model yang membutuhkan pemanggilan eksplisit. Dua belas kontrak media capture dijalankan terpisah. Tes model Whisper nyata dan tiga tes media transkripsi dari 0.4.0 tidak diulang karena jalurnya tidak berubah; hasil lama bukan klaim pengujian ulang versi ini.

Tes native memakai frame BGRA sintetis dan child process dengan deadline 30 detik. Finish dan Drop masing-masing menghasilkan 12 frame; semua frame berhasil didekode dan warna frame akhir cocok. Tes WARP membuat pool free-threaded tanpa capture item/session, serta memeriksa salinan tekstur setelah sumbernya ditimpa. Bukti ini memeriksa kontrak Windows dan encoder, bukan keberhasilan akuisisi monitor fisik. Lihat [log native encoder](../artifacts/audit-0.4.1/native-encoder-drain.log) dan [rincian patch vendor](../vendor/windows-capture/SCREENRECORD-PATCH.md).

Screenshot browser diperiksa pada tema terang/gelap. Tes geometri, hit-testing, dan keyboard mencakup 900x480, 900x640, serta 1280x720, termasuk pesan error panjang. Kontras terukur untuk teks muted 8,62/6,54; teks error 7,59/6,93; tombol utama 9,06; focus 10,31/6,04. Contoh: [dialog gelap](../artifacts/ui/record-dialog-041-dark-900x640.png) dan [error tema terang](../artifacts/ui/record-dialog-041-failure-light-900x640.png). Uji WebView2 native tetap manual.

Tes menggunakan direktori sementara dan media sintetis. Tidak ada panggilan transkripsi cloud berbayar atau pengaktifan perangkat capture. Tiga agen menjalankan audit silang: marker recovery, allowlist perangkat, kontrak audio ekspor, serta kepemilikan worker/antrean encoder tidak menyisakan blocker yang terkonfirmasi dalam jalur yang diperiksa.

## Paket

Installer Windows x64 unsigned: [Screenrecord_0.4.1_x64-setup.exe](../target/release/bundle/nsis/Screenrecord_0.4.1_x64-setup.exe), 56.140.203 byte (53,54 MiB). SHA-256: `5155593f9f31682efa9bcde1c0cb682bf8788286f5d6b8b3c9aadcf6727730bf`.

Build release berhasil. Inspeksi tanpa menjalankan installer membuktikan executable x64 cocok dengan hasil release setelah marker NSIS Tauri, 47 resource cocok checksum, dan notice/lisensi yang diperiksa cocok dengan sumber. FFmpeg/FFprobe hasil ekstraksi berhasil menjalankan `-version`. Aplikasi dan installer tidak diluncurkan/dipasang. Bukti: [installer-evidence-0.4.1.json](../artifacts/installer-evidence-0.4.1.json), [log build](../artifacts/audit-0.4.1/release-build.log), dan [checksum](../target/release/bundle/nsis/Screenrecord_0.4.1_x64-setup.exe.sha256).

## Uji manual yang masih diperlukan

1. Rekam masing-masing monitor 10-15 detik, lalu satu window yang terlihat. Ulangi pada 30/60 fps dengan webcam Off/On dan audio komputer. Pastikan frame layar tersimpan dan kontrol Stop bekerja.
2. Aktifkan preview webcam, ganti kamera, matikan/nyalakan preview, lalu mulai rekaman. Uji kamera sedang dipakai aplikasi lain dan izin Windows ditolak; error harus jelas dan retry tidak mengunci perangkat.
3. Paksa kegagalan sumber dengan menutup window yang dipilih. Setelah error, jendela editor harus kembali dan rekaman baru dapat dimulai. Percobaan tanpa frame tidak boleh berulang menjadi error recovery pada pembukaan berikutnya; file kamera/audio tetap di folder proyek.
4. Pada jendela kecil dan skala Windows 125/150%, periksa Start/Cancel, pengguliran, keyboard, serta tema terang/gelap di WebView2.
5. Rekam suara komputer dan mikrofon, Pause/Resume, lalu ekspor proyek dengan cut, musik, subtitle, kamera dan mask. Dengarkan noise, batas cut, panjang audio, dan sinkronisasi bibir. Periksa frame drop, pemakaian GPU/memori, serta kelancaran 60 fps setelah perubahan kepemilikan tekstur.

Belum ada bukti uji perangkat fisik, instalasi pada mesin bersih, atau penandatanganan paket untuk versi ini. Audit dan tes mengurangi risiko regresi; tidak menjamin ketiadaan bug pada semua perangkat.
