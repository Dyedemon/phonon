# Phonon 插件三层分类设计：Builtin / WasmExample / WasmExternal

- **版本**：v1.0
- **日期**：2026-08-16
- **状态**：Spec 草拟，待用户 review
- **关联**：tasks.md §6（Phonon Plugin 运行时完成度）、§14（合规性，静态扫描/权限）

---

## 0. 目标与非目标

### 0.1 目标

1. 给 Phonon 生态里所有"模块化能力"给出**单一、自洽、可检验**的分类定义与判定表格。
2. 把 DSP 均衡器等**平台能力**作为 **Builtin（内置插件）** 纳入 PluginManager 的统一 list_plugins 视图，在 UI/命令层实现"插件统一管理"。
3. 明确 **WasmExample（扩展示例插件）** 和 **WasmExternal（独立扩展插件）** 两者不属于 Phonon 安装包内容。
4. 对齐 tasks.md §14 合规策略（静态扫描 / 权限白名单 / 卸载保留）在三类插件上**各自的应用方式**。

### 0.2 非目标

- 不改变 DSP 链/均衡器的 **Rust 原生实现**（不把 Equalizer 编译成 wasm）；也不改变 EqPanel/DspPanel 的一等公民 UI 交互。
- 不引入新的动态链接/加载机制（所有 Wasm 仍然通过 wasmtime + wasi_stubs + epoch_interruption 路径）。
- 不建立示例插件的 CI 发布流程（那是下一轮 §13 打包与发布的子任务；本 Spec 只解决 Origin 识别）。

---

## 1. 三层分类总定义

### 1.1 Phonon 架构三层（从不可替换 → 真·动态加载）

```
Phonon 架构总览
├─ ① 核心引擎（Kernel / Non-Interface）
│     纯内核代码，不挂 trait，也绝对不出现在 PluginManager / list_plugins。
│     · PlaybackEngine（播放状态机：play/pause/stop/next）
│     · AudioRingBuffer（SPSC 环形缓冲 + 字节流拼装）
│     · DspChain 调度器（process 顺序、总延迟计算）
│     · Events::broadcast 事件总线
│     · Session JSON 读写
│
├─ ② 默认实现 + 可替换 trait 接口（"可替换但不叫插件"）
│     对外暴露 trait，默认实现位于 phonon-codec / phonon-core / phonon-output 等子包，
│     加载不经过 PluginRuntime，也不出现在 PluginManager 的安装栏里。
│     不叫"插件"，只作为「框架内可替换模块」。
│     · IAudioDecoder trait + 默认实现（FLAC/MP3/DSD/PCM/WAV/OPUS/Vorbis/AAC/WMA...）
│     · IAudioOutput trait + WASAPI/CoreAudio/ALSA/PulseAudio 实现
│     · ITimeStretch trait + Rubato PhaseVocoder 默认实现（PluginTimeStretcher 是 ③ 层回退）
│     · SpectrumAnalyzer FFT（Biquad/FFT，默认 impl，不是 DSP Processor）
│     · CUE Parser（ICueSheet trait + Default）
│
└─ ③ 真正插件（Plugin：.wasm / 动态加载，经过 PluginRuntime）
     本设计的"三类插件"属于这一大类下的三个 Origin：
     ├─ PluginOrigin::Builtin            = 内置插件（Phonon 自带，Rust 原生，不能删/不能 reload）
     ├─ PluginOrigin::WasmExample        = 扩展示例插件（随仓库 plugins/ 子包，CI 构建，不随安装包）
     └─ PluginOrigin::WasmExternal       = 独立扩展插件（外部 Git/第三方分发，用户导入 .wasm）
```

### 1.2 当前（v0.x）插件清单

#### 内置插件（Origin=Builtin）= 5 个 DSP（`phonon-core` Rust 原生实现）

| `plugin_id`（命令层 `enable_dsp()` 用） | manifest.name        | manifest.plugin_type | 描述                                                                 |
|------------------------------------------|----------------------|----------------------|----------------------------------------------------------------------|
| `equalizer`                              | `phonon_equalizer`   | `DspProcessor`       | GEQ 10/15 段、PEQ 16~20 段 IIR Biquad；前端 EqPanel（rAF 节流）交互。 |
| `smart_effect`                           | `phonon_smart_effect`| `DspProcessor`       | 9 模式自适应动态范围（off/auto/pop/rock/jazz/classical/electronic/vocal/bass_boost）。 |
| `surround_sound`                         | `phonon_surround`    | `DspProcessor`       | 6 模式沉浸环绕（off/concert/theater/studio/spacious/immersive）。    |
| `downmix`                                | `phonon_downmix`     | `DspProcessor`       | 多声道 → 立体声下混（FL/FR/C/LFE/BL/BR，带中置/环绕权重）。          |
| `replaygain`                             | `phonon_replaygain`  | `DspProcessor`       | ReplayGain 响度归一：Off / Track / Album；默认关闭保持 Bit-Perfect。 |

