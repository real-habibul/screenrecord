# Berkontribusi ke Screenrecord

Anda bisa membuka issue untuk melaporkan masalah atau mengusulkan fitur. Kami juga menerima pull request untuk kode, tes, dan dokumentasi. [Daftar issue](https://github.com/real-habibul/screenrecord/issues) · [Pull request](https://github.com/real-habibul/screenrecord/pulls)

## Melaporkan masalah

Sertakan versi Screenrecord dan Windows, langkah yang memicu masalah, hasil yang Anda harapkan, serta apa yang terjadi. Untuk masalah perekaman, sebutkan sumber yang dipilih dan perangkat audio atau kamera yang aktif. Untuk masalah editor, sebutkan waktu di timeline dan apakah gejala yang sama terlihat di MP4 hasil ekspor.

Contoh rekaman singkat yang dibuat khusus untuk laporan biasanya cukup. Periksa isi rekaman, tangkapan layar, dan log sebelum membagikannya. Hapus informasi pribadi dan kunci API.

## Menjalankan aplikasi dari sumber

Gunakan Windows 11 x64. Siapkan Node.js 22.12 atau lebih baru beserta npm, rustup dengan target `x86_64-pc-windows-msvc`, Visual Studio C++ Build Tools dengan Windows SDK, dan Microsoft Edge WebView2 Runtime. Saat Anda menjalankan Cargo, rustup memilih Rust 1.98.0 sesuai dengan versi yang digunakan untuk membangun rilis ini. Dari direktori utama repositori, jalankan di PowerShell.

```powershell
npm ci
.\scripts\prepare-media.ps1
npm run tauri -- dev
```

Skrip persiapan mengunduh FFmpeg dan FFprobe serta memeriksa SHA-256 arsip. Perintah terakhir membuka aplikasi desktop beserta backend perekaman.

## Memilih pengujian

| Perubahan | Perintah |
| --- | --- |
| Logika editor Vue dan TypeScript | `npm test` dan `npm run build` |
| Interaksi editor di browser | `npm run test:ui` |
| Kode Rust | `cargo test --locked -p screenrecord-core`, dengan nama crate yang Anda ubah |

Tes browser memakai Playwright dan Chrome. Jika lokasi instalasinya berbeda dari lokasi standar Windows, atur `BROWSER_PATH` ke executable Chrome atau Edge. Untuk menjalankan rangkaian verifikasi yang dipakai CI, gunakan perintah berikut. Skrip ini juga menjalankan tes media sintetis dengan FFmpeg.

```powershell
.\scripts\verify.ps1
```

Untuk menguji satu kelompok saja, jalankan `.\scripts\verify.ps1 -Suite Editor` atau `.\scripts\verify.ps1 -Suite Native`. GitHub menampilkan hasil kedua kelompok secara terpisah dan membangun installer setelah keduanya lulus.

Untuk membuat installer sekaligus memeriksa isinya, jalankan kedua perintah berikut. Pemeriksaan paket memerlukan 7-Zip. Installer dan berkas SHA-256 akan tersedia di `target\release\bundle\nsis\`.

```powershell
.\scripts\verify.ps1 -BuildInstaller
.\scripts\inspect-installer.ps1
```

Tes otomatis menjalankan editor di browser headless dan memproses media sintetis. Seorang pengguna juga telah mencoba versi 0.7.4 di komputer Windows miliknya dan melaporkan bahwa aplikasi berfungsi. Jika Anda menguji pada perangkat lain, sertakan konfigurasinya agar hasil dapat dibandingkan. [Pengujian 0.7.4](docs/RELIABILITY-0.7.4.md) · [Skenario uji perangkat](docs/CHECKLIST-RILIS-0.7.4.md)

## Mengirim pull request

Buat branch di fork Anda, lalu buka pull request ke repositori Screenrecord. Jelaskan masalah yang ditangani, perubahan perilaku, dan tes yang telah dijalankan. Untuk perubahan visual, tangkapan layar sebelum dan sesudah akan membantu peninjauan. [Repositori Screenrecord](https://github.com/real-habibul/screenrecord)

## Peta kode

| Jika Anda mengubah | Mulai dari |
| --- | --- |
| Perekaman layar, kamera, atau audio Windows | [crates/capture](crates/capture) |
| Format proyek, validasi, dan penyimpanan | [crates/core](crates/core) |
| Ekspor MP4, filter video, dan audio hasil ekspor | [crates/render](crates/render) |
| Transkripsi lokal dan cloud | [crates/transcribe](crates/transcribe) |
| Perintah Tauri dan integrasi perekaman | [src-tauri/src/app.rs](src-tauri/src/app.rs) |
| Preview dan interaksi di kanvas | [src/components/Preview.vue](src/components/Preview.vue) |
| Dialog, preset, dan status ekspor video | [src/video-export.ts](src/video-export.ts) |
| Tes editor di browser | [tests/ui](tests/ui) |

Perubahan pada format proyek atau pesan antara frontend dan backend perlu mengikuti [kontrak proyek dan IPC](docs/project-contract.md). Daftar panduan dan laporan versi tersedia di [indeks dokumentasi](docs/README.md).
