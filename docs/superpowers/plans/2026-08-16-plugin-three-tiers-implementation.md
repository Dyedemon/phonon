# Plugin Three-Tier System Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 Phonon 插件系统按"Builtin / WasmExample / WasmExternal"三层分类落地：均衡器等 5 个 DSP 作为 Builtin 插件在 PluginManager 同框出现（统一元数据 + 统一 config/启用命令 remap，保留 Rust 原生 DSP 链），Wasm 两类按用户调整过的规则（权限统一 Manifest、Events 经宿主转发、用户目录优先于示例目录冲突覆盖）运行。

**Architecture:** 方案 B「半深度接入」— 新增 `PluginOrigin` 枚举与 `BuiltinPluginRegistry`（静态元数据层），`list_plugins()` 三段合并+冲突排序；Tauri 层新增 `get/set_plugin_config / toggle_plugin_in_dsp` 三条命令（Builtin remap 到现有 DSP 命令，Wasm 给 JSON）；Wasm 侧新增 `host::emit_event` 宿主回调 import；前端 PluginManager 三段分组（Builtin/示例/外部）+ DspPanel 每个块加 Builtin badge；补单元/集成测试与 i18n。

**Tech Stack:** Rust 2021 (crate types/plugin/core/output) + wasmtime 30 + Tauri v2 (commands/events) + React 18 + TS + i18next (中/英)

---

## File Structure (Lock-in)

| 操作 | 文件 | 职责 |
|------|------|------|
| Modify | `phonon-plugin/src/types.rs` (`PluginInfo`, 新增 `PluginOrigin` 与导出) | 三层 Origin 枚举 + PluginInfo.origin 字段 + `PluginError::Static` 变体（如不存在则补） |
| Create | `phonon-plugin/src/builtin.rs` | BuiltinPlugin 结构体 + `BUILTINS [5]` 静态注册表（5 DSP）+ `pub fn all()` |
| Modify | `phonon-plugin/src/runtime.rs` (`PluginRuntime::scan_plugins`, `::list_plugins`, `::load/reload/remove`) | scan 阶段按 dirs 打 origin；list_plugins 合并冲突（用户目录优先）；Builtin 静态拒绝操作 |
| Modify | `phonon-plugin/src/lib.rs` | 导出 `pub mod builtin` + `pub use builtin::{BuiltinPlugin, all as list_builtins}` |
| Modify | `phonon-tauri/src-tauri/src/events.rs` | 新增 `Event::PluginUserEvent { name, payload: String }` |
| Modify | `phonon-tauri/src-tauri/src/commands.rs` (末尾) | 新增 `plugin_origin()` helper + 3 个命令：`get_plugin_config` / `set_plugin_config` / `toggle_plugin_in_dsp`（Builtin remap；Wasm 直透） |
| Modify | `phonon-tauri/src/api/plugins.ts` | 新增 `PluginOrigin` 类型；`PluginInfo` 加 `origin` 字段；新增 `getPluginConfig / setPluginConfig / togglePluginInDsp` 封装 |
| Modify | `phonon-tauri/src/components/ExtensionsPanel.tsx` → `PluginManager` SubTab 段 | 三段分组渲染（Builtin/示例/外部），禁用删除/热重载按钮，配置跳 DspPanel 锚点，示例查看源码链接 |
| Modify | `phonon-tauri/src/components/DspPanel.tsx` （Equalizer / Smart / Surround / Downmix / ReplayGain 五大块 header） | 每个块右上角加 `<span className="badge builtin-dsp">Builtin</span>`；给每个块补 id 锚点（`id="equalizer"` 等） |
| Modify | `phonon-tauri/src/i18n.ts` (中/英) | 新增 12 条 key（plugins.group.* / plugins.origin.* / plugins.staticBuiltin / plugins.viewSource / plugins.jumpDspPanel） |
| Modify | `.github/workflows/ci.yml` `license-audit` job | 增加示例子包 3 条独立 `cargo deny --manifest-path plugins/.../Cargo.toml check licenses` 步骤（避免 workspace 外 crate 扫不到） |
| Modify | `tasks.md` §6 / §14 | 对应核查项打勾（§6 新增 builtin+origin；§14 新增 example license audit） |
| Create | `phonon-plugin/tests/three_tiers_integration.rs` | 集成测试 8/9（origin 分类 / list_plugins 顺序 / 冲突处理 / Builtin 拒绝操作 remap 调用） |

