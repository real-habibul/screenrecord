"""Collect NVIDIA, GCC runtime and rav1e notices alongside FFmpeg build-stage notices."""
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import re
import shutil
import sys
import tarfile
import urllib.request
import zipfile

BASE = Path(__file__).resolve().parents[1]
CACHE = BASE/'artifacts/third-party-cache'
FALLBACK = json.loads((BASE/'scripts/rav1e-license-sources.json').read_bytes())
NAME = re.compile(r'^(?:licen[sc]e|copying|copyright|notice|unlicense|authors|patents)(?:$|[. _-])', re.I)


def write(out, name, body):
    info = zipfile.ZipInfo(name)
    info.compress_type = zipfile.ZIP_DEFLATED
    out.writestr(info, body)


def pkg_notices(buf):
    with tarfile.open(fileobj=io.BytesIO(buf)) as src:
        return {'/'.join(PurePosixPath(m.name).parts[1:]): src.extractfile(m).read() for m in src
                if m.isfile() and NAME.match(PurePosixPath(m.name).name)}


def main():
    partial = CACHE/'ffmpeg-notices-final.zip'
    shutil.copyfile(CACHE/'ffmpeg-notices.zip', partial)
    with zipfile.ZipFile(partial, 'a') as out:
        source = next((CACHE/'ffmpeg-stages').glob('50-ffnvcodec_*.tar.xz'))
        with tarfile.open(source) as nv:
            for m in nv:
                if m.isfile() and m.name.endswith('.h') and '.git' not in m.name:
                    data = nv.extractfile(m).read()
                    match = re.search(rb'/\*.*?\*/', data, re.S)
                    if not match or b'ermission' not in match[0]:
                        raise ValueError('Unexpected NVIDIA header '+m.name)
                    write(out, '50-ffnvcodec/'+str(PurePosixPath(m.name))+'.txt', match[0]+b'\n')
        with tarfile.open(CACHE/'gcc-16.2.0.tar.xz', mode='r|xz') as gcc:
            for m in gcc:
                path = PurePosixPath(m.name)
                if m.isfile() and len(path.parts)==2 and path.name.startswith('COPYING'):
                    write(out, 'GCC-16.2.0/'+path.name, gcc.extractfile(m).read())
        with zipfile.ZipFile(CACHE/'rav1e-cargo.zip') as crates:
            packed = sorted(n for n in crates.namelist() if n.endswith('.crate'))
            if len(packed)!=275:
                raise ValueError('Wrong rav1e stage registry cache')
            registry_root = PurePosixPath(packed[0]).parent
            winapi = pkg_notices(crates.read(str(registry_root/'winapi-0.3.9.crate')))
            for name in packed:
                stem = PurePosixPath(name).stem
                files = pkg_notices(crates.read(name))
                if not files and stem.startswith(('winapi-i686-', 'winapi-x86_64-')):
                    files = {s:v for s,v in winapi.items() if s in ('LICENSE-MIT','LICENSE-APACHE')}
                if not files:
                    for key, ref in FALLBACK.items():
                        if key.split('/')[0]!=stem:
                            continue
                        cached=CACHE/'rav1e-fallback'/key
                        data=cached.read_bytes() if cached.exists() else urllib.request.urlopen(ref['url'], timeout=30).read()
                        if hashlib.sha256(data).hexdigest()!=ref['sha256']:
                            raise ValueError(key)
                        files[key.split('/')[1]]=data
                if not files:
                    raise ValueError('Missing notice for '+stem)
                for n, data in files.items():
                    write(out, f'50-rav1e/registry/{stem}/{n}', data)
        write(out,'50-rav1e/registry-upstream-notices.json',json.dumps(FALLBACK, indent=2).encode())
    with zipfile.ZipFile(partial) as z:
        if z.testzip():
            raise ValueError('Corrupt FFmpeg notices archive')
        print(f'{len(z.namelist())} notices and index entries')
    shutil.copyfile(partial, BASE/'licenses/ffmpeg-notices.zip')


if __name__=='__main__':
    main()
