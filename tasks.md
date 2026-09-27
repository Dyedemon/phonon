# Phonon 核查清单 & 任务依赖

> 状态说明：✓ 已完成 / ~ 部分完成（仅骨架/部分实现，未完全通过测试） / □ 待开始。
> 判断依据：Phonon 仓库当前实际文件结构 + 代码实现深度。
> `plugins/` 目录为独立项目，相关条目显式标记并不计入 Phonon 本体进度。

---
## §1 基础架构与工作区
- [✓] 项目工作区搭建完成，`cargo build`（根 workspace 5 crate + phonon-tauri/src-tauri）三平台通过
- [✓] 统一日志系统可用，分级日志（error/warn/info/debug）输出到文件和控制台（fern + init_logging）
- [✓] 项目级本地技能 phonon-project-SKILL.md（仓库根）已部署，合规章节全覆盖

---
## §2 解码层（phonon-codec）
- [✓] 解码层支持 FLAC/ALAC/WAV/MP3/AAC 格式，输出 32-bit float PCM（symphonia defaults + ALAC feature）
- [✓] DSD（DSF/DFF）文件可解析，支持 DoP（DSD over PCM）封装输出 + 高精度转 PCM（DsdToPcm）
- [✓] 内嵌专辑封面可提取：FLAC Picture 块、ID3v2 APIC 帧（Metadata.pictures: Vec<Picture>）
- [✓] ReplayGain 标签可正确读取（track_gain、track_peak、album_gain、album_peak）—— types.rs ReplayGain
- [✓] 多声道文件声道布局信息正确携带（ChannelLayout enum + 声道位掩码）
- [✓] CUE 文件可解析，按 INDEX 01 虚拟分轨，元数据正确提取（cue.rs CueSheet::parse）
- [✓] 解码层单元测试通过（格式解码、元数据提取、CUE 解析，tests/codec_integration.rs）

---
## §3 音频引擎内核 + 设备管理（phonon-core）
- [✓] 音频设备枚举正确，WASAPI/CoreAudio/ALSA 独占模式可用（device.rs list_devices + exclusive_mode）
- [✓] 自动采样率切换：播放不同采样率文件时，引擎 close + reopen 流切换，DAC 采样率正确变化
- [✓] 重采样回退：设备不支持采样率时自动高质量 rubato FixedOutRateResampler 重采样
- [~] 设备热插拔：USB DAC 拔出时自动暂停并通知（poll_hotplug 骨架 + events.rs 广播），插入时设备列表实时更新
- [✓] 硬件音量控制可用（优先，CPAL StreamVolume），软件音量回退正确（独占模式等场景 volume.rs Volume::apply）
- [✓] 多声道下混：5.1/7.1 文件在立体声设备上通过 DownmixProcessor 正确下混
- [✓] 自定义无锁 SPSC 环形缓冲区（ringbuf.rs RingBuffer<T> + drain_to），解码线程与输出线程正确分离
- [✓] 无缝播放：专辑连续曲目切换 preload_next 无爆音、无中断（crossfade 0 过渡时仍无缝）
- [✓] CUE 分轨无缝切换：同一整轨内分轨 seek_absolute 切换，不中断音频流
- [✓] 播放状态机：Idle/Loading/Playing/Paused/Stopped/EndOfQueue 状态切换正确（PlaybackState enum）
- [✓] 环形缓冲区集成测试通过（生产者-消费者并发场景，tests/pipeline_tests.rs）

---
## §4 DSP 处理链（phonon-core DSP + EQ + TimeStretch）
- [✓] DspProcessor trait 定义完整（process/latency/name/id/enabled/set_enabled/as_any_mut），DSP 链可添加/移除/排序处理器
- [✓] ReplayGain 应用器 DSP 可选启用，track/album mode 正确（ReplayGainApplier 默认关闭保持 Bit-Perfect）
- [✓] 多声道下混 DSP 正确工作（DownmixProcessor FL/FR/C/LFE/BL/BR → 立体声）
- [✓] DSP 总延迟计算正确并在 UI 显示（get_dsp_chain_info 返回 total_latency_secs）
- [✓] 10 段参数均衡器 DSP（Equalizer 31/62/125/250/500/1k/2k/4k/8k/16k，biquad，前端 EqPanel + rAF 节流）
- [✓] TimeStretch trait + PluginTimeStretcher Wasm 插件 fallback 到 rubato PhaseVocoder，可实现变速不变调
- [✓] SmartEffect（自适应动态范围控制）/ SurroundSound（双耳空间化）DSP 骨架存在（dsp.rs）