---

## Task 1: types.rs — 引入 PluginOrigin 与 PluginInfo.origin

**Files:**
- Modify: `phonon-plugin/src/types.rs`

- [ ] **Step 1: 在 types.rs 合适位置（PluginInfo 上方）新增 PluginOrigin 枚举**

```rust
/// 插件来源分类：三层插件系统的核心枚举。
///
/// 与 docs/superpowers/specs/2026-08-16-plugin-three-tiers-design.md §2 一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PluginOrigin {
    /// Phonon 内置模块（Rust 原生 DSP 等）；静态不可 reload/remove。
    Builtin,
    /// 扩展示例插件：随 workspace/plugins/ 子包，不随安装包发布。
    WasmExample,
    /// 独立扩展插件：外部 Git/第三方分发，用户导入 .wasm。
    WasmExternal,
}

impl Default for PluginOrigin {
    /// 默认视为 WasmExternal（所有旧 PluginInfo 在未补 origin 时按外部插件处理）。
    fn default() -> Self {
        PluginOrigin::WasmExternal
    }
}
```

- [ ] **Step 2: 在 PluginInfo 上加 origin 字段（Default 保证向后兼容反序列化）**

```rust
/// Plugin runtime info (combines manifest + state + path + error).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    pub manifest: PluginManifest,
    pub state: PluginState,
    pub file_path: String,
    pub error_message: Option<String>,
    /// 来源分类（见 `PluginOrigin` 文档）。
    #[serde(default)]
    pub origin: PluginOrigin,
}
```

- [ ] **Step 3: 确认 PluginError::Static 变体存在；不存在则补**（用于 Builtin 操作拒绝）

```rust
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    // ... 已有变体
    /// Builtin 插件不允许的操作（load/reload/remove）。
    /// 参数 0 = 插件名；参数 1 = 拒绝原因（人类可读）。
    #[error("[E5001] 内置插件 '{0}' 无法 {1}。Built-in plugins are static and managed by Phonon runtime.")]
    Static(String, String),
    // ... 已有 SuspiciousContent
}
```

- [ ] **Step 4: Run `cargo check -p phonon-plugin` 验证 types 通过**
Run: `cd d:\Phonon ; cargo check -p phonon-plugin 2>&1 | Select-Object -Last 30`
Expected: `Finished dev ... target(s) in ...` — 无 E 级错误。

- [ ] **Step 5: Commit (conventional commit)**
```bash
git add phonon-plugin/src/types.rs
git commit -m "feat(plugin): add PluginOrigin enum + PluginInfo.origin field"
```

---

## Task 2: 新建 builtin.rs — BuiltinPluginRegistry 5 个 DSP

**Files:**
- Create: `phonon-plugin/src/builtin.rs`
- Modify: `phonon-plugin/src/lib.rs`

- [ ] **Step 1: 创建 phonon-plugin/src/builtin.rs，写入完整内容**

