# Perbaikan 0.1.1

Perubahan ini menangani laporan tombol tutup yang tidak bekerja, noise pada Computer audio, dan label sidebar yang tidak diperlukan.

## Tutup aplikasi

Frontend mendaftarkan handler tutup sebelum memuat daftar proyek. Handler menyimpan perubahan, menampilkan status saat menunggu, lalu memanggil `close_app` pada backend. Permintaan berulang tidak membuat beberapa operasi tutup. Kegagalan save atau backend tetap terlihat dan tidak menghapus edit. Jika save terlambat, permintaan tutup kedaluwarsa tidak boleh menutup aplikasi belakangan. Kontrol editor dinonaktifkan saat menunggu agar edit baru tidak tertinggal setelah save.

Backend menolak tutup segera bila operasi media sedang berlangsung. Build 0.1.1 menyatukan frontend dan backend yang sesuai. Proses debug lama yang masih terbuka tidak berubah menjadi backend baru hanya karena frontend menerima hot reload.

## Computer audio

Analisis file yang dilaporkan pengguna menemukan 1.969 celah nol pendek di antara paket audio. Banyak celah muncul sekitar setiap 10 ms. Format WAV valid, sampel finite, dan sinyal tidak clipping. Filter ekspor mempertahankan waveform sumber dengan perbedaan numerik kecil, sehingga penyebab berada pada capture.

Sebelumnya, setiap timestamp QPC paket dibulatkan menjadi posisi sampel. Jitter timestamp menyebabkan penyisipan nol atau pembuangan sampel di tengah sinyal yang sebenarnya kontinu. Clock audio baru memakai QPC untuk posisi awal dan posisi frame perangkat untuk kesinambungan paket berikutnya. Kehilangan paket dan diskontinuitas yang nyata tetap ditangani. Buffer endpoint diberi ruang 200 ms; polling tetap 4 ms.

Perubahan berlaku pada rekaman baru. Rekaman asli pengguna tidak diubah; noise yang sudah tersimpan pada file lama tetap menjadi bagian dari sumber tersebut.

## Sidebar

Teks `Stored on this computer` dihapus. Tombol `Open projects folder` tetap tersedia.

## Verifikasi

Hasil akhir tes dan identitas installer dicatat pada [QA](QA.md). Analisis waveform tersimpan di `artifacts/audio-noise-audit/`; hasil tes audio baru berada di `artifacts/audio-hotfix/`. Artefak audio lokal tidak disertakan ke paket aplikasi atau Git.

Pengujian native tombol X masih tertunda karena kontrol komputer dihentikan lewat Escape. Tes otomatis alur tutup tidak menggantikan observasi tombol X pada build Windows yang dipasang.
