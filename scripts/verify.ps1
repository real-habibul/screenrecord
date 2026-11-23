param(
    [switch]$BuildInstaller,
    [ValidateSet('All', 'Native', 'Editor', 'Installer')]
    [string]$Suite = 'All'
)
$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))

function Invoke-Checked {
    param([string]$Program, [string[]]$Arguments)
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Program failed with exit code $LASTEXITCODE." }
}

Push-Location -LiteralPath $workspace
try {
    $engine = Join-Path $workspace 'src-tauri\resources\ffmpeg.exe'
    if (-not (Test-Path -LiteralPath $engine)) { & (Join-Path $PSScriptRoot 'prepare-media.ps1') }
    $env:SCREENRECORD_FFMPEG = $engine
    if ($Suite -in @('All', 'Native')) {
        Invoke-Checked 'cargo' @('fmt', '--all', '--', '--check')
        Invoke-Checked 'cargo' @('clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings')
        Invoke-Checked 'cargo' @('test', '--workspace', '--exclude', 'screenrecord-render', '--locked')
        Invoke-Checked 'cargo' @('test', '-p', 'screenrecord-render', '--locked', '--', '--test-threads=1')
        Invoke-Checked 'cargo' @('test', '-p', 'screenrecord-capture', '--locked', 'media_contract', '--', '--ignored', '--test-threads=1')
        Invoke-Checked 'cargo' @('test', '-p', 'screenrecord-transcribe', '--locked', 'media_contract', '--', '--ignored')
        Invoke-Checked 'cargo' @('test', '-p', 'screenrecord-render', '--example', 'render_project', '--locked')
        Invoke-Checked 'cargo' @('test', '--manifest-path', 'vendor/windows-capture/Cargo.toml', '--lib', '--locked', '--target-dir', 'target')
    }
    if ($Suite -in @('All', 'Editor')) {
        Invoke-Checked 'npm.cmd' @('run', 'build')
        Invoke-Checked 'npm.cmd' @('test')
        Invoke-Checked 'npm.cmd' @('run', 'test:ui')
        Invoke-Checked 'npm.cmd' @('audit', '--omit=dev')
    }
    if ($BuildInstaller -or $Suite -eq 'Installer') {
        Invoke-Checked 'npm.cmd' @('run', 'tauri', '--', 'build', '--', '--locked')
        $version = (Get-Content -LiteralPath (Join-Path $workspace 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json).version
        $installer = Join-Path $workspace "target\release\bundle\nsis\Screenrecord_${version}_x64-setup.exe"
        $hash = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash.ToLowerInvariant()
        "$hash  $([IO.Path]::GetFileName($installer))" | Set-Content -LiteralPath "$installer.sha256" -Encoding ascii
        Write-Output "Unsigned installer: $installer"
    }
} finally {
    Pop-Location
}
