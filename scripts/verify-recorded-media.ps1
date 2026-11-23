param([string]$SourceProject = 'artifacts\integration-export\project.json')
$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not [IO.Path]::IsPathRooted($SourceProject)) { $SourceProject = Join-Path $workspace $SourceProject }
$SourceProject = (Resolve-Path -LiteralPath $SourceProject).Path
$project = Get-Content -LiteralPath $SourceProject -Raw | ConvertFrom-Json
$fields = @('screenPath', 'cameraPath', 'microphonePath', 'systemAudioPath')
$sources = @($SourceProject)
foreach ($field in $fields) {
    if ($project.$field) {
        if (-not [IO.Path]::IsPathRooted($project.$field)) { $project.$field = Join-Path (Split-Path -Parent $SourceProject) $project.$field }
        $sources += $project.$field
    }
}
$cursorSource = $null
if ($project.PSObject.Properties.Name -contains 'cursorFile') {
    $cursorName = $project.cursorFile.name
    if ([IO.Path]::GetFileName($cursorName) -ne $cursorName) { throw 'Invalid cursor sidecar filename.' }
    $cursorSource = Join-Path (Split-Path -Parent $SourceProject) $cursorName
    $sources += $cursorSource
}
$before = @($sources | Get-FileHash -Algorithm SHA256 | Select-Object Path, Hash)
$runDirectory = Join-Path $workspace ('artifacts\recorded-media-check-' + [guid]::NewGuid().ToString('N'))
$project.id = [guid]::NewGuid().ToString()
$directory = Join-Path $runDirectory $project.id
New-Item -ItemType Directory -Path $directory -Force | Out-Null
if ($cursorSource) { Copy-Item -LiteralPath $cursorSource -Destination (Join-Path $directory $project.cursorFile.name) }
foreach ($field in $fields) {
    if ($project.$field) {
        $target = Join-Path $directory ([IO.Path]::GetFileName($project.$field))
        Copy-Item -LiteralPath $project.$field -Destination $target
        $project.$field = $target
    }
}
$project.directory = $directory
$project.name = 'Recorded media regression test'
$inputPath = Join-Path $directory 'project.json'
[IO.File]::WriteAllText($inputPath, ($project | ConvertTo-Json -Depth 30 -Compress), [Text.UTF8Encoding]::new($false))
$outputPath = Join-Path $runDirectory 'export.mp4'
$engine = Join-Path $workspace 'src-tauri\resources\ffmpeg.exe'
$probe = Join-Path $workspace 'src-tauri\resources\ffprobe.exe'
$logPath = Join-Path $runDirectory 'export.log'
Push-Location -LiteralPath $workspace
try {
    & cargo run -p screenrecord-render --example render_project --locked -- $inputPath $outputPath $engine '1080p' *> $logPath
    if ($LASTEXITCODE -ne 0) { throw "Recorded media export failed. See $logPath" }
    $probeText = & $probe -v error -show_streams -show_format -of json $outputPath
    if ($LASTEXITCODE -ne 0) { throw 'Cannot probe the recorded-media export.' }
    $media = $probeText | ConvertFrom-Json
    $video = @($media.streams | Where-Object codec_type -eq 'video')[0]
    $audio = @($media.streams | Where-Object codec_type -eq 'audio')[0]
    $expectedDuration = [double]$project.edits.trimEnd - [double]$project.edits.trimStart
    $actualDuration = [double]::Parse($media.format.duration, [Globalization.CultureInfo]::InvariantCulture)
    if ($video.codec_name -ne 'h264' -or $audio.codec_name -ne 'aac' -or [Math]::Abs($actualDuration - $expectedDuration) -gt 0.15) {
        throw 'The recorded-media export violates its codec or duration contract.'
    }
    & $engine -v error -xerror -i $outputPath -f null - *> (Join-Path $runDirectory 'decode.log')
    if ($LASTEXITCODE -ne 0) { throw 'The recorded-media export could not be decoded completely.' }
    foreach ($original in $before) {
        if ((Get-FileHash -LiteralPath $original.Path -Algorithm SHA256).Hash -ne $original.Hash) {
            throw "A source changed during verification: $($original.Path)"
        }
    }
    $evidence = [ordered]@{
        sourceProject = $SourceProject
        output = $outputPath
        videoCodec = $video.codec_name
        audioCodec = $audio.codec_name
        width = $video.width
        height = $video.height
        frames = $video.nb_frames
        duration = $actualDuration
        fullDecodePassed = $true
        sourceHashesUnchanged = $true
        sources = $before
    }
    $evidence | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $runDirectory 'evidence.json') -Encoding utf8
    $evidence | ConvertTo-Json -Depth 6
} finally { Pop-Location }