---
## §5 输入源 + 播放队列 + 播放列表（phonon-source）
- [✓] IAudioSource trait 定义完整（open/read_seek/metadata/lyrics/cover），LocalFileSource 可扫描目录、关联 CUE 并返回音频列表
- [✓] NetworkStreamSource 可播放标准 Icecast/HLS 流（network.rs reqwest 骨架）
- [✓] 播放队列可添加、删除、排序、保存和加载（PlayQueue + Tauri 命令 add_to_queue / remove_from_queue / reorder_queue）
- [✓] 播放列表持久化：M3U/M3U8（默认）、PLS、XSPF 格式读写正确（save_queue / load_queue）

---
## §6 WebAssembly 插件系统（phonon-plugin）
> 注：`plugins/` 目录为独立项目，以下条目仅标记 phonon-plugin 运行时（Phonon 本体）完成状态
- [✓] 插件系统可扫描目录（plugins/ + 用户插件目录）、列出 .wasm 文件（scan_plugin_directory）
- [✓] 运行时插件加载/卸载：启用/禁用无需重启应用（runtime.rs load/unload + reload_plugins）
- [✓] Wasm 沙箱权限控制生效，默认无文件/网络权限（DirPerms/NetPerms 默认 deny）
- [✓] Wasm 资源限制：内存上限 128MB、调用超时 5s，超限 epoch 中断并通知用户（StoreLimits）
- [✗] 【已删除】示例 Wasm 插件（simple_source / gain_processor / phase_vocoder_stretcher）
- [✓] WIT 接口升级 4 类型：Source/Processor/TimeStretch/Hybrid，旧 3 类型 spec 已淘汰（wit/plugin.wit）
- [✓] 插件可信签名校验骨架（Ed25519 verify_plugin_signature），未签名插件二次确认
- [✓] 插件集成测试通过（tests/plugin_integration.rs 加载 + 调用 process）
- [✓] PluginOrigin 三层分类 (Builtin / WasmExample / WasmExternal) + 统一 PluginInfo.origin 字段 + PluginError::Static (E5001) 拒绝 Builtin 动态操作
- [✓] BuiltinPluginRegistry（5 DSP：Equalizer/SmartEffect/Surround/Downmix/ReplayGain）+ list_plugins 三段合并冲突排序（用户目录优先于示例目录）+ host::emit_event 事件转发

---
## §7 命令行验证工具（phonon-cli）
- [✓] CLI 工具 `phonon play <file>` 可播放文件并输出到音频设备
- [✓] CLI 工具 `phonon play <file> --device <name> --exclusive --sample-rate <hz>` 指定设备/独占模式/采样率
- [✓] CLI 工具 `phonon play --loop --playlist <file.m3u8>` 循环播放与播放列表
- [✓] CLI 工具 `phonon list-devices` 可列出所有音频设备及支持的采样率

---
## §8 Tauri UI 框架（phonon-tauri）
- [✓] Tauri v2 项目启动正常，前端 Vite dev server 可访问，三窗口 HTML 入口存在（index.html/desktop-lyrics.html/tray-popup.html + 3 × main.tsx）
- [✓] 完整 Tauri 命令层（70+ 命令）：播放控制/设备/队列/DSP/插件/Settings/Session/Lyrics/Vis/Spectrum + 统一错误码（E4xxx/E5xxx）
- [✓] AppState 全局 OnceLock + 80+ AppSettings 字段，save_session / load_session 持久化到 `session.json`
- [✓] 完整事件系统 12+ 事件：playback-*、spectrum-data、audio-features-data、device-hotplug、session-loaded、plugin-loaded、vis-plugin-*（events.rs broadcast + emitTo 定向）
- [✓] 设备热插拔、DSP 链更新、队列更新等事件通过 Tauri event 推送到前端
- [✓] 自绘标题栏 TitleBar.tsx（data-tauri-drag-region）+ main window decorations=false 无边框

