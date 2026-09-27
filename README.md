# Phonon

**高保真本地音乐播放器 / High-fidelity local music player**

Rust + Tauri v2 打造的桌面播放器，输出链路以"位透明（bit-perfect）"为第一设计目标：不经混音器、不重采样、不做多余变换，文件里是什么，DAC 就收到什么。

[![CI](https://github.com/Dyedemon/phonon/actions/workflows/ci.yml/badge.svg)](https://github.com/Dyedemon/phonon/actions/workflows/ci.yml)
![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)

## 特性

**输出质量**
- **位透明输出链** — 1.0x 速度下不削波、不加增益、不夹取；仅在时间拉伸激活或真实降位深时才施加限幅/抖动（dither）
- **采样率跟随源文件** — 每条曲目按需重建输出流（44.1k 文件 44.1k 出、96k 文件 96k 出），零重采样
- **WASAPI 独占模式** — 绕过 Windows 混音器直接独占设备；独占格式探测阶梯 PCM24 → PCM16 → Float32
- **DoP（DSD over PCM）** — PCM 源经二阶 Σ-Δ 调制器合成 1-bit DSD，以 DoP64/128/256 输出；标记字节严格交替，独占 24-bit PCM 容器逐位搬运
- **原生 DSD（非 DoP）** — DSD 文件（DSF/DFF）优先尝试原生 DSD 直通（Thesycon 约定容器，探测失败自动降级）→ 按文件实际速率 DoP → 两级抽取降为 PCM，三级回退保证任何设备都能出声
- **多声道真直通** — 5.1/7.1 源以源声道数重建输出流（0x3F/0x63F 声道掩码），不经折叠直通；设备不支持时自动回退立体声折叠
- **开发者诊断面板** — 实时显示输出模式、协商容器、设备缓冲、环形缓冲水位、欠载计数，一键复制排障

**功能**
- DSP 链：参数均衡（PEQ）、压缩器、环绕虚拟化，可按处理器启停
- WASM 插件系统（wasmtime）：DSP / 可视化插件，支持插件配置拉取模型
- 媒体库：SQLite + FTS 全文检索，扫描器无锁解析，播放历史自动修剪
- ReplayGain 响度均衡（BS.1770-4 计量）
- CUE 分轨支持、无损格式（FLAC / WAV / APE / DSF / DFF 等）与常见有损格式
- 桌面歌词、托盘弹窗、任务栏交互、硬件音量控制

## 下载

前往 [Releases](https://github.com/Dyedemon/phonon/releases) 页面获取安装包：

| 平台 | 形式 | 说明 |
|---|---|---|
| Windows 10/11 x64 | 自定义安装器 `*_setup.exe` | 内置卸载器，Direct2D 界面 |
| Linux x64 | `.deb` / `.AppImage` | 独占模式与 DSD 为 Windows 专属 |
| macOS | `.app` / `.dmg` | 未签名，首次打开需右键 → 打开（或 `xattr -cr`） |

> 独占模式 / DoP / 原生 DSD / 硬件音量为 Windows 专属能力；Linux 与 macOS 走共享模式 PCM 输出。

## 从源码构建

前置要求：Rust stable、Node.js 20+、npm。

```bash
git clone https://github.com/Dyedemon/phonon.git
cd phonon

# 开发模式（热重载）
cd phonon-tauri
npm install
npm run tauri:dev

# 生产构建（Windows 会额外产出 NSIS 包）
npm run tauri:build
```

Windows 自定义安装器（嵌入应用 + 独立卸载器）：

```powershell
powershell -ExecutionPolicy Bypass -File phonon-tauri/scripts/build-installer.ps1
# 产物：target/release/bundle/nsis/Phonon_0.1.0_x64-setup.exe
```

## 项目结构

```
phonon-core      音频引擎：输出流协商、解码泵、DSP 链、DoP/原生 DSD、独占模式
phonon-codec     解码器封装（symphonia）+ DSD 文件读取（DSF/DFF）
phonon-media     媒体库：扫描、SQLite + FTS、播放历史、ReplayGain
phonon-plugin    WASM 插件运行时（wasmtime）+ 插件 SDK
phonon-source    音源：本地 / 网络 / 播放列表
phonon-winui     Win32 专用 UI 组件（安装器/卸载器共享）
phonon-installer 自定义安装器（Direct2D）
phonon-uninstaller 独立卸载器（Direct2D）
phonon-tauri     Tauri 桌面应用（React 19 前端 + Rust 后端）
```

## 开发

```bash
cargo test --workspace          # 单元 + 集成测试
cargo clippy --all-targets      # 静态检查
cargo fmt --all                 # 格式化
cd phonon-tauri && npx tsc --noEmit   # 前端类型检查
```

CI 在每次 push 时于 Windows / Linux / macOS 三平台运行构建、测试、许可证审计与格式检查。

## 许可

- Phonon 本体：`MIT OR Apache-2.0` 双许可
- `phonon-codec` 解码库：`MPL-2.0`
- 第三方组件许可汇总见 [THIRDPARTY_LICENSES.txt](THIRDPARTY_LICENSES.txt)

使用本软件时，请确认所播放的音源、歌词、封面等素材均为你本人合法拥有或已获授权。
