# Phonon ABC 核心功能设计文档（顶级路线）

- 日期：2026-08-19
- 作者：AI Assistant
- 状态：Draft（待用户书面审阅）
- 关联实现计划：docs/superpowers/plans/（审阅后生成）
- 上游参考：
  - d:\Phonon\tasks.md §3 设备热插拔、§14 播放列表队列、§15 算法质量、§16 Release Gate
  - d:\Phonon\RELEASE_GATE_CHECKLIST.md
  - d:\Phonon\spec.md §6 播放/§10 媒体库/§11 DSP

---

## §1 目标与范围

### 1.1 目标
在不引入外置插件（§6 WasmExternal/WasmExample 业务插件）开发的前提下，完成 Phonon 内核 DSP + 播放队列体验 + 完整媒体库浏览 UI 的三大核心升级，交付一份 **RC 阶段可执行安装包**（未签名，用户本地可安装测试），达到：

- **变速不变调质量**：常用档（0.8x~1.25x）音高容差 ±5 cent，三档策略可切 + 紧急降级回退
- **体验闭环**：媒体库 ↔ 播放队列 ↔ 设备热插拔 三者数据完全关联；正在播放跨视图追踪高亮；播放历史自动记录（防脏）
- **完整的媒体库浏览（全功能级）**：6 个子视图（全部/收藏/最近/专辑/艺术家/流派）+ FTS5 联想搜索 + 专辑详情大卡片页 + 元数据编辑/换封面 + 批量 ReplayGain + 软删清理
- **入口零心智负担**：歌单同步文件夹 ↔ 媒体库 roots 自动双向同步，用户不用理解"加文件夹到底去哪边"

### 1.2 范围（In Scope / Out of Scope）

| 层级 | 包含（In Scope） | 不包含（Out of Scope） |
|---|---|---|
| 内核 DSP（C） | TimeStretch 三档策略 factory（Auto/WSOLA/PV）；容差三层（CI/本地/质量基准）；DSP 高级面板；失败降级 | 新算法实现（SOLA / PSOLA / Elastic 音频拉伸专利算法） |
| 关联 & 策略（B） | 异步 path→track_id 50 条/批注入；30s/50% 有效 recordPlay；跨视图智能高亮；热插拔三档拔出 + 两档插入策略；AppSettings 字段向后兼容 | Device 重连后端配对记忆学习（3 次统计）；跨设备音量状态迁移（RC2 再做） |
| Library Tab（A） | 6 主 Tab 插入；6 子 SubTab 视图；FTS5 联想 300ms debounce；8 列懒加载大表格 + 9 项右键菜单；专辑卡片网格 + Apple Music 风格详情页；首字母分组艺术家/流派；元数据 12 字段编辑弹窗；Canvas 方形裁剪换封面；Library Tab 右上角「…」菜单入口 + 空状态引导页 + 入口一致性自动同步；Settings→音频媒体库精简状态显示 | UI 拖拽（跨组件 dnd）；歌单混合播放模式（智能随机/基于特征推荐）；基于机器学习的去重/相同专辑不同版本合并 |

### 1.3 非功能目标（Top-tier 要求）
- **性能**：Library Tab 首屏 2,000 首 表格滚动 60fps（虚拟列表/IntersectionObserver 懒加载封面）；FTS5 搜索 ≤ 150ms 返回 200 条；联想 suggest ≤ 40ms
- **韧性**：封面失败三次重试 + 占位图；搜索 5s 超时 banner；元数据写回文件失败不丢库内数据；SQLite 损坏自动备份重建 toast；WSOLA crate 初始化失败单会话降级
- **兼容**：AppSettings 所有新字段均有 `From<()>` 或 `#[serde(default)]`；旧配置 JSON 打开不崩
- **无障碍**：所有新开关原生 `<input type="checkbox" switch>`；Modal 可 Esc 关闭；表格行键盘 ↑↓ 导航 + Enter 播放

---

## §2 工期 & 里程碑打包计划

**总工期：8 天有效工作日 + 1 天缓冲 = 最晚 9 天**
（不限时间，按顶级 phonon 路线质量优先，实际可能 10-11 天，不赶）

| 里程碑 | 内容 | 预计完成 | 本地可测安装包产出 |
|---|---|---|---|
| M1 | C + B 完成（变速三档切完 + 容差三层 + CI 绿 + 队列库关联 + 自动历史 + 正在播放高亮 + 热插拔策略 5 开关 + Settings UI） | Day 4 晚（8/22） | ✅ 第 1 份测试包（Windows x64 MSIX + NSIS） |
| M2 | A 完成前半段（Library Tab 框架 + 全部/收藏/最近 三视图 + 搜索联想 + 右键菜单基础 5 项 + 空状态引导 + 入口一致性） | Day 7 中（8/25） | 不打（代码里程碑，手动 dev 模式测） |
| M3 | A 全功能交付（专辑网格 + Apple Music 详情页 + 艺术家/流派分组 + 元数据编辑弹窗 + 换封面 Canvas 裁剪 + Settings 精简状态区 + Library Tab「…」完整菜单 + 错误边界 + 手动核查清单 §15） | Day 9 晚（8/27） | ✅ 第 2 份 = RC 测试包（最终目标：给用户安装测试） |
| 缓冲 | 三平台构建错 / CI 网络抖 / 质量细节打磨回滚 | Day 10~11（8/28~29） | ✅ RC 补丁包（如有 Bug 修复） |

---

## §3 子项目 C：TimeStretch Auto Hybrid（三档变速策略）

### 3.1 三档模式路由
AppSettings 新字段 `dsp.time_stretch_mode: "auto" | "wsola" | "phase_vocoder"`，默认 `"auto"`。

| 模式 | speed < 0.60x | 0.60x ≤ speed < 0.80x | 0.80x ≤ speed ≤ 1.25x | 1.25x < speed ≤ 1.75x | speed > 1.75x |
|---|---|---|---|---|---|
| `"auto"` | Phase Vocoder | WSOLA（如失败降级 PV） | **WSOLA（质量优先）** | WSOLA（如失败降级 PV） | Phase Vocoder |
| `"wsola"` | WSOLA | WSOLA | WSOLA | WSOLA | WSOLA |
| `"phase_vocoder"` | Phase Vocoder | Phase Vocoder | Phase Vocoder | Phase Vocoder | Phase Vocoder |

### 3.2 后端改造
- **文件**：phonon-core/src/engine.rs
  - 现有 `default_time_stretch_factory() -> Box<dyn TimeStretcher>` [L126] 改为 `time_stretch_factory(mode: &TimeStretchMode, speed: f32) -> Box<dyn TimeStretcher>`
  - 入参：从 Engine 创建处透传 AppSettings 读取出的 mode + 当前 playback.speed
  - 每档 factory 先尝试创建，如果 WSOLA 返回 `Err(_)`（平台缺失符号 / 链接失败），单会话内记一次性 warn 并锁死到 `"phase_vocoder"`（同一会话不再反复试）
- **Cargo.toml**：检查 `phonon-core/Cargo.toml` 的 `[features]` 表 —— 如 `timestretch` crate 当前是 optional feature-gate → 移到 `default = [...]` 列表中，确保三平台默认编译

