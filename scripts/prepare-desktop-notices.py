"""Build license and source archives for the Windows desktop dependency graph."""
import argparse
import base64
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import tarfile
import tomllib
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
TARGET = "x86_64-pc-windows-msvc"
CACHE = ROOT / "artifacts/third-party-cache"
LICENSE_NAME = re.compile(r"^(?:licen[sc]e|copying|notice|copyright|authors|patents|unlicense|third[-_ ]party)(?:$|[. _-])", re.I)
FALLBACK = {
    "alloc-stdlib": [("LICENSE", "c0c56f26d9c051cac4d200c34c84e7ae9aaa853e01a982a1df08b09931e518ae")],
    "tauri-plugin": [("LICENSE-MIT", "9dd42ea92cff2ede5cd477cbfcce051b2d0115c0ac7f368ee88cb545055dff1d"),
                     ("LICENSE-APACHE-2.0", "0d542e0c8804e39aa7f37eb00da5a762149dc682d7829451287e11b938e94594")],
    "webview2-com": [("LICENSE", "0dcf41516e608bbcb6cdc5229feb7b86fe4a643b85e7df251133c93408fdac73")],
    "webview2-com-macros": [("LICENSE", "0dcf41516e608bbcb6cdc5229feb7b86fe4a643b85e7df251133c93408fdac73")],
    "webview2-com-sys": [("LICENSE", "0dcf41516e608bbcb6cdc5229feb7b86fe4a643b85e7df251133c93408fdac73")],
}
SDK_URL = "https://api.nuget.org/v3-flatcontainer/microsoft.web.webview2/1.0.3800.47/microsoft.web.webview2.1.0.3800.47.nupkg"
NSIS_URL = "https://codeload.github.com/kichik/nsis/tar.gz/a4ba28e60bd9e7c271cd2bfc1558889ce3355653"
UTIL_URL = "https://codeload.github.com/tauri-apps/nsis-tauri-utils/tar.gz/13d9edd27b69310e108d6fbd49f90992f8a05390"


def checked(path, digest, algorithm='sha256'):
    with path.open('rb') as f:
        actual = hashlib.file_digest(f, algorithm).hexdigest()
    if actual != digest:
        raise ValueError(f'Checksum mismatch for {path.name}')


def fetch(name, url, digest, algorithm='sha256'):
    p = CACHE / name
    if not p.exists():
        p.parent.mkdir(parents=True, exist_ok=True)
        with urllib.request.urlopen(url, timeout=60) as response, p.with_suffix('.download').open('wb') as f:
            while block := response.read(256 * 1024):
                f.write(block)
        checked(p.with_suffix('.download'), digest, algorithm)
        p.with_suffix('.download').replace(p)
    checked(p, digest, algorithm)
    return p


def put(z, name, data):
    info = zipfile.ZipInfo(str(name).replace('\\', '/'))
    info.compress_type = zipfile.ZIP_DEFLATED
    z.writestr(info, data)


def put_notices(tgz, prefix, out):
    found = 0
    with tarfile.open(tgz) as tar:
        for m in sorted(tar, key=lambda m: m.name):
            path = PurePosixPath(m.name)
            if not m.isfile() or not LICENSE_NAME.match(path.name):
                continue
            if '..' in path.parts or path.is_absolute():
                raise ValueError(m.name)
            name = PurePosixPath(*path.parts[1:])
            put(out, f'{prefix}/{name}', tar.extractfile(m).read())
            found += 1
    return found


def cargo_graph():
    common = ['--locked', '--offline']
    meta = json.loads(subprocess.check_output(['cargo', 'metadata', *common, '--format-version', '1', '--filter-platform', TARGET], cwd=ROOT))
    tree = subprocess.check_output(['cargo', 'tree', *common, '--target', TARGET, '-p', 'screenrecord',
                                    '--prefix', 'none', '--format', '{p}', '--edges', 'normal,build'], cwd=ROOT, encoding='utf-8')
    active = {line.split(' (')[0] for line in tree.splitlines()}
    return sorted((p for p in meta['packages'] if p['source'] and f"{p['name']} v{p['version']}" in active), key=lambda p: (p['name'], p['version']))


def rust(notices, sources, rows):
    lock = tomllib.loads((ROOT/'Cargo.lock').read_text(encoding='utf-8'))['package']
    hashes = {(p['name'], p['version']): p['checksum'] for p in lock if 'checksum' in p}
    for package in cargo_graph():
        name, ver = package['name'], package['version']
        stem = f'{name}-{ver}'
        prefix = f'cargo/{stem}'
        src = Path(package['manifest_path']).parent
        cached = src.parents[2]/'cache'/src.parent.name/f'{stem}.crate'
        digest = hashes[(name, ver)]
        checked(cached, digest)
        put(sources, prefix+'.crate', cached.read_bytes())
        count = put_notices(cached, prefix, notices)
        if not count:
            if name == 'selectors':
                copy = fetch('MPL-2.0.txt', 'https://www.mozilla.org/media/MPL/2.0/index.815ca599c9df.txt',
                             'fab3dd6bdab226f1c08630b1dd917e11fcb4ec5e1e020e2c16f83a0a13863e85')
                put(notices, prefix+'/MPL-2.0.txt', copy.read_bytes())
            else:
                commit = json.loads((src/'.cargo_vcs_info.json').read_bytes())['git']['sha1']
                repo = package['repository'].removesuffix('.git').removeprefix('https://github.com/')
                for filename, sha in FALLBACK[name]:
                    p = fetch(name+'-'+filename, f'https://raw.githubusercontent.com/{repo}/{commit}/{filename}', sha)
                    put(notices, f'{prefix}/{filename}', p.read_bytes())
        rows.append(dict(component=stem, source=f'https://static.crates.io/crates/{name}/{stem}.crate',
                         sha256=digest, license=package['license'] or package['license_file']))
    put(notices, 'cargo/windows-capture/LICENCE', (ROOT/'vendor/windows-capture/LICENCE').read_bytes())
    rows.append(dict(component='windows-capture (vendored)', source='vendor/windows-capture in Screenrecord source', license='MIT'))