```rust
//! Builtin Plugin Registry — Phonon 内置插件（Rust 原生 DSP）静态元数据。
//!
//! 这些"插件"的真实 process 逻辑不经过 wasmtime，而是在 phonon-core 的 DSP 链上
//! 作为原生 DspProcessor 实现。本注册表的职责是为 `list_plugins()` 与前端
//! PluginManager 提供与 Wasm 插件同构的元数据视图（manifest + 配置说明）。
//!
//! 对应 Spec §1.2：当前 5 个 Builtin = Equalizer / SmartEffect / Surround / Downmix / ReplayGain。

use serde::{Deserialize, Serialize};

use crate::types::{PluginManifest, PluginPermission, PluginType};

/// 单个内置插件记录（用于 list_plugins → PluginInfo）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuiltinPlugin {
    /// DSP 命令层的 `enable_dsp(id, enabled)` 标识。
    pub id: &'static str,
    /// 插件 manifest（name 一律使用 `phonon_*` 保留前缀，避免与第三方撞名）。
    pub manifest: PluginManifest,
    /// 在 PluginManager 行尾展示的配置说明文案。
    pub config_remarks: &'static str,
}

/// 返回全部内置插件数组（固定顺序：Equalizer → SmartEffect → Surround → Downmix → ReplayGain）。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_count_is_5() {
        assert_eq!(all().len(), 5);
    }

    #[test]
    fn builtin_order_matches_spec() {
        // 顺序按 Spec 固定：Equalizer / SmartEffect / Surround / Downmix / ReplayGain
        let ids: Vec<&str> = all().iter().map(|b| b.id).collect();
        assert_eq!(ids, vec!["equalizer", "smart_effect", "surround_sound", "downmix", "replaygain"]);
    }

    #[test]
    fn builtin_names_use_phonon_prefix_reserved() {
        // 命名保留前缀：phonon_*，永远不与 WasmExample/WasmExternal 撞。
        for b in all() {
            assert!(
                b.manifest.name.starts_with("phonon_"),
                "builtin '{}' 必须使用 'phonon_' 前缀",
                b.manifest.name
            );
        }
    }

    #[test]
    fn builtin_permissions_always_empty() {
        // Builtin 永远不走 Wasm 授权流程；permissions 字段必须空。
        for b in all() {
            assert!(
                b.manifest.permissions.is_empty(),
                "builtin '{}' permissions 必须为空",
                b.manifest.name
            );
        }
    }

    #[test]
    fn builtin_all_plugin_types_dsp() {
        // 当前 5 个都是 DSP Processor；如果以后加 SourceProvider/Visualizer 在这里扩展。
        for b in all() {
            assert_eq!(b.manifest.plugin_type, PluginType::DspProcessor);
        }
    }
}
```

- [ ] **Step 2: 在 phonon-plugin/src/lib.rs 导出 builtin 模块**

```rust
pub mod builtin;
```

- [ ] **Step 3: 运行 builtin 单元测试（5 条）**
Run: `cd d:\Phonon ; cargo test -p phonon-plugin --lib builtin::tests 2>&1 | Select-Object -Last 40`
Expected: `test result: ok. 5 passed; 0 failed`

- [ ] **Step 4: `cargo check -p phonon-plugin` 确保整体无错**
Run: `cd d:\Phonon ; cargo check -p phonon-plugin 2>&1 | Select-Object -Last 20`
Expected: `Finished dev ...`

- [ ] **Step 5: Commit**
```bash
git add phonon-plugin/src/builtin.rs phonon-plugin/src/lib.rs
git commit -m "feat(plugin): add BuiltinPluginRegistry (5 DSPs) with unit tests"
```

---

## Task 3: runtime.rs — PluginRuntime::scan_plugins 打 origin + list_plugins 合并冲突排序

**Files:**
- Modify: `phonon-plugin/src/runtime.rs`
- Test: `phonon-plugin/src/runtime.rs`（在末尾 `#[cfg(test)]` 模块追加）

- [ ] **Step 3a: 在 scan_plugins() 方法内，按扫描来源目录标记 PluginInfo.origin**
实现时新增两个辅助函数（建议放在 `impl PluginRuntime` 之前作为 `fn`）：

```rust
/// 返回用户 plugins 目录（平台 user data）。
/// Windows: %APPDATA%\Phonon\plugins | macOS: ~/Library/Application Support/Phonon/plugins | Linux: ~/.config/phonon/plugins
fn user_plugins_dir() -> std::path::PathBuf {
    // 复用 Tauri 逻辑：优先 dirs::data_dir()/Phonon/plugins；回退 CARGO_MANIFEST_DIR/../../../.phonon_data/plugins（测试环境用）
    if let Some(d) = dirs::data_dir() {
        let p = d.join("Phonon").join("plugins");
        let _ = std::fs::create_dir_all(&p);
        return p;
    }
    let fallback = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".phonon_data")
        .join("plugins");
    let _ = std::fs::create_dir_all(&fallback);
    fallback
}

/// 开发模式下返回 workspace/plugins 目录的候选构建产物路径（target/wasm32-wasi/release/）。
#[cfg(debug_assertions)]
fn example_wasm_product_dirs() -> Vec<std::path::PathBuf> {
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")) // phonon-plugin
        .parent().unwrap().to_path_buf();
    vec![
        workspace_root.join("target").join("wasm32-wasi").join("release"),
        workspace_root.join("target").join("wasm32-wasi").join("debug"),
    ]
}

/// 如果 manifest.name 在示例子包白名单里返回 true。
fn is_example_name(name: &str) -> bool {
    matches!(name, "simple_source" | "gain_processor" | "phase_vocoder_stretcher")
}
```