#### 扩展示例插件（Origin=WasmExample）= 仓库 `plugins/` 3 个子包

| crate 目录                      | manifest.name        | plugin_type     | 用途                                                          |
|---------------------------------|----------------------|-----------------|---------------------------------------------------------------|
| `plugins/phonon-simple-source`  | `simple_source`      | `SourceProvider`| 最小 IAudioSource 示例：单文件 MP3/FLAC；集成测试样本。       |
| `plugins/phonon-gain-processor` | `gain_processor`     | `DspProcessor`  | 极简 ±12dB 增益器：作为 Wasm DSP 开发模板。                  |
| `plugins/phonon-phase-vocoder-stretcher` | `phase_vocoder_stretcher` | `TimeStretch` | 变速不变调示例；`PluginTimeStretcher` 在 rubato 不可用时回退它。 |

#### 独立扩展插件（Origin=WasmExternal）= 用户导入 / 用户目录 plugins

- 当前清单：**无**（框架不携带任何第三方音源/商业音源，符合项目定位"纯净"）。
- 安装方式：用户导入单个 `.wasm`（或复制到用户目录 `plugins/`，定义见 §2.1 `plugin_dirs()`）。

---

## 2. 三类 Origin 硬特征速查表（按用户调整后）

> 2026-08-16 用户确认的三处关键调整：
> ① Events 明确 "Wasm 插件经宿主回调转发至 EVENT_TX"；
> ② 权限合并为 "Wasm 统一 Manifest permissions"；
> ③ 扫描明确 "用户目录优先于示例目录"，冲突时用户目录的外部插件覆盖同名示例插件。

| 维度                    | 内置插件 (Builtin)                                      | 示例插件 (WasmExample)                                                                                    | 外部插件 (WasmExternal)                                                                 |
|-------------------------|---------------------------------------------------------|-----------------------------------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------|
| **来源**                | 二进制内部 `BuiltinPluginRegistry::all()`（静态硬编码） | 开发模式扫描 `workspace/plugins/`；发布模式 **不扫描**                                                    | 扫描用户 `~/.phonon/plugins/`（或平台 user data 路径） + 「导入 .wasm」动作             |
| **发布**                | 随 Phonon 安装包（平台二进制）                          | 随仓库源码（CI 构建成 wasm 放 GitHub Actions artifacts，不打安装包）                                      | 完全独立仓库 / 独立发布渠道 / 独立 License；Phonon 无关联义务                           |
| **License**             | 同 Phonon 本体（MPL-2.0 / MIT+Apache-2.0 双许可）       | 与 Phonon 兼容的宽松许可（cargo-deny 白名单，`plugins/*/Cargo.toml` 需显式声明）                          | 插件作者自担；§14 静态扫描仍然生效，违规直接拒绝加载                                    |
| **权限**                | **无 Permission 字段**，内核级（不走 Wasm grants）      | **统一 `Manifest.permissions`**，默认 `[]` 空，用户在 UI 中显式勾选才能 FileSystem/Network/AudioOutput     | **统一 `Manifest.permissions`**，默认 `[]` 空，流程同上                                  |
| **Events**              | 直接访问 `EVENT_TX`（和 DSP 链/Engine 同一进程）        | **经宿主回调转发至 `EVENT_TX`**：Wasm 侧调用宿主提供的 `host::emit_event(name, payload)`，宿主再广播。    | **经宿主回调转发至 `EVENT_TX`**：同上（Wasm 插件一律走宿主转发路径，Builtin 除外）      |
| **扫描方式**            | `list_plugins()` 时由 `BuiltinPluginRegistry::all()` prepend（无需扫描） | 开发模式：先扫 `workspace/plugins/**/*.wasm`（或 crate 构建产物 `target/wasm32-wasi/release/*.wasm`）；<br>发布模式：完全不扫示例目录 | 扫用户 `~/.phonon/plugins/*.wasm` + 「导入」按钮 invoke `import_plugin(bytes)`          |
| **冲突处理**            | 不参与冲突（Builtin 永远在前，name 永不与用户 Wasm 同名前缀：`phonon_*` 命名保留） | 若用户目录存在同名 `manifest.name` 的 WasmExternal，**用户目录优先**（WasmExample 版本被丢弃/跳过）       | 同 WasmExample — 用户目录优先                                                            |
| **更新**                | 随 Phonon 版本发布                                      | 随仓库 `main` 分支更新，开发者本地 `cargo build --target wasm32-wasi` 重新生成                           | 用户手动替换 `.wasm` 或使用「热重载」按钮                                                |
| **删除按钮**            | Disabled（永不删除）                                    | Enabled（只是本地 wasm 被移除，不影响仓库源）                                                             | Enabled（同上）                                                                          |
| **热重载按钮**          | Disabled（Rust 原生，不适用）                          | Enabled（`reload()` + 若已挂 DSP 链则替换运行时实例 + 保留旧 Arc 引用直到 audio callback 结束）           | Enabled（同上）                                                                          |
| **静态规则扫描（§14 P3）** | 跳过（内核级，已审计）                                 | ✓ 严格启用（`static_scan()` 在 load/reload 两路径都走，任何可疑 URL/eval/外泄标记直接 SuspiciousContent） | ✓ 严格启用（同上）                                                                      |
| **添加到 DSP 链**       | AppState 初始化时自动挂载；PluginManager 的「+ DSP」按钮是 `enable_dsp(id, bool)` toggle | 走 `add_wasm_dsp()` → `create_processor(name)` → `WasmDspProcessor` 加入 DSP 链                         | 同 WasmExample                                                                          |
| **配置界面**            | 一等公民 DspPanel / EqPanel；PluginManager 点「配置」跳 DspPanel 对应锚点；同时支持统一 `get/set_plugin_config` JSON remap | 统一 `get_plugin_config(name) -> Value` + `set_plugin_config(name, Value)` 命令（原始 JSON 给 Wasm）     | 同 WasmExample                                                                          |

