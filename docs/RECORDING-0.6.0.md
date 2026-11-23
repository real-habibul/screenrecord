# Persiapan rekaman 0.6.0

## Merekam Roblox atau jendela lain

1. Buka **New recording > Apps**, pilih aplikasi, lalu pilih jendela yang menampilkan game. **Screen** dan **Window** tetap tersedia untuk merekam sumber lain.
2. Aktifkan **Record computer audio**. Pada **Computer audio source**, pilih **Selected application only** untuk merekam suara dari proses aplikasi dan proses turunannya. Jika audio dimatikan lalu dinyalakan kembali untuk target Apps yang sama, pilihan khusus aplikasi tetap berlaku. Pilih mikrofon tersendiri bila ingin merekam komentar. **Whole system** merekam campuran komputer.
3. Gunakan **Choose area** jika hanya sebagian sumber yang ingin direkam. Gambar bingkai pada cuplikan sumber, geser isinya untuk memindahkan, atau tarik sudutnya untuk mengubah ukuran. Angka **Left**, **Top**, **Width** dan **Height** memakai piksel sumber. Konfirmasi dengan **Use this area**. **Full source** mengembalikan seluruh sumber. Pilihan ini menentukan piksel yang ditulis ke file. Bagian di luar bingkai tidak bisa dibuka kembali di editor.
4. **Test 5 seconds** memakai bingkai, frame rate, webcam, mikrofon, dan sumber audio yang sama dengan tombol **Start recording**. Editor mengecil selama pengambilan. Setelah klip siap, putar video lalu dengarkan audio komputer atau mikrofon secara terpisah. Saat shortcut tersedia, **Ctrl+Shift+R** membatalkan tes yang sedang berjalan.
5. Klik **Start recording** untuk memulai proyek. Cuplikan tes tersimpan sementara di cache aplikasi dan dibuang ketika diganti, dibatalkan atau ditutup. Jika proses Windows berhenti mendadak, aplikasi membersihkan cuplikan miliknya ketika dibuka kembali.

Untuk klip vertikal dari monitor 1920 × 1080, contoh ukuran yang pas adalah **540 × 960**. Isi ukuran itu, geser bingkai ke area permainan, lalu periksa hasilnya dengan **Test 5 seconds**. Jika ingin menyusun versi horizontal dan vertikal dari materi yang sama, rekam sumber penuh dan gunakan **Reframe** di editor.

Windows menyaring audio berdasarkan pohon proses. Jika aplikasi memakai layanan audio dari proses terpisah, jalur aplikasi bisa sunyi. Dengarkan cuplikan tes untuk memeriksanya. Ketika proses terpilih hilang atau aktivasi audio gagal, aplikasi memberikan pesan dan mempertahankan cakupan yang dipilih. [Contoh resmi Microsoft](https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/)

## Audit ketiga fitur

| Fitur | Yang diperiksa |
| --- | --- |
| Tes lima detik | Durasi dihitung sejak frame layar pertama. Pengaturan tes sama dengan rekaman biasa. Pembatalan saat persiapan menghentikan tahap berikutnya, dan hasil terlambat tidak tampil. Browser memutar media sintetis, berpindah jalur audio, melepas decoder saat klip dibuang, dan melanjutkan ke rekaman biasa. Cache yang masih dikunci Windows menyimpan penanda untuk dibersihkan pada pembukaan berikutnya. |
| Suara aplikasi | Parameter WASAPI hanya menyertakan proses terpilih dan turunannya. Pemilik jendela diperiksa saat mulai dan selama rekaman. Pengujian mencakup PID yang berubah, proses berakhir, callback terlambat, pembatalan, paket senyap, dan kesinambungan timestamp. |
| Bingkai bebas | Drag pertama menggambar, drag di dalam pilihan memindahkan, dan sudut mengubah ukuran. Input angka dan tombol panah juga berfungsi. Tes piksel memeriksa tepi, frame statis, penggunaan kembali tekstur, serta posisi kursor. Jika sumber menyusut hingga bingkai keluar dari batasnya, perekaman melaporkan kesalahan. |

## Pengukuran performa

