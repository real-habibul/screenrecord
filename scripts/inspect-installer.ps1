param(
    [string]$InstallerPath,
    [string]$SevenZipPath = 'C:\Program Files\7-Zip\7z.exe'
)
$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $InstallerPath) {
    $version = (Get-Content -LiteralPath (Join-Path $workspace 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json).version
    $InstallerPath = Join-Path $workspace "target\release\bundle\nsis\Screenrecord_${version}_x64-setup.exe"
}
$InstallerPath = (Resolve-Path -LiteralPath $InstallerPath).Path
if (-not (Test-Path -LiteralPath $SevenZipPath)) { throw '7-Zip is required to inspect the installer without running it.' }
$inspectionDirectory = Join-Path $workspace ('artifacts\installer-inspection-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $inspectionDirectory | Out-Null
& $SevenZipPath x $InstallerPath "-o$inspectionDirectory" -y | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Installer extraction failed.' }

$application = Join-Path $inspectionDirectory 'screenrecord.exe'
$applicationBytes = [IO.File]::ReadAllBytes($application)
$peOffset = [BitConverter]::ToInt32($applicationBytes, 0x3c)
$machine = [BitConverter]::ToUInt16($applicationBytes, $peOffset + 4)
if ($machine -ne 0x8664) { throw 'The application is not Windows x64.' }
$releasePath = Join-Path $workspace 'target\release\screenrecord.exe'
$releaseBytes = [IO.File]::ReadAllBytes($releasePath)
$unpatchedMarker = '__TAURI_BUNDLE_TYPE_VAR_UNK'
$packagedMarker = '__TAURI_BUNDLE_TYPE_VAR_NSS'
$releaseText = [Text.Encoding]::ASCII.GetString($releaseBytes)
$markerIndex = $releaseText.IndexOf($unpatchedMarker, [StringComparison]::Ordinal)
if ($markerIndex -ge 0) {
    if ($releaseText.IndexOf($unpatchedMarker, $markerIndex + 1, [StringComparison]::Ordinal) -ge 0) {
        throw 'Unexpected duplicate Tauri bundle marker in the release executable.'
    }
    $markerBytes = [Text.Encoding]::ASCII.GetBytes($packagedMarker)
    [Array]::Copy($markerBytes, 0, $releaseBytes, $markerIndex, $markerBytes.Length)
}
$hasher = [Security.Cryptography.SHA256]::Create()
try {
    $expectedApplicationHash = [BitConverter]::ToString($hasher.ComputeHash($releaseBytes)).Replace('-', '')
} finally { $hasher.Dispose() }
$applicationHash = (Get-FileHash -LiteralPath $application -Algorithm SHA256).Hash
if ($applicationHash -ne $expectedApplicationHash) { throw 'The packaged executable differs beyond the expected Tauri NSIS bundle marker.' }

$assetDirectory = Join-Path $workspace 'dist\assets'
$webAssets = @(Get-ChildItem -LiteralPath $assetDirectory -File)
if ($webAssets.Count -eq 0) { throw 'The frontend build contains no assets.' }
$packagedText = [Text.Encoding]::ASCII.GetString($applicationBytes)
foreach ($asset in $webAssets) {
    if (-not $packagedText.Contains($asset.Name)) {
        throw "The installer does not embed the current frontend asset: $($asset.Name)"
    }
}

$sourceResources = Join-Path $workspace 'src-tauri\resources'
$resourceFiles = @(Get-ChildItem -LiteralPath $sourceResources -Recurse -File)
foreach ($sourceFile in $resourceFiles) {
    $relative = $sourceFile.FullName.Substring($sourceResources.Length).TrimStart('\')
    $packaged = Join-Path $inspectionDirectory ('resources\' + $relative)
    if (-not (Test-Path -LiteralPath $packaged)) { throw "Installer resource is missing: $relative" }
    if ((Get-FileHash -LiteralPath $sourceFile.FullName).Hash -ne (Get-FileHash -LiteralPath $packaged).Hash) {
        throw "Installer resource checksum mismatch: $relative"
    }
}
$notice = Join-Path $inspectionDirectory 'THIRD_PARTY_NOTICES.md'
if ((Get-FileHash -LiteralPath $notice).Hash -ne (Get-FileHash -LiteralPath (Join-Path $workspace 'THIRD_PARTY_NOTICES.md')).Hash) {
    throw 'The installer notices differ from the repository.'
}
$licenseSources = @{
    'LICENSE' = 'LICENSE'
    'public\audio-limiter.js' = 'licenses\audio-limiter.js'
    'vendor\windows-capture\LICENCE' = 'licenses\windows-capture-MIT.txt'
}
foreach ($license in Get-ChildItem -LiteralPath (Join-Path $workspace 'licenses') -Recurse -File) {
    $relative = $license.FullName.Substring($workspace.Length).TrimStart('\')
    $licenseSources[$relative] = $relative
}
foreach ($source in $licenseSources.Keys) {
    $packaged = Join-Path $inspectionDirectory $licenseSources[$source]
    if (-not (Test-Path -LiteralPath $packaged)) { throw "Installer license/source is missing: $source" }
    if ((Get-FileHash -LiteralPath (Join-Path $workspace $source)).Hash -ne (Get-FileHash -LiteralPath $packaged).Hash) {
        throw "Installer license/source checksum mismatch: $source"
    }
}
$engineVersion = & (Join-Path $inspectionDirectory 'resources\ffmpeg.exe') -version
if ($LASTEXITCODE -ne 0) { throw 'The packaged media engine cannot run.' }
$probeVersion = & (Join-Path $inspectionDirectory 'resources\ffprobe.exe') -version
if ($LASTEXITCODE -ne 0) { throw 'The packaged media probe cannot run.' }
$installer = Get-Item -LiteralPath $InstallerPath
$productVersion = (Get-Item -LiteralPath $application).VersionInfo.ProductVersion
if (-not $productVersion -or $installer.VersionInfo.ProductVersion -ne $productVersion) {
    throw 'The packaged application and installer versions differ.'
}
$evidence = [ordered]@{
    inspectedAt = [DateTimeOffset]::Now.ToString('o')
    installer = $InstallerPath
    bytes = $installer.Length
    sha256 = (Get-FileHash -LiteralPath $InstallerPath -Algorithm SHA256).Hash.ToLowerInvariant()
    applicationArchitecture = 'x64'
    productVersion = $productVersion
    applicationMatchesReleaseAfterBundleMarker = $true
    embeddedFrontendAssetsVerified = $webAssets.Count
    applicationSha256 = $applicationHash.ToLowerInvariant()
    resourcesVerified = $resourceFiles.Count
    resourcesMatchSource = $true
    licenseAndSourceFilesVerified = $licenseSources.Count
    audioLimiterSourceAndLicenseMatch = $true
    ffmpegVersion = $engineVersion[0]
    ffprobeVersion = $probeVersion[0]
    signatureStatus = (Get-AuthenticodeSignature -LiteralPath $InstallerPath).Status.ToString()
    extractionDirectory = $inspectionDirectory
    installerExecuted = $false
}
$evidence | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $workspace 'artifacts\installer-evidence.json') -Encoding utf8
$evidence | ConvertTo-Json