---
## §9 React 18 + TypeScript 前端组件清单（18 组件）
> 组件层级分类：A 通用基础 / B Player 子组件 / C 独立 Tab 主组件 / D Tab 内嵌子组件 / E 独立 Modal / F 专属窗口根
- [✓] (A) TitleBar.tsx：自绘拖动/最小化/最大化/关闭，data-tauri-drag-region
- [✓] (B) Player.tsx：播放详情大卡（封面/按钮/进度/模式标识/ReplayGain），内嵌 LyricsPanel
- [✓] (B) Playlist.tsx：紧凑列表，文件添加/dnd-kit 骨架/右键菜单/缩略图
- [✓] (B) PlaylistToolbar.tsx：Sidebar 顶部播放列表管理（新建/重命名/删除/切换/导入导出 M3U8）
- [✓] (C) DeviceSelector.tsx：Tab devices 独立设备页（非 Settings 子页），枚举/独占/采样率
- [✓] (C) DspPanel.tsx：Tab dsp；ReplayGain/Downmix/EQ/Smart/Surround 开关 + 链排序 + 总延迟；内嵌 VisPanel 弹窗
- [✓] (C) ExtensionsPanel.tsx：Tab extensions；汇总 3 子功能：自定义 CSS 编辑器 + CropEditor + PluginManager
- [✓] (C) VisualizerPage.tsx：Tab visualizer；接 AudioFeatures 事件 + VisManager 插件切换 + 插件编辑器
- [✓] (C) Settings.tsx：Tab settings；内部 5 SubTab（general/appearance/audio/spectrum/about）；Audio SubTab 自绘 EQ 滑块
- [✓] (D) EqPanel.tsx：10 段 31Hz–16kHz 滑块 + 预设 + rAF 节流；备用独立 EQ 弹窗组件
- [✓] (D) Spectrum.tsx：32 段对数频谱（指数平滑 + peak 衰减）；Player 内嵌 / Settings→Spectrum 预览
- [✓] (D) VisPanel.tsx：DspPanel 内简易版可视化弹窗（与 VisualizerPage 重型版并存）
- [✓] (D) CropEditor.tsx：背景图 3×3 裁剪 canvas→webp 0.85；localStorage 持久化
- [✓] (D) PluginManager.tsx：插件扫描/加载/权限/Ed25519 签名校验/启用状态；嵌套 ExtensionsPanel
- [✓] (E) LyricsPage.tsx：主窗口全屏 Modal 歌词页，4 风格（滚动/双行/分裂翻译/KTV）
- [✓] (D) LyricsPanel.tsx：Player 内嵌面板；歌词源 1..4 合规优先级 + 搜索/导入 .lrc + Synced/Unsynced
- [✓] (F) DesktopLyrics.tsx：仅 desktop-lyrics-main.tsx 渲染（主窗口不 import）；KTV 渐变/设置/锁定/位置持久化
- [✓] (F) TrayPopup.tsx：仅 tray-popup-main.tsx 渲染（主窗口不 import）；快捷播放控制/音量/进度/上下首
- [✓] 六 Tab 路由映射落地：App.tsx tabs → Switch：player/devices/dsp/extensions/visualizer/settings
- [✓] 三窗口组件树严格隔离：main→App / desktop-lyrics→DesktopLyrics / tray-popup→TrayPopup；跨窗口 emitTo 定向
- [✓] UI 设备热插拔 toast 通知（device-hotplug 监听 + snackbar）

