# Editor audio dan panduan vertikal 0.7.0

## Suara game dan mikrofon

Di **Audio**, centang **Lower computer audio while microphone speaks** lalu atur **Reduction**. Musik impor memiliki kotak pilihan tersendiri. Keduanya mengikuti level mikrofon setelah gain, fader, dan keyframe. Simpan proyek untuk memakai pengaturan yang sama ketika dibuka kembali.

Di **Timeline > Audition**, pilih **Microphone only**, **Computer only**, atau **Full mix**. Seret pada baris **Loop range**, atau tentukan **In** dan **Out** dalam detik video hasil edit. Aktifkan **Loop selection** lalu tekan Play. **From playhead** memindahkan rentang ke posisi pemutaran. Menutup Audition mengembalikan preview ke Full mix. MP4 selalu menggunakan campuran proyek.

Di **Timeline > Volume keyframes**, pilih track, pindahkan playhead, lalu tekan **Add at playhead**. Misalnya, tambahkan titik 100% sesaat sebelum suara ledakan, 30% pada awal dan akhir ledakan, lalu 100% setelahnya. Seret titik untuk mengubah waktu dan level. Gunakan kolom angka untuk posisi presisi. Panah kiri dan kanan menggeser satu frame. Panah atas dan bawah mengubah satu persen. Shift memperbesar langkah.

Level 100% mengikuti fader track. Segmen di antara dua titik berubah secara linear. Volume sebelum titik pertama dan sesudah titik terakhir mengikuti titik terdekat. Keyframe melekat pada waktu rekaman asli ketika video dipotong. **Reset curve** menghapus seluruh titik pada track yang dipilih. Undo memulihkannya.

## Membuka proyek panjang

Waveform dan statistik audio mentah disimpan di subfolder `.audio-analysis` pada proyek untuk WAV float32 yang digunakan recorder dan musik hasil impor. Perubahan trim atau cut memakai ringkasan per blok dan membaca sampel di batas potongan. **Analyze** memindai ulang sumber. Format lain memakai analisis FFmpeg. Cache yang hilang atau rusak dapat dibangun kembali.

Pengukuran berikut memakai satu WAV stereo sintetis 48 kHz berdurasi satu jam di Windows, dengan build release dan cache filesystem Windows yang sudah hangat. Setiap pengukuran berjalan pada proses baru. Angka di tabel adalah median tiga putaran.

| Operasi analisis | Pemindaian lama | Memakai indeks tersimpan |
| --- | ---: | ---: |
| Buka kembali, seluruh rekaman | 2.516 ms | 3,54 ms |
| Hitung ulang setelah trim | 2.528 ms | 3,05 ms |

Pemindaian dan pembuatan indeks pertama memerlukan 2,49 detik. WAV berukuran 1.382.400.044 byte menghasilkan indeks 1.416.077 byte. Jika waktu menjalankan proses uji ikut dihitung, median buka kembali berubah dari 2,56 detik menjadi 55 ms. Angka ini mengukur analisis audio. Waktu membuka antarmuka, video, dan media proyek ikut menentukan durasi yang dirasakan pengguna. [Tes pengukuran cache satu jam](../crates/capture/src/analysis/cache/tests.rs)

Pada uji stres renderer debug, 60 detik audio dengan ducking selesai dalam median 1,25 detik. Memakai seluruh 2.048 titik pada masing-masing dari tiga track memerlukan 1,71 detik. Pengukuran mencakup decode, pemrosesan dan penulisan PCM. Tiga pasang pengukuran berjalan bergantian saat suite lain aktif. [Kasus uji renderer](../crates/render/src/volume_tests.rs)

## Area aman untuk video vertikal

Untuk kanvas 9:16, pilih **Safe area > TikTok** atau **Instagram Reels** di samping **Preview scale**. Tempatkan wajah dan tulisan penting di dalam garis. Bagian teduh memperkirakan area yang terpakai oleh judul, caption dan tombol. Panduan mengikuti pan dan skala preview. Pilihan disimpan sebagai preferensi editor dan hanya terlihat di preview.

Margin ini adalah perkiraan komposisi. Tata letak posting dapat berubah sesuai perangkat, caption, dan penempatan. Pada spesifikasi in-feed, TikTok juga membedakan tata letak menurut caption dan format. [Panduan TikTok](https://ads.tiktok.com/resources/help/article/tiktok-auction-in-feed-ads)

## Verifikasi Windows x64

Sebanyak 402 tes Rust, 174 tes unit frontend dan 196 tes browser lulus. Pengujian memakai media sintetis. Regresi perpindahan fokus dengan Tab membatalkan drag keyframe sebelum edit berikutnya. Tes sinkronisasi awal pemutaran juga lulus sepuluh kali dengan jeda video buatan 180 ms. [Tes interaksi editor](../tests/ui/editor.spec.ts)

Installer NSIS berukuran 56.345.031 byte. Pemeriksaan paket memverifikasi aplikasi x64, tujuh aset frontend, 47 resource, dan checksum SHA-256. Status Authenticode adalah `NotSigned`. Instalasi, upgrade, rekaman dengan perangkat fisik, dan tampilan posting pada ponsel tujuan masih memerlukan uji manual. [Skrip inspeksi installer](../scripts/inspect-installer.ps1)