---

## 3. 后端实现细节（方案 B：半深度接入）

### 3.1 新增枚举与注册器

**在 `phonon-plugin/src/types.rs`：**

```rust
/// 插件来源分类——三层分类的核心枚举。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PluginOrigin {
    /// 内置插件：Phonon 发布包自带，Rust 原生，不通过 wasmtime 执行。
    Builtin,
    /// 扩展示例插件：随仓库 plugins/ 子包，不随安装包，只在开发模式下可见。
    WasmExample,
    /// 独立扩展插件：外部 Git/第三方分发，用户导入 .wasm。
    WasmExternal,
}
```

在 `PluginManifest` / `PluginInfo` 上增加：
```rust
pub struct PluginInfo {
    pub manifest: PluginManifest,
    pub state: PluginState,
    pub file_path: String,
    pub error_message: Option<String>,
    /// 新增：来源分类，供前端分组/徽章/按钮禁用逻辑。
    pub origin: PluginOrigin,
}
```

**在 `phonon-plugin/src/builtin.rs`（新文件）：**
```rust
//! BuiltinPluginRegistry — 内置插件静态注册表。
//!
//! 均衡器等 5 个 DSP 在这里以「虚拟 manifest」形式登记。它们的真实 process
//! 逻辑不经过 wasmtime，而是由 phonon-core 里的 Rust 原生 DSP 实现直接挂在
//! DspChain 上。本注册表的唯一职责是给 `list_plugins()` / `PluginManager`
//! 提供与 Wasm 插件同构的元数据视图。

use crate::types::{PluginManifest, PluginPermission, PluginType};

pub struct BuiltinPlugin {
    pub id: &'static str,              // 命令层 enable_dsp / plugin_id 用
    pub manifest: PluginManifest,
    pub config_remarks: &'static str,  // 展示在 PluginManager 里的说明文案
}

pub fn all() -> &'static [BuiltinPlugin] {
    &BUILTINS
}

static BUILTINS: [BuiltinPlugin; 5] = [
    BuiltinPlugin {
        id: "equalizer",
        manifest: PluginManifest {
            name: "phonon_equalizer".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            author: "Phonon Contributors".into(),
            description: "参数均衡器：支持 GEQ 10/15 段 ISO 1/3-octave（31.5 Hz–16 kHz）与 PEQ 16~20 段（20 Hz–20 kHz），IIR Biquad 实现；前端 EqPanel（rAF 节流）精细控制每段 freq/gain/Q，切换 GEQ/PEQ 模式与 10/15 段数量。".into(),
            plugin_type: PluginType::DspProcessor,
            permissions: vec![],
            min_framework_version: env!("CARGO_PKG_VERSION").into(),
        },
        config_remarks: "DspPanel → Equalizer：Parametric（默认 16 段 ISO 1/3-octave peaking，扩至 20） / Graphic 10 or 15；前级预增益 (preamp) -12~+12 dB。",
    },
    BuiltinPlugin {
        id: "smart_effect",
        manifest: PluginManifest {
            name: "phonon_smart_effect".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            author: "Phonon Contributors".into(),
            description: "Smart Effect 九模式自适应动态范围控制：off/auto/pop/rock/jazz/classical/electronic/vocal/bass_boost；按流派自动调整压缩器与均衡曲线。".into(),
            plugin_type: PluginType::DspProcessor,
            permissions: vec![],
            min_framework_version: env!("CARGO_PKG_VERSION").into(),
        },
        config_remarks: "DspPanel → Smart Effect：单选九模式 + Auto 自动按流派推断开关。",
    },
    BuiltinPlugin {
        id: "surround_sound",
        manifest: PluginManifest {
            name: "phonon_surround".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            author: "Phonon Contributors".into(),
            description: "Surround Sound 六模式沉浸空间化：off/concert/theater/studio/spacious/immersive；对立体声/多声道素材模拟或强化双耳空间印象，耳机/音箱分别优化。".into(),
            plugin_type: PluginType::DspProcessor,
            permissions: vec![],
            min_framework_version: env!("CARGO_PKG_VERSION").into(),
        },
        config_remarks: "DspPanel → Surround Sound：六模式图标化切换。",
    },
    BuiltinPlugin {
        id: "downmix",
        manifest: PluginManifest {
            name: "phonon_downmix".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            author: "Phonon Contributors".into(),
            description: "多声道→立体声下混（FL/FR/C/LFE/BL/BR）；可配置中置/环绕/低音通道权重，保证 5.1/7.1 片源在立体声耳机/音箱上不失真还原。".into(),
            plugin_type: PluginType::DspProcessor,
            permissions: vec![],
            min_framework_version: env!("CARGO_PKG_VERSION").into(),
        },
        config_remarks: "DspPanel → Downmix：启用开关 + 权重滑条（Center/ Surround/ LFE），默认关闭以保留多声道直通。",
    },
    BuiltinPlugin {
        id: "replaygain",
        manifest: PluginManifest {
            name: "phonon_replaygain".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            author: "Phonon Contributors".into(),
            description: "ReplayGain 响度归一化：Off/Track/Album 三模式；读取文件元数据中 ReplayGain tag（track/album gain + peak）自动调整增益，默认关闭以保持 Bit-Perfect 路径。".into(),
            plugin_type: PluginType::DspProcessor,
            permissions: vec![],
            min_framework_version: env!("CARGO_PKG_VERSION").into(),
        },
        config_remarks: "DspPanel → ReplayGain：三模式单选；无 tag 时自动 fallback 到 Off 不调整。",
    },
];
```

