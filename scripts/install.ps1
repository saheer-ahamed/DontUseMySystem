# Installs the release build for the current user, or removes it with -Uninstall.
# Usage: powershell -ExecutionPolicy Bypass -File scripts/install.ps1 [-Desktop] [-Startup] [-Uninstall]
param(
    [switch]$Desktop,
    [switch]$Startup,
    [switch]$Uninstall
)
$ErrorActionPreference = 'Stop'

$name = 'DontUseMySystem'
$dir = Join-Path $env:LOCALAPPDATA "Programs\$name"
$exe = Join-Path $dir 'dontusemysystem.exe'
$links = @(
    (Join-Path ([Environment]::GetFolderPath('Programs')) "$name.lnk"),
    (Join-Path ([Environment]::GetFolderPath('Desktop')) "$name.lnk"),
    (Join-Path ([Environment]::GetFolderPath('Startup')) "$name.lnk")
)

# Killing a locked instance would leave the touchpad off until the next launch or reboot,
# so refuse instead and let the user quit it cleanly.
$running = Get-Process dontusemysystem -ErrorAction SilentlyContinue
if ($running) {
    throw "$name is running. Unlock with Ctrl+Alt+U, quit with Ctrl+Alt+Shift+Q, then run this again."
}

if ($Uninstall) {
    foreach ($l in $links) { if (Test-Path $l) { Remove-Item $l } }
    if (Test-Path $dir) { Remove-Item $dir -Recurse }
    Write-Host "Removed $name. Settings in $env:APPDATA\$name were kept."
    return
}

$src = Join-Path $PSScriptRoot '..\target\release\dontusemysystem.exe'
if (-not (Test-Path $src)) { throw "Build it first: cargo build --release" }

New-Item -ItemType Directory -Force $dir | Out-Null
Copy-Item $src $exe -Force

$shell = New-Object -ComObject WScript.Shell
function New-Link([string]$path) {
    $s = $shell.CreateShortcut($path)
    $s.TargetPath = $exe
    $s.WorkingDirectory = $dir
    $s.Description = 'Lock all input until Ctrl+Alt+U is held'
    $s.Save()
}

New-Link $links[0]
if ($Desktop) { New-Link $links[1] } elseif (Test-Path $links[1]) { Remove-Item $links[1] }
if ($Startup) { New-Link $links[2] } elseif (Test-Path $links[2]) { Remove-Item $links[2] }

Write-Host "Installed $name to $exe"
Write-Host "Start menu: $($links[0])"
if ($Desktop) { Write-Host "Desktop: $($links[1])" }
if ($Startup) { Write-Host "Runs at sign-in: $($links[2])" }
