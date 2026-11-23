# Perubahan 0.3.1

> Arsip pengembangan. Lihat [unduhan publik](https://github.com/real-habibul/screenrecord/releases) untuk versi yang tersedia saat ini.

> Jalur `artifacts/` dan `target/` merujuk ke hasil lokal dari pengujian saat itu. Panduan untuk menjalankan pemeriksaan saat ini ada di [panduan kontribusi](../../CONTRIBUTING.md#memilih-pengujian).

## Delete proyek

Klik kanan proyek atau gunakan tombol Project actions pada barisnya, lalu pilih Delete project. Keyboard menyediakan Shift+F10/ContextMenu dan Escape. Dialog konfirmasi menampilkan nama proyek, dengan Cancel sebagai fokus awal.

Konfirmasi memindahkan seluruh folder proyek, rekaman dan aset di dalamnya ke Windows Recycle Bin. Hasil ekspor yang disimpan di luar folder proyek tetap ada. Pemulihan dilakukan dari Recycle Bin, kemudian tekan Refresh pada daftar proyek. Ini terpisah dari undo perubahan editor.

Penghapusan diblokir saat perekaman atau operasi media aktif. Edit tertunda disimpan dan decoder preview dilepas sebelum permintaan dikirim. Jika operasi gagal, editor mempertahankan edit dan menampilkan error untuk retry. Backend tidak menyediakan fallback hapus permanen. Drive yang tidak mendukung Recycle Bin harus ditangani melalui Explorer oleh pengguna.

Folder diverifikasi terhadap library, UUID dan identitas manifest. Folder/entri symlink, junction, dan reparse point ditolak; backend tidak mengikuti path media eksternal dari manifest. Request library lama tidak dapat menghidupkan kembali proyek yang telah dihapus.

Implementasi Shell mengikuti [IFileOperation flags](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperation-setoperationflags), dengan flag recycle, early failure dan pemeriksaan operasi yang dibatalkan. Callback [PreDeleteItem](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperationprogresssink-predeleteitem) menolak item tanpa flag recycle. Hasil per item baru dianggap berhasil bila [PostDeleteItem](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperationprogresssink-postdeleteitem) menyediakan tujuan Recycle Bin dan tidak melaporkan error. Tidak memakai perintah shell recursive delete.

## Geser dan resize di preview

Mask dan kamera dapat dipindahkan langsung pada preview serta diubah ukurannya melalui pegangan. Mask mengikuti koordinat layar asli saat crop/zoom; kamera mengikuti kanvas keluaran. Tampilan Fit/100%/200% dan scroll tidak mengubah koordinat edit. Batas posisi/ukuran tetap mengikuti validasi proyek.

Perubahan diterapkan ke input angka pada panel properti. Satu gesture menjadi satu langkah undo; Escape atau pointer cancellation mengembalikan keadaan awal. Kontrol angka tetap tersedia untuk penyesuaian tepat dan akses keyboard.

## Verifikasi

Installer final `target/release/bundle/nsis/Screenrecord_0.3.1_x64-setup.exe`, 55.551.642 byte (52,98 MiB), unsigned. SHA-256: `8a08db1de4f5fdceb4e30bfad8cbfe715dd8da5133b074083655c3bd6a6914b6`. Inspeksi ekstraksi memverifikasi EXE x64, checksum 47 resource, source/lisensi limiter, dan runtime FFmpeg/FFprobe. Installer tidak dijalankan. Bukti: `artifacts/installer-evidence-0.3.1.json`.

190 tes otomatis berbeda lulus: 109 Rust/media, 32 unit frontend dan 49 browser. Format, Clippy dengan warnings denied, build/TypeScript dan audit npm produksi lulus; audit npm melaporkan nol kerentanan pada pemeriksaan ini.

Log `artifacts/verification-0.3.1.log` mencatat rangkaian awal yang meluluskan seluruh tes tetapi gagal build saat pemeriksaan recycle tambahan baru dimasukkan. Dependency macro diperbaiki, kemudian callback recycle, sembilan tes integrasi aplikasi, format/Clippy dan build final diverifikasi kembali di `artifacts/finalization-0.3.1.log`. Hitungan 190 adalah tes berbeda, bukan penjumlahan tes yang diulang.

Pengujian mencakup konfirmasi/cancel, edit tertunda, decoder release/unmount sebelum delete, error/retry, proyek terakhir, request lama, Refresh sesudah restore, serta penolakan path/junction dan hasil permanent-delete. Manipulasi canvas diuji pada DPI 2x untuk pergerakan/resize/batas, crop/zoom/scroll, keyboard, Escape dan pembatalan saat busy, properti numerik dan satu langkah Undo.

Tes penghapusan memakai folder fixture milik tes dan pemindah pengganti. Tidak ada proyek pengguna atau isi Recycle Bin pengguna yang diubah. Native Recycle Bin, interaksi Windows, file yang dibuka aplikasi lain, dan drive tanpa recycle tetap perlu diuji manual. Tidak menggunakan computer-use atau kamera fisik.

Bukti visual dengan media sintetis: menu (`artifacts/ui/project-menu-dark.png`), konfirmasi (`artifacts/ui/project-delete-confirmation.png`), pegangan kamera (`artifacts/ui/preview-camera-handles.png`).
