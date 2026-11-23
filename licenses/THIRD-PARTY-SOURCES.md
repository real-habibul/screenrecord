# Third-party source for Screenrecord 0.7.4

Download both ZIP files from the same release as the Windows installer. Each ZIP includes a SHA-256 inventory. [Screenrecord downloads](https://screenrecord.88apps.dev) and [GitHub releases](https://github.com/real-habibul/screenrecord/releases).

| Archive | Contents |
| --- | --- |
| `Screenrecord_0.7.4_third-party-sources.zip` | FFmpeg at commit `a35157abb96167fd7308edc92b55408d5bb084b0`, original build recipes, GCC 16.2.0 including its runtime library sources and exceptions, rav1e Cargo sources and registry metadata, application dependencies and installer sources, and license notices |
| `Screenrecord_0.7.4_ffmpeg-build-inputs.zip` | The generated Windows x64 LGPL shared target Dockerfile and its 84 source-stage inputs, extracted from the original FFmpeg-Builds CI cache. This archive is 2,096,723,604 bytes. |

## Build FFmpeg for Windows x64

Use an x86-64 Linux host with Bash, Python 3, unzip, GNU tar, coreutils and a BuildKit-enabled Docker Engine. The original build uses a Linux container to cross-compile Windows DLLs. Building the toolchain and its libraries needs network access for host packages and sufficient disk and CPU time. Run from a directory containing the two ZIP files and the copy of `rebuild-ffmpeg.sh` extracted from the smaller ZIP.

```sh
bash rebuild-ffmpeg.sh Screenrecord_0.7.4_third-party-sources.zip Screenrecord_0.7.4_ffmpeg-build-inputs.zip
```

Add `--prepare-only` to validate the archive contents and create a prepared Docker build context. The script prints its working directory. The full build uses Docker's local image store and creates the base images under the names expected by the original recipe. Use a separate Docker environment if you have existing images with those names. Rebuilt Windows executables and shared libraries go into the printed output directory. Quit Screenrecord and copy the set of two executables and seven DLLs to the installed application's `resources` directory to use a compatible rebuilt runtime.

The FFmpeg source, dependency cache and target flags match [the upstream build](https://github.com/FFmpeg-Builds/FFmpeg-Builds/actions/runs/42813262313). The inputs archive came from CI artifact `13762902626`, SHA-256 `6260d7ed7f4df686bc1eb9597cbb07afcd685628735966c8f1ddff60ac2ce24a`. The Dockerfile in that ZIP came from the project's `./generate.sh win64 lgpl-shared 8.1`. `ffmpeg-build-stages.json` records the SHA-256 of each input. Each hashed archive name includes the upstream identifier for its download recipe.

The upstream FFmpeg job used image digest `sha256:740f96bbb0801136df45cc488ea6fdfe27a80095a85e026ddb122443a6ab79eb`. The registry no longer serves that digest. The helper builds the toolchain image again, pins crosstool-NG to `fbb637297fef385204304f5ee32a168ea6145280` and uses the archived FFmpeg checkout. Base OS packages and host build tools can differ. An identical rebuilt executable has not been verified. [Original build recipe](https://github.com/FFmpeg-Builds/FFmpeg-Builds/tree/c9fe5114060e507409f7ee07edfd82efaf25d767).

The original rav1e recipe updated its Rust cc dependency at build time. Its supplemental ZIP includes 275 original registry archives from the matching historical build cache plus the offline registry index and reconstructed lockfile. The helper uses this input for rav1e. The vendored librsvg Rust sources are in the separate `99-librsvg` build-stage archive. [rav1e recipe](https://github.com/FFmpeg-Builds/FFmpeg-Builds/blob/c9fe5114060e507409f7ee07edfd82efaf25d767/scripts.d/50-rav1e.sh).

## Desktop dependencies and licenses

`desktop-sources.zip` contains the published crate archives used by the Windows Cargo normal/build dependency graph and the npm tarballs identified as production packages in the lockfile. Build-time packages are included. Each published package is checked against its locked checksum. The local Windows Capture fork is in the application source tree. The NSIS 3.13 and nsis-tauri-utils 0.5.3 sources are under `installer/`. The Windows loader in webview2-com-sys matched the x64 static library from Microsoft.Web.WebView2 1.0.3800.47 byte for byte. Its license and additional notice are inside `runtime-notices.zip`. [Application source](https://github.com/real-habibul/screenrecord) and [Microsoft SDK](https://www.nuget.org/packages/Microsoft.Web.WebView2/1.0.3800.47).

The installed `licenses` directory also contains FFmpeg's GPL 3 and LGPL 3 texts, the AudioWorklet limiter's LGPL 2.1 text and JavaScript source, and the two notices ZIPs. `ffmpeg-binary-inventory.json` identifies the deployed executables and DLLs, their SHA-256 hashes, Windows imports, and FFmpeg's configure flags. Individual library source archives preserve their original notices and any per-file notices. [FFmpeg licensing](https://ffmpeg.org/legal.html).
