$exePath = "d:\Phonon\target\debug\phonon.exe"

if (Test-Path $exePath) {
    Start-Process $exePath
    Write-Host "Phonon 已启动" -ForegroundColor Green
} else {
    Write-Host "未找到编译好的 phonon.exe，需要先安装 Rust 并编译项目" -ForegroundColor Red
    Write-Host "1. 安装 Rust: https://rustup.rs" -ForegroundColor Yellow
    Write-Host "2. 在 phonon-tauri 目录运行: npm install" -ForegroundColor Yellow
    Write-Host "3. 编译运行: npm run tauri:dev" -ForegroundColor Yellow
}