### 3.2 PluginRuntime 改动

**`PluginRuntime::list_plugins()` 改动：**

```rust
pub fn list_plugins(&self) -> Vec<PluginInfo> {
    use crate::builtin;
    use crate::types::PluginOrigin;

    // 1. 收集 Wasm 插件（WasmExample + WasmExternal）。
    //    Origin 的判定由 scan_plugins() 阶段在 PluginInfo.origin 字段上直接打好：
    //      · 用户 plugins_dir（平台 user data，始终扫） → WasmExternal
    //      · workspace/example_plugins_dir（debug 模式 + 命中 EXAMPLE_NAMES 白名单） → WasmExample
    //    因此 list_plugins() 阶段只做「冲突 + 合并排序」，不再靠 file_path 前缀猜 origin
    //    （因为示例 crate 的 wasm 构建产物在 target/wasm32-wasi/release/*，路径前缀不落在 plugins/ 下）。
    let wasm_plugins = self.plugins.lock().unwrap().clone();

    // EXAMPLE_NAMES 白名单：与 workspace/plugins/* 的 manifest.name 一一对应。
    // 任意新增示例子包都需先在这里登记，同时在 §1.2 清单里补项。
    static EXAMPLE_NAMES: &[&str] = &[
        "simple_source",
        "gain_processor",
        "phase_vocoder_stretcher",
    ];

    // 2. 冲突处理：先收集 WasmExternal 写入 set；同名 WasmExample 后面丢弃。
    //    发布模式（not debug_assertions）下 WasmExample 不会被 scan_plugins 扫出来，
    //    因此这一步本身对发布模式是 no-op。
    let mut result: Vec<PluginInfo> = Vec::new();
    let mut external_names = std::collections::HashSet::new();

    // 先压 WasmExternal（用户目录优先于示例目录；§2 表格用户调整点 ③）
    for mut info in wasm_plugins.clone() {
        if matches!(info.origin, PluginOrigin::WasmExternal) {
            external_names.insert(info.manifest.name.clone());
            result.push(info);
        }
    }
    // 再压 WasmExample（仅保留未被用户目录同名插件覆盖的；命中白名单额外校验一次）
    for mut info in wasm_plugins {
        if matches!(info.origin, PluginOrigin::WasmExample)
            && EXAMPLE_NAMES.contains(&info.manifest.name.as_str())
            && !external_names.contains(&info.manifest.name)
        {
            result.push(info);
        }
    }

    // 3. prepend Builtin。
    let mut builtins: Vec<PluginInfo> = builtin::all().iter().map(|b| PluginInfo {
        manifest: b.manifest.clone(),
        state: PluginState::Running, // 内置永远 Running（Rust 原生已挂 DSP 链）
        file_path: format!("builtin:{id}", id = b.id),
        error_message: None,
        origin: PluginOrigin::Builtin,
    }).collect();

    builtins.append(&mut result);
    builtins
}
```

