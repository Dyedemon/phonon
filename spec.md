# Phonon 音频框架 Spec

## Why
构建一个纯净、跨平台、高保真、可扩展的音频应用开发平台。框架本身不携带任何第三方在线音源，但通过标准化的插件接口，让开发者可以像搭乐高一样自由扩展本地播放、DSP 处理、在线流媒体等功能。目标是成为音频领域的"VSCode"——内核极简，一切功能由插件赋予。

## What Changes
- 从零搭建 Phonon 音频框架项目，包含 Rust 音频引擎内核、插件系统、和 Tauri 跨平台 UI
- 实现 Bit-Perfect 高保真音频播放链路（WASAPI/CoreAudio/ALSA 独占模式、自动采样率切换）
- 实现 WebAssembly 插件运行时，支持 Source Provider、DSP Processor、Decoder 三类插件
- 实现基于 Tauri + React 的跨平台桌面 UI Shell
- 框架发布包仅包含本地文件输入插件和标准网络流协议插件，不包含任何第三方商业音源

## Impact
- Affected specs: 无（全新项目）
- Affected code: 全新项目，所有代码从零创建

## Non-Functional Requirements
- 从文件解码到音频输出的总延迟 ≤ 50ms
- 独占模式下 CPU 占用率增量 ≤ 2%（现代 PC 单核）
- 运行时内存占用（不含文件缓存）≤ 100MB
- 系统应使用分级日志（error, warn, info, debug），日志可输出到文件或控制台

## 项目结构与三窗口组件职责归属（v1.1 2026-08-04 增补，基于代码事实）

### 目录说明（关键辨析）
| 目录 | 作用 | 状态 |
|---|---|---|
| `Phonon/` 根 | Rust workspace（5 个 crate：phonon-cli / phonon-codec / phonon-core / phonon-plugin / phonon-source + phonon-plugin-sdk 子 crate） | ✅ 正在使用 |
| `phonon-tauri/` | **实际 Tauri 应用主目录（React + Vite + Tauri v2 src-tauri）**，所有 UI 代码、前端三 HTML 入口、70+ Tauri 命令、打包配置 `tauri.conf.json`、capabilities、`plugins-dist/` 3 个预编译 wasm 都在这里 | ✅ 正在使用（构建/打包根目录） |
| `phonon-app/` | 早期草稿空壳目录（仅残留 `src-tauri/Cargo.lock` + `package-lock.json` 2 个 lock 文件，无源码），**不作为构建或打包入口使用** | ❌ 历史残留（可安全忽略） |
| `plugins/` | 3 个 Rust 示例 Wasm 插件源码（gain_processor / simple_source / phase_vocoder_stretcher） + 预编译 `.wasm` + 7 个 JS Vis 插件 `plugins/vis/*.js` | ✅ 正在使用（SDK 示例） |

### 三窗口 HTML 入口架构（Vite build.rollupOptions.input 多页面）
| 入口 HTML | 加载 entry | 所属窗口 | 渲染的组件根 |
|---|---|---|---|
| `phonon-tauri/index.html` | src/main.tsx | `main` 主窗口（1200×800，无边框 TitleBar 自绘） | `<App />` + 六 Tab 路由 |
| `phonon-tauri/desktop-lyrics.html` | src/desktop-lyrics-main.tsx | `desktop-lyrics` 桌面歌词悬浮窗（透明/无边框/置顶/鼠标穿透切换） | `<DesktopLyrics />`（**仅在该窗口内渲染**） |
| `phonon-tauri/tray-popup.html` | src/tray-popup-main.tsx | `tray-popup` 托盘快捷弹窗（托盘图标坐标附近弹出，500×640） | `<TrayPopup />`（**仅在该窗口内渲染**） |

三窗口之间通过 `emitTo('desktop-lyrics', 'desktop-lyrics-update', payload)` 做定向事件通信；**每个窗口独立 React 根，不共享 App 组件树**（避免桌面歌词窗口意外拉入主窗口重型依赖的 bug）。

### 主窗口 `<App />` 六 Tab 路由（定义于 App.tsx 第 22、1074-1081、1130-1134 行）
| Tab id | Tab 名 | 渲染组件 | 说明 |
|---|---|---|---|
| `player` | Player / 播放主页 | `PlayerView` → 嵌入 `PlaylistToolbar` + `Playlist` + 底部 `PlayerBar`；独立 `<LyricsPage />`（全屏 Modal 开关） | 播放主控界面 |
| `devices` | Devices / 音频设备 | **`DeviceSelector`**（独立 Tab，不是 Settings 子页！） | 设备枚举、独占模式切换、采样率选择 |
| `dsp` | DSP / 处理链 | **`DspPanel`**（内嵌简易 `VisPanel` 作为可视化弹窗，可跳转 settings） | DSP 链顺序 + ReplayGain/EQ/Downmix/SmartEffect/SurroundSound 开关 |
| `extensions` | Extensions / 扩展面板 | **`ExtensionsPanel`**（嵌套三块子功能：① 自定义 CSS 编辑器 ② `CropEditor` 背景图 9 宫格裁剪 ③ **`PluginManager`** 插件列表/加载/权限/签名校验） | 扩展总控 Tab |
| `visualizer` | 可视化页面 | **`VisualizerPage`**（接 `AudioFeaturesData`，含 7 种 JS Vis 插件切换 + 插件编辑器） | 独立重型可视化 Tab |
| `settings` | 设置页 | **`Settings`**（内部 5 SubTab：general / appearance / audio / spectrum / about） | 通用设置 SubTab：语言、主题、音量模式、外观、Spectrum 配色、关于 |