Pada build produksi 0.5.2 dan 0.6.0 final, waktu dari klik **New recording** sampai tombol **Start recording** aktif diukur 24 kali per versi secara bergantian setelah tiga pemanasan. Browser menggunakan daftar perangkat tiruan. Median berubah dari **3,1 ms menjadi 3,2 ms** dan persentil ke-95 dari **4,3 ms menjadi 4,1 ms**. Kedua versi membuka dialog tanpa memulai akuisisi media. [Data per pengulangan](../artifacts/recording-0.6.0/setup-performance-release.json)

Uji editor memakai klip sintetis, empat skenario, dan tiga pengulangan per versi. Di preview 1080p60 selama enam detik, median waktu kerja thread utama adalah **1.113,6 ms** pada 0.5.2 dan **977,9 ms** pada 0.6.0. Persentil ke-95 waktu komposisi per frame bergerak dari **1,2 ms ke 1,1 ms**. Browser melaporkan **1 dari 1.120** frame terlewat pada 0.5.2 dan **3 dari 1.119** pada 0.6.0. Pada 30 fps keduanya mencatat nol frame terlewat. Persentil ke-95 saat menggeser playhead bergerak dari **64,3 ms ke 62,1 ms**. [Pengukuran editor dan hasil setiap pengulangan](../artifacts/perf-0.5.1/editor-final-0.6.0.json)

Pada uji salinan piksel sintetis, 60 penyalinan dari gambar 1920 × 1080 dengan D3D11 WARP memerlukan median 135,9 ms untuk seluruh gambar dan 22,0 ms untuk area 640 × 360. Setiap batch menyertakan pembacaan kembali hasil agar pekerjaan benar-benar selesai. Pengujian ini mengukur biaya penyalinan pada perangkat lunak. FPS permainan, penggunaan CPU/GPU saat merekam, dan waktu akuisisi sumber Windows masih memerlukan uji perangkat. [Pengukuran area dan thumbnail](../artifacts/recording-0.6.0/region-evidence.json)

## Verifikasi otomatis

Sebanyak **703 tes otomatis lulus**. Cakupannya mencakup 255 tes aplikasi, core, capture, dan transkripsi, 86 tes render dan integrasi ekspor, satu tes pembaca proyek contoh, 17 tes media tambahan, 13 tes komponen Windows, 159 unit test frontend, dan 172 skenario browser. Tes browser memakai sumber dan media sintetis. [Rincian verifikasi](../artifacts/recording-0.6.0/verification.json)

Satu ekspor kamera berbasis AMD AMF mencapai batas 180 detik pada percobaan paralel. Kasus yang sama lulus secara terpisah, seluruh rangkaian render lulus berurutan, dan pengulangan khusus AMF menghasilkan 54 frame yang seluruhnya berhasil didekode. Penyebab timeout awal belum terkonfirmasi. [Bukti pengulangan AMF](../artifacts/recording-0.6.0/render-verification.json)

Installer NSIS Windows x64 berukuran **56.293.533 byte**, bertambah 73.874 byte dari 0.5.2. Pemeriksaan setelah ekstraksi mencocokkan versi 0.6.0, executable rilis, tujuh aset frontend, dan 47 berkas runtime beserta lisensi yang diperiksa oleh skrip. FFmpeg dan FFprobe hasil ekstraksi dapat dijalankan. Installer belum ditandatangani secara digital. SHA-256 tersimpan di samping installer. [Bukti pemeriksaan paket](../artifacts/recording-0.6.0/installer.json)

## Uji perangkat Windows 11

- Putar Roblox dan aplikasi bersuara lain secara bersamaan. Pilih **Selected application only**, lalu dengarkan cuplikan dan MP4 dari rekaman biasa. Bandingkan dengan **Whole system**. Ulangi setelah Pause/Resume dan setelah Roblox dibuka ulang.
- Di **Screen**, **Window** dan **Apps**, tempatkan sesuatu yang mudah dikenali di luar tepi bingkai. Periksa tepi rekaman dan posisi animasi klik di preview serta MP4. Coba monitor kedua dengan skala DPI berbeda dan pindahkan jendela sebelum merekam.
- Pilih webcam dan mikrofon fisik, ucapkan satu kata sambil bertepuk tangan, lalu periksa suara dan gerak di cuplikan. Coba batal saat mulai, jalankan tes ulang, lalu mulai rekaman biasa. Periksa akses perangkat, audio game dan kelancaran bermain pada 60 fps.
