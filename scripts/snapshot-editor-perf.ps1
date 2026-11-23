param([Parameter(Mandatory = $true)][string]$Name)
$ErrorActionPreference = 'Stop'
if ($Name -notmatch '^[a-zA-Z0-9][a-zA-Z0-9.-]*$' -or $Name.Contains('..')) { throw 'Use a simple snapshot name without path components.' }
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$artifacts = [IO.Path]::GetFullPath((Join-Path $workspace 'artifacts/perf-0.5.1'))
$destination = [IO.Path]::GetFullPath((Join-Path $artifacts $Name))
if (-not $destination.StartsWith($artifacts + [IO.Path]::DirectorySeparatorChar) -or (Test-Path -LiteralPath $destination)) { throw 'Snapshot destination must be new and inside the benchmark artifacts.' }
New-Item -ItemType Directory -Path $destination | Out-Null
foreach ($directory in @('src', 'public', 'tests', 'testdata')) { Copy-Item -LiteralPath (Join-Path $workspace $directory) -Destination $destination -Recurse }
foreach ($file in @('index.html', 'package.json', 'package-lock.json', 'vite.config.ts', 'vitest.config.ts', 'playwright.config.ts', 'tsconfig.json')) { Copy-Item -LiteralPath (Join-Path $workspace $file) -Destination $destination }
$files = @(Get-ChildItem -LiteralPath $destination -File -Recurse | Sort-Object FullName | ForEach-Object {
    [pscustomobject]@{ path = [IO.Path]::GetRelativePath($destination, $_.FullName).Replace('\', '/'); bytes = $_.Length; sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
})
$manifest = [pscustomobject]@{ capturedAt = (Get-Date).ToUniversalTime().ToString('o'); sourceRoot = $workspace; snapshot = $destination; packageVersion = (Get-Content -LiteralPath (Join-Path $destination 'package.json') -Raw | ConvertFrom-Json).version; files = $files }
$manifest | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $artifacts ($Name + '-manifest.json')) -Encoding utf8
[pscustomobject]@{ snapshot = $destination; files = $files.Count; bytes = ($files | Measure-Object bytes -Sum).Sum } | ConvertTo-Json