### 18 个组件职责归属完整表（与 phonon-tauri/src/components 目录一 一 对应）
| 组件文件 | 职责归属（层级） | 在哪里被渲染 / 被谁 import | 完成度（代码现状） |
|---|---|---|---|
| `TitleBar.tsx` | 通用基础 | 主窗口根 `<App />` 最顶部，自绘拖动/最小化/最大化/关闭（data-tauri-drag-region） | ✅ 完整实现 |
| `Player.tsx` | Player 子组件 | 主 Tab `PlayerView` / `PlayerBar` 的内部播放详情大卡（封面+播放按钮+进度条 + `LyricsPanel` 内嵌面板） | ✅ 完整实现 |
| `Playlist.tsx` | Player 子组件 | 主 Tab `PlayerView` 的 `player-playlist-col` 区块内，紧凑列表模式（dnd-kit 骨架） | ✅ 完整实现 |
| `PlaylistToolbar.tsx` | Player 子组件 | 主 Tab `PlayerView` 顶部 Sidebar，播放列表管理（新建/重命名/切换/导入导出 M3U8） | ✅ 完整实现 |
| `DeviceSelector.tsx` | 独立 Tab 主组件 | Tab `devices` 直接渲染（第 1130 行），**独立设备页**，非 Settings 子页 | ✅ 完整实现 |
| `DspPanel.tsx` | 独立 Tab 主组件 | Tab `dsp` 直接渲染（第 1131 行），可跳转到 settings/visualizer；内嵌简易 Vis 弹窗 `VisPanel` | ✅ 完整实现 |
| `ExtensionsPanel.tsx` | 独立 Tab 主组件 | Tab `extensions` 直接渲染（第 1132 行），**汇总三块子功能：自定义CSS+CropEditor+PluginManager** | ✅ 完整实现 |
| `VisualizerPage.tsx` | 独立 Tab 主组件 | Tab `visualizer` 直接渲染（第 1133 行），接收 `window.CustomEvent('audio-features')`，重型可视化+插件编辑器 | ✅ 完整实现 |
| `Settings.tsx` | 独立 Tab 主组件（内 5 SubTab） | Tab `settings` 直接渲染（第 1134 行），general/appearance/audio/spectrum/about 5 SubTab 内部自绘 EQ 条（不用 EqPanel） | ✅ 完整实现 |
| `EqPanel.tsx` | DSP 子组件（备用独立组件） | Settings 里的 EQ 是内部自绘的滑块数组，EqPanel 保留作为"单独弹出 EQ 窗口"的备用组件（例如 DspPanel 跳转到详细 EQ） | ✅ 完整实现 |
| `Spectrum.tsx` | DSP 子组件 | 主 Tab Player 区域内嵌播放详情内或 Settings→Spectrum 预览，32 段对数 + 指数平滑 + peaks 衰减 | ✅ 完整实现 |
| `VisPanel.tsx` | DspPanel 子组件（简易弹窗版可视化） | DspPanel 内部作为轻量可视化弹窗打开，与 VisualizerPage（重型版）并行存在 | ✅ 完整实现 |
| `CropEditor.tsx` | ExtensionsPanel 子组件 | 嵌套在 ExtensionsPanel 内，背景图 3×3 裁剪，输出 `canvas.toDataURL('webp', 0.85)` | ✅ 完整实现 |
| `PluginManager.tsx` | ExtensionsPanel 子组件 | 嵌套在 ExtensionsPanel 内，Wasm 插件扫描/加载/权限/签名校验 UI + 插件 enable 状态 | ✅ 完整实现 |
| `LyricsPage.tsx` | 独立 Modal 全屏页 | `<App />` 第 1140-1154 行独立 Modal 开关（PlayerBar "打开歌词"按钮触发），带封面/播放控制/进度/滚动 | ✅ 完整实现 |
| `LyricsPanel.tsx` | Player 内嵌子组件 | 内嵌在 Player 主区域播放详情侧栏，负责搜索/导入/选择歌词源 + 显示当前歌词（Synced/Unsynced） | ✅ 完整实现 |
| `DesktopLyrics.tsx` | **桌面歌词窗口专属根组件** | 仅在 `desktop-lyrics-main.tsx` 渲染（主窗口不 import），KTV 渐变/设置/锁定/位置持久化 | ✅ 完整实现 |
| `TrayPopup.tsx` | **托盘弹窗窗口专属根组件** | 仅在 `tray-popup-main.tsx` 渲染（主窗口不 import），快捷播放控制/音量/进度/下一首上一首 | ✅ 完整实现 |

