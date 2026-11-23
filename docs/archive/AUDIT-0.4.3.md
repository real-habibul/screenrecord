# Audit rekaman 0.4.3

> Arsip pengembangan. Lihat [unduhan publik](https://github.com/real-habibul/screenrecord/releases) untuk versi yang tersedia saat ini.

> Jalur `artifacts/` dan `target/` merujuk ke hasil lokal dari pengujian saat itu. Panduan untuk menjalankan pemeriksaan saat ini ada di [panduan kontribusi](../../CONTRIBUTING.md#memilih-pengujian).

Audit ini berangkat dari kegagalan terbaru pada versi 0.4.2 yang terpasang, bukan dari asumsi bahwa tes sintetis membuktikan kompatibilitas perangkat. Hash executable terpasang cocok dengan paket 0.4.2 yang sebelumnya dibangun.

## Bukti kasus terbaru

- Dua percobaan pukul 13:21:50 dan 13:22:12 tidak mencapai frame layar pertama. Keduanya memakai monitor yang sama, 1920x1080/30 fps, audio komputer aktif, serta webcam dan mikrofon nonaktif.
- Kedua jurnal memiliki epoch nol, durasi nol, dan tidak ada segmen layar. WAV hanya berisi header 92 byte. Pesan tersimpan adalah timeout startup sekitar 15 detik.
- Monitor dan pengaturan yang sama berhasil merekam sekitar 3,55 detik pada pukul 13:09. Hasil tersebut berstatus ready dan mempunyai segmen layar serta audio.
- Masalah webcam dari 0.4.2 tidak menjelaskan kegagalan terbaru ini. Epoch nol juga belum membuktikan tidak ada event Windows: proses dapat tertahan sebelum callback aplikasi tercapai.

Folder pengguna hanya dibaca untuk metadata dan log. Tidak ada pemutaran isi media, modifikasi proyek, aktivasi capture, atau computer-use dalam audit ini.

## Temuan yang diperbaiki

1. **P1 - Rollback startup native tidak lengkap.** Callback FrameArrived menyimpan referensi kuat ke frame pool. Ketika konfigurasi berikutnya gagal sebelum objek selesai dibuat, cleanup normal belum tersedia dan referensi itu dapat tertinggal. Guard rollback kini memiliki token event, session, dan pool sejak alokasi; kegagalan melepas registrasi serta menutup session sebelum pool.
2. **P1 - Lifetime DispatcherQueue pada worker capture.** Jalur free-threaded membuat DispatcherQueue yang tidak dipakai oleh frame pool, lalu sejumlah error awal dapat melewati shutdown/pumping yang diwajibkan Windows. Worker kini hanya menyiapkan Win32 message queue sebelum callback dapat terjadi. Tes memastikan WM_QUIT yang datang sebelum loop tidak hilang.
3. **P1 - Error rekaman dapat tertutup oleh proyek lama.** Polling yang melihat active menjadi idle sebelumnya membuka proyek terbaru tanpa memeriksa identitas. Kegagalan tanpa proyek baru dapat membuka rekaman lama dan menghapus error yang baru diterima. Hasil terminal kini dicocokkan dengan identitas sesi.
4. **P1 - Event gagal sebelum ACK Start dapat hilang.** UI sebelumnya mengabaikan semua event terminal saat menunggu Start. Event sesi terkait kini ditahan sampai identitas ACK tersedia; event sesi lain tetap diabaikan.
5. **P2 - Stop sebelum status awal diketahui dan balasan health yang kedaluwarsa.** Kontrol Stop kini memerlukan sesi aktif dengan identitas yang diketahui. Balasan error status yang terlambat memakai guard revisi operasi seperti balasan sukses.
6. **P2 - Inisialisasi UI dapat buntu setelah satu RPC gagal.** Hasil inisialisasi yang berhasil kini tetap dipakai ketika permintaan lain gagal, dan permintaan gagal dapat dicoba ulang.
7. **Diagnostik startup kurang membedakan tahap native.** Metadata baru merekam checkpoint startup/frame, jumlah event, jumlah pemanggilan handler dan HRESULT tanpa membaca pixel/audio. Versi aplikasi serta fase error/recovery juga tersimpan pada folder proyek, sehingga diagnosis tidak bergantung pada log aplikasi yang mungkin tidak tersedia.
8. **P1 - Inisialisasi WinRT pada thread native.** Pembuangan dan pekerjaan encoder memanggil metode WinRT, bukan hanya melepas pointer. Jalur cleanup startup tertentu, static-frame, dan dua worker transcode sebelumnya tidak menginisialisasi WinRT pada thread yang melakukan panggilan. Cleanup native kini menginisialisasi apartment lebih dahulu, mempertahankannya sampai pekerjaan dan destructor selesai, dan mengarantina pekerjaan yang belum diambil jika inisialisasi gagal. Worker encoder juga menginisialisasi runtime sebelum mengambil payload; kegagalan init mempertahankan objek di pemilik yang telah diinisialisasi untuk dilepas dengan aman. Rujukan: [kontrak RoInitialize](https://learn.microsoft.com/en-us/windows/win32/api/roapi/nf-roapi-roinitialize).

Perbaikan native di atas adalah bug kode yang terkonfirmasi. Keterkaitannya dengan dua timeout fisik terbaru masih belum terbukti. Versi ini tidak dinyatakan bebas bug atau siap produksi hanya berdasarkan tes otomatis.

## Verifikasi dan paket

Gate kode final lulus 455 tes berbeda:

| Cakupan | Lulus | Bukti |
| --- | ---: | --- |
| Workspace Rust | 240 | `artifacts/audit-0.4.3/workspace-final.log` |
| Media capture sintetis tambahan | 14 | `artifacts/audit-0.4.3/media-final.log` |
| Vendor native tanpa sumber capture | 12 | `artifacts/audit-0.4.3/vendor-capture-tests.log` |
| Unit frontend | 74 | `artifacts/audit-0.4.3/frontend-unit.log` |
| Browser headless | 115 | `artifacts/audit-0.4.3/frontend-browser-final.log` |

Format Rust, Clippy workspace/all-targets dengan `-D warnings`, TypeScript/build frontend, dan audit npm produksi lulus. Audit npm melaporkan 0 vulnerability. Benchmark, fixture child yang hanya dijalankan oleh tes induknya, dan pengujian model/cloud yang tidak berubah tidak dihitung sebagai tes tambahan.

Regresi frontend mereproduksi tujuh kegagalan pada kode sebelum perbaikan, lalu seluruh suite dijalankan ulang dengan kode dibekukan. Tes native memakai pool WARP tanpa capture item/session, membuktikan referensi event benar-benar dilepas saat rollback dan pesan Stop awal tetap diterima. Tes apartment memeriksa finalizer berjalan sebelum runtime ditutup serta pekerjaan tidak dijalankan/dibuang saat inisialisasi gagal. Regresi encoder memeriksa keluaran file dan stream melalui Finish maupun Drop, termasuk decode seluruh frame dan warna frame terakhir. Tes diagnostik membuktikan arsip, error asli, serta file media tetap dipertahankan. Tidak ada tes di sini yang membuktikan monitor fisik telah berhasil merekam setelah perbaikan.

Build rilis `npm run tauri -- build -- --locked` berhasil. Installer 0.4.3 (`target/release/bundle/nsis/Screenrecord_0.4.3_x64-setup.exe`) berukuran 56.153.021 byte (53,55 MiB), dengan SHA-256 `41d1a57cec9a6ed26d45a2abd0063411c25b7d8fc03b823d1667e5ffa84db04d`.

Inspeksi tanpa menjalankan installer/aplikasi membuktikan executable x64 versi 0.4.3 cocok dengan hasil build setelah penanda bundle NSIS; 47 resource serta notice/lisensi cocok checksum; FFmpeg dan FFprobe hasil ekstraksi berhasil menjalankan `-version`. Paket masih **unsigned**. Hash 19 file sumber terpilih juga cocok dengan snapshot sebelum bundling. Bukti: build (`artifacts/audit-0.4.3/release-build.log`), inspeksi paket (`artifacts/installer-evidence-0.4.3.json`), dan ringkasan verifikasi (`artifacts/audit-0.4.3/verification-evidence.json`).

## Uji manual berikutnya

1. Pasang 0.4.3, pilih monitor yang bermasalah, nonaktifkan webcam/mikrofon, aktifkan computer audio, lalu rekam 10-15 detik. Ulangi tiga kali dengan Stop normal.
2. Ulangi sekali pada desktop yang diam dan sekali sambil mengubah isi layar. Periksa apakah kegagalan bergantung pada perubahan isi desktop atau keadaan monitor.
3. Jika startup masih gagal, folder percobaan sekarang menyimpan `part-00000/capture-startup-diagnostic.json`, `recording-failure.json`, dan `startup-failure.json`. Data tersebut membedakan tidak ada event, tersendat sebelum callback, serta kegagalan encoder/GPU. File bersifat lokal dan tidak diunggah otomatis.
4. Setelah kasus layar saja berhasil, uji webcam/mikrofon, Pause/Resume, Stop dari kontrol mengambang, dan rekam ulang. Periksa bahwa error sesi gagal tetap terlihat dan proyek lama tidak terbuka sebagai hasil rekaman baru.

Rujukan native dan detail patch tersedia pada [catatan vendor](../../vendor/windows-capture/SCREENRECORD-PATCH.md). Pengujian perangkat, WebView2 native, clean install/uninstall, dan signing tetap memerlukan bukti terpisah.