在 `scan_plugins()` 扫描到每个 .wasm 时：
- 如果文件在 `user_plugins_dir()` 下 → `.origin = WasmExternal`
- 否则如果 `cfg(debug_assertions)` + 在 `example_wasm_product_dirs()` 下 + `is_example_name(&manifest.name)` → `.origin = WasmExample`
- 其他回退 → `.origin = WasmExternal`（`#[serde(default)]` 已是）

- [ ] **Step 3b: 重写 `list_plugins()` 为「冲突合并排序」版本（见 Spec §3.2 代码块）**

```rust
pub fn list_plugins(&self) -> Vec<PluginInfo> {
    use crate::builtin;
    use crate::types::PluginOrigin;

    let wasm_plugins = self.plugins.lock().unwrap().clone();
    static EXAMPLE_NAMES: &[&str] = &["simple_source", "gain_processor", "phase_vocoder_stretcher"];

    let mut result: Vec<PluginInfo> = Vec::new();
    let mut external_names = std::collections::HashSet::new();

    // Pass 1: WasmExternal 先压（用户目录优先；命中集合）
    for info in wasm_plugins.clone() {
        if matches!(info.origin, PluginOrigin::WasmExternal) {
            external_names.insert(info.manifest.name.clone());
            result.push(info);
        }
    }
    // Pass 2: WasmExample 后压，白名单命中且未被 External 覆盖才留
    for info in wasm_plugins {
        if matches!(info.origin, PluginOrigin::WasmExample)
            && EXAMPLE_NAMES.contains(&info.manifest.name.as_str())
            && !external_names.contains(&info.manifest.name)
        {
            result.push(info);
        }
    }

    // Pass 3: prepend Builtin
    let mut builtins: Vec<PluginInfo> = builtin::all().iter().map(|b| PluginInfo {
        manifest: b.manifest.clone(),
        state: crate::types::PluginState::Running,
        file_path: format!("builtin:{id}", id = b.id),
        error_message: None,
        origin: PluginOrigin::Builtin,
    }).collect();

    builtins.append(&mut result);
    builtins
}
```

- [ ] **Step 3c: 在 load() / reload() / remove_plugin() 顶部加 Builtin 拒绝守卫**

三个函数入口都加：

```rust
// 如果 manifest.name 命中 Builtin 前缀名 + origin=Builtin → 拒绝
use crate::builtin;
if builtin::all().iter().any(|b| b.manifest.name == file_path_or_plugin_name) {
    return Err(crate::types::PluginError::Static(
        file_path_or_plugin_name.to_string(),
        "be loaded dynamically (call load_plugin/reload_plugin/remove_plugin)".into(),
    ));
}
```

- [ ] **Step 3d: 在 runtime.rs 的末尾 tests 模块追加 6 条单元测试**

```rust
#[cfg(test)]
mod three_tiers_tests {
    use super::*;

    fn fake_info(name: &str, origin: PluginOrigin) -> PluginInfo {
        PluginInfo {
            manifest: PluginManifest {
                name: name.into(), version: "0.1.0".into(), author: "test".into(),
                description: String::new(), plugin_type: PluginType::DspProcessor,
                permissions: vec![], min_framework_version: "0.1.0".into(),
            },
            state: PluginState::Discovered,
            file_path: format!("/tmp/{name}.wasm"),
            error_message: None,
            origin,
        }
    }

    #[test]
    fn list_plugins_prepends_5_builtins_sorted() {
        let rt = PluginRuntime::new(Default::default()).unwrap();
        let list = rt.list_plugins();
        assert!(list.len() >= 5, "至少有 5 个 builtin");
        for (i, expect_id) in ["equalizer","smart_effect","surround_sound","downmix","replaygain"].iter().enumerate() {
            assert_eq!(list[i].origin, PluginOrigin::Builtin);
            let expected_name = format!("phonon_{expect_id}");
            assert_eq!(list[i].manifest.name, expected_name);
        }
    }

    #[test]
    fn external_then_example_when_no_conflict() {
        let rt = PluginRuntime::new(Default::default()).unwrap();
        // 手工塞 1 个 External + 1 个 Example 入 runtime.plugins（私有字段；通过 scan_plugins 调用间接验证：此 test 改为验证合并逻辑函数）
        // 简化：直接构造集合 + 走合并辅助 fn
        unimplemented!() // 实现时把 list_plugins 的合并逻辑抽成纯函数，便于单元测试
    }

    #[test]
    fn name_conflict_user_external_wins_over_example() {
        unimplemented!()
    }

    #[test]
    fn load_builtin_refused_with_static_error() {
        let rt = PluginRuntime::new(Default::default()).unwrap();
        let res = rt.load("phonon_equalizer"); // Builtin 名
        assert!(matches!(res, Err(PluginError::Static(_, _))));
        let msg = res.unwrap_err().to_string();
        assert!(msg.contains("E5001"), "错误码 E5001 未出现：{}", msg);
    }

    #[test]
    fn reload_builtin_refused() {
        unimplemented!()
    }

    #[test]
    fn remove_builtin_refused() {
        unimplemented!()
    }
}
```