### 组件层级关系速览（避免 Panel/Page 混淆）
```
Main Window (App.tsx)
├─ TitleBar (自绘)
├─ Toast 容器
├─ 六 Tab 路由
│  ├─ player: PlayerView
│  │   ├─ Sidebar → PlaylistToolbar
│  │   ├─ Split
│  │   ├─ Playlist (Playlist.tsx 紧凑列表)
│  │   └─ Now-Playing 大卡 → Player.tsx 内嵌套 LyricsPanel (内嵌歌词面板)
│  │   └─ 全局底部 PlayerBar → 触发 open lyrics → 打开 LyricsPage 独立 Modal
│  ├─ devices → DeviceSelector
│  ├─ dsp → DspPanel → 按需弹出 VisPanel（简易版可视化）
│  ├─ extensions → ExtensionsPanel
│  │     ├─ Custom CSS 编辑器
│  │     ├─ CropEditor (背景图 3×3 裁剪)
│  │     └─ PluginManager
│  ├─ visualizer → VisualizerPage (重型版，接 audio-features 事件 + 7 插件 + 编辑器)
│  └─ settings → Settings (5 SubTab: general/appearance/audio/spectrum/about → 内部自绘 EQ 滑块)
└─ LyricsPage Modal (独立全屏歌词页，独立路由开关)

Desktop Lyrics Window (desktop-lyrics-main.tsx)
└─ DesktopLyrics.tsx (独立窗口根组件，不进入 Main Window 组件树)

Tray Popup Window (tray-popup-main.tsx)
└─ TrayPopup.tsx (独立窗口根组件，不进入 Main Window 组件树)
```

## ADDED Requirements

### Requirement: 音频引擎内核 — PCM 播放与设备管理
系统 SHALL 提供独立的音频引擎内核，负责将 32-bit float PCM 数据直接写入音频设备缓冲区，完全独立于 UI 和插件系统。

#### Scenario: 独占模式播放 FLAC 文件
- **WHEN** 用户通过命令行或 UI 播放一个 FLAC 文件
- **THEN** 引擎应自动检测文件采样率，以独占模式打开音频设备，设置匹配的采样率，并将解码后的 PCM 数据写入设备缓冲区，实现 Bit-Perfect 输出

#### Scenario: 播放过程中切换采样率不同的文件
- **WHEN** 当前正在播放 44.1kHz 文件，用户切换到 96kHz 文件
- **THEN** 引擎应自动关闭当前音频流，以新采样率重新打开设备，实现无缝切换，DAC 指示灯应正确反映采样率变化

#### Scenario: 设备不支持当前采样率
- **WHEN** 文件的采样率不被音频设备支持
- **THEN** 引擎应使用最高质量重采样器（rubato 或 libsamplerate SRC_SINC_BEST_QUALITY）将 PCM 转换到设备支持的最近采样率

#### Scenario: 音频设备热插拔
- **WHEN** 正在使用的 USB DAC 被拔出
- **THEN** 引擎应通过回调通知 UI，播放自动暂停；当新设备插入时，引擎应通知 UI 可用设备列表已更新，用户可选择切换到新设备继续播放

#### Scenario: 设备移除后自动切换
- **WHEN** 当前设备被移除，且用户配置了自动切换策略
- **THEN** 引擎应自动切换到下一个可用的音频设备并恢复播放

### Requirement: 音量控制策略
系统 SHALL 提供明确的音量控制来源，优先使用硬件音量，软件音量仅在硬件不支持时作为回退方案。

#### Scenario: 硬件音量调节
- **WHEN** 音频设备支持硬件音量控制（通过操作系统 API）
- **THEN** 系统应通过硬件音量接口调节 DAC 音量，UI 上标识为"硬件音量"，不修改 PCM 数据

#### Scenario: 软件音量回退
- **WHEN** 音频设备不支持硬件音量控制（如 WASAPI 独占模式）
- **THEN** 系统应作为最后一个 DSP 节点应用软件音量衰减，UI 上清晰标识为"软件音量"，用户知晓当前非 Bit-Perfect 路径

### Requirement: 解码层 — 全格式原生解码
系统 SHALL 支持 FLAC、ALAC、WAV、MP3、AAC、DSD（DSF/DFF）等主流音频格式的原生解码，输出统一为 32-bit float PCM。

#### Scenario: 解码 24-bit/192kHz FLAC 文件
- **WHEN** 引擎加载一个 24-bit/192kHz FLAC 文件
- **THEN** 解码器应正确解析所有元数据，输出 32-bit float PCM 流，且不进行任何软件音量衰减

#### Scenario: 解码 DSD 文件
- **WHEN** 引擎加载 DSF 或 DFF 文件
- **THEN** 解码器应提取原始 1-bit DSD 流，并支持 DoP（DSD over PCM）封装或高精度转 PCM 输出

#### Scenario: 提取内嵌专辑封面
- **WHEN** 音频文件包含内嵌封面图片（如 FLAC 的 Picture 块、MP3 的 ID3v2 APIC 帧）
- **THEN** 解码器应在元数据中提供 AlbumArt 字段，包含图片二进制数据和 MIME 类型

#### Scenario: 提取 ReplayGain 标签
- **WHEN** 音频文件包含 ReplayGain 标签（REPLAYGAIN_TRACK_GAIN、REPLAYGAIN_ALBUM_GAIN 等）
- **THEN** 解码器应在元数据中提供 ReplayGain 字段，包含 track_gain、track_peak、album_gain、album_peak 值

#### Scenario: 多声道文件声道布局
- **WHEN** 引擎加载多声道音频文件（如 5.1、7.1）
- **THEN** 解码器应在输出时携带声道布局信息（如 FrontLeft, FrontRight, FrontCenter, LFE, BackLeft, BackRight），输出设备打开时匹配声道数；若设备声道数不足，应通过内置下混 DSP 将多声道混音至立体声

### Requirement: CUE 文件解析与虚拟分轨
系统 SHALL 支持解析 CUE 文件，将整轨音频按 CUE 索引虚拟分轨，每个音轨视为独立播放实体。