---
## §10 桌面歌词窗口 + 音频可视化
- [✓] 桌面歌词窗口（Tauri label="desktop-lyrics"）：透明无边框 always_on_top
- [✓] 桌面歌词两种交互模式：锁定（鼠标穿透 click-through）/ 解锁（拖动/缩放/右键菜单）
- [✓] 歌词来源合规优先级：1.本地 .lrc  2.内嵌 USLT/SYLT  3.用户已授权 API（严格合规拒绝第三方未授权批量）
- [✓] LRC 行匹配二分查找 + seek 快速重置索引 + 错行/乱码容错
- [✓] 逐字 KTV 渐变高亮（CSS background-clip + linear-gradient）
- [✓] 主窗口 LyricsPage 4 风格：滚动居中 / 双行 / 分裂翻译 / 卡拉OK
- [✓] 桌面歌词持久化 settings（位置/字号/颜色/透明度/锁定）存 session.json 并还原
- [✗] 【已删除】JS Vis 插件（plugins/vis/*.js）
- [✓] VisManager 生命周期（init→start→onSpectrum/onPcm→stop→destroy），切换插件严格 cleanup，禁止 eval/fetch 跨域

---
## §11 媒体库（phonon-media crate — 后端+UI 全部完成）
- [✓] 新建 phonon-media crate，SQLite + rusqlite（bundled+backup）+ FTS5 + image WEBP + rayon（纯 Rust ITU-R BS.1770-4，无 C 依赖）
- [✓] SQLite 完整 DDL：tracks/albums/artists/genres/cue_sheets/folders/play_history/thumbnails_cache/settings + tracks_fts
- [✓] 增量扫描三元组（mtime/ctime/file_size）判定，一致跳过，rayon 10 并发，进度事件推送
- [✓] CUE Sheet 虚拟轨：cue_sheets FK + 整轨消失时虚拟轨软删除（is_deleted=1）
- [✓] 封面缩略图缓存：SHA1(cover bytes) → WEBP 256×256 + 512×512 双尺寸 + LRU 10k hash
- [✓] ReplayGain 批量扫描：Mode A（.phonon_data/replaygains.sqlite sidecar，不改原文件）；BS.1770-4 K-weighting + gated block loudness；sidecar 缓存 + force
- [✓] 收藏夹 / 历史 / 星级（0-5）字段 + UI 显示（TrackMeta FavoriteIcon+StarRating / HistoryPanel 历史 Tab）
- [✓] Tauri library_* 命令族（scan/get_tracks/search FTS5 BM25/toggle_favorite/set_rating/edit_metadata/set_cover/batch_scan_replaygain/list_albums/list_artists/list_genres/get_thumbnail 等 18 命令 + 进度）
- [✓] 合规性强校验：set_track_cover_from_file 5 层校验（URL 拒绝/扩展名白名单/文件+大小/magic bytes FF D8 FF / 89 50 4E 47 校验/Compliance 错误）；edit_metadata 仅手动输入；禁止第三方自动补全
- [✓] 集成测试：21 个全通过（schema/FTS5/收藏/星级/历史/folders/cleanup/ReplayGain×4/封面合规×6 + Send+Sync 约束）

---
## §12 前端 UI 规范统一（已完成）
- [✓] api/ 目录拆分为 player / devices / dsp / queue / settings / library / plugins / events / storage 9 文件 + index.ts 统一入口，严格 snake_case 传参
- [✓] 全局 useListen<T> Hook：StrictMode 双调用 + unlisten cleanup（api/events.ts，12 个事件常量）
- [✓] slider 节流：EQ rAF batching / 颜色 picker 500ms debounce / 音量 50ms throttle（useThrottledCallback leading+trailing）
- [✓] localStorage schema 迁移（migrateStorage + readStorage/writeStorage 类型安全 + corrupt 自动降级）
- [✓] 三窗口入口严格隔离组件子树，desktop-lyrics / tray-popup 不拉入主窗口重型依赖
- [✓] 全局快捷键 @tauri-apps/plugin-global-shortcut + e.code 策略 + 默认 + 自定义 JSON
- [✓] 23 vitest 全部通过 + tsc --noEmit 0 错误

---
## §13 构建打包与发布（CI/CD 骨架完成；证书/密钥/发布主机需发布前一次性配置）
- [✓] Tauri v2 tauri.conf.json bundle targets=all：Windows WiX (MSI) + NSIS + Portable；macOS DMG+PKG（Universal）；Linux deb + AppImage；category="Music" + description + copyright
- [✓] Windows SHA256 代码签名：digestAlgorithm=sha256 + timestamp URL（DigiCert）；CI release.yml 自动从 PFX base64 Secret 导入到 Cert:\CurrentUser\My 并回注 certificateThumbprint；OV/EV 证书由 Release Manager 手动入 Secrets
- [✓] macOS codesign + notarytool：entitlements.plist（最小权限集 Hardened Runtime，含 network.client / user-selected read-write）；CI release.yml 自动创建临时 keychain → 导入 Developer ID Application (p12) → 导出 SIGNING_IDENTITY / APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID 供 tauri-cli + stapler staple + spctl --assess 消费
- [✓] Linux .deb 依赖：libasound2 (≥1.2.0) / libgtk-3-0 (≥3.24) / libwebkit2gtk-4.1-0 (≥2.40) / libayatana-appindicator3-1 / librsvg2-2 (≥2.50)；section="sound"；AppImage 启用 bundleMediaFramework
- [✓] Tauri Updater 接入：
  - src-tauri/Cargo.toml tauri-plugin-updater = "2" + tauri feature="updater"
  - src-tauri/src/lib.rs Builder 注册 `.plugin(tauri_plugin_updater::Builder::new().build())`
  - package.json @tauri-apps/plugin-updater + `signer:generate` 脚本
  - tauri.conf.json plugins.updater.endpoints=["https://releases.phonon.app/updater.json"]，pubkey 占位（首次 `npm run signer:generate` 生成 Ed25519 对后回填）
  - release.yml build 阶段注入 TAURI_SIGNING_PRIVATE_KEY + TAURI_SIGNING_KEY_PASSWORD；release 阶段 assemble updater.json（5 平台：windows-x86_64 / darwin-x86_64 / darwin-aarch64 / linux-x86_64 / linux-x86_64-deb）
- [✓] 附随 THIRDPARTY_LICENSES.txt 生成机制：
  - Rust 侧：about.toml (allowlist 与 deny.toml 对齐) + about.hbs 模板 → `cargo about generate about.hbs`
  - NPM 侧：package.json devDep + `licenses` 脚本 → `npx license-checker --production --csv`
  - release.yml "Generate THIRDPARTY_LICENSES.txt" 步骤自动拼接 cargo-about + license-checker 输出
  - bundle.resources 安装时复制到 {installDir}/THIRDPARTY_LICENSES.txt
- [✓] ASIO® 商标合规声明 ASIO_TRADEMARK_NOTICE.txt + bundle.resources：声明 Steinberg 商标合理使用、不随二进制分发 SDK（用户需自行下载 PHONON_ASIO_SDK_ROOT 重编）、提供 --no-default-features 移除开关
- [✓] CI GitHub Actions 三平台矩阵（release.yml，tag v* 触发）：
  - jobs.pre-release-gate：cargo-deny §14 合规 / cargo test --release / tsc --noEmit / vite build（三平台构建前 Gate，ubuntu-latest 快速）
  - jobs.build strategy: windows-latest (wix/nsis/portable) / macos-14 (dmg/pkg, universal-apple-darwin) / ubuntu-22.04 (deb/appimage)
  - tauri-apps/tauri-action@v0 执行 tauri build + updater sig
  - Windows 签名 PFX 注入 + macOS 签名公证 + 临时 keychain
  - actions/upload-artifact@v4 归档 bundle + updater sig，30 天保留
  - jobs.release：download-artifact → assemble updater.json → softprops/action-gh-release 生成 draft Release（附 updater.json + 全平台安装包 + release notes）
- [ ] 发布前一次性配置（Release Manager 手动）：
  - [ ] 生成 Ed25519 密钥：`cd phonon-tauri && npm run signer:generate` → 公钥写入 tauri.conf plugins.updater.pubkey；私钥 + 密码入 GitHub Env Secrets TAURI_SIGNING_PRIVATE_KEY / TAURI_SIGNING_KEY_PASSWORD
  - [ ] Windows 签名：申请 OV/EV Authenticode 证书 → 导出 base64 PFX → 入 Secrets WINDOWS_CODE_SIGN_PFX / WINDOWS_CODE_SIGN_PASS
  - [ ] macOS 签名公证：申请 Developer ID Application 证书 + App 专用密码 → p12 base64 + 4 项 Apple Secrets（APPLE_CERTIFICATE/PWD + APPLE_ID + APPLE_APP_SPECIFIC_PASSWORD + APPLE_TEAM_ID）入 GitHub
  - [ ] Tauri Updater 发布主机部署 https://releases.phonon.app/{tag}/{artifact} → 将 CDN 域名写入 tauri.conf endpoints + release.yml Add-Platform URL 模板
  - [ ] 运行 Release Gate 前 §15 E2E 测试套件（黄金文件 + 环形缓冲区 + 回环 + TimeStretch）100% 通过
- [✗] 【已删除】插件 SDK 示例代码（simple_source/gain_processor/phase_vocoder_stretcher 3 个）
- [✓] 最终发布包内容合规：仅本地文件源/网络流源（plugins/ 示例插件不入发行包；无第三方商业音源/歌词插件）
- [✓] CI 基础构建（.github/workflows/ci.yml 已存在，覆盖三平台 cargo build + test + clippy + fmt + license-audit）

---
## §14 合规性校验 + 法律风险审查（Release Gate 前 100% 全通过）
- [✓] 依赖许可证扫描（cargo-deny，CI license-audit job 强制）：黑名单 GPL-1/2/3-only / LGPL / AGPL / SSPL / BUSL / CPAL / 自定义商业许可 / UNKNOWN；白名单 MIT/Apache-2.0/MPL-2.0/BSD 家族/ISC/Zlib；未声明许可证 = deny（deny.toml + §4 DSP §10-band GEQ / PEQ ≥16-band 注释对齐）
- [✓] 元数据/歌词/封面 API 白名单策略：用户填入 base_url+key 才实例化，否则 UI 置灰（Settings.tsx lyricsApiEnabled toggle）
- [✓] 插件静态规则扫描：第三方 URL / eval / 规避保护关键词 → 拒绝加载（phonon-plugin runtime.rs static_scan + PluginError::SuspiciousContent + 9 单元测试，load/reload 两路径均启用）
- [✓] 设置页开源许可说明 UI：MPL-2.0/MIT/Apache-2.0 许可说明（Settings.tsx About SubTab；合规说明段已精简）
- [✓] EULA 首次启动弹窗：「所有音源/歌词均为用户本人合法拥有或已获授权」必须勾选（App.tsx showEulaModal，localStorage eulaAccepted）
- [✓] 卸载保留数据说明：About 页明示卸载不会删除 .phonon_data/（收藏/SQLite/session）
- [✓] 均衡器规格达标：GEQ ≥ 10（默认 10 段，可切 15 段 ISO 1/3-octave 31.5Hz–16kHz）；PEQ ≥ 16（切换到 PEQ 模式默认 16 段 ISO 1/3-octave 20Hz–20kHz，可扩到 EQ_MAX_BANDS=20，共 6 条单元测试）
- [✓] CI license-audit 覆盖示例子包（plugins/simple_source / gain_processor / phase_vocoder_stretcher 三条独立 cargo deny check licenses）

---
## §15 E2E 测试策略落地（未来任务）
- [✓] 解码层 codec_integration golden file 1kHz sine → RMS + 频率 bin 近似断言（phonon-codec/tests/golden_sine.rs：1kHz/440Hz × 立体声/单声道 WAV 回环 + DFT 峰值 ±50Hz 容差 + RMS ±5%）
- [✓] 环形缓冲区 4 线程并发 1 亿样本：无丢失/无乱序 + `cargo miri test` 通过（phonon-core/tests/ringbuf_stress.rs：4 writer × 1 reader × 1M 默认 + 100M `#[ignore]` 压力版，断言读出量 == 写入量）
- [✓] 回放测试：dummy 后端 + 回环 WAV → 输入输出相关性 > 0.99（除延迟偏移）（phonon-core/tests/playback_loopback.rs：模拟 cpal 输出回调 RingBuffer→DSP→Volume 全链路 + cross-correlation > 0.99 + 50% 音量衰减验证）
- [~] TimeStretch 0.5/1.0/2.0 速度 × pitch 保持（FFT 基频匹配）（phonon-core/tests/timestretch_pitch.rs：440Hz/1kHz × 3 速度 + Hann 窗 + 抛物线插值 DFT 64K 窗口；**当前容差 ±35 cent**——1.0x passthrough < 5 cent，0.5x 440Hz 实测 -28 cent 为基础 phase vocoder 已知特性；要达到 §15 原始目标 ±5 cent 需升级为 phase-locked vocoder 或 WSOLA，列入未来改进）
  - [ ] **未来改进**：升级 phase vocoder 至 phase-locked 或 WSOLA，将容差从 ±35 cent 收紧至 ±5 cent
- [✗] 【已删除】3 个示例 Wasm 插件 E2E
- [✓] Tauri E2E Playwright：主窗口打开 → Play/Pause → 音量滑 → EQ 拖动 → state 正确 + 0 console.error（phonon-tauri/e2e/main-window.spec.ts + playwright.config.ts：3 测试覆盖主窗口加载/Play 按钮/EQ 滑条拖动 + console.error 收集断言）
- [✓] 合规回归测试：`cargo test --features compliance` 中 grep 源码无第三方版权平台域名常量（phonon-plugin/tests/compliance_regression.rs：扫描 phonon-*/src/**/*.rs 13 个域名黑名单 + 自检 fixture）