**`PluginRuntime::load()` / `reload()` / `remove_plugin()`：**
- 针对 origin=Builtin，一律返回 `PluginError::Static(name.to_string(), "built-in plugins are static")` 明确告诉前端禁用按钮。
- load/reload 继续执行 `static_scan()`（对 WasmExample / WasmExternal 生效；Builtin 永远不会走到 load）。

### 3.3 Tauri 命令层（commands.rs）新增 remap 命令

```rust
// ================================================================
// 统一插件配置：Builtin remap 到对应 DSP 命令；Wasm 给原始 JSON
// ================================================================

#[derive(Serialize, Deserialize)]
#[serde(tag = "op", content = "value")]
pub enum EqConfigPatch {
    Mode { mode: String },
    Bands { bands: Vec<EqBand> },
    GeqCount { count: u32 },
    Preamp { db: f32 },
}

#[tauri::command]
pub fn get_plugin_config(state: State<'_, AppState>, name: String) -> Result<serde_json::Value, String> {
    use phonon_plugin::PluginOrigin;
    let origin = plugin_origin(&state.plugin_runtime, &name);
    match (origin, name.as_str()) {
        (PluginOrigin::Builtin, "phonon_equalizer")   => get_eq_state_inner(&state).map(|s| serde_json::to_value(s).unwrap()),
        (PluginOrigin::Builtin, "phonon_smart_effect")=> Ok(serde_json::json!({ "mode": get_smart_effect_inner(&state)? })),
        (PluginOrigin::Builtin, "phonon_surround")    => Ok(serde_json::json!({ "mode": get_surround_inner(&state)? })),
        (PluginOrigin::Builtin, "phonon_replaygain")  => Ok(serde_json::json!({ "mode": get_rg_inner(&state)?.to_string() })),
        (PluginOrigin::Builtin, "phonon_downmix")     => Ok(serde_json::json!({ "enabled": get_dsp_enabled_inner(&state, "downmix")? })),
        (PluginOrigin::Builtin, other)                => Err(format!("未知内置插件 '{other}'")),
        _ /* Wasm */ => state.plugin_runtime.get_config(&name).map(|v| serde_json::to_value(v).unwrap())
                                          .map_err(|e| e.to_string()),
    }
}

#[tauri::command]
pub fn set_plugin_config(state: State<'_, AppState>, name: String, value: serde_json::Value) -> Result<(), String> {
    use phonon_plugin::PluginOrigin;
    let origin = plugin_origin(&state.plugin_runtime, &name);
    match (origin, name.as_str()) {
        (PluginOrigin::Builtin, "phonon_equalizer") => {
            // value 支持三种 patch：{mode} / {bands} / {geq_band_count} / {preamp_db}
            if let Some(mode) = value.get("mode").and_then(|v| v.as_str()) {
                set_eq_mode_inner(&state, mode.to_string())?;
            }
            if let Some(bands) = value.get("bands") {
                let bands: Vec<EqBand> = serde_json::from_value(bands.clone()).map_err(|e| e.to_string())?;
                set_eq_bands_inner(&state, bands)?;
            }
            if let Some(count) = value.get("geq_band_count").and_then(|v| v.as_u64()) {
                set_geq_bands_inner(&state, count as u32)?;
            }
            if let Some(db) = value.get("preamp_db").and_then(|v| v.as_f64()) {
                set_eq_preamp_inner(&state, db as f32)?;
            }
            Ok(())
        }
        (PluginOrigin::Builtin, "phonon_smart_effect") => {
            let mode = value.get("mode").and_then(|v| v.as_str()).ok_or("missing mode")?;
            set_smart_effect_inner(&state, mode.to_string())
        }
        (PluginOrigin::Builtin, "phonon_surround") => {
            let mode = value.get("mode").and_then(|v| v.as_str()).ok_or("missing mode")?;
            set_surround_inner(&state, mode.to_string())
        }
        (PluginOrigin::Builtin, "phonon_replaygain") => {
            let mode = value.get("mode").and_then(|v| v.as_str()).ok_or("missing mode")?;
            set_replay_gain_inner(&state, mode.to_string())
        }
        (PluginOrigin::Builtin, "phonon_downmix") => {
            let enabled = value.get("enabled").and_then(|v| v.as_bool()).ok_or("missing enabled")?;
            enable_dsp_inner(&state, "downmix".to_string(), enabled)
        }
        (PluginOrigin::Builtin, other) => Err(format!("未知内置插件 '{other}'")),
        _ /* Wasm */ => state.plugin_runtime.set_config(&name, value.to_string())
                                          .map_err(|e| e.to_string()),
    }
}

#[tauri::command]
pub fn toggle_plugin_in_dsp(state: State<'_, AppState>, name: String, enabled: bool) -> Result<(), String> {
    use phonon_plugin::PluginOrigin;
    match plugin_origin(&state.plugin_runtime, &name) {
        PluginOrigin::Builtin => {
            let id = name.strip_prefix("phonon_").unwrap_or(&name).to_string();
            enable_dsp_inner(&state, id, enabled)
        }
        _ => {
            if enabled { add_wasm_dsp_inner(&state, name)?; }
            else       { remove_dsp_inner(&state, name)?; }
            Ok(())
        }
    }
}
```

