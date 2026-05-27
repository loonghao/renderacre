param(
    [string]$Version = $env:RENDERACRE_VERSION,
    [string]$InstallDir = $env:RENDERACRE_INSTALL_DIR,
    [string]$Repo = $env:RENDERACRE_REPO
)

$ErrorActionPreference = "Stop"

if (-not $Version) { $Version = "latest" }
if (-not $InstallDir) { $InstallDir = Join-Path $env:LOCALAPPDATA "renderacre\bin" }
if (-not $Repo) { $Repo = "loonghao/renderacre" }

if ($Version -eq "latest") {
    $latest = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest"
    $Version = $latest.tag_name
    if (-not $Version) { throw "could not resolve latest renderacre release" }
}

$asset = "renderacre-$Version-windows-x86_64.zip"
$url = "https://github.com/$Repo/releases/download/$Version/$asset"

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) "renderacre-install-$([System.Guid]::NewGuid())"
New-Item -ItemType Directory -Force -Path $tmp | Out-Null

try {
    $archive = Join-Path $tmp $asset
    Write-Host "Downloading $url"
    Invoke-WebRequest -Uri $url -OutFile $archive

    Expand-Archive -Path $archive -DestinationPath $tmp -Force
    $bundleDir = Get-ChildItem -Path $tmp -Directory -Filter "renderacre-*" | Select-Object -First 1
    if (-not $bundleDir) { throw "downloaded archive did not contain a renderacre bundle" }

    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    Copy-Item -Force (Join-Path $bundleDir.FullName "renderacre-controller.exe") $InstallDir
    Copy-Item -Force (Join-Path $bundleDir.FullName "renderacre-worker.exe") $InstallDir

    Write-Host "Installed renderacre-controller.exe and renderacre-worker.exe to $InstallDir"
    Write-Host "Add this directory to PATH if it is not already available."
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