### 3.3 测试容差三层
- **CI 默认**（现有逻辑保留）：`tolerance_for_speed()` —— 0.5x/2.0x 极端档 100 cent，其余 60 cent
- **本地精确验证**：环境变量 `EXACT_TS=1 cargo test -p phonon-core --test timestretch_pitch` → 0.6–1.75x 容差 **收紧至 ±5 cent**，极端档 ±15 cent
- **新增质量基准单测（不进 CI，本地跑）**：phonon-core/tests/timestretch_quality.rs
  - `phase_continuity`：相邻 1024-sample 重叠帧相位差 < 0.3π（0.8x~1.25x，5 个测试信号）
  - `transient_no_smear`：瞬态（频谱包络变化 > 6dB 且幅度 > -3dBFS）前后输出幅度不削波（THD+N < -40dB）

### 3.4 Settings → DSP 高级折叠面板
- 复用现有 EQ Preset / ReplayGain 区块的卡片折叠风格（`Settings.tsx` `AudioConfigGroup` pattern）
- 标题：「⚙️ 高级变速算法」
- 控件：
  - 下拉：模式（Auto（推荐）/ 质量优先 WSOLA / 兼容性优先 Phase Vocoder）
  - 说明小字：「Auto 在 0.8x~1.25x 常用档使用高精度 WSOLA（±5 cent），极端档位自动切到更稳定的 Phase Vocoder，避免 artifacts」
  - 只读信息（根据当前选择档位动态显示）：当前模式 = "Auto"，当前 1.00x 档位实际使用："WSOLA（音高校准）"

### 3.5 降级 & 错误处理
- Engine 创建时 try WSOLA 3 次（间隔 10ms）仍失败 → 单会话 fallback lock → warn! 日志 → 下次启动再试
- 若某模式在某特定采样率（384 kHz DSD 转 PCM）下创建失败 → 当次 play call 自动降级另一模式，不中断播放
- Windows/macOS/Linux 三平台 CI 各加一条 case：用 `EXACT_TS=1` 跑 0.8/1.0/1.25x 三档（在 CI 里把 EXACT_TS 作为可选 `continue-on-error: true` 的 job 跑，不阻塞 PR merge，失败就记录 Issue）

---

## §4 子项目 B：PlayQueue ↔ 库关联 + 自动历史 + 热插拔策略

### 4.1 异步 path→track_id 批量匹配注入（核心数据流）
目标：无论用户从**哪条路径**把文件加入队列（Library Tab 点播放 / PlaylistToolbar 导入文件夹 / 同步文件夹 / 单文件拖拽 / open_url 命令），只要路径在 phonon-media tracks 表里存在，QueueItem 就自动带上完整元数据。

**流程**：
```ts
// 前端 hooks/usePlaylist.ts（或现有 queue.add_to_queue 封装层）
async function addToQueue(uris: string[]) {
  // 1. 先加入队列（立即响应，封面占位符 🎵 + 空星）
  await queue.addToQueue(uris);
  // 2. 后台异步批量匹配（不阻塞播放/UI），50 条/批
  const batches = chunk(uris, 50);
  for (const batch of batches) {
    try {
      const tracks = await library.getTracksByPaths(batch); // 新增 API
      if (tracks.length) {
        playlist.patchQueueItems(
          (item) => tracks.find((t) => t.path === item.path),
          (item, track) => ({
            ...item,
            libraryTrackId: track.id,
            coverHash: track.thumbnailHash ?? null,
            favorite: track.favorite,
            rating: track.rating ?? 0,
            albumId: track.albumId ?? null,
            artistIds: track.artistIds ?? [],
          }),
        );
        // UI 自动刷新：css transition 0.2s opacity fade-in
      }
    } catch (e) {
      console.warn('Library track batch match failed, continuing', e);
      // 单批失败不影响后续批次和已匹配的
    }
    await new Promise((r) => setTimeout(r, 16)); // 让给 UI 一帧
  }
}
```

- **新增 Tauri 命令**：`library_get_tracks_by_paths(paths: Vec<String>) -> Vec<LibraryTrack>`
  - phonon-media `library.get_tracks_by_paths(paths)`（现有 query 实现 WHERE path IN (...) + 去重，保持输入顺序）
  - 注册到 commands.rs / events.rs

- **前端 API**：api/library.ts 新增 `getTracksByPaths(paths: string[]): Promise<LibraryTrack[]>`
- **QueueItem TS 定义**：phonon-tauri/src/types.ts（或 queue.ts 内）新增 6 个可选字段，全 `?` 不影响旧代码

### 4.2 自动记录播放历史（防脏 → 30s/50% 有效阈值）
**Hook 位置**：App.tsx 现有 `useEffect` 监听 `active_meta` 变化的那个 effect [约 L1500 active_meta set 处]：
```
effect 触发条件：
  (a) active_meta.libraryTrackId 非空（说明是库内曲目）
  AND
  (b) 从 playhead 开始进入"有效播放区间"
有效播放区间 = (player.playhead >= 30_000ms) OR (player.playhead >= total_duration * 0.5)
避免误计入：点一下立刻切歌（典型首 5 秒误触）
```
- 达到有效阈值时，fire-and-forget `library.recordPlay(trackId, positionAtThresholdMs)`（`await` 掉 catch 即可，不阻塞播放器）
- `library.recordPlay` 后端逻辑：
  - 当天同 trackId 已存在 → `play_count += 1, total_play_time_ms += positionAtThresholdMs`
  - 不存在 → 新建一行 `play_count = 1`
  - 返回值不关心（前端用 void）

### 4.3 跨视图正在播放智能追踪高亮
**全局监听**：LibraryPage 订阅 Tauri event `active-meta-changed`（或上层 context 传 activeMeta + playheadState）
- 当前激活的子视图（全部/收藏/专辑详情/搜索结果）的渲染表格：
  - 匹配 `activeMeta.path === row.path || activeMeta.libraryTrackId === row.libraryTrackId`
  - 命中行做三件事：
    1. 左侧 48px 封面替换为 ▶ 三竖条跳动动画（CSS keyframes）
    2. 背景 `bg-blue-500/12 dark:bg-blue-400/10`（和 Playlist 正在播放行一致）
    3. **条件触发 scrollIntoView**：仅当 `Date.now() - user.lastManualScrollAt > 3000`（用户 3 秒内没碰过滚动条），用 `{ behavior: 'smooth', block: 'nearest' }`

### 4.4 热插拔策略（三档拔出 + 两档插入）
**AppSettings 新增**（全部 `#[serde(default)]`）：
```ts
hotplug: {
  // 现有字段，保留
  show_notification: boolean, // 默认 true
  // 新字段
  on_device_removed: 'pause' | 'switch_default' | 'switch_last_used', // 默认 'pause'
  on_new_device_inserted: 'ignore' | 'auto_switch',                    // 默认 'ignore'
  last_used_device_id: string | null,                                   // 唯一写入入口 = commands::set_device(new_id)，在实际切换**之前**写入旧设备 id（不是新的）
}
```

#### 4.4.0 last_used_device_id 写入责任链（唯一入口 + 切换前写入，防丢失）
**写入规则**：只有 `commands::set_device(new_id)` 命令的成功流程内部会写此字段，UI 手动切、热插拔 auto_switch、策略 switch_default 三条路径都走同一个命令，不会重复写也不会遗漏。