#### Scenario: 加载整轨 + CUE 文件
- **WHEN** 用户通过本地文件源加载一个包含 FLAC 文件和同名 CUE 文件的目录
- **THEN** 系统应解析 CUE 文件，按照 INDEX 01 时间点将整轨虚拟分割为多个 Track，每个 Track 显示独立的标题、艺术家、时长

#### Scenario: CUE 文件内嵌元数据
- **WHEN** CUE 文件包含 PERFORMER、TITLE、REM DATE 等元数据字段
- **THEN** 系统应正确提取并显示这些元数据，优先使用 CUE 中的信息

#### Scenario: CUE 分轨无缝播放
- **WHEN** 用户在 CUE 分轨列表中连续播放
- **THEN** 系统应在同一整轨文件内通过 Seek 实现分轨切换，不中断音频流

### Requirement: 无缝播放与环形缓冲区
系统 SHALL 实现基于无锁 SPSC 环形缓冲区的播放引擎，支持无缝曲目过渡。

#### Scenario: 专辑无缝播放
- **WHEN** 播放队列中连续两首曲目属于同一专辑
- **THEN** 解码线程应在当前曲目剩余帧数低于阈值时预加载下一曲目，将数据紧接在队列末尾，输出流不中断，不产生爆音

#### Scenario: 用户手动切换曲目
- **WHEN** 用户在播放过程中点击"下一首"
- **THEN** 引擎应清空当前缓冲区，开始解码新曲目，切换延迟不超过 200ms

### Requirement: 输入源抽象层 (Source Provider API)
系统 SHALL 定义统一的 IAudioSource trait，所有音源（本地文件、网络流、插件提供的在线源）必须实现此接口。

#### Scenario: 本地文件源搜索
- **WHEN** 用户通过本地文件源搜索指定目录
- **THEN** 系统应返回该目录下所有支持的音频文件列表，包含元数据（标题、艺术家、专辑、时长、采样率、位深、声道数、专辑封面）

#### Scenario: 网络流输入播放
- **WHEN** 用户提供一个标准的 Icecast 或 HLS 流地址
- **THEN** 网络流源应建立连接，持续接收数据并解码，将 PCM 数据送入播放缓冲区

#### Scenario: 插件源注册
- **WHEN** 用户将符合 IAudioSource 接口的 Wasm 插件放入插件目录
- **THEN** 框架启动时自动扫描并加载该插件，插件提供的音源出现在可用音源列表中

### Requirement: 播放列表持久化
系统 SHALL 支持播放列表的保存与加载，兼容主流播放列表格式。

#### Scenario: 保存播放列表为 M3U8
- **WHEN** 用户保存当前播放列表
- **THEN** 系统应默认以 M3U8 格式（UTF-8 编码）保存，包含每个曲目的文件路径、标题、时长

#### Scenario: 加载外部 M3U/PLS 播放列表
- **WHEN** 用户打开一个 .m3u、.m3u8 或 .pls 播放列表文件
- **THEN** 系统应正确解析文件路径（相对路径和绝对路径），将曲目添加到播放列表

#### Scenario: 导入/导出 XSPF 播放列表
- **WHEN** 用户导入或导出播放列表
- **THEN** 系统应支持 XSPF（XML Shareable Playlist Format）格式，保持与 VLC、Audacious 等播放器的互操作性

### Requirement: WebAssembly 插件系统
系统 SHALL 基于 wasmtime 运行时提供 WebAssembly 插件系统，支持 Source Provider、DSP Processor、Decoder 三种插件类型，插件运行在沙箱中。

#### Scenario: 加载 Wasm 音频源插件
- **WHEN** 框架启动并扫描插件目录，发现一个 .wasm 插件文件
- **THEN** 框架应通过 wasmtime 加载该插件，验证其导出函数签名是否匹配 IAudioSource 接口，若匹配则注册为可用音源

#### Scenario: 运行时加载/卸载插件
- **WHEN** 用户通过 UI 启用一个已扫描但未加载的插件
- **THEN** 框架应在运行时加载并注册该插件，无需重启应用；用户禁用插件时，框架应卸载并释放相关资源，若该插件正在使用（如作为当前音源），应先安全停止再卸载

#### Scenario: Wasm 插件权限控制
- **WHEN** 一个 Wasm 插件尝试访问文件系统或网络
- **THEN** 框架应仅允许插件访问其声明的、且被用户授权的权限范围，默认无任何权限

#### Scenario: Wasm 插件资源限制
- **WHEN** 一个 Wasm 插件运行
- **THEN** wasmtime 运行时应对每个插件设置最大内存上限（默认 128MB）和指令执行时间配额（如单次调用超时 5s），超限后终止插件并通过 UI 通知用户

#### Scenario: 加载 Wasm DSP 插件（均衡器）
- **WHEN** 用户启用一个 Wasm 均衡器插件
- **THEN** DSP 插件应在音频处理链中生效，对 PCM 数据进行实时 biquad 滤波处理，延迟不超过 10ms

#### Scenario: 插件加载失败处理
- **WHEN** 一个 .wasm 插件文件损坏或接口不匹配
- **THEN** 框架应记录错误日志，跳过该插件，不影响其他插件和内核的正常运行

### Requirement: DSP 处理链架构
系统 SHALL 维护一个有序的 DSP 处理器链，音频数据在输出前依次经过每个启用的处理器。

#### Scenario: 多个 DSP 处理器串联
- **WHEN** 用户同时启用了均衡器、ReplayGain 应用器和卷积引擎
- **THEN** PCM 数据应按 DSP 链顺序依次经过每个处理器，每个处理器接收上一级的输出作为输入