def npm(notices, sources, rows):
    packages = json.loads((ROOT/'package-lock.json').read_bytes())['packages']
    unique = {}
    for path, package in packages.items():
        if path and not package.get('dev'):
            name = path.split('node_modules/')[-1]
            unique[(name, package['version'])] = package
    def download(item):
        (name, ver), p = item
        prefix = f'{name}-{ver}'.replace('@', '').replace('/', '-')
        algorithm, digest = p['integrity'].split('-', 1)
        archive = fetch('npm/'+prefix+'.tgz', p['resolved'], base64.b64decode(digest).hex(), algorithm)
        return name, ver, p, prefix, archive
    with concurrent.futures.ThreadPoolExecutor(6) as pool:
        for name, ver, p, prefix, archive in pool.map(download, sorted(unique.items())):
            put(sources, 'npm/'+prefix+'.tgz', archive.read_bytes())
            if not put_notices(archive, 'npm/'+prefix, notices):
                raise ValueError('Missing npm notices: '+name)
            rows.append(dict(component=f'{name}@{ver}', source=p['resolved'], integrity=p['integrity'], license=p.get('license')))


def platform(notices, sources):
    sdk = fetch('webview2.nupkg', SDK_URL, '56c9f26bdd07916a2d1949fb58a5c7e434dfa1173577dca879206050c4e718db')
    with zipfile.ZipFile(sdk) as z:
        lib = next(p for p in cargo_graph() if p['name']=='webview2-com-sys')
        if z.read('build/native/x64/WebView2LoaderStatic.lib') != (Path(lib['manifest_path']).parent/'x64/WebView2LoaderStatic.lib').read_bytes():
            raise ValueError('WebView2 loader mismatch')
        for name in ('LICENSE.txt', 'NOTICE.txt'):
            put(notices, 'WebView2-SDK-1.0.3800.47/'+name, z.read(name))
    nsis = Path(os.environ.get('SCREENRECORD_NSIS_DIR', Path(os.environ['LOCALAPPDATA'])/'tauri/NSIS'))
    if 'v3.13' != subprocess.check_output([nsis/'makensis.exe','/VERSION'], text=True).strip():
        raise ValueError('Update the pinned NSIS source to match the installed compiler')
    checked(nsis/'Plugins/x86-unicode/additional/nsis_tauri_utils.dll', '5ba143b5db4a87d32d6e7802e033330aae56cbceabe0d1e3ba41948385ad4709')
    put(notices, 'NSIS-3.13/COPYING', (nsis/'COPYING').read_bytes())
    for file in sorted((nsis/'Docs').rglob('License.txt')):
        put(notices, 'NSIS-3.13/'+file.relative_to(nsis).as_posix(), file.read_bytes())
    for name, url, sha in [('nsis', NSIS_URL, 'bf9ebdefc2b26850bc5da8f2f505cb34aa364dd8cf49dd49f93d532c94accb4a'),
                           ('nsis-tauri-utils', UTIL_URL, '9b277f7cdec4277b48d319c32e10b5dc4303e9efce8c3b44f1db77f86a8131c7')]:
        source = fetch(name+'.tar.gz', url, sha)
        put(sources, 'installer/'+name+'.tar.gz', source.read_bytes())
        put_notices(source, 'installer/'+name, notices)
    sysroot = Path(subprocess.check_output(['rustc','--print','sysroot'], text=True).strip())
    docs = sysroot/'share/doc/rust'
    if not (docs/'COPYRIGHT-library.html').is_file():
        raise RuntimeError('Install rust-docs for the release toolchain with rustup component add rust-docs')
    put(notices, 'rust/COPYRIGHT-library.html', (docs/'COPYRIGHT-library.html').read_bytes())
    for file in sorted((docs/'licenses').glob('*.txt')):
        put(notices, 'rust/licenses/'+file.name, file.read_bytes())
    version = subprocess.check_output(['rustc','-Vv'], text=True)
    put(notices, 'rust/version.txt', version.encode())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, default=CACHE)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    rows = []
    with zipfile.ZipFile(args.out/'desktop-sources.zip', 'w') as sources, zipfile.ZipFile(args.out/'runtime-notices.zip', 'w') as notices:
        rust(notices, sources, rows)
        npm(notices, sources, rows)
        platform(notices, sources)
        inventory = json.dumps(rows, indent=2, ensure_ascii=False).encode()
        for z in (sources, notices):
            put(z, 'inventory.json', inventory)
    print(f'{len(rows)} packages. Wrote runtime-notices.zip and desktop-sources.zip into {args.out}.')


if __name__ == '__main__':
    main()
