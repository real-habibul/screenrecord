# Editor 0.2.0

Implementasi editor untuk proyek rekaman Screenrecord: track musik, subtitle otomatis/manual, dan penyamaran area layar. Pekerjaan dibagi antara agen UI, agen render, agen transkripsi, dan root untuk penyimpanan/IPC/paket. Tidak ada pengujian computer-use atau aktivasi perangkat capture pada pekerjaan ini.

## Musik

Pilih Import music dari editor. File MP3, WAV, M4A, AAC, FLAC, OGG, dan Opus dinormalisasi menjadi WAV stereo float32 48 kHz di folder proyek. File asal tidak diubah. Batas impor 512 MiB dan satu jam; konversi memiliki batas waktu dua menit. Hasil WAV membutuhkan sekitar 384.000 byte per detik, atau 23 MB per menit.

Setiap klip mempunyai posisi di rekaman, titik awal dalam lagu, durasi, volume, fade in, dan fade out. Maksimum 32 klip. Musik, mikrofon, dan audio komputer melewati limiter yang sama. Undo menghapus klip dari edit tetapi mempertahankan asetnya agar redo dan proyek tetap aman.

## Subtitle

Generate subtitles memakai Deepgram Nova-3. Pilih audio mikrofon atau komputer dan bahasa Indonesia, Inggris, atau otomatis. Tombol Upload audio and generate mengirim audio pada trim aktif menggunakan API key yang dimasukkan untuk pekerjaan itu. Key tidak disimpan dalam proyek atau pengaturan dan tidak dicatat dalam log. Layanan dapat mengenakan biaya; pembatalan tidak menarik kembali audio atau pekerjaan yang sudah diterima penyedia.