**说明**：上述 `*_inner()` 表示对现有命令（`set_eq_mode` / `set_eq_bands` / `enable_dsp` / ...）的直接内联调用或提取公共私有函数。这一步是"半深度接入"的核心——所有操作都最终回到现有 100% 已验证的 DSP 命令链，不引入新的 process 路径，因此无运行时风险。

### 3.4 Wasm 事件转发（§2 表格 Events 行的落地）

在 `phonon-plugin/src/runtime.rs` 的 `linker_init` 中新增一个 host import：

```rust
// host::emit_event(name: &str, payload: &str) → i32 (0 ok, 1 err)
linker.func_wrap(
    "host", "emit_event",
    |mut caller: Caller<'_, HostState>, name: Val, payload: Val| -> Result<i32> {
        let (name, payload) = read_two_strs(&mut caller, name, payload)?;
        if let Some(tx) = &*EVENT_TX.read().unwrap() {
            // payload 已经是 JSON；由 wasm 插件生成；宿主不对其 schema 做假设。
            let _ = tx.send(events::Event::PluginUserEvent { name, payload });
            Ok(0)
        } else {
            Ok(1)
        }
    }
)?;
```

对应的前端事件订阅：在 `App.tsx` 监听 **`plugin-user-event`**（事件系统新增 `Event::PluginUserEvent { name, payload }` 变体，与原有 `plugin-loaded` / `vis-plugin-*` 命名一致但语义独立），Payload: `{ name: string, payload: string }`；宿主转发时使用现有 `AppHandle.emit_to::<MainWindowLabel>` 发送到主窗口。

所有 Wasm 插件（WasmExample / WasmExternal）emit 事件必须走这条宿主转发路径。Builtin 可直接 `EVENT_TX.send(...)`，无额外约束。

---

## 4. 前端呈现（ExtensionsPanel → PluginManager 三段分组 + DspPanel 徽章）

### 4.1 类型层（`plugins.ts`）

```typescript
export type PluginOrigin = 'Builtin' | 'WasmExample' | 'WasmExternal';

export interface PluginInfo {
  manifest: PluginManifest;
  state: PluginState;
  file_path: string;
  error_message: string | null;
  origin: PluginOrigin;            // 新增
}
```

### 4.2 PluginManager（ExtensionsPanel → PluginManager SubTab）分组视图

三段 **垂直分组**，中间用 `<hr className="plugin-group-divider">` 分隔；每段有自己的 SectionHeader：

1. **Section「内置插件」**（背景色略深，上方）
   - 徽章样式：`Builtin`（深蓝 badge，圆角 8px）
   - 禁用按钮：删除/导入/扫描（扫描按钮保留在 SectionHeader 外面，属于全局按钮；但对每行 Builtin 删除按钮 disabled）
   - 热重载按钮 disabled
   - 「+ DSP」按钮 = enable/disable toggle（调用 `toggle_plugin_in_dsp(manifest.name, enabled)`）
   - 「配置」按钮 = 两动作
     a. `onNavigate('dsp')` 切到 DspPanel Tab；
     b. URL hash `#equalizer` / `#smart-effect` / ... 对应锚点（`scrollIntoView({behavior: 'smooth'})`）。
   - 行首展示 `manifest.description` + `config_remarks`（通过 `get_plugin_config(manifest.name)` 拿到 current state 显示为「当前状态：16 段 PEQ / 模式 Parametric」）。