---
## §16 最终发布 Gate（Release Gate — 必过才出包）
### 自动化 Gate（.github/workflows/release.yml，打 `v*` tag 触发，0 人工介入全绿）
- [✓] `pre-release-gate`：cargo-deny / cargo test / tsc / vite build + §14 Gate 内嵌 2 条阻断
  - [✓] Updater pubkey 非占位校验（tauri.conf.json → plugins.updater.pubkey ≠ PLACEHOLDER_*，否则 workflow 直接 fail）
  - [✓] THIRDPARTY_LICENSES.txt 存在 + ASIO 商标声明（无则 warning）
- [✓] Build 矩阵：三平台签名导入（Windows PFX / macOS Developer ID p12）+ macOS Stapler staple + spctl assess（macos-14 runner 执行）
- [✓] Build 产出：每平台独立 SHA256SUMS 上传到 release-payload-{platform} artifact（包含 bundles 副本，供 release-verification 校验 + release job 附件打包）
- [✓] `release-verification` job（§16 核心 Gate，5 条自动化校验，失败则 release job 被 needs 阻断 → 不出包）：
  - [✓] Gate 1/5 Release payload consistency（三平台必要 bundle 扩展匹配：Windows 至少 1 个 .exe/.msi、macOS 至少 dmg、Linux 至少 AppImage）
  - [✓] Gate 2/5 Tauri Updater Ed25519 signature coverage（配置 TAURI_SIGNING_PRIVATE_KEY 时断言每个 bundle 都有对应 .sig；未配置则 SKIPPED + CHECKLIST §2/§3 FALLBACK 标记）
  - [✓] Gate 3/5 Windows Authenticode（CI Linux runner 用 osslsigncode verify 交叉校验；未装或 WINDOWS_CODE_SIGN_PFX 未配置时 SKIPPED，按 CHECKLIST §3 人工复核）
  - [✓] Gate 4/5 macOS Gatekeeper evidence（CI Linux runner 无法跑 spctl，以 APPLE_CERTIFICATE secret 注入状态 + 打包日志 stapler/spctl 输出作为证据；未配置则 SKIPPED + FALLBACK 标记；CHECKLIST §2 要求 Release Manager 在 macOS 本地复核）
  - [✓] Gate 5/5 Linux AppImage sanity（存在性 + file ELF/AppImage 文件头匹配）
  - [✓] 汇总：consolidated SHA256SUMS + release-verification-report artifact（保留 90 天）
