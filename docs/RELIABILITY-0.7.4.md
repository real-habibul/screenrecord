# Hasil pengujian keandalan Screenrecord 0.7.4

Pengujian lokal pada Windows x64, 23 November 2026. Sumber gambar dan suara sintetis. Tidak ada pengambilan layar, kamera, atau mikrofon pengguna. Pemeriksaan perangkat yang tersisa tercantum di [checklist manual](CHECKLIST-RILIS-0.7.4.md).

## Startup dan frame pertama

Perubahan ukuran monitor atau jendela antara pembuatan frame pool dan pemasangan callback dapat membuat ukuran yang dicatat berbeda dari buffer yang sudah dialokasikan. Capture sekarang menggunakan ukuran alokasi aktual, mengabaikan frame sementara berdimensi nol dan melanjutkan saat tersedia frame yang dapat digambar. Jika worker perangkat yang dibutuhkan sudah keluar saat startup, penyebabnya dikembalikan. Jika konsumen encoder masih hidup tetapi antrean penuh dan tidak dikosongkan selama 10 detik, pengiriman berakhir dengan error sehingga aplikasi bisa mulai menangani kegagalan.

Tiga regresi sintetis semula gagal lalu lulus setelah perbaikan. Tes encoder Windows menggunakan frame sintetis, dengan keempat kombinasi keluaran file/stream dan finalisasi eksplisit/pelepasan objek. Decoder membaca 12 frame di setiap kasus. Warna awal dan akhir diperiksa. Dua percobaan fisik terdahulu pada monitor 1920 × 1080, 30 fps, dengan audio komputer belum memiliki diagnostik per tahap. Penyebabnya belum bisa dipastikan dari catatan lama. [Riwayat timeout](AUDIT-0.4.3.md)

## Durasi dan sinkronisasi

- **Pemutaran editor selama 10 menit waktu nyata.** Chrome memutar proyek sintetis 3 jam 2 detik sambil menjalani 19 perubahan volume, kamera dan posisi putar. Pemantau mengambil 3.000 sampel selama 600,022 detik. Dalam sampel ketika trek aktif, selisih maksimum kamera terhadap layar 0,4 ms dan audio terhadap layar 87,521 ms. Tidak muncul error browser atau peringatan editor. Angka ini mengukur jam pemutar browser. Sinkronisasi fisik mikrofon dan kamera memerlukan pengukuran perangkat. Pemantau juga mencatat hasil per 30 detik. [Skenario pemutaran 10 menit](../tests/ui/editor.spec.ts)

- **Preview proyek tiga jam.** Editor menjalankan video H.264 dan audio AAC yang terpisah, melewati bagian yang dipotong sepanjang 7.199 detik, lalu menyimpan dan membuka ulang hasil edit. Tampilan waktu pada 3.600,1 detik semula kehilangan sepersepuluh detik. Regresi kini memeriksa angka pada layar, gambar hasil decode dan sinyal audio setelah potongan.
- **Audio RF64 delapan jam.** Berkas WAV stereo float 48 kHz berukuran logis 11.059.200.092 byte dibuat sebagai sparse file, dengan penanda berbeda di detik 1, 11.200, dan 28.798. FFmpeg, Chrome, dan Edge membaca durasi 28.800 detik serta menghasilkan amplitudo penanda yang sesuai di ketiga titik. Server uji lokal menggunakan permintaan rentang seperti protokol aset editor. Tes unit di repositori juga memeriksa ukuran 64-bit pada header RF64. [Tes format WAV](../crates/capture/src/wav.rs)
- **Ekspor dari sumber delapan jam.** Sumber 29,97 fps dan audio 44,1 kHz diuji dengan empat potongan di awal, sekitar jam pertama dan keempat, serta mendekati akhir. Ekspor 1080p30 sepanjang 36,508 detik menghasilkan 1.096 frame dan 37 pasangan kilatan/bunyi. Deviasi kilatan terhadap bunyi paling besar sekitar 33 ms, setara satu frame keluaran. Ini menguji pengalamatan media panjang dan batas potongan. [Skenario ekspor dan pemeriksaan penanda](../crates/render/src/reliability_tests.rs)

Uji lain menghasilkan **MP4 satu jam** dengan memakai ulang 120 potongan video berdurasi 30 detik dan mengenkode seluruh audio. Hasilnya memuat 108.000 frame 1080p30. Audio dan video mulai di detik nol dan berakhir di detik 3.600. Kilatan dan nada diperiksa di awal, tengah, dan menjelang akhir.

## Jika penyimpanan gagal

Volume uji dibatasi 1 MiB dalam tmpfs WSL. Tes menjalankan kode Windows, memenuhi volume privat ini, memeriksa kapasitas yang tersisa, melepas berkas pengisi lalu mencoba kembali. Pada komputer ini WSL meneruskan kegagalan tulis sebagai error Windows 31. Untuk ekspor, Windows melaporkan sisa nol byte. Hasil ini menguji kegagalan tulis nyata. Perilaku seluruh driver penyimpanan Windows tetap memerlukan pengujian di perangkat.

