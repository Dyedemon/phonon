$ErrorActionPreference = "Continue"

# ============================================================
# build-installer.ps1 — Build the self-contained Phonon installer
#
# Steps:
#   1. Build the Tauri app (frontend + backend)
#   2. Build the uninstaller (done inside tauri:build)
#   3. Stage files into a temp directory (rename app.exe → phonon.exe)
#   4. Zip the staging directory
#   5. Build the installer with PHONON_APP_ZIP set (embeds the zip)
#   6. Copy final phonon-setup.exe to the bundle output directory
#   7. Cleanup temp files
# ============================================================

$projectRoot = "d:\Phonon"
$tauriDir = Join-Path $projectRoot "phonon-tauri"
$targetRelease = Join-Path $projectRoot "target\release"
$stagingDir = Join-Path $env:TEMP "phonon-installer-staging"
$zipPath = Join-Path $env:TEMP "phonon-app-bundle.zip"
$outputDir = Join-Path $targetRelease "bundle\nsis"

Write-Host "========== Phonon Installer Build ==========" -ForegroundColor Cyan

function Invoke-Step {
    param([string]$Label, [scriptblock]$Action)
    Write-Host "`n[$Label]" -ForegroundColor Yellow
    & $Action
    if ($LASTEXITCODE -ne 0) {
        throw "Step '$Label' failed with exit code $LASTEXITCODE"
    }
}

# ---- Step 1: Build Tauri app ----
Invoke-Step "1/7] Building Tauri app..." {
    Push-Location $tauriDir
    cmd /c "npm run tauri:build 2>&1"
    $code = $LASTEXITCODE
    Pop-Location
    if ($code -ne 0) { throw "Tauri build failed (exit $code)" }
    Write-Host "  Tauri build complete." -ForegroundColor Green
}

# ---- Step 2: Build uninstaller (if not already done by tauri:build) ----
Write-Host "`n[2/7] Building uninstaller..." -ForegroundColor Yellow
if (-not (Test-Path (Join-Path $targetRelease "uninstall.exe"))) {
    Push-Location $projectRoot
    cmd /c "cargo build --release -p phonon-uninstaller 2>&1"
    $code = $LASTEXITCODE
    Pop-Location
    if ($code -ne 0) { throw "Uninstaller build failed" }
}
Write-Host "  Uninstaller ready." -ForegroundColor Green

# ---- Step 3: Stage files ----
Write-Host "`n[3/7] Staging files..." -ForegroundColor Yellow

if (Test-Path $stagingDir) { Remove-Item $stagingDir -Recurse -Force }
New-Item -ItemType Directory -Path $stagingDir -Force | Out-Null

# Copy and rename app.exe -> phonon.exe
$appExe = Join-Path $targetRelease "app.exe"
$phononExe = Join-Path $stagingDir "phonon.exe"
if (-not (Test-Path $appExe)) { throw "app.exe not found at $appExe" }
Copy-Item $appExe $phononExe -Force
Write-Host "  phonon.exe ($([math]::Round((Get-Item $phononExe).Length / 1MB, 1))) MB)" -ForegroundColor Gray

# Copy uninstall.exe
$uninstallSrc = Join-Path $targetRelease "uninstall.exe"
$uninstallDst = Join-Path $stagingDir "uninstall.exe"
if (-not (Test-Path $uninstallSrc)) { throw "uninstall.exe not found at $uninstallSrc" }
Copy-Item $uninstallSrc $uninstallDst -Force
Write-Host "  uninstall.exe ($([math]::Round((Get-Item $uninstallDst).Length / 1KB, 1))) KB)" -ForegroundColor Gray

# Copy additional files from Tauri resources (exclude phonon-uninstaller.exe —
# uninstall.exe is already in staging from the dedicated build step above)
$tauriResources = Join-Path $tauriDir "src-tauri\resources"
if (Test-Path $tauriResources) {
    Get-ChildItem $tauriResources -File | Where-Object {
        $_.Name -ne 'phonon-uninstaller.exe'
    } | ForEach-Object {
        Copy-Item $_.FullName $stagingDir -Force
        Write-Host "  $($_.Name) ($($_.Length) bytes)" -ForegroundColor Gray
    }
}

# ---- Step 4: Create zip ----
Write-Host "`n[4/7] Creating app bundle zip..." -ForegroundColor Yellow

if (Test-Path $zipPath) { Remove-Item $zipPath -Force }
Add-Type -AssemblyName System.IO.Compression.FileSystem
[System.IO.Compression.ZipFile]::CreateFromDirectory($stagingDir, $zipPath, [System.IO.Compression.CompressionLevel]::Optimal, $false)

$zipSize = (Get-Item $zipPath).Length
Write-Host "  Created: $([math]::Round($zipSize / 1MB, 1)) MB" -ForegroundColor Green

# ---- Step 5: Build installer with embedded zip ----
Write-Host "`n[5/7] Building installer (with embedded app bundle)..." -ForegroundColor Yellow

$env:PHONON_APP_ZIP = $zipPath
Push-Location $projectRoot
cmd /c "cargo build --release -p phonon-installer 2>&1"
$buildCode = $LASTEXITCODE
Pop-Location
Remove-Item env:\PHONON_APP_ZIP -ErrorAction SilentlyContinue

if ($buildCode -ne 0) { throw "Installer build failed (exit $buildCode)" }
Write-Host "  Installer built." -ForegroundColor Green

# ---- Step 6: Copy to output ----
Write-Host "`n[6/7] Copying installer to output..." -ForegroundColor Yellow

if (-not (Test-Path $outputDir)) {
    New-Item -ItemType Directory -Path $outputDir -Force | Out-Null
}

$installerSrc = Join-Path $targetRelease "phonon-setup.exe"
$installerDst = Join-Path $outputDir "Phonon_0.1.0_x64-setup.exe"

if (-not (Test-Path $installerSrc)) { throw "phonon-setup.exe not found at $installerSrc" }
Copy-Item $installerSrc $installerDst -Force

$installerSize = (Get-Item $installerDst).Length
Write-Host "  Output: $installerDst" -ForegroundColor Green
Write-Host "  Size: $([math]::Round($installerSize / 1MB, 1)) MB" -ForegroundColor Green

# ---- Step 7: Cleanup ----
Write-Host "`n[7/7] Cleaning up..." -ForegroundColor Yellow
if (Test-Path $stagingDir) { Remove-Item $stagingDir -Recurse -Force }
if (Test-Path $zipPath) { Remove-Item $zipPath -Force }
Write-Host "  Cleanup complete." -ForegroundColor Green

# ---- Summary ----
Write-Host "`n========== Build Complete ==========" -ForegroundColor Cyan
Write-Host "Installer: $installerDst" -ForegroundColor White
Write-Host "Size: $([math]::Round($installerSize / 1MB, 1)) MB" -ForegroundColor White