- [ ] **Step 3e: Run tests**
Run: `cd d:\Phonon ; cargo test -p phonon-plugin three_tiers 2>&1 | Select-Object -Last 30`
Expected: 至少 load_builtin_refused 1 passed；实现纯函数合并后其余 5 条也 passed。

- [ ] **Step 3f: Commit**
```bash
git add phonon-plugin/src/runtime.rs
git commit -m "feat(plugin): list_plugins 3-tier merge + scan origin tagging + builtin refused"
```

---

## Task 4: 事件系统新增 PluginUserEvent + runtime 里 host::emit_event

**Files:**
- Modify: `phonon-tauri/src-tauri/src/events.rs`（新增事件变体 + serialize 为 tauri event 名）
- Modify: `phonon-plugin/src/runtime.rs`（linker_init 中增加 `host.emit_event` host import）

- [ ] **Step 4a: 在 events.rs 的 Event 枚举上加 PluginUserEvent 变体**

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum Event {
    // ... 已有变体：PlaybackStateChanged / SpectrumData / DeviceHotplug /
    //              PluginLoaded / VisPluginStarted / SessionLoaded ...
    /// Wasm 插件发起的自定义事件（经宿主 host.emit_event 转发）。
    /// name = 事件名（插件侧自定义，如 "gain-changed"）；payload = 插件传的 JSON 字符串。
    PluginUserEvent { name: String, payload: String },
}
```

并在同文件「事件→ Tauri event 名称映射」处追加：
```rust
pub fn event_topic(e: &Event) -> &'static str {
    match e {
        // ... 已有 match 分支
        Event::PluginUserEvent { .. } => "plugin-user-event",
    }
}
```

- [ ] **Step 4b: runtime.rs 的 linker_init 加 host import**

```rust
// host::emit_event(name_ptr: i32, name_len: i32, payload_ptr: i32, payload_len: i32) -> i32
//   返回 0 = ok，1 = EVENT_TX 未初始化，2 = UTF-8/内存读失败
linker.func_wrap(
    "host", "emit_event",
    |mut caller: Caller<'_, HostState>,
     name_ptr: i32, name_len: i32,
     payload_ptr: i32, payload_len: i32| -> Result<i32> {
        let memory = match caller.get_export("memory") {
            Some(Extern::Memory(m)) => m,
            _ => return Ok(2),
        };
        let read_str = |ptr: i32, len: i32| -> Option<String> {
            if ptr < 0 || len < 0 { return None; }
            let mut buf = vec![0u8; len as usize];
            memory.read(&caller, ptr as usize, &mut buf).ok()?;
            String::from_utf8(buf).ok()
        };
        let name    = read_str(name_ptr,    name_len)   .unwrap_or_default();
        let payload = read_str(payload_ptr, payload_len).unwrap_or_default();

        // 经宿主回调转发至 EVENT_TX（Spec §2 Events 行；Builtin 直连）。
        let tx_guard = EVENT_TX.read().unwrap();
        if let Some(tx) = tx_guard.as_ref() {
            let _ = tx.send(Event::PluginUserEvent { name, payload });
            Ok(0)
        } else {
            Ok(1)
        }
    }
)?;
```

- [ ] **Step 4c: Test build**
Run: `cd d:\Phonon ; cargo check -p phonon-plugin -p phonon-tauri-src-tauri 2>&1 | Select-Object -Last 30`
Expected: `Finished` 无错误。（ phonon-tauri-src-tauri 用 phonon-tauri/src-tauri 的包名，看 `Cargo.toml` 里实际 name）

- [ ] **Step 4d: Commit**
```bash
git add phonon-tauri/src-tauri/src/events.rs phonon-plugin/src/runtime.rs
git commit -m "feat(events): add PluginUserEvent + Wasm host.emit_event forward"
```

---

## Task 5: commands.rs 新增 plugin_origin / get_plugin_config / set_plugin_config / toggle_plugin_in_dsp

**Files:**
- Modify: `phonon-tauri/src-tauri/src/commands.rs`（末尾新增 4 项）

- [ ] **Step 5a: 新增 plugin_origin 私有 helper**

```rust
/// 根据 manifest.name 查询 PluginOrigin（Builtin 查表；否则从 plugin_runtime PluginInfo.origin 拿）。
fn plugin_origin(rt: &PluginRuntime, name: &str) -> PluginOrigin {
    use phonon_plugin::builtin;
    use phonon_plugin::PluginOrigin;
    if builtin::all().iter().any(|b| b.manifest.name == name) {
        return PluginOrigin::Builtin;
    }
    rt.list_plugins().iter()
        .find(|p| p.manifest.name == name)
        .map(|p| p.origin)
        .unwrap_or(PluginOrigin::WasmExternal)
}
```

- [ ] **Step 5b: 新增 get_plugin_config 命令**（Spec §3.3 代码块；提取现有命令的 `*_inner` 公共私有函数）

- [ ] **Step 5c: 新增 set_plugin_config 命令**（Equalizer 四字段 patch：mode/bands/geq_band_count/preamp_db）

- [ ] **Step 5d: 新增 toggle_plugin_in_dsp 命令**（Builtin enable_dsp；Wasm add/remove_wasm_dsp）

- [ ] **Step 5e: 注册 3 条命令到 lib.rs `#[tauri::generate_handler![]]` 宏**
（在现有命令数组后面追加 `get_plugin_config, set_plugin_config, toggle_plugin_in_dsp`）