#### Scenario: 调整 DSP 处理器顺序
- **WHEN** 用户在 UI 中拖动 DSP 处理器列表调整顺序
- **THEN** 系统应实时更新处理链顺序，后续音频数据按新顺序处理

#### Scenario: DSP 延迟声明
- **WHEN** 一个 DSP 处理器注册到处理链
- **THEN** 该处理器必须声明其引入的最大额外延迟（如 0ms、5ms），系统应计算总处理链延迟并显示在 UI 上

#### Scenario: ReplayGain 响度归一化
- **WHEN** 用户启用 ReplayGain 功能
- **THEN** 系统应作为可选 DSP 节点，根据音频文件元数据中的 ReplayGain 标签（track/album mode）自动调整增益，使不同曲目感知响度一致；此功能为可选，默认关闭以保持 Bit-Perfect 路径纯净

### Requirement: Tauri 跨平台 UI Shell
系统 SHALL 提供基于 Tauri + React 的跨平台桌面 UI，通过 Tauri invoke 命令与 Rust 后端通信。

#### Scenario: 播放控制
- **WHEN** 用户在 UI 中点击播放/暂停/上一首/下一首按钮
- **THEN** UI 应通过 Tauri invoke 调用后端对应命令，播放状态实时反映在 UI 上

#### Scenario: 文件浏览器与播放列表
- **WHEN** 用户通过 UI 文件浏览器选择音频文件或文件夹
- **THEN** 文件应被添加到播放列表，UI 显示曲目信息（标题、艺术家、时长、专辑封面），支持拖拽排序

#### Scenario: 播放进度与波形显示
- **WHEN** 音频正在播放
- **THEN** UI 应实时显示播放进度条、当前时间/总时长，并可选显示频谱或 VU 表

#### Scenario: 专辑封面显示
- **WHEN** 音频文件包含内嵌专辑封面
- **THEN** UI 应在播放控制栏和播放列表项中显示专辑封面缩略图；若封面不存在，显示默认占位图

#### Scenario: 设置页面
- **WHEN** 用户打开设置页面
- **THEN** UI 应显示音频输出设备选择、采样率配置、音量控制模式（硬件/软件）、DSP 插件管理与链排序、插件目录配置、ReplayGain 模式选择等选项

#### Scenario: 设备热插拔通知
- **WHEN** 音频设备插入或移除
- **THEN** UI 应通过 toast 或通知栏提示用户设备变化，设备列表实时更新

### Requirement: 命令行工具
系统 SHALL 提供命令行播放器工具，用于验证完整音频链路和自动化测试。

#### Scenario: 播放单个文件
- **WHEN** 用户执行 `phonon play <file>`
- **THEN** CLI 应解码文件并输出到音频设备

#### Scenario: 列出音频设备
- **WHEN** 用户执行 `phonon list-devices`
- **THEN** CLI 应列出所有可用音频设备及其支持的采样率

#### Scenario: 指定设备与独占模式
- **WHEN** 用户执行 `phonon play <file> --device <name> --exclusive`
- **THEN** CLI 应使用指定设备并以独占模式播放

#### Scenario: 循环播放与播放列表
- **WHEN** 用户执行 `phonon play --loop --playlist <file.m3u8>`
- **THEN** CLI 应加载播放列表文件并循环播放

### Requirement: 自动化测试
系统 SHALL 为核心模块提供单元测试和集成测试，CI 流水线应包含自动化测试步骤。

#### Scenario: 解码层单元测试
- **WHEN** 运行 `cargo test` 在 phonon-codec crate
- **THEN** 应执行各格式解码正确性测试、元数据提取测试、CUE 解析测试

#### Scenario: 环形缓冲区集成测试
- **WHEN** 运行 `cargo test` 在 phonon-core crate
- **THEN** 应验证生产者-消费者并发场景下无数据丢失、无竞态条件

#### Scenario: CI 回放测试
- **WHEN** CI 流水线运行
- **THEN** 应包含使用虚拟音频设备（dummy/null）的回放测试，验证解码→缓冲区→输出的完整链路不崩溃

### Requirement: 构建与打包
系统 SHALL 提供完整的构建流水线，支持 Windows、macOS、Linux 三平台编译和打包。

#### Scenario: 发布包内容
- **WHEN** 执行构建打包命令
- **THEN** 生成的安装包应包含：音频引擎内核、Tauri UI、本地文件输入插件、网络流输入插件、示例插件、插件 SDK 文档，不包含任何第三方商业音源插件

#### Scenario: 跨平台编译
- **WHEN** 在 CI 环境中分别编译 Windows、macOS、Linux 目标
- **THEN** 每个平台应生成对应的原生安装包（Windows：MSIX/NSIS/Portable ZIP；macOS：DMG+PKG（Universal: arm64+x86_64）经 notarytool 公证 + stapler 钉票；Linux：deb/rpm/AppImage），且功能一致

#### Scenario: 代码签名与 Gatekeeper/SmartScreen
- **WHEN** 生成正式 Release 构建
- **THEN** Windows 包必须经 SHA256 代码签名（推荐 EV 证书绕过 SmartScreen，OV 建立声誉）并带时间戳；macOS 包必须 `codesign --deep --options runtime` + `notarytool submit --wait` 通过 + `stapler staple` + `spctl --assess` 验证 Gatekeeper 通过

