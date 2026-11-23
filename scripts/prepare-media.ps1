param([string]$ArchivePath)
$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$cache = Join-Path $workspace 'artifacts\media-cache'
$destination = Join-Path $workspace 'src-tauri\resources'
$url = 'https://github.com/FFmpeg-Builds/FFmpeg-Builds/releases/download/autobuild-2026-11-10-12-39/ffmpeg-n8.1.3-34-ga35157abb9-win64-lgpl-shared-8.1.zip'
$expectedHash = '3045c2213ed8cbb05edf0d5a631b4490061de3b7567ec145644156e7c98da2c6'
New-Item -ItemType Directory -Force -Path $cache, $destination | Out-Null
if (-not $ArchivePath) {
    $ArchivePath = Join-Path $cache 'ffmpeg-lgpl.zip'
    if (-not (Test-Path -LiteralPath $ArchivePath)) {
        Invoke-WebRequest -Uri $url -OutFile $ArchivePath
    }
}
$actualHash = (Get-FileHash -LiteralPath $ArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualHash -ne $expectedHash) { throw 'FFmpeg archive checksum mismatch. No runtime files were copied.' }
$unpacked = Join-Path $cache 'unpacked'
Expand-Archive -LiteralPath $ArchivePath -DestinationPath $unpacked -Force
$distribution = Get-ChildItem -LiteralPath $unpacked -Directory | Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'bin\ffmpeg.exe') } | Select-Object -First 1
if (-not $distribution) { throw 'FFmpeg archive has an unexpected layout.' }
Get-ChildItem -LiteralPath (Join-Path $distribution.FullName 'bin') -File | Where-Object { $_.Extension -eq '.dll' -or $_.Name -in @('ffmpeg.exe','ffprobe.exe') } | ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $destination -Force }
foreach ($entry in Get-ChildItem -LiteralPath $distribution.FullName -File) { Copy-Item -LiteralPath $entry.FullName -Destination $destination -Force }
$licenseDirectory = Join-Path $distribution.FullName 'doc'
if (Test-Path -LiteralPath $licenseDirectory) {
    $generatedDocs = [IO.Path]::GetFullPath((Join-Path $destination 'ffmpeg-doc'))
    if (-not $generatedDocs.StartsWith($workspace + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Documentation path escaped the workspace.' }
    if (Test-Path -LiteralPath $generatedDocs) { Remove-Item -LiteralPath $generatedDocs -Recurse -Force }
    Copy-Item -LiteralPath $licenseDirectory -Destination $generatedDocs -Recurse
}
@{
    source = $url
    sha256 = $expectedHash
    build = 'FFmpeg-Builds autobuild-2026-11-10-12-39, n8.1.3-34-ga35157abb9, win64-lgpl-shared'
    buildRecipes = 'https://github.com/FFmpeg-Builds/FFmpeg-Builds/tree/autobuild-2026-11-10-12-39'
    ffmpegSource = 'https://github.com/FFmpeg/FFmpeg/tree/a35157abb9'
} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $destination 'media-provenance.json') -Encoding utf8
$ffmpegVersion = & (Join-Path $destination 'ffmpeg.exe') -version
if ($LASTEXITCODE -ne 0) { throw 'Packaged FFmpeg cannot run.' }
$ffprobeVersion = & (Join-Path $destination 'ffprobe.exe') -version
if ($LASTEXITCODE -ne 0) { throw 'Packaged FFprobe cannot run.' }
Write-Output $ffmpegVersion[0]
Write-Output $ffprobeVersion[0]
Write-Output 'Verified FFmpeg runtime prepared.'