**写入时机（关键，防切换失败丢记录）**：在**任何实际硬件切换动作之前**先完成写入：
```rust
// commands.rs: set_device(new_id) 伪代码
async fn set_device(app: AppHandle, new_id: String) -> Result<(), CommandError> {
    let mut state = app.state::<AppState>().lock().await;
    // ① 先取旧设备 id + 写入（切换**前**就存好，防止第 ③ 步失败后丢失"上一台成功使用的设备"记录）
    let old_device_id = state.audio.current_device.as_ref().map(|d| d.id.clone());
    if let Some(ref old_id) = old_device_id {
        if old_id != &new_id {
            let mut settings = app.settings().write().await;
            settings.hotplug.last_used_device_id = Some(old_id.clone());
            settings.persist().await; // 立刻落盘
        }
    }
    // ② 标记 state.current_device 为 switching（可选中间态，UI 显示"切换中…"）
    // ③ 真正执行硬件切换：cpal build_stream + engine reinit
    let switch_result = do_actually_switch_hardware(&new_id).await;
    // ④ 只有成功才更新 state.current_device（失败保持旧的，但 last_used_device_id 第①步已存，不丢）
    if switch_result.is_ok() {
        state.audio.current_device = Some(DeviceInfo::from(new_id));
    }
    switch_result.map_err(|e| e.into())
}
```
**关键点**：就算第 ③ 步硬件切换失败（比如新设备是蓝牙刚连上就掉了），`last_used_device_id` 在第 ① 步就已存到磁盘的 AppSettings.json，下一次 `switch_last_used` 策略不会拿空值。

**后端改造文件**：phonon-tauri/src-tauri/src/state.rs [L419 hotplug_monitor 回调扩展] + commands.rs `set_device` 命令实现。

#### 4.4.1 Device Removed（且是 current_device）
```rust
match settings.hotplug.on_device_removed {
    Pause => { /* 现状：pause + 记忆 pos + toast */ }
    SwitchDefault => {
        if let Ok(Some(def)) = default_output_device() {
            if def.id() != removed_device_id { // 防止 default 就是被拔的这台（极罕见）
                set_device(def.id()).await;
                engine.resume().await;
                toast("已切换到默认设备并继续播放");
            } else { fallthrough to Pause }
        }
    }
    SwitchLastUsed => {
        if let Some(last_id) = settings.hotplug.last_used_device_id {
            if devices_list.iter().any(|d| d.id == last_id) {
                set_device(last_id).await; engine.resume();
                toast("已切换到上次使用的设备并继续播放");
            } else { fallthrough to SwitchDefault }
        } else { fallthrough to SwitchDefault }
    }
}
```

#### 4.4.2 New Device Inserted
```rust
match settings.hotplug.on_new_device_inserted {
    Ignore => refresh_device_selector_list(),
    AutoSwitch => {
        // 直接调 set_device(new_id)，last_used_device_id 由命令内部在
        // 实际切换之前写入（遵循 4.4.0 唯一写入责任链，不再手动写）
        set_device(new_id).await;
        toast(format!("已自动切换到新设备：{}", new_name));
    }
}
```

#### 4.4.3 Settings UI（音频 SubTab 新增「🔌 设备热插拔策略」卡片）
- 原生 switch 2 个 + 下拉 1 个：
  - 设备拔出时：[下拉，3 项]（默认：保持暂停）
  - 插入新设备时自动切换：[原生 switch off]（右侧小字说明："插入 USB DAC/蓝牙耳机时自动切过去"）
  - 显示热插拔通知：[保留现有 switch]

### 4.5 热插拔 AppSettings 迁移（向后兼容关键！）
- 旧配置 JSON 中 `hotplug` 可能只有 `{ show_notification: true }`，**不应该**出现 serde 的 missing field error
- 做法：`AppSettings.hotplug` 的子结构 `HotplugConfig` 每个字段加 `#[serde(default = "xxx")]` 函数：
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct HotplugConfig {
      #[serde(default = "default_true")]
      pub show_notification: bool,
      #[serde(default)] // 'pause' = HotplugOnRemove::default()
      pub on_device_removed: HotplugOnRemove,
      #[serde(default)] // 'ignore' = HotplugOnNew::default()
      pub on_new_device_inserted: HotplugOnNew,
      #[serde(default)]
      pub last_used_device_id: Option<String>,
  }
  ```
- 热启动（旧配置升级）无需 migrate script，serde default 就搞定。

---

## §5 子项目 A：Library Tab 全功能（方案 3 布局 + 入口去重）

### 5.1 Tab 架构（与 Settings SubTab 复用）
**App.tsx 主 Tab 栏**：在「扩展」之前插入「📚 媒体库」→ 顺序：播放器 / 设备 / DSP / **📚 媒体库** / 扩展 / 设置
- `active_tab` 新增 `'library'` variant
- 渲染：`<LibraryPage />` 组件

**LibraryPage 子 SubTab**（100% 复用 [Settings.tsx SubTab 组件模式](file:///d:/Phonon/phonon-tauri/src/components/Settings.tsx#L12-L15)，同一套 `<div role="tablist">` + `aria-selected` + 蓝色下划线 CSS）：
```
[🎵 全部]  [⭐ 收藏]  [🕒 最近]  [📀 专辑]  [🎤 艺术家]  [🎼 流派]
```

### 5.2 顶部工具条（全局共享，随子 SubTab 轻微调整）
```
[搜索框：🔍 搜索媒体库...（输入 300ms debounce 联想）]  [☆ 仅收藏 toggle]  [↓ 评分 ≥X]  [共 N 首 / N 张]
                                                                                       [⋮ 三竖点「…」菜单]