#### Scenario: Tauri Updater 差分更新
- **WHEN** 新版本发布
- **THEN** CI 使用 Ed25519 私钥签名新版本二进制，生成 `updater.json`（version、notes、pub_date、每个平台 url+signature），客户端启动时通过 Tauri v2 updater 插件校验签名后拉取更新

#### Scenario: 第三方代码版权随附
- **WHEN** 分发 Phonon 安装包
- **THEN** 安装包或应用设置页必须附随 `THIRDPARTY_LICENSES.txt`（Symphonia / wasmtime / rubato / cpal / Tauri / React 等全部依赖的版权声明），ASIO® 商标仅以"兼容 Steinberg ASIO 接口"字样说明，不随包分发 Steinberg ASIO SDK 二进制文件；用户若启用 ASIO 需自行安装 ASIO4ALL 或厂商驱动

### Requirement: Wasm 插件系统 — 四种类型 + TimeStretch + Hybrid
系统 SHALL 基于 wasmtime 运行时提供四种插件类型：Source（音源）、Processor（DSP 处理器）、TimeStretch（时间拉伸/音高变换）、Hybrid（同时实现多个接口）。

#### Scenario: 接口升级为 4 类型
- **WHEN** 插件加载时调用 `plugin-type()`
- **THEN** 返回值为 `source` / `processor` / `timestretch` / `hybrid` 四选一；hybrid 可同时实现多个接口（如一个插件既是 Source 又提供内建 DSP 处理）

#### Scenario: 运行时插件签名与可信校验
- **WHEN** 用户加载 `.wasm` 插件
- **THEN** 框架优先校验与 `.wasm` 同目录的 `.wasm.sig` 签名文件（Ed25519），签名通过才允许加载；官方签名插件在 UI 标为"可信"；第三方未签名插件必须由用户二次确认后才启用

#### Scenario: TimeStretch 插件 fallback
- **WHEN** 用户在设置中开启"音高校正 + 速度调整"但未加载 TimeStretch Wasm 插件
- **THEN** 框架自动回退到内置 rubato::PhaseVocoder 做变速不变调；若加载了 TimeStretch 插件则优先使用插件，并报告新增延迟用于 UI 总延迟显示

#### Scenario: Hybrid 插件权限模型
- **WHEN** 一个 Hybrid 插件同时声明 Source + Processor 能力
- **THEN** 两类接口的权限（文件读/网络）必须分别单独获得用户授权；任一接口被禁用时不影响另一接口的启用

### Requirement: 桌面歌词窗口
系统 SHALL 提供独立的桌面歌词窗口（Tauri 多窗口 label="desktop-lyrics"），以透明置顶悬浮层的形式显示逐字高亮的 KTV 效果滚动歌词。

#### Scenario: 窗口交互模式（锁定 / 解锁）
- **WHEN** 用户在托盘菜单或主窗口设置切换"歌词锁定"
- **THEN** 锁定状态下窗口支持鼠标穿透（click-through），用户无法拖动/缩放；解锁状态下可自由拖动位置、拉伸宽度、右键打开菜单（样式切换、透明度、字号、对齐方式、关闭桌面歌词）

#### Scenario: 歌词来源优先级（强制合规）
- **WHEN** 用户播放一首需要歌词的曲目
- **THEN** 歌词解析器按以下优先级取词，**仅允许合规来源**：
  1. 同目录同名 `.lrc` 文件（用户手动导入，本地路径）
  2. 音频文件内置 USLT/SYLT 标签（embedded lyrics）
  3. 用户在 Settings 中配置的"官方歌词授权 API"（**仅允许用户本人已获得合法授权的接口**）
  4. 以上均无 → UI 显示 `未找到歌词，请手动导入 .lrc 文件`，**严禁从未经授权的第三方平台自动批量下载与收集歌词**

#### Scenario: LRC 行匹配与容错算法
- **WHEN** 当前播放位置推进
- **THEN** 歌词行的切换规则：
  - `[mm:ss.xx]` 精确时间戳优先匹配；缺失时按相邻时间戳的位置做线性插值定位
  - 播放跳转、seek 或切歌时以二分查找 O(log N) 重置当前行索引
  - 允许 `<mm:ss.xx>词</mm:ss.xx>` 逐字标签（增强 LRC），用于 KTV 逐字渐变
  - 异常解析（错行、乱码、空时间戳）仅记录 warning 日志，不中断播放

#### Scenario: 逐字 KTV 渐变高亮 + 多种样式
- **WHEN** 当前行支持逐字时间戳
- **THEN** 渲染时用 CSS linear-gradient + background-clip text 做字级 KTV 渐变（从 0% 到 100%）；主窗口歌词页支持四种风格切换：滚动居中 / 双行 / 分裂翻译 / 卡拉OK；桌面歌词额外支持阴影、描边、透明度

#### Scenario: 桌面歌词持久化设置
- **WHEN** 用户调整桌面歌词字号、位置、颜色、样式、透明度、锁定状态
- **THEN** 立即写入 AppSettings.desktop_lyrics_* 并持久化到 `session.json`；下次启动桌面歌词窗口直接恢复位置与样式

### Requirement: 音频可视化系统
系统 SHALL 包含一套音频可视化系统：轻量 32 段频谱面板（嵌入主 UI） + 7 种 JS 可视化插件（VisPanel 切换），输入来自后端推送的 FFT 频谱与 PCM 块。