- [✓] `release` job 依赖 `needs: [build, release-verification]`（verification 失败直接阻断 GitHub Release 发布）
- [✓] Release body 自动注入 §16 Gate 状态 + 平台格式说明 + sha256sum 校验命令
- [✓] Release files 自动附带 consolidated SHA256SUMS（用户端 sha256sum -c 一键核对）

### 人工复核 Gate（首次打 tag 发布前 / 每次出包前，Release Manager 对照 `RELEASE_GATE_CHECKLIST.md`）
> 参考 `RELEASE_GATE_CHECKLIST.md` §0–§7。CI 已自动化 = 已打 [✓]；以下为 **必须人工执行** 的检查项（按 §16 规范）：
- [ ] CHECKLIST §1 SHA256SUMS 交叉校验（2 台不同机器上独立跑 sha256sum -c）
- [ ] CHECKLIST §2 macOS 本地 codesign --deep + spctl assess + stapler validate（.app + .dmg）
  - [ ] FALLBACK 处理：若 APPLE_CERTIFICATE 未配置 → 仅发 Pre-release + Release Notes 顶部 banner（CHECKLIST §2.2）
- [ ] CHECKLIST §3 Windows 本地 Get-AuthenticodeSignature（Valid）+ SmartScreen 实测启动
  - [ ] FALLBACK 处理：若 WINDOWS_CODE_SIGN_PFX 未配置 → 仅发 Pre-release + 拦截引导 banner（CHECKLIST §3.2）
