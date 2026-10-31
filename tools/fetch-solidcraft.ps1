# Windows: download the pinned SolidCraft CLI (inside the portable package) and web app
# (upstream.toml) into upstream\<tag>\ and verify them, as tools/fetch-solidcraft.sh does on macOS.
# Each pinned sha256 must match both the release's SHA256SUMS.txt and the downloaded zip.
#   powershell -ExecutionPolicy Bypass -File tools\fetch-solidcraft.ps1
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'   # Invoke-WebRequest is very slow with its progress bar
$root = Split-Path -Parent $PSScriptRoot
$toml = Get-Content -Raw (Join-Path $root 'upstream.toml')
function Get-Pin($key) {
    $m = [regex]::Match($toml, "(?m)^$key *= *`"(.*)`"")
    if ($m.Success) { $m.Groups[1].Value } else { '' }
}
$repo = Get-Pin 'repo'; $tag = Get-Pin 'tag'
$asset = Get-Pin 'windows_asset'; $pinned = Get-Pin 'windows_sha256'; $binary = Get-Pin 'windows_binary'
$webAsset = Get-Pin 'web_asset'; $webPinned = Get-Pin 'web_sha256'
if (-not $asset) { throw "upstream.toml pins no Windows package for $tag" }

$dir = Join-Path $root "upstream\$tag"
New-Item -ItemType Directory -Force $dir | Out-Null
$base = "https://github.com/$repo/releases/download/$tag"
function Get-Asset($name) {
    $path = Join-Path $dir $name
    if (-not (Test-Path $path)) { Invoke-WebRequest -UseBasicParsing "$base/$name" -OutFile $path }
    $path
}
$sums = Get-Content (Get-Asset 'SHA256SUMS.txt')
function Test-Asset($name, $want) {
    $listed = ($sums | Where-Object { ($_ -split '\s+')[1] -eq $name } | ForEach-Object { ($_ -split '\s+')[0] }) -join ''
    $actual = (Get-FileHash -Algorithm SHA256 (Join-Path $dir $name)).Hash.ToLower()
    if ($listed -ne $want -or $actual -ne $want) {
        throw "sha256 mismatch for ${name}: pinned $want, SHA256SUMS $listed, file $actual"
    }
}

Get-Asset $asset | Out-Null
Test-Asset $asset $pinned
$cli = Join-Path $dir 'cli'
if (Test-Path $cli) { Remove-Item -Recurse -Force $cli }
Expand-Archive (Join-Path $dir $asset) -DestinationPath $cli

if ($webAsset) {
    Get-Asset $webAsset | Out-Null
    Test-Asset $webAsset $webPinned
    $web = Join-Path $dir 'web'
    if (Test-Path $web) { Remove-Item -Recurse -Force $web }
    Expand-Archive (Join-Path $dir $webAsset) -DestinationPath $web
}
& (Join-Path $cli $binary) --version