#### Scenario: 32 段频谱映射与平滑
- **WHEN** 后端以 ~20Hz 推送 `spectrum-data { spectrum: [f32; 1024], rms, peak }`
- **THEN** 前端将 1024 个线性 bin 映射为 32 段对数频率（20Hz–20kHz），对每段取 max；经过指数平滑（α≈0.42）+ 衰减峰值条（peaks 每帧回落 0.02），CSS `height` 归一化 0-1 渲染

#### Scenario: JS Vis 插件生命周期
- **WHEN** 用户在 VisPanel 切换可视化插件（bars / chroma / circle / mirror / particles / radial3d / wave，全部位于 `plugins/vis/*.js`）
- **THEN** VisManager 走 `init(canvas, ctx) → start() → onSpectrum(spec)  / onPcm(pcm) → stop() → destroy()` 生命周期；切换插件时严格调用旧插件 destroy 再 init 新插件，避免内存泄漏与重复动画帧

#### Scenario: 自定义 Vis 插件本地加载
- **WHEN** 用户将符合规范的第三方 `myvis.js` 放入用户插件目录 `%APPDATA%/Phonon/plugins/vis`
- **THEN** VisPanel 自动扫描并显示在切换列表中；加载前做合规校验：禁止 `fetch()` 跨域、禁止 `eval`，仅暴露 `canvas`/`ctx`/`spectrum`/`pcm` 四个受控对象

### Requirement: 媒体库管理与元数据索引
系统 SHALL 提供独立的媒体库（新建 `phonon-media` crate，SQLite FTS5），统一管理用户扫描目录的全部曲目、CUE 虚拟轨、封面缩略图、ReplayGain 批量扫描结果、收藏夹/历史/星级。

#### Scenario: 数据模型与软删除
- **WHEN** 用户将"音乐库路径"添加到媒体库设置
- **THEN** 库表包含 tracks/albums/artists/genres/folders/cue_sheets/play_history/favorites/thumbnails_cache/settings，核心 tracks 表有 `is_deleted` 软删除标志：文件从磁盘消失时先标记 `is_deleted=1`，待用户在媒体库设置页"清理已删除条目"确认后才物理删除

#### Scenario: 增量扫描（mtime/size 三元组判定）
- **WHEN** 启动自动扫描或用户手动触发扫描
- **THEN** 扫描线程对每个文件取 (mtime, ctime, file_size) 三元组，与 DB 中现存记录比较；完全一致则跳过，不一致才通过 `phonon-codec` 重新解析元数据；扫描支持 rayon 并行（默认 10 并发）并通过 `library-scan-progress` 事件推送到 UI

#### Scenario: CUE Sheet 虚拟轨关联
- **WHEN** 扫描到 `.cue` 文件
- **THEN** 解析器逐条 FILE → TRACK 生成 N 条虚拟轨（`cue_sheet_id` 非空），每条虚拟轨的 `duration_ms` 由相邻 INDEX 01 差值 / 整轨文件总时长推算；删除整轨文件时所有虚拟轨同步 `is_deleted=1`

#### Scenario: 缩略图缓存（WEBP LRU）
- **WHEN** 曲目内含封面 Picture 块或用户手动绑定本地封面 JPEG
- **THEN** 媒体库计算封面字节 SHA1 后，同时生成 256×256 与 512×512 两种 WEBP 缩略图写入 `thumbnails_cache`；UI 读取时按视图大小按需选尺寸，LRU 上限：默认 10000 个不同 hash，超出时逐出最久未访问

#### Scenario: ReplayGain 批量扫描
- **WHEN** 用户选择若干缺少 ReplayGain 标签的曲目启动批量扫描
- **THEN** 使用 ebur128（或 ffmpeg -filter ebur128，**不随 Phonon 发行版分发 ffmpeg 二进制，需用户自备**）逐个扫描，扫描结果写入：
  - 模式 A（推荐默认）：写入 `.phonon_data/replaygains.sqlite` 只读覆盖，不修改用户原文件
  - 模式 B（用户显式勾选"允许写回 Vorbis Comment"并二次确认）：FLAC/OGG/OPUS 安全写回 ReplayGain 标签；MP3/M4A/DSF 一律拒绝写回避免损坏文件，仅走模式 A

#### Scenario: 合规性 — 元数据仅手动，不联网
- **WHEN** 用户在曲目列表点击"编辑元数据"或"更换封面"
- **THEN** 仅接受两种输入：① 文本框手动键入标题/艺术家/专辑/流派；② 本地 JPEG/PNG 文件作为封面；**严禁调用任何第三方平台 API 做自动匹配、自动补全、批量下载与收集元数据/封面**；所有元数据变更均实时写回 SQLite +（可配置）写回音频文件标签

#### Scenario: 搜索 API 全文检索（FTS5）
- **WHEN** 用户在顶部搜索框键入关键词
- **THEN** 底层使用 `tracks_fts` 虚拟表（FTS5 content=tracks）对 title/artist/album/genre 做 BM25 排序，返回 TOP 200；模糊匹配失败再退回 LIKE 通配符搜索；搜索范围包含已收藏、已打分曲目时允许按播放次数加权

### Requirement: 前端 UI 规范
系统 SHALL 统一前端 React 18 + TypeScript + Vite 架构，所有 UI 遵循主题 CSS 变量系统、高频 invoke 节流、listen 事件清理、三窗口多入口等约定。

