param(
    [ValidatePattern('^[A-Za-z0-9][A-Za-z0-9._-]*$')][string]$Distribution = 'Ubuntu',
    [ValidateSet('screenrecord', 'screenrecord-capture', 'screenrecord-render')]
    [string[]]$Package = @('screenrecord', 'screenrecord-capture', 'screenrecord-render')
)
$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$previous = $env:SCREENRECORD_TEST_QUOTA_ROOT
$previousDistro = $env:SCREENRECORD_TEST_QUOTA_DISTRO
$previousMount = $env:SCREENRECORD_TEST_QUOTA_MOUNT
$previousEngine = $env:SCREENRECORD_FFMPEG
$mounted = $false
$mountPoint = (& wsl.exe -d $Distribution --exec mktemp -d /tmp/screenrecord-quota-XXXXXXXX).Trim()
if ($LASTEXITCODE -ne 0 -or $mountPoint -cnotmatch '^/tmp/screenrecord-quota-[a-zA-Z0-9]{8}$') {
    throw 'Could not create an isolated quota directory in WSL.'
}
Push-Location -LiteralPath $workspace
try {
    $uidNumber = (& wsl.exe -d $Distribution --exec id -u).Trim()
    if ($LASTEXITCODE -ne 0 -or $uidNumber -notmatch '^\d+$') { throw 'Missing WSL test user.' }
    & wsl.exe -d $Distribution -u root --exec mount -t tmpfs -o "size=1m,mode=0700,uid=$uidNumber" tmpfs $mountPoint
    if ($LASTEXITCODE -ne 0) { throw 'Could not mount the private 1 MiB test volume.' }
    $mounted = $true
    $env:SCREENRECORD_TEST_QUOTA_DISTRO = $Distribution
    $env:SCREENRECORD_TEST_QUOTA_MOUNT = $mountPoint
    $env:SCREENRECORD_TEST_QUOTA_ROOT = "\\wsl.localhost\$Distribution" + $mountPoint.Replace('/', '\')
    $env:SCREENRECORD_FFMPEG = Join-Path $workspace 'src-tauri/resources/ffmpeg.exe'
    foreach ($case in @(
        @{ Package = 'screenrecord'; Name = 'disk_full_preserves_capture_and_last_save_then_retries' },
        @{ Package = 'screenrecord'; Name = 'disk_full_recovery_rebuilds_recorded_video_from_the_original_segment' },
        @{ Package = 'screenrecord-capture'; Name = 'disk_full_during_pcm_capture_keeps_checkpointed_samples' },
        @{ Package = 'screenrecord-render'; Name = 'full_export_refuses_full_disk_and_retries_after_space_is_freed' }
    )) {
        if ($Package -notcontains $case.Package) { continue }
        $lines = & cargo test -p $case.Package --lib --locked $case.Name -- --ignored --nocapture
        $lines | Write-Output
        if ($LASTEXITCODE -ne 0 -or -not ($lines -match 'test result: ok\. 1 passed;')) {
            throw "Disk-full regression failed or was skipped: $($case.Name)"
        }
    }
} finally {
    $env:SCREENRECORD_TEST_QUOTA_ROOT = $previous
    $env:SCREENRECORD_TEST_QUOTA_DISTRO = $previousDistro
    $env:SCREENRECORD_TEST_QUOTA_MOUNT = $previousMount
    $env:SCREENRECORD_FFMPEG = $previousEngine
    Pop-Location
    if ($mounted) {
        & wsl.exe -d $Distribution -u root --exec umount $mountPoint
        if ($LASTEXITCODE -ne 0) { throw "Could not unmount $mountPoint." }
    }
    & wsl.exe -d $Distribution --exec rmdir $mountPoint
    if ($LASTEXITCODE -ne 0) { throw "Could not remove the empty test mount point $mountPoint." }
}
