---
name: phonon-project
description: Phonon 高保真音频应用开发平台规范与约束。触发条件：用户在 Phonon 项目（c:\Users\30951\Desktop\ran\Phonon）内进行任何开发、调试、重构、插件编写、UI 调整、音频处理相关工作。
---

# Phonon 项目技能

## 项目定位
Phonon 是纯净、跨平台、高保真、可扩展的音频应用开发平台。目标是「音频领域的 VSCode」。
- **纯净**：发布包仅含本地文件源和标准网络协议插件，不含第三方商业音源。
- **高保真**：原生 Bit-Perfect 音频链路（独占模式、自动采样率切换、硬件音量优先）。
- **可扩展**：通过 Wasm 沙箱提供 Source Provider / DSP Processor / Audio Decoder / Visualizer 四类扩展。
- **跨平台**：Rust 引擎 + Tauri UI，三平台原生体验。

## Cargo Workspace 结构
- `phonon-core`：音频引擎内核（播放状态机、环形缓冲区、设备管理、DSP 链、特征提取）。
- `phonon-codec`：解码层（Symphonia 封装 + DSD 解析 + CUE 支持 + 元数据）。
- `phonon-source`：输入源抽象（IAudioSource trait、本地文件源、网络流源、播放列表）。
- `phonon-plugin`：Wasm 插件运行时（wasmtime 集成、权限控制、资源限制、生命周期）。
- `phonon-cli`：命令行验证工具。
- `phonon-tauri`：Tauri 集成与命令层（AppState、事件推送、任务栏、托盘）。
- 前端：React + TypeScript + Vite。
- `plugins/`：Wasm 插件源码与编译产物；`plugins/vis/`：JS 可视化脚本。

## 核心技术约束（硬性）

### Rust 引擎层
- 音频实时路径必须用同步线程 + 无锁结构，禁止在音频回调内分配堆内存或加锁。
- 网络和 I/O 密集任务用 `tokio`。
- 错误处理：`anyhow` + `thiserror`，底层错误向上传播时附加上下文。
- 日志：`log` crate 宏，禁止 `println!` 用于调试输出。
- 命名：Crate 名前缀 `phonon-`，模块小写下划线，类型 CamelCase，trait 名前缀 `I` 或无前缀。
- 端到端延迟 ≤ 50ms；独占模式 CPU 增量 ≤ 5%；运行时内存（不含文件缓存）≤ 512MB。

### 前端层（phonon-tauri/src）
- React + TypeScript，Vite 构建。
- 高频 Tauri invoke 必须用 rAF 批处理或 500ms 防抖，禁止在 HMR 期间高频调用（会触发 'Couldn't find callback id' 错误）。
- 颜色/透明度变更：双通道覆盖（setProperty('important') + `<style>` 标签 !important）。
- 主题效果：仅 dark/light 模式设置 data-theme；自定义预设由 handlePresetChange 控制。
- 自定义 CSS：注入 `<style id="phonon-custom-css">`，存 localStorage 启动恢复。
- 自定义背景图：直接设 `document.body.style.backgroundImage`（cover/center/fixed）。
- Spectrum 组件必须显式接收 `customSpectrumColors` prop，避免 ReferenceError。
- 自定义颜色预设存 localStorage `phonon-custom-presets`，带红色 × 删除按钮（hover 显示 + confirm() 确认）。

### Tauri v2 约束
- tauri-plugin-global-shortcut 必须在 `capabilities/default.json` 显式声明 `global-shortcut:allow-register` 权限。
- 全局快捷键必须在应用启动时注册（非仅设置页打开时），使用 e.code（如 'KeyA'、'Space'）而非 e.key。
- 关闭行为必须可配置：「最小化到托盘」（默认）或「退出程序」。
- AppHandle.spawn() 不存在；WndProc 回调内无 tokio 上下文，需用 `std::thread::spawn`。


### 桌面歌词
- 手动滑块拖动优先，3 秒无操作后恢复自动同步。
- 使用 getBoundingClientRect() 计算位置（非 offsetTop）。
- 展开/收起不得影响外层播放器布局。
- 双行模式才启用滚动过渡效果和对齐模式（默认/居中/左右分离）。
- KTV 式逐字右移滚动：仅在内容超出显示区域时启用，跟随播放位置不提前滚动。
- 逐字高亮：linear-gradient(90deg, played-color, unplayed-color) + background-clip:text。
- 滚动用 frame-delta 归一化指数平滑（halfLifeMs=90ms）。
- 控制按钮 36x36px，8px 圆角，6px 间距。
- 分离模式间距：max(100px, 26% 窗口宽度)。
- ScrollingText 用 translate3d GPU 加速 + ResizeObserver 防动画重启。


### Wasm 插件
- 默认无权限：不注入任何 WASI 功能。
- 按需授权：用户在 UI 为特定插件授予受限文件目录或网络地址白名单。
- 内存上限 512MB；调用超时 5s（tokio::time::timeout 包裹）。
- 插件崩溃仅影响自身，内核记录日志并通知 UI，不影响其他插件或播放。
- 生命周期：发现 → 加载（验证导出函数签名）→ 运行时启用/禁用 → 卸载。

## 开发约定
- 优先编辑现有文件，不创建新文件除非必要。
- 不主动创建文档文件（*.md / README）除非用户明确要求。
- 不添加未要求的功能、重构、错误处理、注释、类型注解。
- 不为不可能发生的场景添加防御性代码。
- 测试：每个 crate 应有单元测试和集成测试，关键路径（解码、环形缓冲区）必须有并发测试。
- 分级日志：error, warn, info, debug。

## 合法合规要求
- 框架本身不携带任何第三方商业音源，一切功能通过插件赋予。
- 发布包仅包含本地文件源和标准网络协议插件。
- Wasm 插件默认无权限，不能访问文件系统或网络。
- 用户授权必须明确，通过 UI 进行，不自动授予。
- 插件崩溃隔离，不影响宿主或其他插件。
- 不生成鼓励或指导自残、自杀、毒品、赌博或色情的内容。
- 未成年人安全：拒绝生成涉及未成年人的暴力、色情、毒品、自残等内容。
