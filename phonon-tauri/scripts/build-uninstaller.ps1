$ErrorActionPreference = "Stop"

$uninstallerSrc = "d:\Phonon\target\release\uninstall.exe"
$uninstallerDst = "d:\Phonon\phonon-tauri\src-tauri\resources\phonon-uninstaller.exe"

Write-Host "Building phonon-uninstaller (release)..."
cargo build --release -p phonon-uninstaller
if ($LASTEXITCODE -ne 0) { throw "Failed to build phonon-uninstaller" }

$dstDir = Split-Path $uninstallerDst -Parent
if (-not (Test-Path $dstDir)) { New-Item -ItemType Directory -Path $dstDir -Force | Out-Null }

Copy-Item $uninstallerSrc $uninstallerDst -Force
Write-Host "Uninstaller copied to $uninstallerDst ($((Get-Item $uninstallerDst).Length) bytes)"