- [ ] **Step 5f: `cargo check -p phonon-tauri-src-tauri`**
Expected: `Finished`。

- [ ] **Step 5g: Commit**
```bash
git add phonon-tauri/src-tauri/src/commands.rs phonon-tauri/src-tauri/src/lib.rs
git commit -m "feat(commands): unified plugin config + dsp toggle (Builtin remap + Wasm passthrough)"
```

---

## Task 6: 前端 api/plugins.ts + App.tsx listener + ExtensionsPanel PluginManager 三段分组

**Files:**
- Modify: `phonon-tauri/src/api/plugins.ts`（类型 + 3 新 invoke 封装）
- Modify: `phonon-tauri/src/App.tsx`（监听 plugin-user-event 转应用状态总线 toasts/自定义处理）
- Modify: `phonon-tauri/src/components/ExtensionsPanel.tsx`（PluginManager SubTab 三段分组 + 徽章 + 按钮禁用策略 + 查看源码链接 + 跳转锚点）

- [ ] **Step 6a: api/plugins.ts 新增类型与封装**

```ts
export type PluginOrigin = 'Builtin' | 'WasmExample' | 'WasmExternal';

export interface PluginInfo {
  // ... 现有的 manifest/state/file_path/error_message
  origin: PluginOrigin;
}

export async function getPluginConfig(name: string): Promise<any> {
  return invoke<any>('get_plugin_config', { name });
}
export async function setPluginConfig(name: string, value: any): Promise<void> {
  return invoke<void>('set_plugin_config', { name, value });
}
export async function togglePluginInDsp(name: string, enabled: boolean): Promise<void> {
  return invoke<void>('toggle_plugin_in_dsp', { name, enabled });
}
```

- [ ] **Step 6b: App.tsx 监听 plugin-user-event**

```tsx
useEffect(() => {
  const unlisten = listen<{ name: string; payload: string }>(
    'plugin-user-event',
    (ev) => {
      console.debug('[plugin-user-event]', ev.payload.name, ev.payload.payload);
      // 示例：内置转 toast 钩子；外部按需要接入 lyrics / vis 总线
    }
  );
  return () => { unlisten.then(f => f()); };
}, []);
```

- [ ] **Step 6c: ExtensionsPanel → PluginManager SubTab 三段分组实现**

