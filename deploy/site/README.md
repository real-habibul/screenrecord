# Hosting situs Screenrecord

Situs ini menyajikan berkas statis dari `site/`. Container Nginx terhubung ke jaringan eksternal `coolify`. Traefik menangani HTTPS untuk `screenrecord.88apps.dev`.

Simpan `compose.yaml` dan `nginx.conf` di direktori deployment. Untuk setiap rilis, siapkan satu direktori di `www/releases/` dengan struktur berikut.

```text
index.html
site.css
assets/
LICENSE
THIRD_PARTY_NOTICES.md
download/Screenrecord_0.7.4_x64-setup.exe
download/Screenrecord_0.7.4_x64-setup.exe.sha256
source/Screenrecord-0.7.4.zip
source/Screenrecord_0.7.4_third-party-sources.zip
source/Screenrecord_0.7.4_ffmpeg-build-inputs.zip
```

Bangun ZIP kode dari commit rilis dengan `git archive`. Sebelum mengaktifkan rilis, cocokkan checksum installer dan ketiga ZIP dengan berkas lokal. Arahkan symlink relatif `www/current` ke direktori rilis dengan penggantian atomik. Untuk rollback, arahkan symlink kembali ke rilis sebelumnya.

Jalankan `docker compose config -q` dan `docker compose run --rm --no-deps --entrypoint nginx web -c /etc/nginx/screenrecord.conf -t` saat konfigurasi berubah. `docker compose up -d` menjalankan layanan. Setelah aktivasi, periksa judul halaman, tautan sumber, lisensi, dan checksum berkas melalui URL publik.
