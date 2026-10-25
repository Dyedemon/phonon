# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-10-25

### Added
- **媒体库**：Library tab 上线——全部/收藏/最近/专辑/艺术家/流派六个子页、FTS+拼音搜索、
  多选批量操作（Ctrl 加选 / Shift 范围选）、右键菜单（播放/下一首播放/加入队列/编辑元数据）
- **上下文播放**：媒体库双击 = 当前视图整表入队连播；列表子页"播放全部"；专辑页以专辑为上下文
- **下一首播放**（Play Next）：插到当前曲之后不打断播放，随机模式下同样生效（洗牌序即时重排）
- **标签文件回写**（lofty）：元数据编辑保存时写回音频文件本体（FLAC/MP3/M4A 等按容器分派），
  库与文件双向一致；播放中的文件自动跳过文件侧写入
- **批量整理工具**："补全标签"（从 Artist - Title 文件名回填标题/艺术家）与
  "归入专辑"（无专辑曲目按艺术家归组），库+文件同步写入
- **拼音搜索**：FTS 索引收录全拼+首字母（zhaolei / zl 均可命中赵雷），多音字有首字母兜底
- **快速找歌**：播放器侧栏搜索框，搜到即插播，免跨 tab
- **随机播放**：与循环模式四选一的互斥菜单；RepeatAll 洗牌整轮不重复
- **播放队列正名**：队列标题、空态引导、批量管理（多选删除/搜索过滤）
- 队列变更即时会话落盘（防托盘隐藏+强杀丢队列）；曲目行的播放统计轻量局部更新

### Fixed
- 同步文件夹不再把整个目录塞进播放队列（替换用户队列的事故根源）
- 媒体库表头与数据行对齐（sticky 表头）；收藏图标全局统一为爱心
- LIKE 搜索回退路径的 ESCAPE 子句错误（此前触发即报错）
- FTS5 external-content 列名对齐（rebuild 路径）
- TrackFilter 裸 bool 字段缺 serde(default) 导致库查询整体失败
- 媒体库 roots 跨重启持久化；编辑弹窗/专辑页文案与接线修正
- 启动 logo 死锁（会话保存后台线程与 load_session 锁序环）
- CHANGELOG 移除未实现的 ASIO 宣称

## [0.1.0] - 2026-08-29

### Added
- 完整的音频播放引擎，支持 WASAPI 独占模式、DSD (DoP / 原生 DSD 直通) 输出
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