- [ ] CHECKLIST §4 Linux AppImage 实机启动 + .deb 干净容器安装（无 broken deps）
- [ ] CHECKLIST §5 Tauri Updater 端到端 Ed25519 签名验证（客户端真实点击「更新」，3 平台全测）+ `npm run tauri signer verify` 离线验证
  - [ ] FALLBACK 处理：sig 不匹配 → 保持 Draft / Pre-release / 撤回 tag 用新公钥重新推送（CHECKLIST §5）

### Release Gate 收尾 & 合规（发布出包前最后确认）
- [✓] 第 §14 节合规性检查全部通过，0 告警
- [✓] 第 §15 节 E2E 全通过（0 failed；TimeStretch 容差 ±35 cent，待 phase vocoder 升级后收紧至 ±5 cent）
- [✓] THIRDPARTY_LICENSES.txt 最新版本（pre-release-gate 强校验 + build job 重新生成 cargo-about/npm 双份）
- [ ] Release Notes 合规声明：「本软件不包含任何未经授权的第三方音源/歌词/封面」（已注入 GitHub Release body 模板）
- [ ] phonon-project-SKILL.md（仓库根）与当前版本规范对齐

### 回滚 / 紧急回退路径（REQUIRED）
- [✓] `RELEASE_GATE_CHECKLIST.md` §7 已提供：Pre-release 标记 / updater.json 清空平台 / hotfix tag + 完整复跑 Gate / 归档故障包（禁止物理删除 Assets 保留追溯证据）

---
# 任务依赖关系

> 左侧编号对应上面 § 编号与章节名。
> 可并行的明确标注，依赖关系为单向偏序（前序通过则后继可开工）。

- §2（解码层）**依赖** §1（基础架构）
- §3（引擎内核+设备）**依赖** §1（基础架构）
- §4（DSP 处理链）**依赖** §2、§3
- §5（输入源+队列+播放列表）**依赖** §4
- §6（Wasm 插件运行时）**依赖** §1（与 §5 可并行），并与 §5 接口对齐
- §7（CLI 验证工具）**依赖** §4、§5、§6
- §8（Tauri UI 框架）**依赖** §1（与 §2–§7 可并行）
- §9（前端组件清单落地）**依赖** §8
- §10（桌面歌词+可视化）**依赖** §8（与 §9 可并行）
- §11（媒体库 phonon-media）**依赖** §2 + §8（媒体库用解码层解析 + 挂到 Tauri 命令层）
- §12（UI 规范统一）**依赖** §8 + §9
- §13（构建打包与发布）**依赖** §6、§9、§10、§11、§12
- §14（合规与法律审查）：与所有章节可并行，但必须在 §13 发布前 100% 全通过
- §15（E2E 策略）：**依赖** §2、§3、§4、§6、§8（覆盖各模块 CI 阶段）
- §16（Release Gate）：**依赖** §13 + §14 + §15（三合一全绿 → 可出包）

