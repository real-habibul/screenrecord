# Mode Apps 0.4.4

Dialog New recording menambahkan mode Apps di samping Screen dan Window. Apps menampilkan proses aplikasi yang sedang berjalan dan mempunyai jendela yang memenuhi syarat perekaman. Aplikasi yang belum dibuka atau hanya mempunyai proses latar tidak muncul sebagai sumber video.

Pilih aplikasi, lalu pilih jendelanya jika terdapat beberapa jendela. Rekaman tetap menargetkan satu jendela. Jendela baru dan popup tidak diikuti otomatis. Computer audio tetap merekam campuran suara komputer, sehingga dapat memuat suara aplikasi lain.

## Dasar teknis

- [Windows Graphics Capture](https://learn.microsoft.com/en-us/windows/apps/develop/media-authoring-processing/screen-capture) menyediakan sumber layar atau jendela. [CreateForWindow](https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow) membuat capture item untuk satu HWND, bukan seluruh proses.
- Nama aplikasi berasal dari basename executable melalui [QueryFullProcessImageNameW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-queryfullprocessimagenamew), memakai hak query terbatas dan handle yang ditutup setelah pemakaian. Jika nama executable tidak dapat dibaca, judul jendela asli dipakai sebagai pengganti.
- Aplikasi dikelompokkan menurut PID. Source ID `app:<pid>:<hwnd>` diperiksa terhadap pemilik jendela saat memulai dan melanjutkan rekaman, termasuk sebelum/sesudah pembuatan item native. Ketidakcocokan ditolak tanpa memilih sumber pengganti.
- PID dan HWND dapat dipakai ulang oleh Windows. Pengecekan ini menolak jendela hilang dan pemilik berbeda; keduanya bukan identitas permanen aplikasi. UI menghapus pilihan yang tidak lagi ada ketika daftar diperbarui.
- Pemilihan Apps tidak memerlukan injection, pembacaan memori proses, atau hak administrator. Tidak ada upload atau telemetry. Skema proyek tersimpan tetap versi 4.

## Verifikasi

Verifikasi kode final lulus 476 tes berbeda. Setelah preflight ditambahkan, tes Tauri/capture/media dan Clippy dijalankan ulang; hasil crate yang tidak berubah tetap berasal dari gate workspace sebelumnya. Regresi terarah dan pengulangan tes media tidak dihitung dua kali.

| Cakupan | Lulus | Bukti |
| --- | ---: | --- |
| Tauri | 35 | [app-final.log](../artifacts/apps-0.4.4/app-final.log) |
| Capture | 79 | [capture-tests.log](../artifacts/apps-0.4.4/capture-tests.log) |
| Core, render beserta integrasi, transcribe | 137 | [workspace-final.log](../artifacts/apps-0.4.4/workspace-final.log) |
| Media capture sintetis tambahan | 14 | [verifikasi serial](../artifacts/apps-0.4.4/media-serial-verification.log), [pengulangan paralel](../artifacts/apps-0.4.4/media-parallel-recheck.log) |
| Vendor native tanpa sumber capture | 12 | [vendor-tests.log](../artifacts/apps-0.4.4/vendor-tests.log) |
| Unit frontend | 79 | [frontend-unit.log](../artifacts/apps-0.4.4/frontend-unit.log) |
| Browser headless final | 120 | [frontend-browser-final.log](../artifacts/apps-0.4.4/frontend-browser-final.log) |

Format Rust, Clippy workspace/all-targets dengan `-D warnings`, TypeScript/build frontend, dan audit npm produksi lulus. Audit npm melaporkan 0 vulnerability. Tes perangkat dan instalasi tetap dilakukan pengguna secara manual.

Satu pengulangan tes media sempat gagal saat proses FFmpeg pada tes kamera sintetis keluar dengan kode 1 ketika Stop. [Kegagalan asli](../artifacts/apps-0.4.4/media-preflight-failure.log) tetap disimpan. Stderr dari percobaan itu belum tersedia karena folder sementara tes telah dibuang. Pengulangan terpisah, paralel dan serial semuanya lulus; ini belum membuktikan penyebab awal. Assertion tes kini mencetak stderr terbatas bila kegagalan terulang, tanpa mengubah syarat sukses, batas waktu, atau kode kamera produksi.

Build rilis berhasil. [Installer Windows x64 0.4.4](../target/release/bundle/nsis/Screenrecord_0.4.4_x64-setup.exe) berukuran 56.161.576 byte (53,56 MiB), dengan SHA-256 `0f4856ee1b5f21328b8ebd646cfea7efa0c47b6209d5a06004df0e146c061afd`.

Inspeksi tanpa memasang atau menjalankan aplikasi membuktikan executable x64 versi 0.4.4 cocok dengan hasil build setelah penanda bundle NSIS. Sebanyak 47 resource serta notice/lisensi cocok checksum, dan FFmpeg/FFprobe hasil ekstraksi berhasil menjalankan `-version`. Paket masih **unsigned**. Hash 21 file sumber terpilih cocok dengan snapshot sebelum bundling. Bukti: [build](../artifacts/apps-0.4.4/release-build.log), [inspeksi installer](../artifacts/installer-evidence-0.4.4.json), dan [ringkasan verifikasi](../artifacts/apps-0.4.4/verification-evidence.json).

Audit menemukan bahwa Enter pada kolom pencarian dapat memicu submit form dan memulai sumber yang masih terpilih. Regresi mereproduksi satu submit sebelum perbaikan, lalu memverifikasi Enter tidak lagi memulai rekaman dan tombol Start eksplisit tetap berfungsi. Penanganan Enter dibatasi pada kolom pencarian Apps. Bukti awal ada di [frontend-search-before.log](../artifacts/apps-0.4.4/frontend-search-before.log).

Pemeriksaan sumber juga dipindahkan lebih awal, sebelum kontrol perekaman disiapkan atau folder proyek dibuat. Aplikasi yang sudah tidak tersedia saat Start ditolak tanpa membuat percobaan kosong. Pemeriksaan pada startup native tetap dilakukan ulang. Jika target hilang sesudah pemeriksaan awal dan bukti startup belum lengkap, penanda recovery tetap dipertahankan secara konservatif.

UI memakai komponen sumber, warna netral/amber dan tipografi yang sudah ada; ENERGY 1 / RHYTHM 1 / MOTION 1 tetap dipertahankan. Penambahan terbatas pada Apps, pencarian nama/judul, pilihan jendela dan status yang diperlukan. Screenshot headless [gelap](../artifacts/apps-0.4.4/apps-dark-900x640.png) dan [terang](../artifacts/apps-0.4.4/apps-light-900x640.png) telah diperiksa pada 900x640: tombol Start tetap terlihat, panel dapat digulir dan fokus keyboard terlihat. Delapan [pasangan warna teks bantu/fokus](../artifacts/apps-0.4.4/contrast-pairs.json) yang diperiksa memenuhi ambang 4,5:1 untuk teks normal dan 3:1 untuk fokus. Pemeriksaan ini bukan sertifikasi aksesibilitas seluruh aplikasi.

## Uji manual

1. Tutup versi lama, pasang 0.4.4, buka Roblox, lalu pilih New recording > Apps > Refresh devices.
2. Pilih aplikasi Roblox yang ditampilkan. Jika ada beberapa jendela, pilih jendela yang diinginkan dan mulai rekaman dengan tindakan eksplisit.
3. Rekam 10-15 detik, Stop, lalu periksa hasil. Ulangi dengan Pause/Resume.
4. Sebelum merekam percobaan berikutnya, tutup jendela terpilih lalu Refresh devices. Pastikan Start tidak merekam aplikasi lain sampai Anda memilih sumber lagi.
5. Uji aplikasi dengan beberapa jendela, periksa pilihan jendela serta hasil rekamnya. Nama/judul yang sama tidak berarti dua proses merupakan satu instance.
6. Pilih aplikasi, tutup jendelanya, lalu tekan Start tanpa Refresh. Sumber harus ditolak sebelum folder proyek dibuat; tidak ada tugas recovery baru dari percobaan yang sudah kedaluwarsa sebelum pemeriksaan awal.

Mode Apps tidak menjadi bukti bahwa timeout capture fisik pada versi sebelumnya telah hilang. [Audit 0.4.3](AUDIT-0.4.3.md) memuat diagnostik dan keterbatasan kasus tersebut.