（按 Spec §4.2：Builtin 顶部 → 示例中部 → 外部底部；SectionHeader + 禁用按钮 + 跳转 DspPanel 锚点 + 示例源码链接）

- [ ] **Step 6d: Run `tsc --noEmit`**
Run: `cd d:\Phonon\phonon-tauri ; npx tsc --noEmit 2>&1 | Select-Object -Last 40`
Expected: 无 TS 错误。

- [ ] **Step 6e: Commit**
```bash
git add phonon-tauri/src/api/plugins.ts phonon-tauri/src/App.tsx phonon-tauri/src/components/ExtensionsPanel.tsx
git commit -m "feat(ui): PluginManager 3-tier grouping + unified api + event listener"
```

---

## Task 7: DspPanel 五大块 Builtin badge + 锚点 id + i18n 12 条双语

**Files:**
- Modify: `phonon-tauri/src/components/DspPanel.tsx`（五大块 Header + 锚点 id）
- Modify: `phonon-tauri/src/i18n.ts`（zh/en 各 12 条 key）

- [ ] **Step 7a: DspPanel 每个 DSP 块 Header 左上角或右上角加 badge + id 锚点**

五大块的 id 依次为：`id="equalizer"`, `id="smart-effect"`, `id="surround-sound"`, `id="downmix"`, `id="replaygain"`。

badge CSS class `badge builtin-dsp` 已在 Spec §4.3 给出：

```css
.badge {
  display: inline-flex; align-items: center;
  font-weight: 600; letter-spacing: 0.02em;
  border-radius: 999px; user-select: none;
}
.badge.builtin-dsp {
  font-size: 10px; padding: 1px 6px; border-radius: 8px;
  background: rgba(99,102,241,0.12); color: #6366f1;
  border: 1px solid rgba(99,102,241,0.3);
}
```

- [ ] **Step 7b: i18n.ts 中/英追加 12 条 key**（Spec §4.4 代码块原样写入）

- [ ] **Step 7c: tsc --noEmit**
Expected: 通过。

- [ ] **Step 7d: Commit**
```bash
git add phonon-tauri/src/components/DspPanel.tsx phonon-tauri/src/i18n.ts
git commit -m "feat(ui): DspPanel Builtin DSP badges + anchor ids + i18n 12 keys"
```

---

## Task 8: CI license-audit job 追加示例子包 3 条扫描 + tasks.md 回写

**Files:**
- Modify: `.github/workflows/ci.yml` `license-audit` job 末尾
- Modify: `tasks.md` §6 / §14

- [ ] **Step 8a: ci.yml 追加 3 条示例插件独立 license 扫描（Spec §5.3 修复方案 A：推荐）**

```yaml
      - name: Check licenses for phonon-simple-source (example)
        run: cargo deny --manifest-path plugins/phonon-simple-source/Cargo.toml check licenses

      - name: Check licenses for phonon-gain-processor (example)
        run: cargo deny --manifest-path plugins/phonon-gain-processor/Cargo.toml check licenses

      - name: Check licenses for phonon-phase-vocoder-stretcher (example)
        run: cargo deny --manifest-path plugins/phonon-phase-vocoder-stretcher/Cargo.toml check licenses
```

- [ ] **Step 8b: tasks.md §6 末尾增加 2 个勾：**
```
- [✓] PluginOrigin 三层分类 (Builtin / WasmExample / WasmExternal) + 统一 PluginInfo.origin 字段
- [✓] BuiltinPluginRegistry（5 DSP：Equalizer/Smart/Surround/Downmix/ReplayGain）+ list_plugins 合并冲突排序
```
§14 末尾补 1 个勾：
```
- [✓] CI license-audit 覆盖示例子包（plugins/phonon-simple-source / gain-processor / phase-vocoder-stretcher 三条独立 cargo deny check licenses）
```

- [ ] **Step 8c: Commit**
```bash
git add .github/workflows/ci.yml tasks.md
git commit -m "chore(ci,tasks): example plugin license audit + tasks §6/§14 backwrite"
```

---

## Task 9: 集成测试 phonon-plugin/tests/three_tiers_integration.rs（7 条）

**Files:**
- Create: `phonon-plugin/tests/three_tiers_integration.rs`

7 条用例（与 Spec §5.2 8/9 对应，1 条已在 unit tests 覆盖）：