- Jika ruang habis sebelum rekaman dimulai, versi sebelumnya meninggalkan direktori kosong. Sekarang aplikasi menulis dan menyinkronkan penanda pemulihan sebelum membuat direktori rekaman. Percobaan berikutnya berhasil setelah ruang dibebaskan. [Tes pembuatan proyek dan pemulihan setelah disk penuh](../src-tauri/src/app.rs)
- Penyimpanan ulang menjaga manifest sebelumnya serta media asal. Di Windows, manifest read-only ditolak dan berhasil disimpan kembali setelah akses dipulihkan. Saat penyimpanan otomatis bertabrakan dengan penutupan editor, perubahan terakhir tetap dapat disimpan dan pesan disk penuh hilang setelah berhasil. [Tes penyimpanan manifest](../crates/core/src/storage.rs) · [Tes penyimpanan otomatis saat editor ditutup](../tests/ui/editor.spec.ts)
- Penulis audio PCM mengembalikan kegagalan ketika volume penuh. Sebanyak 29 paket yang sebelumnya sudah ditulis dan disinkronkan dapat didekode kembali dengan setiap nilai sampel yang sama. Setelah tersedia ruang, pembuatan WAV baru berhasil.
- **Pemulihan rekaman yang dijeda.** Dua bagian rekaman masih utuh, tetapi hanya bagian pertama yang sudah selesai menjadi MP4. Pada volume yang penuh, kode lama mengembalikan bagian pertama saja seolah pemulihan selesai. Regresi ini direproduksi sebelum perbaikan. Sekarang kegagalan menulis hasil menghentikan pemulihan dan mempertahankan penandanya. Setelah ruang tersedia, kedua bagian utuh terbaca lagi. Pengujian juga mendekode 15 frame video yang dipulihkan dari satu segmen dan mencocokkan byte sumber.
- Saat ekspor MP4 kehabisan ruang, aplikasi menyertakan jumlah ruang yang tersisa pada pesan gagal menulis. Tidak ada MP4 parsial di tujuan. Setelah tersedia ruang, ekspor ulang ke tujuan yang sama dapat diputar. [Tes ekspor saat ruang disk habis](../crates/render/src/reliability_tests.rs)

Pengujian terpisah memakai kunci baca eksklusif Windows pada segmen layar, rekaman kamera, WAV mikrofon, WAV audio komputer, dan metadata kursor. Sebelum perbaikan, kelima kunci dapat menghasilkan pemulihan yang tampak sukses dengan data yang terlewat. Sekarang pemulihan menunggu sampai data bisa dibaca kembali. Kasus berkas opsional hilang dan WAV yang benar-benar rusak tetap diuji. [Tes kunci eksklusif dan pemulihan beberapa bagian](../crates/capture/src/media_tests.rs)

## Menjalankan ulang

Jalankan perintah di bawah dari direktori utama repositori setelah memasang dependensi dan menyiapkan FFmpeg. [Panduan build dan tes](../CONTRIBUTING.md)

Untuk menguji potongan pada proyek tiga jam dan pemutaran selama sepuluh menit di Chrome, jalankan dua skenario Playwright berikut. Tes membuat sendiri media sintetisnya.

```powershell
$env:SCREENRECORD_UI_SOAK = '1'
npm run test:ui -- tests/ui/editor.spec.ts -g 'a three-hour source|ten-minute editor playback soak'
Remove-Item Env:\SCREENRECORD_UI_SOAK
```

Tes renderer berikut memeriksa empat potongan dari sumber delapan jam, lalu membuat keluaran satu jam dan memeriksa kilatan serta bunyi pada hasil dekode.

```powershell
$env:SCREENRECORD_FFMPEG = (Resolve-Path .\src-tauri\resources\ffmpeg.exe).Path
cargo test -p screenrecord-render --lib --locked eight_hour_source_preserves_flash_and_tone_sync_across_fractional_cuts -- --ignored --nocapture
```

Untuk empat tes kuota, jalankan `.\scripts\test-disk-full.ps1`. Skrip memakai volume sementara 1 MiB di WSL Ubuntu. Parameter `-Distribution` menerima nama distribusi WSL lain yang memiliki `mount` dan `stat`. [Skrip tes kuota](../scripts/test-disk-full.ps1)

## Hasil kandidat sebelum pengemasan publik

| Pemeriksaan | Hasil |
| --- | --- |
| Editor | 175 tes unit, 228 tes Chrome dan 4 skenario terarah di Edge lulus. |
| Backend | 277 tes standar aplikasi, core, capture dan transkripsi lulus. Suite renderer lulus 90 tes pustaka dan 5 tes integrasi. |
| Pengujian tambahan | 4 tes kuota, 17 tes media capture opt-in dan 15 tes standar pustaka Windows lulus. |
| Build | Pemeriksaan format dan Clippy lulus. Frontend dan installer rilis berhasil dibangun. Audit npm untuk dependensi produksi melaporkan nol temuan. |
| Inspeksi kandidat | Installer NSIS Windows x64 0.7.4 berukuran 56.363.267 byte. EXE di dalam paket cocok dengan hasil build. Pemeriksaan memverifikasi tujuh aset frontend dan 47 berkas pendukung. Installer belum ditandatangani. Pemeriksaan mengekstrak isi paket tanpa memasangnya. |

SHA-256 kandidat yang diperiksa sebelum pengemasan publik adalah `a97f69ecb105b7bc667cd3bc4e2d6d7936b6de4ad2b5b350e6e532bc12d146d4`. Untuk memeriksa installer yang diunduh, gunakan berkas SHA-256 yang menyertai paket publik. [Rilis Screenrecord](https://github.com/real-habibul/screenrecord/releases)

Daftar perintah untuk suite standar tersedia di skrip verifikasi. Opsi `-BuildInstaller` juga membangun installer dan menghitung SHA-256. Skrip inspeksi membandingkan isi paket dengan hasil build. [Verifikasi Windows](../scripts/verify.ps1) · [Inspeksi installer](../scripts/inspect-installer.ps1)