2. **Section「扩展示例插件」**（中部）
   - 徽章样式：`示例`（黄色 badge）
   - 每个示例插件右侧新增「查看源码」超链接：
     - 根据 `file_path` 里的 `plugins/<name>/` 反推 crate 名 → 链接到 `https://github.com/<owner>/Phonon/blob/main/plugins/<crate>/src/lib.rs`（owner 读 `Cargo.toml` [package] repository 字段，默认 phonon）。
   - 删除/热重载按钮 enabled；
   - 「导入」按钮 = 如果状态 Discovered → 调 `load_plugin()`。

3. **Section「已安装扩展插件」**（底部，现状样式）
   - 徽章：无（或灰色 `扩展`）。
   - 完全按现状逻辑运行。

### 4.3 DspPanel → Builtin DSP 徽章

DspPanel 每个 DSP 块（ReplayGain / Downmix / Equalizer / SmartEffect / SurroundSound）的 Header 左侧加一个 16x16 小 badge `Builtin`，或在块右上角用 `position: absolute; top: 8px; right: 8px` 放小灰蓝色 Tag `<span className="badge builtin-dsp">Builtin</span>`。CSS 定义：

```css
.badge.builtin-dsp {
  font-size: 10px; padding: 1px 6px; border-radius: 8px;
  background: rgba(99,102,241,0.12); color: #6366f1; /* indigo */
  border: 1px solid rgba(99,102,241,0.3);
}
```

### 4.4 i18n 新增（`i18n.ts` 中/英两条）

```ts
// 新增 key（中）
'plugins.group.builtin':     '内置插件',
'plugins.group.example':     '扩展示例插件',
'plugins.group.external':    '已安装扩展插件',
'plugins.origin.Builtin':    'Builtin',
'plugins.origin.WasmExample':'示例',
'plugins.origin.WasmExternal':'扩展',
'plugins.staticBuiltin':     '此插件为 Phonon 内置模块，无法删除或热重载。',
'plugins.viewSource':        '查看源码',
'plugins.jumpDspPanel':      '转到 DSP 面板配置',

// 新增 key（英）
'plugins.group.builtin':     'Built-in Plugins',
'plugins.group.example':     'Example Plugins',
'plugins.group.external':    'Installed External Plugins',
'plugins.origin.Builtin':    'Builtin',
'plugins.origin.WasmExample':'Example',
'plugins.origin.WasmExternal':'External',
'plugins.staticBuiltin':     'This is a built-in Phonon module; cannot remove or hot-reload.',
'plugins.viewSource':        'View source',
'plugins.jumpDspPanel':      'Open DSP panel to configure',
```

---

## 5. 错误处理 & 测试策略 & §14 合规联动

### 5.1 错误处理

- **Builtin 操作尝试**：`load_plugin('phonon_equalizer')` → 错误码 `E5xxx`，消息为中文 + 英文双语（`"[E5001] 内置插件无法动态加载 / Built-in plugins are static"`）。
- **Wasm 冲突**：WasmExample 被同名 WasmExternal 覆盖时，`scan_plugins()` 返回 `warnings: [{type: "name-conflict", kept_origin: "WasmExternal", dropped: {name, path}}]`，前端在 PluginManager SectionHeader 上显示黄色 toast。
- **set_plugin_config patch 非法字段**：Equalizer 收到 `{ invalid_key: 123 }` → 静默忽略未识别字段，只处理合法字段；但如果没有任何合法字段，返回 `Err("[E5002] set_plugin_config: no recognized patch keys for 'phonon_equalizer'")`。
- **Wasm emit_event 转发失败**：`host::emit_event` 返回 1 → Wasm 侧通过 `ph_log(WARN, ...)` 记录，不中断 plugin 运行。

### 5.2 测试策略

**单元测试（phonon-plugin crate）**：
1. `test_builtin_plugin_list_count` — `list_plugins()` 的前 5 项 origin 必须都是 Builtin，name 分别是 `phonon_equalizer/smart_effect/surround/downmix/replaygain`，顺序固定。
2. `test_origin_discrimination_example_vs_external` — 伪造 workspace/plugins/ 下 path 和用户目录 path 两个 PluginInfo，断言 origin 被正确标记。
3. `test_name_conflict_user_dir_wins` — 把同名 Example 和 External 同时塞扫描结果，断言最终 list_plugins 只保留 External。
4. `test_set_plugin_config_eq_patch_mode_only` — `set_plugin_config("phonon_equalizer", {mode:"Parametric"})` 不改动 bands 只改 mode。
5. `test_set_plugin_config_eq_patch_bands_count_preamp` — 一次发送带 4 个 key 的 JSON patch，全部被正确应用。
6. `test_toggle_plugin_in_dsp_builtin_calls_enable_dsp` — 断言启用/禁用对应 dsp id。
7. `test_builtin_load_reload_remove_refuse` — 三个操作都返回 PluginError::Static。