```

- **联想搜索**（300ms debounce）：新命令 `library_search_suggest(q: String) -> LibrarySuggest`
  ```ts
  interface LibrarySuggest {
    tracks: LibraryTrack[];    // 最多 5 条
    albums: LibraryAlbum[];    // 最多 3 条
    artists: LibraryArtist[];  // 最多 3 条
  }
  ```
  联想下拉 UI：点击条目 → 如果是 track → 切到「🎵 全部」并立即替换队列播放；如果是 album → 切到「📀 专辑」并打开该专辑详情页；如果是 artist → 切到「🎤 艺术家」打开详情页。
- **用户按回车** → 走 `library.search(q, 200)` → 结果直接显示在「🎵 全部」子视图里（自动切 SubTab）
- **「…」菜单**（Library Tab 专属右上角管理入口）：
  - ＋ 添加音乐目录…
  - ⏳ 立即扫描媒体库
  - 🔊 批量扫描 ReplayGain（N 首未扫描）
  - 🧹 清理已不存在的条目（N 首）
  - ↻ 重建全文搜索索引
  - ⚠️ 清空媒体库…（红色 + 二次确认 Modal）

### 5.3 🎵 全部 / ⭐ 收藏 / 🕒 最近 —— 9 列大表格

| 列 # | 列名 | 内容 | 交互 |
|---|---|---|---|
| 1 | 封面 | 48×48，懒加载 | 加载失败 → 占位图 🎵 |
| 2 | # | 序号（当前正在播放时替换成 ▶ 三竖条跳动动画） | 点击空白处选中整行（多选：shift/ctrl） |
| 3 | 标题 | 最多 2 行省略（overflow-ellipsis） | **双击行 = 替换队列并播放** |
| 4 | 艺术家 | 1 行省略 | 点击蓝色 → 跳转到艺术家详情页 |
| 5 | 专辑 | 1 行省略 | 点击蓝色 → 跳转到专辑详情页 |
| 6 | 时长 | `mm:ss`，靠右对齐 | 只读 |
| 7 | 格式 | 彩色徽标（FLAC=绿 / ALAC=蓝 / DSD=金 / MP3=橙 / 其它=灰）+ 采样率 Hz 小字 | 悬浮 tooltip：完整 Codec / 位深 / 采样率 |
| 8 | ★ 收藏 | 行内收藏星，点击切换 | 切换后写库 + 本地 optimistic update |
| 9 | ⭐ 评分 | 行内 5 星评分（hover to preview，click to set） | 点击写库 |

- **虚拟化策略（明确数字阈值）**：
  - **≥ 10,000 首启用自实现简易虚拟列表**（零新依赖，不引 `react-window` / `react-virtuoso`）：固定行高 72px + 3 可视区上下缓冲（上 buffer ≈ 可视行数、下 buffer ≈ 可视行数），动态计算 `paddingTop / paddingBottom` + 只渲染可视行 DOM，滚动 60fps。
  - **< 10,000 首** 普通 DOM 渲染 + 封面 `IntersectionObserver` 懒加载：只有行进入可视区时才调用 `library.getThumbnail(hash, 'small')`，URL 在 api/library.ts 层做 `Map<hash, Promise<string>>` cache，同一张封面不重复 fetch。
- **右键菜单 9 项**（支持单条 / 多条批量，多条时禁用"编辑元数据/换封面"这种只能单操作的项）：
  1. ▶ 立即播放（替换队列）
  2. ⏭ 播放下一首
  3. ➕ 加入当前歌单末尾
  4. 📂 加入歌单…（Modal：已有歌单多选列表 + 新建歌单输入框）
  5. ✏️ 编辑元数据…（仅单条）
  6. 🖼 更换封面…（仅单条，打开文件选择 + 裁剪 Modal）
  7. ⭐⭐⭐⭐⭐ 设置评分（1~5 子菜单）
  8. 📁 在文件夹中显示（Tauri `shell.open(path.dirname(file))`）
  9. 🗑 从媒体库移除（软删；多条可选；二次确认"将从库中移除 N 首条目，但不会删除磁盘上的物理文件 ✓"）

- **`⭐ 收藏` / `🕒 最近` 子视图**：复用全部曲目的 **9 列**表格组件，只是 `library.getTracks(filter)` 的 filter 参数分别传 `{ favorite_only: true }` 和 `{ play_history: { limit: 200, order_by: 'last_played_at DESC' } }`。「最近播放」子视图在第 2 列「#」之前**额外多一列**「上次播放时间」（相对时间，`3 分钟前 / 昨天 21:40 / 8 月 15 日`）。

### 5.4 📀 专辑 — 卡片网格 + Apple Music 风格详情页

#### 5.4.1 网格视图
- CSS Grid：`grid-template-columns: repeat(auto-fill, 160px); gap: 20px`（自适应宽度）
- 每张卡片（160px 宽）：
  - 140×140 专辑封面 → 悬浮 0.3s fade in ▶ 浮层按钮（右下方 36px 圆形播放按钮）→ 点击 = 替换队列播放整张专辑
  - 专辑名（最多 2 行省略，14px 半粗）
  - 艺术家（1 行省略，12px 半透明；点击跳转艺术家详情页）
  - 12 首 · 45 分钟（11px 灰色）
- 空状态（库里 0 张专辑）：大卡片引导 + 按钮「立即扫描」（如果 roots 为空就跳到 3.9）

#### 5.4.2 专辑详情页（顶级视觉）
点卡片进入后，顶部 300px 横幅 + 曲目表格：
```
┌───────────────────────────────────────────────────────────────┐
│ [← 返回专辑列表]                                               │
│                                                               │
│ ┌──────────────┐                                              │
│ │              │  **《Midnights》**  (28px 白字加粗)          │
│ │  300×300     │  Taylor Swift  (蓝色, 点击→艺术家详情)       │
│ │  封面         │  🗓 2022年10月21日  ·  🎼 Pop  ·  13 首       │
│ │              │  💿 44 分 27 秒  ·  🎧 44.1 kHz / 16-bit FLAC│
│ │              │  （可选简介，来自数据库 meta，3 行省略）       │
│ │              │                                              │
│ └──────────────┘  [▶ 播放专辑]  [+ 加入队列]  [☆ 收藏整张专辑]│
├───────────────────────────────────────────────────────────────┤
│  #  曲目                 艺术家           时长   收藏   评分    │
│  1  Lavender Haze       Taylor Swift     3:22   ☆      ⭐⭐⭐ │
│  2  Maroon              Taylor Swift     3:38   ⭐     ⭐⭐⭐⭐│
│  ...                                                           │
└───────────────────────────────────────────────────────────────┘
```
- 状态栈管理：`libraryPageNavStack: Array<{type: 'album'|'artist'|'genre', id: string}>`，最上层决定渲染详情页 / 网格。`← 返回` = `pop()`，浏览器级前进后退**不做**（太重，不引 router）。
- `[☆ 收藏整张专辑]` = 批量把专辑内所有 track 的 favorite 设为 true（乐观 UI：立即设为 ★，失败 toast 回滚）

### 5.5 🎤 艺术家 / 🎼 流派 — 首字母分组列表 + 详情页

#### 5.5.0 首字母分组实现方案（后端做 + pinyin crate + sort_key 存库）
**为什么不前端做**：保证 SQL ORDER BY 排序结果和前端分组字母 **100% 一致**（避免出现"前端按 Z 分组，后端却按 W 排序"的诡异错位——前端各平台拼音库实现不一致）。

**Rust 后端实现**：
1. **依赖**：`phonon-media/Cargo.toml` 引入 `pinyin` crate（MIT 许可，license 白名单），仅用「取字符串第一个有效字符的大写拼音首字母」能力
2. **存储**：phonon-media schema.rs 给 `artists` / `albums` 表各加一列 `sort_key TEXT NOT NULL DEFAULT '#'`，并加 `INDEX idx_artists_on_sort_key`（索引分组查询提速）
   - sort_key 值域：`'A'~'Z'` 或 `'#'`（数字 / 符号 / emoji / 无法提取拼音首字母的统一归到 `#`）
3. **写入时机**：`library_scan` 增量扫描写入 artist/album 行时**当场计算并存好**，不做 lazy compute
4. **查询返回格式**：后端命令 `library_list_artists()` / `library_list_albums()` 返回结构改为预分组：
   ```ts
   interface ArtistGenreGroupedResult {
     // key = sort_key 首字母，按 '#','A','B'... 顺序排序；空组 key 不出现
     groups: Array<{ key: string; items: Array<LibraryArtist | LibraryAlbum> }>;
     total: number;
   }
   ```
   前端**不再做任何拼音计算**，直接渲染字母分组。
5. **SQLite 版本号迁移 & 旧库数据回填**（见下方 5.5.1）

- **主列表渲染**：按后端返回的 `groups[]` 顺序（`# → A → B → … → Z`）渲染字母分段标题 + 段内卡片列表（120px 圆形头像占位符 🎤 + 姓名 + N 首歌曲）
- **详情页**：复用专辑详情页的"顶部大 Banner + 曲目表格"结构，Banner 变成艺术家名/流派名 + 统计（N 张专辑、M 首歌、总时长）+ 下方多一个「该艺术家所有专辑（卡片横向滚动）」区块。

#### 5.5.1 旧库数据 sort_key 回填迁移（PRAGMA user_version）
**问题**：如果用户此前已经跑过 `library.scan()`（开发环境），`artists` / `albums` 表里有若干行但 `sort_key IS NULL` —— 不回填的话这些行全会归到 `#` 组。

**回填方案（非阻塞、不打断查询）**：
1. `phonon-media Library::open()` 初始化后立刻检查 `PRAGMA user_version`：
   - `user_version = 0`（旧库，无 sort_key 列 / 列空）→ 走迁移
   - `user_version = 2`（含 sort_key 及回填完成）→ 跳过
