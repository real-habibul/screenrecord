#!/usr/bin/env bash
set -euo pipefail
if [[ "$#" -lt 2 || "$#" -gt 3 ]]; then
    echo "Usage: bash rebuild-ffmpeg.sh THIRD_PARTY_SOURCES.zip FFMPEG_BUILD_INPUTS.zip [--prepare-only]" >&2
    exit 2
fi
source_zip="$(realpath "$1")"
inputs_zip="$(realpath "$2")"
case "${3:-}" in ''|--prepare-only) ;; *) exit 2 ;; esac
work="$(mktemp -d "${TMPDIR:-/tmp}/screenrecord-ffmpeg-XXXXXXXX")"
echo "Build directory: $work"
unzip -q "$source_zip" -d "$work/sources"
(cd "$work/sources" && sha256sum --quiet -c SHA256SUMS.txt)
mkdir "$work/recipe"
tar -xf "$work/sources/ffmpeg/ffmpeg-builds.tar.gz" -C "$work/recipe" --strip-components=1
cd "$work/recipe"
unzip -q "$inputs_zip"
sha256sum --quiet -c SHA256SUMS.txt
unzip -q "$work/sources/ffmpeg/rav1e-cargo/rav1e-cargo.zip" -d rav1e-material
(cd rav1e-material && sha256sum --quiet -c CRATES-SHA256SUMS)
mv Dockerfile.win64-lgpl-shared-8.1 Dockerfile

python3 - <<'PY'
from pathlib import Path
def replace_once(path, old, new):
    p = Path(path)
    text = p.read_text()
    if text.count(old) != 1:
        raise SystemExit("Unexpected pinned recipe in "+path)
    p.write_text(text.replace(old, new))
replace_once('images/base-win64/Dockerfile', 'cd /ct-ng &&', 'cd /ct-ng && git checkout fbb637297fef385204304f5ee32a168ea6145280 &&')
replace_once('Dockerfile','RUN --mount=src=scripts.d/50-rav1e.sh', 'RUN --mount=src=rav1e-material,target=/rav1e-material --mount=src=scripts.d/50-rav1e.sh')
replace_once('scripts.d/50-rav1e.sh','    cargo update cc','''    mkdir -p "$CARGO_HOME/registry/cache/index.crates.io-1949cf8c6b5b557f"
    cp /rav1e-material/registry-cache/*.crate "$CARGO_HOME/registry/cache/index.crates.io-1949cf8c6b5b557f/"
    tar -xzf /rav1e-material/registry-index.tar.gz -C "$CARGO_HOME"
    cp /rav1e-material/Cargo.lock Cargo.lock
    export CARGO_NET_OFFLINE=true''')
PY

[[ "${3:-}" != --prepare-only ]] || exit 0
repo=ghcr.io/ffmpeg-builds/ffmpeg-builds
docker build --platform linux/amd64 -t "$repo/base:latest" images/base
docker build --platform linux/amd64 -t "$repo/base-win64:latest" images/base-win64
docker build --platform linux/amd64 -t screenrecord-local/ffmpeg-win64-lgpl:8.1 .
mkdir "$work/output"
docker run --rm --platform linux/amd64 \
    -v "$work/sources/ffmpeg/ffmpeg.tar.gz:/source.tar.gz:ro" -v "$work/output:/output" \
    screenrecord-local/ffmpeg-win64-lgpl:8.1 bash -eu -c '
    mkdir -p /ffbuild/source
    cd /ffbuild/source
    tar -xf /source.tar.gz --strip-components=1
    ./configure --prefix=/ffbuild/prefix --pkg-config-flags=--static $FFBUILD_TARGET_FLAGS $FF_CONFIGURE \
        --extra-cflags="$FF_CFLAGS" --extra-cxxflags="$FF_CXXFLAGS" --extra-libs="$FF_LIBS" \
        --extra-ldflags="$FF_LDFLAGS" --extra-ldexeflags="$FF_LDEXEFLAGS" \
        --cc="$CC" --cxx="$CXX" --ar="$AR" --ranlib="$RANLIB" --nm="$NM" --extra-version=20261110
    make -j"$(nproc)"
    make install
    cp /ffbuild/prefix/bin/ffmpeg.exe /ffbuild/prefix/bin/ffprobe.exe /ffbuild/prefix/bin/*.dll /output/
'
echo "Windows binaries: $work/output"