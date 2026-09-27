# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-08-29

### Added
- 完整的音频播放引擎，支持 WASAPI 独占模式、ASIO、DSD (DoP) 输出
- 可插拔 DSP 链路架构，支持 EQ、环绕声、重低音、限幅器等效果
- WebAssembly 插件系统（内置 DSP 效果 / 外部扩展插件）
- 桌面歌词窗口，支持翻译行和双语显示
- 系统托盘弹窗，快捷控制播放和桌面歌词
- 多语言支持（中文 / 英文），包含主界面、桌面歌词、托盘弹窗
- 自动更新功能（基于 Tauri Updater）

### Fixed
- 修复 `play_url` 临时文件泄漏问题：新增 LAST_STREAM_TMP 追踪，每次播放前清理上次的临时文件
- 修复 `set_device` 持双锁做文件 I/O 问题：改为先 clone 设置快照再写磁盘
- 修复 `reset_to_defaults` 和 `set_plugin_limits` 同类持锁 I/O 问题
- 修复 `AlbumEditModal` 7 处错误 i18n key，新增 `library.albumEdit.*` 翻译
- 修复 `DspPanel` 500ms 轮询浪费 CPU：改为 2s 轮询 + 事件驱动更新
- 修复 Settings 页面 DSD 标题硬编码，新增 `settings.dsdMode` i18n key
- 修复 PluginManager 10 处硬编码中文字符串，完整国际化
- 修复桌面歌词和托盘弹窗硬编码中文字符串，新增独立窗口 i18n 字典
- 修复 phonon-plugin release 模式下未使用变量警告
- 修复 TypeScript 构建错误（App.tsx、LibraryPage 等 8 处）

### Performance
- DspPanel 轮询间隔从 500ms 降低到 2000ms，通过事件监听保证交互响应性
- 优化 settings 持久化模式，避免在持有锁时执行磁盘 I/O
- 临时文件清理兜底时间从 1 小时缩短到 10 分钟

### Security
- 配置文件使用 serde JSON 序列化，所有字段均有 Default 实现，旧配置可安全读取