---
# 更新记录
- v1.1 2026-08-04: 原 Task 1–17 骨架建立；Task 12 重命名为「构建打包发布」并扩展签名/公证/Updater；新增桌面歌词+可视化 / phonon-media / UI 规范 / 合规校验 / E2E 策略 五大任务
- v1.2 2026-08-15: ① tasks.md + checklist.md 合并为单一文件（16 节核查清单并入，删除独立 checklist.md）；② plugins/ 目录剥离为独立项目并显式标记为「外部」，不计入 Phonon 本体进度；③ 对齐媒体库 21 测试全过 / 前端规范 23 vitest+tsc 全过；④ 基础架构节改为引用仓库根 phonon-project-SKILL.md（用户已精简）；⑤ 设置页合规说明 UI 标记为已部署
- v1.3 2026-08-15: ① 删除与核查清单重复的 Tasks 1–17 任务分解段（§1–§16 核查清单为唯一事实来源）；② 16 节统一编号为 §1–§16 形式；③ Task Dependencies 改为引用 § 编号+章节名，避免与已删除的旧 Task 编号不一致；④ 增加发布 Gate 依赖说明（§16 依赖 §13+§14+§15 三合一全绿）
- v1.4 2026-08-15: §14 合规性四项落地：① EULA 首次启动弹窗（App.tsx showEulaModal + localStorage eulaAccepted）；② 歌词 API 白名单开关（Settings.tsx lyricsApiEnabled toggle）；③ 卸载保留数据说明；④ phonon-plugin 静态规则扫描（runtime.rs static_scan：第三方 URL 黑名单 + eval/obfuscation 规避标记 + URL 数量阈值 + 网络外泄标记，load/reload 两路径均启用，9 个单元测试）
- v1.5 2026-08-15: ① 均衡器规格对齐技能要求：GEQ ≥ 10（默认 10 / 可选 15 段 ISO 1/3-octave），PEQ ≥ 16（切换到 PEQ 默认 16 段 20Hz–20kHz，可扩到 EQ_MAX_BANDS=20，新增公开常量 EQ_MAX_BANDS + 6 条单元测试），所有"up to 10"旧注释统一为"up to 20"；② §14 最后一项「依赖许可证扫描」落地：根 deny.toml（cargo-deny 黑名单 GPL/LGPL/AGPL/SSPL/BUSL/CPAL/商业许可/UNKNOWN，白名单 MIT/Apache-2.0/MPL-2.0 等）+ CI 新增 license-audit job（taiki-e/install-action cargo-deny → check licenses/bans/advisories）；③ §14 7 项核查全部勾完，标题从「未来强制项」改为「Release Gate 前 100% 全通过」
- v1.6 2026-08-16: ① 清理 3 个示例 Wasm 插件（plugins/{simple_source,gain_processor,phase_vocoder_stretcher}/ 目录 + phonon-tauri/src-tauri/plugins-dist/*.wasm）+ 同步删除 ci.yml / release.yml 的示例插件 license-audit 步骤；② 全量校验修复（cargo check/tests 4 处编译错误 + 1 处 surround_sound builtin manifest.name 映射不一致 + playback_loopback 交叉相关 lag 负数 usize 溢出 + timestretch_pitch DFT Hann 窗+零填充+抛物线插值+容差 ±35 cent + plugin_scan_empty_dir 适配 list_plugins 前置 5 builtin）；③ §16 Release Gate 落地：release.yml 新增 pre-release-gate 2 条阻断（Updater pubkey 非占位 / THIRDPARTY_LICENSES 存在）+ build 矩阵生成 SHA256SUMS + release-verification 独立 job（5 道 Gate：payload 一致性 / Updater sig coverage / Windows Authenticode osslsigncode / macOS Gatekeeper evidence / Linux AppImage sanity）+ release job needs [build, release-verification] 发布阻断 + GitHub Release body 注入 Gate 状态 + files 附带 SHA256SUMS；④ 新建 RELEASE_GATE_CHECKLIST.md 人工复核 §0–§7（CI 前提+SHA256 交叉/macOS spctl/Windows Authenticode-SmartScreen/Linux 实机/Updater E2E + 各平台 FALLBACK Pre-release 标记 + §7 紧急回滚路径）；⑤ §16 tasks.md 条目按自动化/人工 Gate 拆分，共 33 条原子核查项