Permintaan mengaktifkan `mip_opt_out=true`. Model mendukung bahasa Indonesia dan Inggris menurut [dokumentasi model](https://developers.deepgram.com/docs/models-languages-overview); parameter opt-out didokumentasikan pada [Model Improvement Partnership](https://developers.deepgram.com/docs/the-deepgram-model-improvement-partnership-program). Ini tidak mengubah rekaman, webcam, musik impor, atau ekspor lokal menjadi upload otomatis.

Audio diproses berurutan dalam potongan maksimum 600 detik dengan konteks batas. Hasil memiliki waktu kata sumber dan disusun menjadi cue yang bisa dikoreksi. Kegagalan, pembatalan, atau audio tanpa ucapan mempertahankan subtitle yang ada. Penggantian subtitle merupakan satu langkah undo.

Cue dapat diedit teks dan waktunya; warna, latar, ukuran Segoe UI, serta posisi atas/bawah dapat diatur. Preview memakai raster libass yang sama dengan ekspor. SRT diekspor dengan waktu dimulai dari trim; berkas yang sudah ada tidak ditimpa. Batas: 10.000 cue, 500 karakter/3 baris per cue, 1 MiB total teks, dan tidak ada cue bertumpuk.

## Blur dan blok solid

Gambar persegi panjang pada preview, pilih blur atau solid, lalu atur waktu aktif dan ukurannya. Koordinat mengikuti konten layar asli saat crop/zoom, bukan posisi jendela aplikasi. Maksimum 100 area dengan 16 aktif bersamaan. Blok solid berwarna hitam pekat lebih sesuai untuk password, nomor identitas, dan isi form yang harus tertutup penuh. Blur dapat menyisakan pola atau bentuk teks.

Area belum mengikuti objek yang bergerak otomatis. Perubahan layout halaman, scroll, atau posisi form perlu ditangani dengan area dan interval terpisah. Efek hanya diterapkan pada layar rekaman; track webcam dan subtitle berada di atasnya. Kernel blur browser dan FFmpeg berbeda, sehingga tampilan blur tidak dijanjikan identik per piksel; cakupan dan blok solid diuji terpisah.

## Penyimpanan dan batas rilis

Format proyek naik menjadi versi 3. Proyek versi 1/2 dibuka tanpa penulisan dan dimigrasikan saat save. Aplikasi versi sebelumnya menolak versi 3 sehingga tidak menghapus track baru secara diam-diam. Cadangkan seluruh folder proyek sebelum membuka hasil edit dengan build lain. Aset musik tetap tersimpan walaupun klip dihapus atau di-undo; pembersihan aset otomatis belum disediakan.

Installer masih unsigned. Pembuktian perangkat, clean install/uninstall, serta distribusi lisensi/sumber dari rilis sebelumnya tetap terbuka. Tidak ada klaim siap produksi berdasarkan tes otomatis saja.

Installer final: [Screenrecord_0.2.0_x64-setup.exe](../target/release/bundle/nsis/Screenrecord_0.2.0_x64-setup.exe), 55.477.384 byte (52,91 MiB). SHA-256: `72998fac7827ef94c282135baf52ef0c641ca6878899c91cf950c5df6b290ae2`. Pemeriksaan ekstraksi membuktikan aplikasi x64 sesuai build, 47 resource cocok checksum, source limiter/lisensi tersedia, dan FFmpeg/FFprobe bundel dapat dijalankan. Installer tidak dieksekusi dan aplikasi tidak dibuka. Bukti: [installer-evidence-0.2.0.json](../artifacts/installer-evidence-0.2.0.json).

## Verifikasi otomatis

Rangkaian final lulus 150 tes otomatis: 102 Rust/media, 20 unit frontend, dan 28 browser. Format, Clippy dengan warnings denied, build/TypeScript, dan audit npm produksi lulus; audit npm melaporkan nol kerentanan pada pemeriksaan ini. Log: [verification-0.2.0.log](../artifacts/verification-0.2.0.log).

| Cakupan | Bukti |
| --- | --- |
| Core 27 tes | Migrasi v1/v2 ke v3 tanpa tulis saat load, path musik relatif/relokasi/penolakan escape, batas cue/region, metadata delapan jam, save bersamaan |
| Capture 28 tes | 25 unit dan tiga kontrak media sintetis; tidak mengaktifkan perangkat |
| Render 25 tes | 19 library, lima integrasi, satu loader; komposisi gabungan 143 frame, perbandingan chunk, audio decode identik, full decode, sumber tetap utuh |
| Transkripsi 16 tes | 15 unit/mock dan satu FFmpeg/localhost; timestamp sumber, error tersanitasi, batas payload, opt-out, redirect, cancel sebelum/saat upload |
| Aplikasi 5 tes | Musik mono tidak kehilangan gain saat normalisasi, file asal utuh, SRT trim/no-overwrite, pembatalan saat persiapan, publikasi proyek |
| Vendor 1 tes | Backpressure dan EOS antrean encoder |
| Frontend 20 unit + 28 browser | Drag/trim, undo/autosave tertunda, cue search/edit/SRT, job ID lama ditolak, serial raster/retry, musik seek/limiter, mask source coordinates, decode media rekaman yang sudah ada |

Hasil subtitle dengan job ID lama ditolak sehingga tidak mengganti hasil job baru. Preview caption diproses satu per satu dengan cue aktif terlebih dahulu. Undo dibatasi 100 entri/16 MiB; cache raster frontend dibatasi delapan gambar/64 MiB, dan cache PNG disk 128 berkas/64 MiB per proyek. Cache disk dibersihkan hanya untuk nama berkas milik aplikasi.

Tes cloud memakai server HTTP loopback dengan respons simulasi, bukan API berbayar atau rekaman pribadi. Tes browser memakai fixture IPC; tidak menggantikan WebView2 native. Dua benchmark performa lama tidak dijalankan ulang dalam tahap fitur ini. Fixture subprocess yang diabaikan oleh runner utama dijalankan hanya oleh tes pemiliknya.

Uji frame privasi membuktikan blok solid hanya menutup ROI pada frame aktif dan area di luar ROI tetap sama; blur mengurangi variasi pola sintetis lebih dari 99%. Ini tidak membuktikan isi teks sensitif tak dapat ditebak dari blur. Tes PCM menguji fade/offset/overlap dan limiter; pipeline impor mono diuji agar nilai 0,2 tetap 0,2 pada kedua kanal dengan toleransi 1e-5.

## Checklist uji manual

1. Pasang build 0.2.0, buka salinan proyek lama, dan pastikan layar, webcam, mikrofon serta Computer audio tetap dapat diputar. Uji tombol X saat bersih dan setelah edit belum tersimpan.
2. Impor satu MP3 dan satu WAV. Atur posisi lagu, source start, durasi, volume dan kedua fade. Putar sebelum/saat/setelah klip, seek maju/mundur, lalu bandingkan audio hasil MP4.
3. Uji undo/redo untuk perubahan musik, subtitle, dan area; tutup/buka proyek kembali. Pastikan file musik asli di luar proyek tidak berubah.
4. Tambahkan subtitle manual, gunakan beberapa baris dan tanda kurung/backslash. Ubah gaya, crop dan rasio portrait; bandingkan preview dengan MP4. Ekspor SRT dan periksa waktu setelah trim.
5. Dengan API key Deepgram milik Anda, generate dari ucapan pendek Indonesia/Inggris. Periksa teks, jeda, pergantian cue, serta sinkronisasi. Coba key salah, batalkan proses, dan pastikan subtitle lama tetap ada. Uji rekaman lebih dari 10 menit untuk batas potongan.
6. Tutup teks form dengan blok solid, lalu uji blur terpisah. Aktifkan zoom/crop dan cari frame sebelum awal, pada awal, sebelum akhir, dan pada akhir efek. Periksa MP4 frame demi frame sebelum membagikan rekaman sensitif.
7. Ekspor komposisi yang mengandung musik, subtitle, blur, webcam, dan zoom. Periksa audio, gambar, durasi dan cue terakhir. Batalkan ekspor dan periksa sumber tetap utuh.
8. Uji layar 100%/150%/200% DPI, tema gelap/terang, jendela minimum, keyboard, serta proyek panjang. Catat versi build, langkah, durasi, format media, dan pesan error jika ada masalah.
