"""Assemble the two public source companions for the Screenrecord 0.7.4 installer."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import zipfile

ROOT = Path(__file__).resolve().parents[1]
CACHE = ROOT/'artifacts/third-party-cache'
RELEASE = ROOT/'artifacts/public-release'
PINNED = {
    'ffmpeg.tar.gz': '0dd30eb8eca5370dc2dad5f9958d9d7221fa4e8a1d407fd06fbc305c15c9314e',
    'ffmpeg-builds.tar.gz': 'fb708487f587ffbf6c80c8d34fe0f96f7da8a1f88b8b6b5d1b27d91715fa2595',
    'gcc-16.2.0.tar.xz': 'e6738e29597f733270731aa90600f37ffdc045079dfc27ec7e8192cc81085c3e',
}


def filehash(p):
    with p.open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()


def add(z, path, name):
    info = zipfile.ZipInfo(name)
    info.file_size = path.stat().st_size
    with path.open('rb') as src, z.open(info, 'w') as dst:
        shutil.copyfileobj(src, dst, 1024*1024)


def archive(role, cache, out):
    if role == 'inputs':
        output = 'Screenrecord_0.7.4_ffmpeg-build-inputs.zip'
        files = sorted((cache/'ffmpeg-stages').glob('*.tar.xz'))
        dockerfile = cache/'FFmpeg-Builds-c9fe5114060e507409f7ee07edfd82efaf25d767'/'Dockerfile'
        if len(files) != 84 or any('/'+p.name+',' not in dockerfile.read_text() for p in files):
            raise ValueError('Prepare the 84 source stages selected by the pinned Dockerfile first')
        entries = {'.cache/downloads/'+p.name: p for p in files}
        entries['Dockerfile.win64-lgpl-shared-8.1'] = dockerfile
    else:
        output = 'Screenrecord_0.7.4_third-party-sources.zip'
        entries = {'ffmpeg/'+name: cache/name for name in PINNED}
        for name, expected in PINNED.items():
            if filehash(cache/name) != expected:
                raise ValueError('Unexpected archive '+name)
        entries.update({name: cache/name for name in ('desktop-sources.zip', 'ffmpeg-build-stages.json')})
        entries['BUILD.md'] = ROOT/'licenses/THIRD-PARTY-SOURCES.md'
        entries['rebuild-ffmpeg.sh'] = ROOT/'licenses/rebuild-ffmpeg.sh'
        for name in ('ffmpeg-notices.zip','runtime-notices.zip','audio-limiter-LGPL-2.1.txt','ffmpeg-GPL-3.0.txt','ffmpeg-LGPL-3.0.txt'):
            entries['notices/'+name] = ROOT/'licenses'/name
        entries['limiter/audio-limiter.js'] = ROOT/'public/audio-limiter.js'
        entries['limiter/af_alimiter.c'] = cache/'af_alimiter.c'
        entries['ffmpeg/binary-inventory.json'] = ROOT/'licenses/ffmpeg-binary-inventory.json'
        for name in ('Cargo.lock','package-lock.json'):
            entries['screenrecord/'+name] = ROOT/name
        entries['ffmpeg/rav1e-cargo/rav1e-cargo.zip'] = cache/'rav1e-cargo.zip'
    out.mkdir(parents=True, exist_ok=True)
    target = out/output
    partial = target.with_suffix('.part')
    sums = []
    with zipfile.ZipFile(partial, 'w') as z:
        for name, path in sorted(entries.items()):
            sums.append(filehash(path)+'  '+name)
            add(z, path, name)
        z.writestr(zipfile.ZipInfo('SHA256SUMS.txt'), ('\n'.join(sums)+'\n').encode())
    with zipfile.ZipFile(partial) as z:
        bad = z.testzip()
        if bad:
            raise ValueError('ZIP readback failed for '+bad)
    partial.replace(target)
    print(target, target.stat().st_size, filehash(target), sep='\n', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('role', choices=['inputs', 'sources'])
    parser.add_argument('--cache', type=Path, default=CACHE)
    parser.add_argument('--output', type=Path, default=RELEASE)
    a = parser.parse_args()
    archive(a.role, a.cache, a.output)