```rust
// integration test snippets here:
// 1. test_list_plugins_frontend_shapes → list_plugins() 返回 Vec<PluginInfo>，origin 均非 null，Builtin 排序在前
// 2. test_scan_plugins_release_mode_skips_examples → cfg(not debug) 时 example 不出现在 list_plugins（通过环境变量模拟或内部 cfg(test)）
// 3. test_origin_discrimination_workspace_vs_userdir → 两个假 file_path 打 origin 正确
// 4. test_set_plugin_config_eq_mode_only → 发 patch {"mode":"Parametric"} 不带 bands，查 get_eq_state.mode 变了，bands 数量仍是 10
// 5. test_set_plugin_config_eq_full_patch → 同时发 4 字段，全部正确应用
// 6. test_toggle_plugin_in_dsp_builtin_enable_disable → 先关再开，查 DSP chain list_processors[].enabled 翻转
// 7. test_builtin_load_reload_remove_all_refuse → 三操作均 Err(E5001)
```

Run: `cd d:\Phonon ; cargo test -p phonon-plugin --test three_tiers_integration 2>&1 | Select-Object -Last 30`
Expected: 全部 passed。

Commit:
```bash
git add phonon-plugin/tests/three_tiers_integration.rs
git commit -m "test(plugin): 3-tier plugin integration tests (7 cases)"
```

---

## Task 10: 最终全量校验

- [ ] **Step 10a: `cargo test -p phonon-plugin` — 所有 unit + integration tests 通过**
- [ ] **Step 10b: `cargo check --workspace` 全工作区无 E**
- [ ] **Step 10c: `cd phonon-tauri ; npx tsc --noEmit` 前端 TS 无错**
- [ ] **Step 10d: `cd phonon-tauri ; npx vite build` 前端打包成功**
- [ ] **Step 10e: 本地验证：`cargo tauri dev` 启动后手动观察 ExtensionsPanel → PluginManager 三段分组 + DspPanel 五大块 Builtin badge**

Commit summary commit (可选)：
```bash
git commit --allow-empty -m "release(plugin): three-tier system — Builtin/Example/External 全量落地"
```

---

## Plan Self-Review（跑完后执行）

**1. Spec coverage 检查：**
- §1.2 5 Builtin + 3 Example 清单 → Task 2 builtin.rs + Task 3 EXAMPLE_NAMES ✓
- §2 表格权限 → Builtin permissions 空（Task 2 测试）；Wasm Manifest.permissions（Task 1 PluginManifest 保持现状）✓
- §2 Events → Task 4a PluginUserEvent + Task 4b host::emit_event ✓
- §2 扫描/冲突 → Task 3a scan 打 origin + Task 3b list_plugins Pass1/2 ✓
- §3.1 types.rs → Task 1 ✓
- §3.2 runtime list_plugins + load/reload/remove refuse → Task 3b/c + unit tests ✓
- §3.3 commands 3 条 + plugin_origin → Task 5 a-e ✓
- §3.4 events + emit_event → Task 4 ✓
- §4.1 types.ts origin → Task 6a ✓
- §4.2 PluginManager 三段分组 → Task 6c ✓
- §4.3 DspPanel badge + 锚点 → Task 7a ✓
- §4.4 i18n 12 条 → Task 7b ✓
- §5.1 错误码 E5001/E5002 → Task 1 Static + Task 5 set_plugin_config 校验 ✓
- §5.2 单元/集成测试 → Task 2（5 builtin ut）+ Task 3d（6 条 three_tiers ut）+ Task 9（7 条 integration）✓
- §5.3 License WasmExample → Task 8a 三条独立 cargo deny ✓

**2. Placeholder scan：**
- Task 3d/9 有 `unimplemented!()` 占位符 — 执行时必须替换为真实代码（这是 plan 里允许的「步骤提示占位符」，不是 spec 级占位符）。

**3. Type consistency：**
- 所有 Tauri 命令名：get_plugin_config / set_plugin_config / toggle_plugin_in_dsp → api/plugins.ts 同名 ✓
- 前端事件名：plugin-user-event → events.rs event_topic 同名 ✓
- DSP id 到 manifest.name 的映射：plugin_origin 查表 builtin::all().manifest.name == phonon_equalizer 等 → toggle_plugin_in_dsp strip_prefix "phonon_" 对齐 Task 5d ✓