**集成测试（commands.rs 层；现有 `cargo test` 目录加文件）**：
8. `test_list_plugins_frontend_shapes` — 调用 `list_plugins()` 返回 Vec<PluginInfo>，每个 info.origin 非 null，Builtin 排序在前。
9. `test_scan_plugins_release_mode_skips_examples` — 在 `#[cfg(not(debug_assertions))]` 场景下，scan_plugins 不扫 workspace/plugins。（测试用 `#[cfg(test)] mod release_tests`）。

### 5.3 与 §14 合规的联动

| §14 项                 | Builtin 行为                                                  | WasmExample / WasmExternal 行为                                |
|------------------------|---------------------------------------------------------------|----------------------------------------------------------------|
| 插件静态规则扫描 P3    | 跳过（内核级，已审计）                                        | `load()` / `reload()` 都走 `static_scan()`；SuspiciousContent 直接拒绝 |
| API 白名单             | N/A（Builtin 不访问元数据/歌词/封面 API）                     | Wasm 如果请求歌词 API，需要用户 UI 开 `lyricsApiEnabled` + 授予 Network permission（双条件） |
| EULA 弹窗              | N/A                                                           | Wasm 首次 Load 时显示 "您授权该插件访问其 manifest 中声明的权限"（已有确认框，沿用不改） |
| 卸载保留数据           | 配置保存在 `session.json` → 不删                              | Wasm 配置按 plugin_name 为 key 写入 `session.json` → 不删；.wasm 用户目录文件保留原样 |
| License 黑名单（deny.toml）| Phonon 工作区 cargo-deny 扫描（CI 已有 license-audit job 全绿） | **修复实现细节**：plugins/* 三个 crate 默认**不在**根 `Cargo.toml [workspace].members` 里，cargo-deny 扫不到它们。实施时二选一：<br>**方案 A（推荐，对构建影响最小）**：在 `deny.toml` 增加 `[workspace] members = true`（已默认 true）+ 对示例子包单独跑一轮 `cargo deny --manifest-path plugins/phonon-simple-source/Cargo.toml check licenses` 三条独立命令，合入 `license-audit` job；<br>**方案 B**：将 3 个 plugins/* 加入 workspace.members，再设置 `[workspace] default-members = [phonon-core, ...]`（排除 plugins/）使 `cargo build` 不因缺 `wasm32-wasi` target 报错。两种方案最终都要 CI 绿灯。 |

---

## 6. 实施计划（粗粒度，写 Spec 之后转 writing-plans skill 再细分）

- Step 1：`types.rs` 加 `PluginOrigin` + `PluginInfo.origin` 字段；新建 `builtin.rs`；`runtime.rs` 改 `list_plugins()` 三段合并与冲突处理。
- Step 2：`load/reload/remove` 加 Builtin 拒绝分支；补单元测试 1-7。
- Step 3：`commands.rs` 加 `plugin_origin()` helper + 3 个新命令 `get_plugin_config / set_plugin_config / toggle_plugin_in_dsp`。
- Step 4：`runtime.rs` 加 `host::emit_event` host import；事件系统补 `PluginUserEvent`（如果不存在则添加）。
- Step 5：前端 `plugins.ts` 加 `PluginOrigin` 类型；PluginManager 做三段分组（SectionHeader + 按钮禁用 + 跳转 + 查看源码链接）。
- Step 6：DspPanel 加 Builtin badge；i18n.ts 补双语；补 EqPanel 锚点 id。
- Step 7：跑 `cargo test -p phonon-plugin` + `tsc --noEmit`；更新 tasks.md §6 / §14。

---

## 7. Spec 自我审查

1. **占位符扫描**：无 TBD/TODO。manifest.description/config_remarks 用了真实文案。
2. **内部一致性**：§2 表格的 Events/权限/扫描三处与 §3.4 / §3.1 / §3.2 实现一一对应，无矛盾。
3. **范围检查**：聚焦在"插件三层分类 + 半深度接入 Builtin DSP"，没越界去动 EqPanel 布局/均衡器算法。
4. **歧义检查**：`manifest.name` 命名规则明确 Builtin 前缀 `phonon_*`，避免与用户 Wasm 撞名。Origin 判定规则："workspace/plugins/ 路径前缀 = Example，其余 External"——明确无歧义。发布模式（release）跳过示例目录用 `#[cfg(not(debug_assertions))]` 控制，可检验。