2. **迁移步骤**：
   - Step 1（schema 迁移）：`ALTER TABLE` 加 `sort_key TEXT NOT NULL DEFAULT '#'` 列（SQLite 1.0.34+ 支持，秒级）+ 建索引
   - Step 2（后台异步回填，`thread::spawn`，不阻塞 `Library::open()` 返回）：
     - 500 行/批，`SELECT id, name FROM artists WHERE sort_key = '#'`（默认值占位，待重算）
     - 用 pinyin crate 逐行算 sort_key → `UPDATE` 写回
     - 每批完成发 Tauri event `library-migration-progress { stage: 'sort_key_backfill', done: N, total: T, eta_ms }`
   - Step 3：全部批完成 → `PRAGMA user_version = 2`（标记已迁移，下次启动不再走）
3. **前端 UI 配合**：LibraryPage 订阅 `library-migration-progress` 事件。如有回填任务在跑：
   - Library Tab 顶栏（工具条下方）显示绿色进度条 + 文字「正在为艺术家/专辑计算首字母分组（N/T，剩余 ETA 12 秒）…」
   - 回填期间 sort_key = '#' 的条目先临时显示在 `#` 组，每批 UPDATE 完后端再 push 一条 partial-refresh event 给前端重渲染对应分组（渐进式迁移，不会突然全表刷一下）
   - 10,000 首量级 pinyin 查询 + SQLite UPDATE 应在秒级完成；对 100,000 首以上预计 < 30 秒。

### 5.6 ✏️ 编辑元数据弹窗（12 字段表单）
- Modal 样式：和 EQ Preset 保存弹窗一致（宽 640px，深色玻璃卡片）
- 左侧封面区：120×120 预览 + 「更换封面…」按钮（直接复用 §5.7 裁剪弹窗）
- 右侧表单：
  - 标题 `*` / 艺术家 / 专辑艺术家 / 专辑 / 流派 / 年份
  - 曲目号（`2` / 共 `13`，两个 number input）
  - 碟号（`1` / 共 `2`，两个 number input）
  - 评分（5 星选择器，和行内一致）
  - 歌词（多行 textarea，可带 LRC `[mm:ss.xx]` 时间戳——如果 phonon-codec 目前不支持解析 LRC 存库就 UI 上允许编辑，后端忽略带时间戳行，存纯歌词文本）
- 底部：
  - 「取消」 / 「保存（写库 + 写文件）」
  - 保存执行顺序（**顶级韧性，绝不丢数据**）：
    1. 先写 phonon-media SQLite（`library_edit_metadata(track_id, meta)`）—— 100% 成功（因为我们控制库）
    2. **后**尝试异步写音频文件（`codec.write_metadata_to_file(path, meta)`）
       - 如果成功（格式支持 + 权限 OK）：Toast「✓ 已保存到媒体库并写回文件」
       - 如果失败：Toast「⚠️ 已保存到媒体库；文件元数据回写失败：{具体原因}，稍后可在文件权限开放后再从「…」→「重新回写选中曲目文件元数据」触发」
    - **关键点**：哪怕 2 失败，**绝不回滚 1**（用户输入始终入库，不会因为一个写文件错误就丢了 10 分钟编辑工作）
- **文件写回格式支持矩阵**（RC 版本的能力，不强求全格式）：
  - ✓ FLAC（Vorbis Comment）
  - ✓ MP3（ID3v2.4）
  - ✓ WAV（RIFF INFO，有限支持）
  - ~~DSD/DSF~~（只读，编辑弹窗打开时显示 "DSD 文件暂不支持文件回写，已仅保存到媒体库" 顶部小字 banner）
- 如果 phonon-codec 目前**还没有** `write_metadata_to_file()` 函数，RC 版先只做"写库"，保存按钮下方加一行灰色说明：「文件元数据回写功能将在下个版本支持，本次已仅保存到 Phonon 媒体库」。功能本身代码结构预留（写一个 stub `Result::Err("not implemented for RC")`，不阻塞 Tab 交付）。

### 5.7 🖼 更换封面（1:1 Canvas 裁剪，零新依赖）
- 打开文件：`dialog.open({ multiple: false, filters: [{name: 'Image', extensions: ['jpg','jpeg','png','webp','bmp']}] })`
- 裁剪器：**纯原生实现**，不引入 `react-image-crop` 新依赖

  ```
  // ================================================================
  // CropCoverModal.tsx — Coordinate System（代码注释里原样保留）
  // ================================================================
  // 约定（屏幕坐标系，单位：像素）：
  //   - 显示容器 displayBox：固定 400×400，左上角 = (0, 0)
  //       x 向右增大为正，y 向下增大为正（屏幕标准）
  //   - 裁剪框 cropBox：固定 320×320 居中，不可移动
  //       { x: (400-320)/2 = 40, y: 40, w: 320, h: 320 }
  //   - 图片显示（经过 fit + scale + 拖拽偏移）：
  //       * fitRatio = min(displayBoxW/imgNaturalW, displayBoxH/imgNaturalH)
  //         作用：让原图进入 400×400 容器时，至少有一个方向贴边
  //       * scale：用户滑条设置（0.5 ≤ scale ≤ 3.0）
  //       * displayImgW = imgNaturalW * fitRatio * scale
  //         displayImgH = imgNaturalH * fitRatio * scale
  //       * offsetX / offsetY（用户拖拽状态，可负）：
  //         图片左上角相对容器左上角 (0,0) 的偏移
  //         → 向右拖 offsetX 增大（正），向下拖 offsetY 增大（正）
  //         → 例：把图片向右拖 50px + 向下拖 30px → offsetX=+50, offsetY=+30
  // ================================================================
  ```

  - `<input type="file">` 读 File → `URL.createObjectURL` 渲染 `<img onload>` 读取 `img.naturalWidth / img.naturalHeight`（记为 imgNaturalW/H）
  - 初始默认值：`scale = 1.0`，`offsetX/Y` 计算到让图片居中贴满裁剪框
  - UI：图片容器上叠加 1:1 固定方形 mask（虚线边框，外部半透明黑色）
  - **交互 1：拖动图片**
    - `mousedown` 记录起点，`mousemove` 增量更新 offsetX/Y，`mouseup` 结束
    - **边界 clamp**（拖拽后强制约束裁剪框不跑出图片矩形，避免裁到透明）：
      ```
      minOffsetX = cropBox.x + cropBox.w - displayImgW  // 向左拖的最小限制
      maxOffsetX = cropBox.x                                // 向右拖的最大限制
      minOffsetY = cropBox.y + cropBox.h - displayImgH
      maxOffsetY = cropBox.y
      offsetX = clamp(offsetX, minOffsetX, maxOffsetX)
      offsetY = clamp(offsetY, minOffsetY, maxOffsetY)
      ```
  - **交互 2：缩放滑条**（50% – 300%），实时预览；缩放后重新对 offsetX/Y 做一遍边界 clamp
  - **坐标转换 + 输出 Blob**（用户点「确认裁剪」时）：
    ```ts
    // Step 1：显示坐标 → 原图像素坐标
    // 裁剪框显示区域对应：
    const dispCropX = cropBox.x;               // = 40
    const dispCropY = cropBox.y;               // = 40
    const dispCropSize = cropBox.w;            // = 320
    // 图片左上角在显示坐标系下 = (offsetX, offsetY)
    // 所以裁剪框左上对应图片显示坐标系下的 (dispCropX - offsetX, dispCropY - offsetY)
    const imgDispCropX = dispCropX - offsetX;  // 正数（应该落在图片内，边界 clamp 保证）
    const imgDispCropY = dispCropY - offsetY;
    // 显示 → 原图缩放比例：
    const ratio = imgNaturalW / (imgNaturalW * fitRatio * scale); // = 1 / (fitRatio * scale)
    // 等价：ratio = 1 / (fitRatio * scale) （W 和 H 一致）
    const sx = imgDispCropX * ratio;  // 原图 x 起点
    const sy = imgDispCropY * ratio;  // 原图 y 起点
    const sSize = dispCropSize * ratio; // 原图像素上裁多大的正方形

    // Step 2：Canvas 输出 512×512
    const canvas = document.createElement('canvas');
    canvas.width = 512; canvas.height = 512;
    const ctx = canvas.getContext('2d')!;
    ctx.imageSmoothingEnabled = true;
    ctx.imageSmoothingQuality = 'high';
    ctx.drawImage(
      originalImgElement, // img onload 的 HTMLImageElement
      sx, sy, sSize, sSize, // 从原图哪里裁（像素）
      0, 0, 512, 512        // 画到 canvas 哪里（像素）
    );
    const blob = await new Promise<Blob>(res =>
      canvas.toBlob(b => res(b!), 'image/jpeg', 0.88)
    );
    ```