#### Scenario: 三窗口 HTML 入口
- **WHEN** Phonon 启动并创建窗口
- **THEN** Vite 以多页面模式输出三个入口：
  - `index.html` → main.tsx 主窗口 1280×800（无边框，TitleBar 自绘拖动）
  - `desktop-lyrics.html` → desktop-lyrics-main.tsx 桌面歌词窗口（透明/无边框/置顶）
  - `tray-popup.html` → tray-popup-main.tsx 托盘快捷弹窗（500×640 无边框，托盘图标坐标附近弹出）
三个入口共享 `api/` 与 `components/` 目录代码；UI 状态通过 Tauri 事件双向同步

#### Scenario: 主题系统（Dark/Light + 自定义 CSS 变量双通道注入）
- **WHEN** 用户切换主题或编辑自定义颜色/透明度
- **THEN** 按 CSS 变量统一改色：`--accent / --accent-hover / --bg / --bg-elevated / --bg-card / --text / --text-muted / --border / --danger / --success / --radius / --radius-sm / --shadow / --transition / --font-family / --font-size`；主题生效采用**双通道注入**防覆盖：
  1. `style.setProperty(var, value + ' !important')` 通道 1 强值
  2. `<style id="phonon-custom-css">` tag 把全部 20+ 变量写入 `:root`，切换 Dark/Light data-theme 时也不被覆盖

#### Scenario: 自定义预设 + 背景图 3×3 裁剪
- **WHEN** 用户在主题页保存配色/保存背景图
- **THEN** 自定义预设存 `localStorage['phonon-custom-presets']`（name→vars 映射，不允许同名，删除时二次确认且当前主题不能被删）；背景图通过 CropEditor 9 宫格裁剪（默认居中 + 用户自由拖拽四角）后 `canvas.toDataURL('image/webp', 0.85)` 存 localStorage，`document.body.style.background*` 即时生效

#### Scenario: 字体缩放（全局 zoom）
- **WHEN** 用户设置字号（12–20，默认 14）
- **THEN** `document.documentElement.style.zoom = userSize / 14` 全局缩放；rem / em 单位配合 zoom 保持布局一致

#### Scenario: 高频 invoke 节流 & listen cleanup
- **WHEN** 用户拖动 EQ 10 段滑块、颜色 picker 或音量滑块
- **THEN** 严格节流：EQ 滑块 `requestAnimationFrame` batching 只在最后一次 dragend 后 invoke 一次；颜色 picker 使用 500ms debounce；全局 `useListen<T>` Hook 必须在 `useEffect` cleanup 中调用 unlisten，避免 React StrictMode 双调用导致重复监听

#### Scenario: 全局快捷键（code 非 key）
- **WHEN** 用户按下系统级快捷键
- **THEN** 键位一律以 `KeyboardEvent.code`（KeyA / Space / MediaPlayPause / ArrowRight）注册，不以 `.key` 本地化字符匹配；默认：Space 播放暂停、Alt+←/→ 上/下一曲、Ctrl+↑/↓ 音量±；用户在设置页 Hotkeys 可完全自定义并持久化

### Requirement: 法律与合规性约束（强制）
系统 SHALL 在设计、开发、分发、使用的全流程遵守版权法、DMCA/千禧年数字版权法、信息网络传播权保护条例等适用法律法规，**严禁内置任何形式的未经授权第三方平台内容接口**。

#### Scenario: 音源合规
- **WHEN** 用户加载音源
- **THEN** Phonon 内核只内置两类源：① 用户本人合法拥有的本地文件（FLAC/ALAC/MP3/WAV/DSD 等）；② 标准协议的网络流（Icecast/HLS 等公开合法广播流）；任何**第三方**会员/曲库类音源必须由用户本人通过 Wasm 插件自行实现并承担相应法律责任，Phonon 发行版不随附此类插件，也不为此类插件提供下载/分发渠道

#### Scenario: 歌词与封面合规
- **WHEN** 显示歌词或封面
- **THEN** 歌词仅允许来自「本地 .lrc / 内嵌 USLT / 用户已获授权的官方歌词 API」三类；封面仅允许来自「音频文件内嵌 Picture 块 / 用户手动上传本地 JPEG/PNG」两类；**不得从事未经授权的批量下载与收集行为，不得规避或破坏任何数字版权保护技术措施**

#### Scenario: 插件与逆向合规
- **WHEN** 第三方 Wasm 插件提交到社区市场或被用户加载
- **THEN** 平台层对插件做静态规则扫描：如果插件以 `host.call_api()` 绕过权限、或以明文硬编码任何版权平台密钥/爬虫 URL，则拒绝加载并在 UI 标红提示"插件未通过合规校验"；禁止任何包含"规避或破坏保护"用途的插件被标记为可信

#### Scenario: 开源许可合规模型
- **WHEN** 分发 Phonon 或以插件形式链接 Symphonia
- **THEN** 解码层（phonon-codec）整体以 **MPL-2.0** 许可公开（与 Symphonia 上游许可一致）；其余内核、Tauri UI、Wasm Host 默认选择 MIT OR Apache-2.0；插件 SDK 采用 MIT；任何修改 MPL 覆盖文件的衍生作品必须公开对应修改部分的源代码

---
# Spec 更新记录
- v1.1 2026-08-04: 新增插件 4 类型（含 timestretch/hybrid）、桌面歌词、可视化 7 插件、媒体库 SQLite FTS5、前端 UI 三入口+主题系统、构建打包签名公证 Updater、完整法律合规章节；纠正 IAudioSource/DspProcessor/TimeStretch trait 模型，新增 FAQ 与测试策略，与 Trae 8 个 Phonon 技能（16 个中/英文目录）内容对齐
