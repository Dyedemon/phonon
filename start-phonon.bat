@echo off
if exist "d:\Phonon\target\debug\phonon.exe" (
    start "" "d:\Phonon\target\debug\phonon.exe"
    echo Phonon 已启动
) else (
    echo 未找到 phonon.exe，需要先安装 Rust 并编译
    echo 1. 安装 Rust: https://rustup.rs
    echo 2. cd /d d:\Phonon\phonon-tauri
    echo 3. npm install
    echo 4. npm run tauri:dev
)
pause