- 写库：Blob → 调 `library_set_track_cover(track_id, blob)` 后端命令
  - 后端逻辑：存 `thumbnails_cache/` 目录为 WEBP 256px（small）+ 512px（large）两张，写回 `tracks.thumbnail_hash`（SHA1 前缀）
  - 写成功后前端 `Map<hash, url>` cache 里清掉旧 hash 的 URL，强制下一次 render 重新取新缩略图（即时刷新 UI）

### 5.8 入口一致性（消除重复心智负担，顶级 UX）
**核心机制：歌单同步文件夹 ↔ 媒体库 roots = 自动双向同步，不用用户记两边都加一次**

#### 5.8.1 PlaylistToolbar「同步文件夹」 → 自动加入媒体库（新增 hook，**含 opt-out 开关**）
- 新增 opt-out 开关（AppSettings 字段，见 §6）：`library.auto_sync_playlist_folder_to_roots: boolean`，默认 `true`
  - Settings → 音频 SubTab 「📚 媒体库状态」卡片里新增原生 switch：「同步文件夹时自动加入媒体库」+ 右侧小字说明：「关闭后"同步文件夹"只做歌单绑定，不触发媒体库索引（仅临时播放、不想后台扫描元数据时用）」

- 现有 `syncFolder()` [PlaylistToolbar.tsx#L247-L288](file:///d:/Phonon/phonon-tauri/src/components/PlaylistToolbar.tsx#L247-L288) 成功回调尾部追加：
  ```ts
  // ① 先读用户 opt-out 开关
  const settings = await settingsApi.get();
  if (!settings.library?.auto_sync_playlist_folder_to_roots) {
    toast('✓ 已创建歌单同步（按你的设置，未加入媒体库索引）');
    return; // 用户关了就不做 roots 同步
  }
  // ② 开关开着才执行
  try {
    const currentRoots = await library.getRoots();
    if (!currentRoots.includes(selectedFolderPath)) {
      const deduped = Array.from(new Set([...currentRoots, selectedFolderPath]));
      await library.setRoots(deduped);
      // 静默 kick 一次扫描，不阻塞 UI
      library.scan({ mode: 'incremental' }).catch(() => {});
      toast('📚 已同步加入媒体库，正在后台索引元数据…');
    }
  } catch (e) {
    // 非关键路径，失败不影响同步文件夹本身
    console.warn('Sync folder to library roots hook failed', e);
  }
  ```
- PlaylistToolbar「导入文件夹」：如果路径 `someParentDir` 的祖先在媒体库 roots 已覆盖 → 直接走 `library.getTracksByPathPrefix(someParentDir)` 拿元数据（比 scan_folder 快 N 倍，因为已经解析好存在库里），**直接构造带封面/星级的 QueueItem**，首屏不闪占位图。

#### 5.8.2 Library Tab「+ 添加音乐目录」 → 可选新建同步歌单
- 从 Library Tab「…」菜单或空状态页点「+ 添加音乐目录」选了 D:\X\ 后：
  - Modal（一行字，不打断流程）：「已加入媒体库 📚。要同时新建一个绑定了"**D:\\X**"的同步歌单吗？这样文件夹增删文件会自动进这个歌单。」[不，谢谢] [✓ 创建同步歌单]
  - 如果 yes → 调 `playlist.create({ name: basename('D:\\X') })` + `playlist.setFolderSync(playlistId, 'D:\\X')`，自动切到播放器 Tab 并激活这个新歌单

#### 5.8.3 Settings → 音频 SubTab「媒体库配置」精简为**状态展示 + 跳转按钮**（入口去重！）
- 原来设计的「目录增删列表 / 添加目录按钮 / 立即扫描 / 清理 / 重建索引」**全部移出 Settings**，入口物理上不重复
- Settings 里保留的内容（只读状态 + 跳转按钮 + 批量操作快捷按钮）：
  ```
  ── 📚 媒体库状态 ──────────────────────────────────────────────
  已配置目录：3 个（共 1,240 首 / 56 GB）
  上次扫描：2026-08-19 20:30（+12 / ~2 / -0）
  ReplayGain：已扫描 856 / 未扫描 384

  [打开媒体库 Tab 管理目录…]   [立即批量扫描 ReplayGain（384 首）]
  ──────────────────────────────────────────────────────────────
  ```
- 「打开媒体库 Tab 管理目录…」按钮 = `setActiveTab('library')` + Library Tab 自动展开「…」菜单（用一个 `autoOpenMenuOnMount: true` query state 传 1 次）

### 5.9 3.9 空状态引导页（首启未配置 roots 时）
**Library Tab 内容区不显示空表格**，一张居中 520×320 卡片：
```
┌──────────────────────────────────────────────────────┐
│  📚  你的音乐库还是空的                                │
│                                                       │
│  先把音乐文件夹添加进来，Phonon 会在后台自动：          │
│  • 解析所有歌曲的标题/艺术家/专辑/流派/封面             │
│  • 计算音频响度（ReplayGain），切换专辑不会音量忽大忽小│
│  • 构建全文搜索索引，秒级搜到你想听的那首              │
│                                                       │
│         [⚙ 添加音乐目录…]   [📁 先扫描一个文件夹试试]  │
│                                                       │
│  💡 小贴士：你在"播放器 → 同步文件夹"里加过的目录，     │
│     已经自动同步到这里的媒体库啦！可能正在后台扫描中…   │
│     [查看扫描进度条（如有在跑）]                        │
└──────────────────────────────────────────────────────┘
```
- 「📁 先扫描一个文件夹试试」= 选目录 → 临时加入 roots → `library.scan()` 并订阅进度事件 → 扫完自动跳转「🎵 全部」子视图展示前 50 条结果
- 如果**正在扫描**（用户刚从同步文件夹过来已经在扫了）：卡片下方额外显示一条 `███████░░░░ 42%` 进度条 + 「正在扫描 {currentFolder}… 已找到 N 首」

### 5.10 错误边界 & 降级（4 类韧性机制）
1. **封面失败 3 次指数退避重试**：`setTimeout(fn, 300ms)` → 900ms → 2700ms，仍失败 → 永久降级为 🎵 占位图，控制台 warn 一次但不抛全局错
2. **搜索超时**：>2s spinner 灰 + 显示「搜索超时，请重试」按钮；>5s 自动取消 Promise（AbortController）
3. **元数据文件写回失败**：§5.6 已述——库内数据永远先写成功，不回滚；失败 toast 提供「稍后从菜单重试选中项文件写回」的入口
4. **SQLite 损坏保护**（极罕见但必须处理）：
   - phonon-media `Library::open()` 时如果 `sqlite3_errmsg` 是 `corrupt`：
     1. 把 `library.db` 复制为 `library.db.corrupt-YYYYMMDD-HHMMSS` 到同目录
     2. 删旧文件，新建空库
     3. 前端启动时收到 `library-corrupted` Tauri event → 顶部红色 banner 7 秒 + 关闭按钮：「⚠️ 媒体库数据库已损坏，已自动备份为 library.db.corrupt-20260819，当前使用全新媒体库」→ 提供「尝试修复旧库」按钮（调新命令 `library_try_recover_corrupt(path_to_backup)`，如果 phonon-media 实现简单就做，否则仅保留备份路径 + 手动提示，可 RC2 补）

### 5.11 新增组件文件清单（A 子项目）
| 新文件路径 | 职责 |
|---|---|
| phonon-tauri/src/components/LibraryPage.tsx | 顶层：SubTab 路由 + 工具条 + 状态栈导航 + 「…」菜单 + 空状态引导 + 错误边界 |
| phonon-tauri/src/components/library/TrackTable.tsx | 9 列懒加载表格 + 多选 + 9 项右键菜单 + 正在播放高亮 + 虚拟列表（≥10k 首启用，固定 72px 行高零新依赖） |
| phonon-tauri/src/components/library/AlbumGrid.tsx | 160px 卡片网格 + hover 播放按钮 |
| phonon-tauri/src/components/library/AlbumDetailPage.tsx | Apple Music 风格 300px 封面 + 元数据条 + 曲目表格（复用 TrackTable） |
| phonon-tauri/src/components/library/ArtistGenreList.tsx | 首字母分组列表 + 详情页（复用 AlbumDetailPage 结构） |
| phonon-tauri/src/components/library/SearchSuggestDropdown.tsx | 3 分类联想下拉 UI |
| phonon-tauri/src/components/library/EditMetadataModal.tsx | 12 字段编辑弹窗（封面预览 + 换封面入口） |
| phonon-tauri/src/components/library/CropCoverModal.tsx | 纯 Canvas 1:1 裁剪 + 缩放滑条（零新依赖） |
| phonon-tauri/src/components/library/LibraryMoreMenu.tsx | 右上角「⋮」菜单（7 项管理操作） |

---

## §6 AppSettings 字段新增清单（全部向后兼容）

```ts
// 新增子结构（所有字段均有 serde default / Rust enum Default）
interface AppSettings {
  // —— 新增 C 子项目字段 ——
  dsp: {
    // ... 现有 eq / replaygain 字段保留
    time_stretch_mode: 'auto' | 'wsola' | 'phase_vocoder'; // 默认 'auto'
  };
  // —— 新增 B 子项目字段 ——
  hotplug: {
    // 保留
    show_notification: boolean; // 默认 true
    // 新增
    on_device_removed: 'pause' | 'switch_default' | 'switch_last_used'; // 默认 'pause'
    on_new_device_inserted: 'ignore' | 'auto_switch';                    // 默认 'ignore'
    last_used_device_id: string | null;                                   // 默认 null
  };
  // —— 新增 A 子项目字段（入口一致性 opt-out）——
  library: {
    auto_sync_playlist_folder_to_roots: boolean; // 默认 true（顶级体验：同步文件夹自动入媒体库）
  };
}
```

**兼容保障**：所有字段在 Rust 端用 `#[serde(default)]` 或 `#[serde(default = "default_true")]`；Enum 用 `#[derive(Default)]`；`HotplugConfig` 作为子结构体整体默认化。旧 JSON（不含任何新字段）解析 100% 成功，不触发 migrate。

---

## §7 新增 Tauri 命令 & 前端 API 封装

### 7.1 新增 Tauri 命令（commands.rs 注册）
| 命令 | Rust 后端（入/出） | 所属模块 |
|---|---|---|
| `library_get_tracks_by_paths` | `fn(paths: Vec<String>) -> Vec<LibraryTrack>`（WHERE path IN，保持输入顺序，500 条上限） | phonon-media / library.rs |
| `library_search_suggest` | `fn(query: String) -> LibrarySuggest { tracks[..5], albums[..3], artists[..3] }`（FTS5 prefix 查询） | phonon-media / search.rs |
| `library_edit_metadata` | `fn(track_id: String, partial: PartialMeta) -> Result<LibraryTrack, LibError>` | phonon-media / library.rs |
| `library_set_track_cover` | `fn(track_id: String, cover_bytes: Vec<u8>, mime: String) -> Result<ThumbnailHash, LibError>` | phonon-media / thumbnails.rs + library.rs |
| `library_try_recover_corrupt` | `fn(backup_path: String) -> Result<u32, LibError>`（恢复成功曲目数；RC 可先实现 stub 返回 0 + `todo!()` 标在 Issue） | phonon-media / library.rs |
| `library_write_metadata_back_to_file` | `fn(track_id: String) -> Result<(), CodecError>`（§5.6 步骤 2 单独触发重试时用） | phonon-codec（或 stub） |

### 7.2 前端 API 封装（api/library.ts 新增）
对应上面 6 条命令 + §5.8 入口一致性用到的 `getRoots()` / `setRoots()` / `scan()`（已存在，检查导出即可）：
```ts
getTracksByPaths(paths: string[]): Promise<LibraryTrack[]>
searchSuggest(query: string): Promise<LibrarySuggest>
editMetadata(trackId: string, partial: PartialMeta): Promise<LibraryTrack>
setTrackCover(trackId: string, blob: Blob): Promise<string /* hash */>
tryRecoverCorrupt(backupAbsPath: string): Promise<number /* restored count */>
writeMetadataBackToFile(trackId: string): Promise<void>
```

### 7.3 类型定义补全
- `LibrarySuggest`、`PartialMeta`、`HotplugConfig`（`on_device_removed` / `on_new_device_inserted` union）、`TimeStretchMode` 加到 api/types.ts（或各自文件就地 export + index.ts re-export）

---

## §8 验收标准（每个子项目，给你装上 RC 包后自己可逐项验证）

### 8.1 C：TimeStretch Auto Hybrid
1. [ ] Settings→DSP→高级变速算法：下拉能切 Auto/WSOLA/PV，选 Auto 后说明文字显示当前档位实际使用
2. [ ] 本地开播放器放歌：1.0x / 1.25x / 0.8x 三档切，播放 10 秒切歌无 artifacts；1.75x 切回无爆音
3. [ ] EXACT_TS=1 跑本地 timestretch_pitch 测试：0.8/1.0/1.25x 三档容差 ±5 cent 全部通过（或 <3 条失败且失败点标注为 platform-specific 浮动）
4. [ ] 三平台 CI：`-p phonon-core --test timestretch_pitch` 不红（默认容差层）

### 8.2 B：关联 + 自动历史 + 热插拔策略
1. [ ] **关联**：从媒体库点一首库里有的歌加入队列 → QueueItem 封面/收藏/评分立即显示（无占位闪烁）；从 PlaylistToolbar 导入文件夹且文件夹在媒体库内 → 所有曲目秒出封面/星级（不扫重扫文件）
2. [ ] **自动历史**：点一首库里的歌 → 播放 35 秒 → 手动暂停 → 切到「🕒 最近」子视图 → 这首歌出现（相对时间 "刚刚"）；首 10 秒快速切歌 10 首 → 「🕒 最近」不增加 10 条脏数据（只入有效播放那些）
3. [ ] **跨视图高亮**：打开 Library Tab 正在播 A 歌 → 切到「专辑」→ 进 A 歌所在专辑 → A 歌行有 ▶ 动画 + 背景高亮 + 自动滚到可视区
4. [ ] **热插拔策略 1（默认 pause）**：播放中拔耳机线 → 播放暂停，toast 通知；插回去 → 不自动恢复（用户需手动点播放，符合默认安全）
5. [ ] **热插拔策略 2（switch_default）**：Settings→音频切到"拔出→切默认设备"→ 拔耳机 → 自动切扬声器 → 继续播放（如果默认设备是扬声器且可用）；Toast 有说明
6. [ ] **兼容**：关应用 → 手动删 AppSettings JSON 里新增的 4 个字段 → 重开 → 启动无崩溃，4 字段自动补默认值

### 8.3 A：Library Tab 全功能
1. [ ] **入口齐全**：主 Tab 栏有「📚 媒体库」；进 Tab 有 6 个 SubTab；右上角有搜索框 + 筛选 + 「…」菜单 + 统计
2. [ ] **三基本视图**：🎵 全部（9 列表格封面懒加载 + 双击播放 + 右键 9 项菜单 + ≥1 万首启用虚拟列表）/ ⭐ 收藏（只显示收藏）/ 🕒 最近（按时间倒序 + 相对时间列）
3. [ ] **搜索联想**：输入 "a" → 300ms 后下拉 5 首 + 3 专辑 + 3 艺术家；点专辑联想项 → 直接跳专辑详情页
4. [ ] **专辑详情**：进某专辑详情页 → 顶部 300px 封面条 + [▶播放专辑] 按钮 → 点后替换队列播放整张专辑；曲目表格行内 ★ 和评分可点
5. [ ] **元数据编辑**：右键某首 → 编辑元数据 → 改标题 + 评 5 星 → 保存 → Toast 显示「✓ 已保存到媒体库并写回文件」或「仅保存到媒体库（回写失败原因）」；列表和 Player 当前行立即反映变化（乐观 UI）
6. [ ] **换封面**：右键某首 → 更换封面 → 选 JPG → 拖动裁剪框 + 70% 缩放 → 确认 → 表格封面 1 秒内刷新（无缓存）
7. [ ] **入口一致性（关键 UX）**：去播放器 Tab → PlaylistToolbar 同步文件夹选 D:\X → 成功后 Toast「📚 已同步加入媒体库…」→ 切到 Library Tab → 右上角「…」→ 立即扫描 → 扫完「🎵 全部」里有 D:\X 的内容
8. [ ] **空状态引导**：关应用 → 把媒体库数据库改名（模拟首次）→ 开应用 → 进 Library Tab → 显示大空卡片 → 点「📁 先扫描一个文件夹试试」→ 选目录 → 扫完自动跳「🎵 全部」显示结果
9. [ ] **韧性**：断网（模拟封面 CDN 挂）+ 开 1000 首列表 → 滚动 60fps 不卡顿；封面失败重试 3 次后稳定显示 🎵 占位图，无全局 Error Boundary 崩溃

### 8.4 打包 RC 验收（最终交付）
1. [ ] Windows x64：`cargo tauri build` 产出 NSIS .exe 安装包（~80 MB）+ MSIX 安装包
2. [ ] 安装包双击能装上（未签名 SmartScreen 警告可接受，RC 阶段）
3. [ ] 启动应用 → 第一次进入 Library Tab 显示空状态（全新环境）
4. [ ] 按 §8.1–§8.3 的冒烟 10% 关键 case（切 1.25x 无 artifacts + 同步文件夹自动入媒体库 + 元数据编辑写库）全部通过

---

## §9 风险 & 回滚预案

| 风险 | 概率 | 影响 | 缓解 & 回滚预案 |
|---|---|---|---|
| **R1：timestretch crate 三平台某平台编译/链接失败** | 中（20%） | 阻塞 C 子项目 → 连带 M1 测试包延迟 | 默认 fallback 机制已设计：如果 WSOLA 在某平台编译错，Cargo.toml 用 `[target.'cfg(not(any(target_os = "x")))'.dependencies]` 条件启用；M1 测试包先用默认 Phase Vocoder 打，等修复后在补丁包再切；不影响 A/B |
| **R2：phonon-media library_get_tracks_by_paths 在 10,000 首级 IN 查询性能差** | 中（30%） | B 子项目关联流程 50 条/批太慢 → 20 秒才能补齐封面/星级 | 提前准备：library 表 `path` 字段加 `INDEX idx_tracks_on_path`（schema.rs 里若没就补）；如果仍慢，把批量查询改成 `WHERE path = ANY($1)`（Postgres 风格，SQLite 支持用 carray 或临时表） |
| **R3：写回文件元数据（FLAC/MP3）边界 case 坏文件** | 低（10%） | 用户编辑后原文件损坏（不可接受！） | 写回前 **先 backup 副本**：`file.ext` → `file.ext.bak-YYYYMMDD-HHMMSS` 到同目录，写回 100% 成功完成（flush + fsync + reopen 校验可解析）后 30 天自动删 backup；如果写回失败，自动从 backup 还原原文件 + Toast「已自动还原为原文件」 |
| **R4：App.tsx 改 6 主 Tab 后窄屏（<1100px）宽度挤** | 高（70%） | Tab 栏折行或被裁切，丑 | 改前先算现有 5 Tab 像素宽：播放器/设备/DSP/媒体库/扩展/设置 = 6 个中文字 × 14px + 左右 24px padding ≈ 每个 Tab 108px，6 个 = 648px，加上 Logo + 右侧最小按钮 400px → 总宽 1048px < 1100px；**做响应式**：<1100px Tab 文案自动缩减（播放器→▶ / 媒体库→📚 / 设置→⚙ 只留图标）+ 图标模式 |
| **R5：C+B 打 M1 测试包后你反馈"变速 1.25x 有 artifacts/歌曲元数据编辑后列表不刷新"等回归 Bug** | 中（40%） | 延迟 A 开始时间，回滚 C/B 部分改动 | C/B 改动全部切分支（`feature/cb-timescale-hotplug-assoc`），A 单独切 `feature/a-library-full`；M1 包测完如果 Bug 严重，可独立 revert 对应 commit，不影响另一分支 |

---

## §10 Spec 自审清单（编写后立即自检通过）

- [x] 无 "TBD" / "TODO" 占位符（所有未实现能力的降级路径明确）
- [x] 内部一致：§3 time_stretch_mode 枚举和 §6 AppSettings 字段一致；§4 hotplug 三档两档和 §6 枚举默认值一致；§5.8 入口一致性和 PlaylistToolbar 现有 syncFolder 数据流不冲突
- [x] 范围聚焦：只做 C+B+A，不夹带 3D 可视化 / 插件系统 / 签名发布等 Out of Scope 内容
- [x] 无歧义：所有"点击后跳哪里 / 写顺序 / fallback 顺序 / 阈值（30s/50%/50 条批/300ms debounce）"都是明确数字，不给"合理阈值"这种模糊描述
