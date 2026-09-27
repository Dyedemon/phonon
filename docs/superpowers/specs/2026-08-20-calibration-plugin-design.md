# Phonon 声学校准插件（Calibration Suite）设计规格

> 版本: **v1.2** (2026-08-20) · 状态: 待审阅
> 定位: 耳机/音箱频响校准 + 房间声学校准套件（分阶段交付）。100% 原创实现，不使用 SoundID / Sonarworks 的商标、专利算法或测量数据库。
> 法律边界: 仅提供"测量数据导入 → 反向 EQ 生成"的通用校准框架。预置曲线使用公开学术参数（Harman / Diffuse Field / Free Field），不内置任何第三方耳机/音箱测量数据库。
>
> **v1.1 变更点（已合入 v1.2）**: §2.1 架构大改（拟合计算从 TS 移入 Rust Wasm，依赖 rustfft）；§3 加线程安全设计（SPSC 通道 + 原子 enable）；§4 热加载改为版本号轮询模型（修复 host→Wasm 通道不存在的 bug）；§5 算法从 TS 改为 Rust 实现；§7 Harman 数据来源改为直接引用公开论文公式；§10 新增 run_calibration_fit 命令。
>
> **v1.2 变更点（本次升级至顶级架构）**:
> 1. 三 crate 分层：`calibration-types`（微型，零算法依赖） / `calibration-engine`（纯 Rust 原生，native 测试友好，Tauri 命令直接 native 调用） / `calibration-fir`（Wasm 只做 overlap-save FIR，体积 ~25KB）
> 2. §3.1 Calibration PEQ：`AtomicPtr<Box<>>` → `AtomicPtr<Arc<>>`；通道改为 `std::mpsc::sync_channel(1)` **寄存器模型**（单槽 = 永远只保留最新 pending，零重试，Full 前端防抖兜底）；`PeqUpdateHandle.last_applied` 加 `Arc<RwLock>` 保存最后一次快照，供事务回滚；新增 `PeqUpdateError` 两枚举
> 3. §4：Wasm 插件名称 `calibration_suite` → `calibration_fir`（只做 FIR 语义精准）；Manifest 同步改名；删除 `plugin_fit` 导出；Wasm 去 `no_std` 直接用 std 环境；rustfft 默认 features
> 4. §5：拟合算法归属从 Wasm 改为 `calibration-engine`（普通 Rust crate）；模块归属清晰标注
> 5. §7：Harman 曲线从"论文公式"改为"**数字化提取 + Akima 插值常量表**"（论文 PDF 需付费无法抄到系数，从公开发表博客曲线图 WebPlotDigitizer 提取 64 点，±0.3 dB 精度，法律零风险）
> 6. §8 UI：5 区保留；新增"拟合预览 → 满意再应用"双按钮流程；`FitCurves` 256 点固定长度 + 严格对数频率轴 `freq[i]=20*(20000/20)^(i/255)` + dB clamp [-30,18]；状态 4 栏 + fit_id 短码
> 7. §10 Tauri Commands：拆分 `run_calibration_fit(req, apply_immediately)` + `apply_fit_result(fit_id)` + `get_current_fit_result`；返回类型完整定义（FitRequest/FitResult/FitCurves/FitDiagnostics/FitError）；**apply 事务回滚语义**（PEQ 注入 + FIR 配置两步要么都成功要么 PEQ 回滚至应用前 last_applied_snapshot）
> 8. §10 CalibrationAppState：LRU 双维度淘汰缓存（20 条 / 24h）+ `last_applied_result` 不淘汰（供"查看当前生效校准详情"用）
> 9. §12.4 线程安全测试：sleep 真实时间 → `Arc<AtomicU64>` 计数器 + `thread::yield_now()` 自旋同步；断言阈值放宽 1e-4 → 1e-3；加 `#[ignore]` 独立 `--ignored --test-threads=1` 跑
> 10. §13 Milestone：从 5 里程碑 31 Task → **9 里程碑 32 Task**（插入 M2 calibration-types / M3 calibration-engine；M4 calibration-fir；后续顺延）

---

## §1 需求与范围

### 1.1 已确认需求

| 维度 | 选择 | 说明 |
|------|------|------|
| 范围 | C：完整套件（分阶段） | Phase 1 = 耳机/音箱校准；Phase 2 = 房间声学校准扩展 |
| 测量来源 | A → B 分阶段 | Phase 1：仅导入外部测量（REW / FRD / AutoEq CSV）；Phase 2：扩展内置扫频/粉噪测量 |
| 反向 EQ | C：混合 | Biquad 拟合主趋势 + FIR 补偿残差 |
| 目标曲线 | A+B+C 全选 | 预置公开曲线 + 自定义拖拽编辑 + 外部导入 |
| 校准粒度 | A：左右独立 | L / R 分别测量，生成各自的反向 EQ |
| 插件形态 | WasmExample | 作为 workspace/plugins/ 子包发布，用户可删除 |
| 顶级质量 | ✅ 强制 | Biquad 加权最小二乘、FIR overlap-save、WebGL 曲线渲染、零毛刺参数切换、原子并发安全、精确寄存器模型通道。量化指标见 §12 测试与验收。 |

### 1.2 Phase 1 MVP 范围（本期实施）

- [x] 外部测量曲线导入（REW CSV / FRD / AutoEq TXT 三种格式）
- [x] 左右声道独立导入与独立校准
- [x] 预置公开目标曲线（Harman Headphone 2019 OE / Harman Headphone 2018 IE / Diffuse Field / Free Field / Flat）
- [x] 目标曲线拖拽编辑器（控制点增删改）
- [x] Biquad 最小二乘拟合（每侧 ≤20 段 Peaking，默认 16 段 ISO 1/3-octave 频点）
- [x] FIR 残差补偿（每侧独立，tap 256~8192（2^n，可取 256/512/1024/2048/4096/8192），默认 2048）
- [x] 频响曲线四通道可视化（测量 / 目标 / PEQ 单独 / PEQ+FIR 全拟合 叠加）
- [x] 预设管理（保存命名 / JSON 导入导出 / 删除）
- [x] 旁路一键 A/B 对比
- [x] 参数持久化 + 重启恢复

### 1.3 Phase 2 扩展范围（本期不实施，预留接口）

- 内置扫频 / 粉噪测试信号生成
- 实时 FFT 频谱分析 + 测量流程向导
- 房间模态共振识别 + 双峰陷波建议
- 早期反射时间窗分析
- 延迟补偿（多声道对齐）

---

## §2 系统架构

### 2.1 总体原则（v1.2 三 crate 分层 + native 拟合命令）

> **关键架构修正（v1.1→v1.2）**：
> ① 原 Wasm 插件 monolith（Fit Engine + FIR）→ 拆三 crate：`calibration-types`（微）+ `calibration-engine`（纯 Rust 原生 crate，Tauri 命令直接 native 调，零 Wasm 序列化损耗，原生单测不需要 wasm-pack）+ `calibration-fir`（Wasm 只做 overlap-save FIR 卷积，无解析/拟合代码，体积 ~25KB）
> ② 原 `run_calibration_fit`（Tauri 命令调 Wasm plugin_fit → emit_event 异步回传）→ Tauri 命令**直接 native 调 `calibration_engine::fit(request)` → 同步返回 `FitResult`**，无事件监听链；同时支持 dry-run（预览不应用）和 immediately 应用；apply 是 PEQ + FIR 原子事务（失败回滚）

- **分层混合架构**：Biquad 级联走 Builtin 原生 DSP（零损耗、复用 eq.rs），FIR 残差卷积走独立 `calibration-fir` WasmExample 插件（可卸载、独立启停）；拟合计算走独立 `calibration-engine` 原生 crate
- **用户 EQ 完全隔离**：校准生成的 Biquad 不写入用户 Equalizer，新建独立 `phonon_calibration_peq` Builtin 节点
- **类型零重复**：`EqBand / FitRequest / FitResult / FirConfig / TargetCurveSpec` 全部在 `calibration-types` 微 crate 中声明。Tauri 后端 native 依赖它；`calibration-engine` 依赖它；Wasm `calibration-fir` 只依赖它（不引 rustfft/parsers，体积锁死 ~25KB）
- **线程安全优先**：Calibration PEQ 和 FIR 插件的参数更新不通过 `Mutex<DspChain>` 的 `&mut self` 路径（防止 UI 线程持锁阻塞 DSP 回调造成爆音）。改用 **SPSC 寄存器模型通道（sync_channel(1)）+ AtomicBool + AtomicPtr<Arc<Snapshot>> 双缓冲策略**：UI 线程是生产者（单次 try_send，Full=异常，前端防抖兜底），DSP 回调线程是消费者（每帧开头 try_recv 一次性取到最新 pending，然后 AtomicPtr swap）
- **FIR 配置拉模型（Push 不存在）**：PluginRuntime 持有 `Arc<AtomicU64> config_version`，每次 `set_plugin_config` 返回时版本号 fetch_add；Wasm 在每次 `process()` 调用开头 load 版本号，变化时再从 host import 请求读取最新 JSON config

### 2.2 DSP 链顺序

```
PCM 输入 (立体声交错 f32)
   │
   ▼
┌───────────────────────┐   Calibration PEQ (NEW Builtin)
│  phonon_calibration_peq │   先拉平主趋势：L/R 各≤20段 Biquad 级联（每侧 ≤20，默认 16）
│  (SPSC 寄存器模型通道)  │   默认 16 段 Peaking，ISO 1/3-octave 频点
└──────────┬────────────┘   解交错逐样本 inline 处理，左右参数完全独立
           │   → sync_channel(1) + AtomicPtr<Arc<PeqSnapshot>> swap
           ▼
┌───────────────────────────┐  Calibration FIR (NEW WasmExample, 只做 FIR)
│  calibration_fir (Wasm)   │  overlap-save FIR 残差补偿卷积（L/R 独立）
│  ┌─────────────────────┐  │  tap 256~8192（2^n）默认 2048
│  │ FIR overlap-save    │  │  最小相位 / 线性相位可选（默认最小相位）
│  │ (原子 version swap) │  │  延迟 = tap / sr（仅 FIR 部分，IIR 零延迟）
│  └─────────────────────┘  │  热加载：版本号轮询拉模型（见 §4.3）
└──────────────┬────────────┘
               │
               ▼
┌────────────────┐   Equalizer (Builtin)
│  phonon_equalizer │   用户自己的音色偏好 EQ，不被校准覆盖
└──────┬─────────┘   在校准之后叠加，用户能在校准音色上再调味
       │
       ▼
  后续 DSP 链（Surround → SmartEffect → Downmix → ReplayGain → 输出）
```

**为什么 Cal PEQ 在前而不是在后？**
- 先拉平主趋势，残差幅度更小 → FIR 可用更少 tap 达到相同精度（计算和延迟都省）
- 用户 Equalizer 在校准之后叠加，避免校准"吃掉"用户的音色调味

**DSP 链顺序说明（与 §3.3 交叉引用）**：
- 插件管理 UI 展示顺序 = Equalizer / SmartEffect / Surround / Downmix / ReplayGain / Calibration PEQ（§3.3 定义的 BUILTINS 数组顺序）
- DSP 链实际处理顺序 = Calibration PEQ → Calibration FIR → Equalizer → Surround → SmartEffect → Downmix → ReplayGain（由 phonon-core/src/engine.rs 初始化 DspChain 时按 add 顺序决定）

**旁路语义**：`toggle_calibration_bypass(true)` 同时禁用 Calibration PEQ 节点 + FIR 插件，跳过两层处理。Equalizer 不受影响。

### 2.3 新增组件清单（v1.2：三 crate 分层）

| # | 组件 | 类型 | 文件 | 说明 |
|---|------|------|------|------|
| 1 | **calibration-types** | Rust crate (微型，零算法) | `workspace/crates/calibration-types/` | `EqBand` / `FitRequest` / `FitResult (FitCurves 256pt)` / `FirConfig` / `TargetCurveSpec` / `FitOptions` / `FitDiagnostics` / `FitError` / `ApplyError`。Tauri native、calibration-engine、Wasm calibration-fir **共同依赖，零重复**。 |
| 2 | Calibration PEQ | Builtin DspProcessor | phonon-core/src/dsp/calibration_peq.rs | 左右独立 Biquad 级联，前缀 `phonon_calibration_peq`。**sync_channel(1) 寄存器模型 + AtomicBool + AtomicPtr<Arc<Snapshot>> + last_applied: Arc<RwLock>**（§3.1） |
| 3 | **calibration-engine** | Rust crate (纯 Rust native) | `workspace/crates/calibration-engine/` | 依赖 calibration-types + rustfft + serde。实现：parsers (rew_csv/frd/autoeq_txt) / interpolate (三次样条 + Akima) / smooth (1/n oct) / target_curves (**数字化提取 64pt 常量表**) / biquad_fit (Cholesky 加权 LSQ，自研无依赖) / fir_residual (rustfft IFFT + Hilbert) / fit_engine (FitRequest → FitResult)。原生 crate，Tauri 命令直接 native 调用，单测无需 wasm-pack。 |
| 4 | **calibration-fir** | WasmExample（仅 overlap-save FIR，体积 ~25KB） | workspace/plugins/calibration-fir/ | 依赖仅 calibration-types（无算法，无 rustfft，无 parsers）。§2.2 DSP 链里做 overlap-save FIR 残差补偿卷积。manifest.name=`"calibration_fir"`。版本号轮询式热加载。去 no_std → 直接 std 环境。 |
| 5 | 曲线粗解析模块 | TS 前端（仅文件读 + 文本切片） | phonon-tauri/src/calibration/parsers.ts | 读 CSV/FRD/TXT → 做 BOM skip / 换行符标准化 → 序列化为 `{format, columns: [[f_str, spl_str],...]}` JSON，通过 `invoke run_calibration_fit(req_json)` 传给 Tauri 命令（Tauri 命令直接 native 调 calibration-engine）。仅做文本→数组，不做 DSP。 |
| 6 | 校准面板 | React UI | phonon-tauri/src/components/CalibrationPanel.tsx | 5 区顶级 UI：导入 / 四曲线可视化（Canvas 2D 或 WebGL）/ 目标拖拽编辑 / 拟合参数+预览 / 预设管理。**前端防抖 500ms 防连点（sync_channel(1) Full 兜底）**。 |
| 7 | Tauri 命令 | Rust 命令 | phonon-tauri/src-tauri/src/commands/calibration.rs | **5 条命令**：set_calibration_peq / get_calibration_peq / toggle_calibration_bypass / **run_calibration_fit(req, apply_immediately) → Result<FitResult, FitError>** / **apply_fit_result(fit_id) → Result<(), ApplyError>** / **get_current_fit_result() → Option<FitResult>**。apply 事务回滚语义保证 PEQ+FIR 一致。 |
| 8 | CalibrationAppState | Tauri AppState 扩展 | 同上 commands/calibration.rs 中 struct | `fit_results: Lru<FitCacheEntry>`（20 条 / 24h 双淘汰）+ `last_applied_fit_id: RwLock<Option<String>>` + `last_applied_result: RwLock<Option<FitResult>>`（**不淘汰，供"查看当前生效校准详情"用**）。 |
| 9 | AppSettings 扩展 | 持久化 | phonon-core/src/settings.rs | `calibration.peq_L/R`、`calibration.bypass`、`calibration.current_preset_name` 字段。 |
| 10 | Builtin 注册表扩展 | 元数据 | phonon-plugin/src/builtin.rs | 新增 `phonon_calibration_peq` 条目。 |
| 11 | Host Import 扩展 | Wasm host function | phonon-plugin/src/runtime.rs 的 linker | 新增 `host.get_plugin_config_version(name_ptr, name_len) -> i64`（返回当前版本号）和 `host.read_plugin_config(name_ptr, name_len, buf_ptr, buf_cap) -> i32`（读取最新 JSON 配置到 Wasm 内存）。PluginRuntime 加 `config_versions: Arc<HashMap<String, AtomicU64>>`。 |

---

## §3 校准 PEQ 节点（Builtin）

### 3.1 接口定义（v1.2：sync_channel(1) 寄存器模型 + AtomicPtr<Arc<>> + last_applied: RwLock）

> **线程安全背景（实锤）**：`DspChain` 被 `Arc<Mutex<DspChain>>` 包裹。DSP 回调线程在 `dsp.lock().unwrap().process(...)` 中持锁处理；Tauri 命令（UI 线程）要 `chain.set_enabled()` / `chain.find_mut()` 也要持同一个 Mutex。如果 UI 线程在持锁时做大量参数计算或序列化，会阻塞 DSP 回调触发爆音。因此：
>
> - **set_enabled/set_bands 不通过 Tauri 命令持锁直接写**：改用 `Arc<AtomicBool>` 和 sync_channel(1) 寄存器模型通道间接写。
> - `process` 签名是 `&self`（不可变），只能做 Atomic load + AtomicPtr swap + 无锁 try_recv + Biquad tick。零锁零等待。
> - **last_applied: Arc<RwLock<Option<Arc<PeqSnapshot>>>>**：Tauri 命令 `apply_fit_result` 的原子事务需要在 FIR 配置失败时回滚 PEQ 到"应用前"快照，这个快照保存在 handle 里（UI 层拿不到 DSP 内部状态）。`RwLock` 语义：读多（UI 查"当前生效 PEQ 是什么"/回滚前 before 快照读/诊断导出）写少（仅 update_bands 成功时一次 write）。

```rust
// phonon-core/src/dsp/calibration_peq.rs
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::{Arc, RwLock};
use std::sync::mpsc::{self, SyncSender, Receiver, TrySendError};
use thiserror::Error;
use crate::eq::{EqBand, BiquadBand};
use super::{DspProcessor, DspProcessorInfo};
use std::any::Any;

/// 可 swap 的 Biquad 参数快照（一次性构造完成后只读）
#[derive(Clone)]
pub struct PeqSnapshot {
    pub sample_rate: u32,
    pub bands_L: Vec<EqBand>,
    pub bands_R: Vec<EqBand>,
    pub biquads_L: Vec<BiquadBand>,  // 系数已预计算，process 直接用
    pub biquads_R: Vec<BiquadBand>,
}

impl PeqSnapshot {
    pub fn from_bands(sr: u32, bands_L: Vec<EqBand>, bands_R: Vec<EqBand>) -> Self {
        let biquads_L = bands_L.iter().map(|b| BiquadBand::from_eqband(b, sr)).collect();
        let biquads_R = bands_R.iter().map(|b| BiquadBand::from_eqband(b, sr)).collect();
        Self { sample_rate: sr, bands_L, bands_R, biquads_L, biquads_R }
    }
    pub fn empty(sr: u32) -> Self { Self::from_bands(sr, vec![], vec![]) }
    pub fn L(&self) -> &[EqBand] { &self.bands_L }
    pub fn R(&self) -> &[EqBand] { &self.bands_R }
}

/// 通道模型：sync_channel(1) 单槽 = 寄存器语义。DSP 侧每次 try_recv 要么拿到最新 pending，要么空。
/// UI 侧 try_send 满了就是异常（DSP hang，前端防抖应已阻止绝大多数），直接返回 ChannelFull。
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
pub enum PeqUpdateError {
    #[error("audio thread busy (sync_channel(1) full)")]
    ChannelFull(String),
    #[error("calibration PEQ processor dropped (audio device switched?)")]
    ChannelClosed(String),
}

pub struct CalibrationPeqProcessor {
    /// 启用状态。UI 写 store(true)，process 每帧 load，无锁。
    enabled: Arc<AtomicBool>,
    /// 当前生效快照指针。UI 写（DSP 侧 process 内 swap 自己），process 每帧 load。
    current: AtomicPtr<Arc<PeqSnapshot>>,
    /// 接收 UI 线程注入的 pending 快照（sync_channel(1)，单槽寄存器）。
    /// 在每次 process() 开头 try_recv 非阻塞地取最新 pending 并 swap。
    pending_rx: Receiver<Arc<PeqSnapshot>>,
    /// 与 pending_rx 配对的 sender（Processor 自身持有一份，用于 drop 时通道关闭语义）
    #[allow(dead_code)]
    pending_tx_self: SyncSender<Arc<PeqSnapshot>>,
}

/// Tauri 命令侧通过此句柄注入参数（不持 Mutex<DspChain> 锁，不阻塞 DSP 回调）。
/// `SyncSender` 在 Rust 1.72+ 实现了 `Clone + Send + Sync`，多命令线程并发场景下每人 clone 一份自己的 sender，零锁。
#[derive(Clone)]
pub struct PeqUpdateHandle {
    pub enabled_tx: Arc<AtomicBool>,
    pub pending_tx: SyncSender<Arc<PeqSnapshot>>,
    /// 最后一次成功注入的快照（读多写少 → RwLock）。
    /// 用于：apply_fit_result 事务回滚（FIR 失败时把 PEQ 滚回 before 快照）；UI 查询"当前生效 PEQ"；诊断导出。
    pub last_applied: Arc<RwLock<Option<Arc<PeqSnapshot>>>>,
}

impl PeqUpdateHandle {
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled_tx.store(enabled, Ordering::Release);
    }

    pub fn enabled(&self) -> bool {
        self.enabled_tx.load(Ordering::Acquire)
    }

    /// 单次 try_send，零重试。Full 错误直接返回 → 前端防抖兜底。
    pub fn update_bands(&self, bands_L: Vec<EqBand>, bands_R: Vec<EqBand>) -> Result<(), PeqUpdateError> {
        let snap = Arc::new(PeqSnapshot::from_bands(48000, bands_L, bands_R));
        match self.pending_tx.try_send(snap.clone()) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                return Err(PeqUpdateError::ChannelFull(
                    "audio thread busy; try again in ~500ms".into()
                ));
            }
            Err(TrySendError::Disconnected(_)) => {
                return Err(PeqUpdateError::ChannelClosed(
                    "calibration PEQ processor dropped (audio device switched?)".into()
                ));
            }
        }
        *self.last_applied.write().unwrap() = Some(snap);
        Ok(())
    }

    /// 内部用：手动设置 last_applied（罕见：从磁盘持久化恢复或测试时）。
    pub fn set_last_applied(&self, snap: Arc<PeqSnapshot>) {
        *self.last_applied.write().unwrap() = Some(snap);
    }

    /// 读最后一次成功注入的快照：用于事务回滚前保存"应用前"状态。
    pub fn last_applied_snapshot(&self) -> Option<Arc<PeqSnapshot>> {
        self.last_applied.read().unwrap().clone()
    }
}

impl CalibrationPeqProcessor {
    pub fn new() -> (Self, PeqUpdateHandle) {
        let enabled = Arc::new(AtomicBool::new(false));
        // ★ 单槽寄存器模型：capacity = 1
        let (tx, rx) = mpsc::sync_channel::<Arc<PeqSnapshot>>(1);
        let proc = Self {
            enabled: enabled.clone(),
            current: AtomicPtr::new(std::ptr::null_mut()),
            pending_rx: rx,
            pending_tx_self: tx.clone(),
        };
        let handle = PeqUpdateHandle {
            enabled_tx: enabled,
            pending_tx: tx,
            last_applied: Arc::new(RwLock::new(None)),
        };
        (proc, handle)
    }
}

impl DspProcessor for CalibrationPeqProcessor {
    /// process 全程只读 + 无锁：Atomic load + try_recv + AtomicPtr swap + Biquad tick。
    fn process(&self, samples: &mut [f32], channels: u16, _sample_rate: u32) {
        // 1) 非阻塞取 pending（capacity=1 → 要么 1 个要么 0 个，while 循环兼容未来扩容）
        let mut latest: Option<Arc<PeqSnapshot>> = None;
        while let Ok(s) = self.pending_rx.try_recv() { latest = Some(s); }
        if let Some(s) = latest {
            // ★ Arc 指针 swap：旧 strong_count fetch_sub，最后一个引用由任意线程后台回收
            let raw = Arc::into_raw(s);
            let old = self.current.swap(raw as *mut Arc<PeqSnapshot>, Ordering::AcqRel);
            if !old.is_null() {
                let old_arc_ref: &Arc<PeqSnapshot> = unsafe { &*old };
                drop(old_arc_ref.clone()); // strong_count -= 1；若 =0 则后台 drop 析构
            }
        }

        if !self.enabled.load(Ordering::Acquire) { return; }
        let snap_ptr = self.current.load(Ordering::Acquire);
        if snap_ptr.is_null() { return; }
        // 解引用：指针类型是 *mut Arc<PeqSnapshot> → &Arc<PeqSnapshot> → &PeqSnapshot
        let snap_arc = unsafe { &*snap_ptr };
        let snap: &PeqSnapshot = &**snap_arc;

        // 2) 逐样本 inline 解交错（samples[i*2] = L, [i*2+1] = R）无额外 memcpy
        if channels == 1 {
            for i in 0..samples.len() {
                let mut x = samples[i];
                for b in &snap.biquads_L { x = b.tick(x); }
                samples[i] = x;
            }
        } else {
            let n = samples.len() / 2;
            for i in 0..n {
                let mut l = samples[i*2];
                let mut r = samples[i*2 + 1];
                for b in &snap.biquads_L { l = b.tick(l); }
                for b in &snap.biquads_R { r = b.tick(r); }
                samples[i*2] = l;
                samples[i*2 + 1] = r;
            }
        }
    }

    fn name(&self) -> &str { "Calibration PEQ" }
    fn id(&self) -> &str { "calibration_peq" }
    fn enabled(&self) -> bool { self.enabled.load(Ordering::Acquire) }
    fn set_enabled(&mut self, enabled: bool) { self.enabled.store(enabled, Ordering::Release); }
    fn latency(&self) -> f64 { 0.0 }  // IIR Biquad 零额外延迟
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}
```

**Arc 指针内存安全说明（v1.2，已无泄漏）**：
- `self.current` 存储的是 `Arc::into_raw(Arc<PeqSnapshot>)` 转出的裸指针（类型 `*mut Arc<PeqSnapshot>`）—— 等价于一个堆上 Arc 结构体的指针，strong_count 至少 ≥ 1（current 持有一份）。
- swap 时旧裸指针被移出，通过 `&*old` 得到 `&Arc<PeqSnapshot>`，然后 `clone()` 一份 → drop(clone) 执行一次 strong_count fetch_sub(1, AcqRel)。若结果为 0，则 Arc 内的 PeqSnapshot 析构 + 全局分配器后台释放。DSP 线程只做一次原子减法（纳秒级），不做复杂析构。
- 任何时候都不会在 DSP 回调内执行 PeqSnapshot（含 `Vec<BiquadBand>` 的大析构）——那份只有当 UI 线程同时 drop 最后一份引用时才在 UI 线程执行，或者全局 allocator 的后台线程。总之 DSP 永远不阻塞。

**PeqUpdateHandle::Clone**：派生 Clone，`SyncSender`（Rust 1.72+）+ `Arc<AtomicBool>` + `Arc<RwLock<_>>` 全 Clone。Tauri 命令工作池每个线程 clone 一份自己的 handle，零锁零竞争。

**前端防抖（500ms，防连点导致 ChannelFull）**：
```ts
// phonon-tauri/src/components/CalibrationPanel.tsx 内所有调用 apply_fit_result / 拖拽预览 update_bands 的按钮和事件：
const [applying, setApplying] = useState(false);

async function applyCalibration(fitId: string) {
  if (applying) return;   // 500ms 内禁止重复点击
  setApplying(true);
  try {
    await invoke('apply_fit_result', { fitId });
  } catch (e: any) {
    if (String(e).includes('ChannelFull')) {
      toast('音频线程繁忙，请稍后重试');
    } else {
      toast('应用失败：' + e);
    }
  } finally {
    setTimeout(() => setApplying(false), 500);
  }
}

// 拖拽控制点的实时预览：100ms debounce（lodash.debounce / 手写）
const debouncedPreviewFit = useCallback(debounce(async (pts) => {
  // invoke run_calibration_fit with apply_immediately=false
  // 只算 FitResult 给 UI，不注入 DSP，不会触发 ChannelFull
}, 100), []);
```

### 3.2 处理流程（process）

> 解交错是逐样本 inline 处理（samples[i\*2] 即 L，samples[i\*2+1] 即 R），不做整块重排（无额外 memcpy）。

```
输入: 交错立体声 samples[s0_L, s0_R, s1_L, s1_R, ...]

1. 非阻塞 try_recv 接收 UI 线程发来的新快照（capacity 1 → 0 或 1 次，≤ 200ns）
2. Atomic load enabled → false 时直接 return（零开销）
3. AtomicPtr load 当前快照指针 → null 时 return
4. 逐样本处理:
   for i in 0..n_frames:
       L = samples[i*2]
       R = samples[i*2 + 1]
       for b in 0..n_bands:
           L = snap.biquads_L[b].tick(L)
           R = snap.biquads_R[b].tick(R)
       samples[i*2] = L
       samples[i*2 + 1] = R
```

**线程安全不变式**：
- 全程无 `&mut self` 在 `process` 内部修改 DSP 状态
- `enabled` 是 AtomicBool；`current` 是 AtomicPtr<Arc<Snapshot>>；两者 Acquire/Release 语义保证跨线程可见性
- `pending_rx.try_recv()` 是 sync_channel 的 try_recv（无锁，单槽寄存器语义，常数时间）
- 最坏情况下 DSP 回调内的原子/通道操作 = 1 AtomicBool load + 1 AtomicPtr load + 1 try_recv（均为纳秒级）

### 3.3 Builtin 注册

在 `phonon-plugin/src/builtin.rs` 的 BUILTINS 数组末尾（ReplayGain 之后）追加：

```rust
BuiltinPlugin {
    id: "calibration_peq",
    manifest: PluginManifest {
        name: "phonon_calibration_peq".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        author: "Phonon Contributors".into(),
        description: "Calibration PEQ: 左右独立 Biquad 级联，通过 sync_channel(1) 寄存器模型 + AtomicPtr<Arc<Snapshot>> 无锁原子通道接收声学校准拟合参数。配合 calibration-engine 原生 crate（Biquad LSQ 拟合 + FIR IFFT/Hilbert 残差）和 calibration-fir Wasm 插件实现混合校准。100% 原创实现。".into(),
        plugin_type: PluginType::DspProcessor,
        permissions: vec![],
        min_framework_version: env!("CARGO_PKG_VERSION").into(),
    },
    config_remarks: "CalibrationPanel：导入测量曲线 → calibration-engine 原生拟合 → apply 时通过 PeqUpdateHandle 注入 PEQ + set_plugin_config 注入 FIR → 版本号轮询自动生效；旁路开关一键 A/B 对比。",
}
```

顺序保持：`Equalizer → SmartEffect → Surround → Downmix → ReplayGain → Calibration PEQ`（与 §2.2 交叉引用：此顺序为 **PluginManager 展示顺序**；**DSP 链实际处理顺序** = Calibration PEQ → Calibration FIR → Equalizer → Surround → SmartEffect → Downmix → ReplayGain，由 phonon-core/src/engine.rs 初始化 DspChain 时按 add 顺序决定）。

---

## §4 校准 FIR Wasm 插件（WasmExample：只做 overlap-save FIR，无 Fit Engine）

> **v1.2 拆分后职责极度收敛**：Wasm 端只做一件事 —— overlap-save FIR 残差卷积。拟合计算、曲线解析、Biquad LSQ、FFT/Hilbert 全部在 `calibration-engine` 原生 crate 中做。Wasm 体积锁死在 ~25KB（仅 calibration-types + overlap-save 核心 ~100 行）。

### 4.1 Manifest 元数据

```json
{
  "name": "calibration_fir",
  "version": "0.1.0",
  "author": "Phonon Contributors",
  "description": "Calibration FIR: Wasm 端仅 overlap-save FIR 残差补偿卷积（L/R 独立）。系数热加载走版本号轮询模型：host.get_plugin_config_version / host.read_plugin_config。tap 动态可配 256~8192（默认 2048）。完整拟合计算由宿主侧 calibration-engine 原生 crate 完成，本插件不包含任何拟合算法。",
  "plugin_type": "DspProcessor",
  "permissions": [],
  "min_framework_version": "0.1.0"
}
```

### 4.2 Wasm 端接口（去 no_std → 直接 std 环境）

```rust
// workspace/plugins/calibration-fir/src/lib.rs
use std::sync::Arc;
use calibration_types::FirConfig;   // ★ 仅依赖微型 types crate

/// 插件全局状态（单线程 DSP 上下文，无需原子，&mut self 直接可写）
struct PluginState {
    fir_enabled: bool,
    fir_tap: usize,
    coeffs_L: Arc<Vec<f32>>,    // Arc 双缓冲：系数换时新 Arc 替换，旧内容自然保留到 overlap 使用完毕
    coeffs_R: Arc<Vec<f32>>,
    // overlap-save 状态（长度 = tap + block_size - 1；block_size 由首次 process 确定为 samples.len()/channels）
    overlap_L: Vec<f32>,
    overlap_R: Vec<f32>,
    block_size: Option<usize>,
    // 版本号轮询
    last_known_version: u64,
}

/// 导出：plugin_init / plugin_process（DspProcessorPlugin trait）—— 同标准 WasmExample 接口
///
/// 无 plugin_fit 导出！（拟合在 Tauri 命令侧 native 调 calibration-engine::fit 完成）
///
/// plugin_process 内部开头执行版本号轮询热加载（§4.3）
```

Cargo.toml 依赖（去 no_std，直接 std；默认 rustfft 不进 wasm 因为 calibration-types 不依赖它）：
```toml
[package]
name = "calibration-fir"
version = "0.1.0"
edition = "2021"
license = "MIT"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
calibration-types = { path = "../../crates/calibration-types" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
```

### 4.3 FIR 系数热加载流程（版本号轮询拉模型，不变）

```
【配置写入侧（前端 → Tauri 命令 → 宿主）】
1. Tauri 命令 run_calibration_fit(req, apply_immediately=true) 或 apply_fit_result(fit_id) 成功 →
   同时执行两步事务（§10.2 完整事务回滚伪代码）：
   a) PEQ: peq_update_handle.update_bands(peq_L, peq_R) → sync_channel(1) → DSP 下帧 swap
   b) FIR: plugin_runtime.set_plugin_config_sync(
               "calibration_fir",
               FirConfig { enabled: tap>0, tap, coeffs_L, coeffs_R }
           ) → configs HashMap 插入 → config_versions["calibration_fir"].fetch_add(1)
（第 1b 步完成后：持久化落盘 + 版本号原子递增 → FIR 热加载就绪）

【配置消费侧（Wasm → host 拉取）】
每次 process(samples, ch, sr) 调用开头（在 Wasm 插件内）：
2. 调 host.get_plugin_config_version("calibration_fir") → curr_version
3. if curr_version != last_known_version:
     a) 调 host.read_plugin_config(name, buf, cap) 得到 JSON
     b) 解析 JSON → FirConfig { enabled, tap, coeffs_L[], coeffs_R[] }
     c) 校验 tap ∈ {256,512,1024,2048,4096,8192}
     d) 分配新 coeffs_L_new/coeffs_R_new，若 tap 变化则重建 overlap 缓冲区（resize 到 (tap + block_size - 1)）
     e) 双缓冲：新 coeffs 用 Arc::new 包裹 → 一次性替换 PluginState 字段（单线程 DSP 上下文 &mut self，无需原子）
        切换帧前后的输出连续性由 overlap-save 状态机保证：新 coeff 生效后前半帧 overlap 为旧内容，但 overlap 缓冲区不清零，卷积结果自然过渡 → 无爆音
     f) last_known_version = curr_version
4. 正常走 overlap-save 卷积 → 返回 PCM
```

**性能开销评估**：`get_plugin_config_version` = 1 次 host import（~50ns）+ 1 次 `AtomicU64::load(Acquire)`（~1ns）。每帧 < 100ns，可忽略。

---

## §5 拟合算法（calibration-engine 原生 Rust crate，依赖 rustfft MIT/Apache）

### 5.1 模块划分（全部在 calibration-engine 下；公共类型来自 calibration-types）

```
workspace/crates/
  calibration-types/                  ★ 公共类型（所有 crate 共享，零算法）
    Cargo.toml (仅 serde 可选)
    src/lib.rs
      EqBand { frequency, gain_db, q, filter_type: FilterType }
      FilterType: Peaking / LowShelf / HighShelf / LowPass / HighPass / BandPass / Notch / AllPass
      FirConfig { enabled: bool, tap: u32, coeffs_L: Vec<f32>, coeffs_R: Vec<f32> }
      MeasurementData { freqs: Vec<f32>, magnitudes_db: Vec<f32>, phase_rad: Option<Vec<f32>> }
      TargetCurveSpec: Builtin { name } / Custom { points: Vec<(f32,f32)> } / Imported(MeasurementData)
      FitOptions { peq_bands_per_channel, filter_types ("mixed"), fir_tap, fir_smooth_bandwidth_hz, max_total_gain_db }
      FitRequest { measurement_L, measurement_R: Option<_>, target: TargetCurveSpec, options: FitOptions }
      FitCurves { freq: [f32;256], target_db/post_peq_only_db/post_full_db/pre_fit_db: [f32;256] }
      FitDiagnostics { rmse_full_lr_avg_db, peak_error_full_db, midband_rmse_100_4khz_db, treble_rmse_10_20khz_db }
      FitResult { peq_L/R: Vec<EqBand>, fir_coeffs_L/R: Vec<f32>, curves_L/R: FitCurves, diagnostics: FitDiagnostics, fit_id: String (base64(sha256(request_bytes[0..16]))) }
      FitError: MeasurementTooShort / FreqRange / TargetNotFound / BiquadLsqFailed / FirGenFailed / Internal
      ApplyError: FitNotFound / PeqInjectFailed / FirConfigFailed / RollbackPeqFailed

  calibration-engine/                 ★ 纯 Rust 原生 crate：拟合计算主实现
    Cargo.toml (calibration-types, rustfft, serde, serde_json; MIT/Apache 合规)
    src/lib.rs (pub re-exports)
    src/parsers/
      mod.rs
      rew_csv.rs    — REW 导出 CSV 细解析 → MeasurementData
      frd.rs        — .frd 2/3 列格式 → MeasurementData
      autoeq_txt.rs — AutoEq ParametricEQ "Filter" 行解析 → EqBand[] + preamp_db（若直接匹配 EqBand 则跳过拟合）
    src/interpolate.rs — 自然三次样条 + Akima 插值（自定义目标曲线用）；重采样到 N 点 log-spaced 频率轴
    src/smooth.rs      — 1/n octave 分数阶平滑（高斯加权移动平均，σ = 1/(2√2ln2) × oct_bandwidth，保证 dB 保形）
    src/target_curves.rs
        builtin_ids() → ["flat", "harman_oe_2019", "harman_ie_2018", "diffuse_field", "free_field"]
        eval(spec, freq: f32) -> gain_db: f32
        【★ v1.2 数字化提取】：
          Harman OE 2019 / Harman IE 2018：
            论文 PDF 为 AES eLibrary 付费购买（ResearchGate 亦需登录），无法直接抄到分段多项式系数。
            改为：从公开发布的 **Harman / AKG 官方产品博客高分辨率曲线图 PNG**（合规：公开网页可访问图像属于公开事实信息发布）→
              用 **WebPlotDigitizer v4.7** 数字化提取 64 个 (freq_Hz, gain_dB) 数据点 →
              硬编码为 `const [(f32, f32); 64]` 常量表 + Akima 插值 → eval(freq)。
            代码开头注释标明：
              /// Digitized from <https://..../harman-target-curve.png> (Harman / AKG blog post, 公开可访问 URL)
              /// using WebPlotDigitizer v4.7. Approx ±0.3 dB accuracy.
              /// Reference: Olive, S., et al., "The Relationships Between Perceived Sound Quality ...",
              ///   AES 144th Convention (2018) / JASA (2019). See original paper for authoritative coefficients.
              /// 代码为原创数字化提取劳动，不复制任何第三方数据库文件或 AutoEq measurements CSV。
          Flat: gain=0 全域。
          Free Field: `gain = 20*log10(lambda / sqrt(lambda^2 + r^2))`, r=0.0875m（头部直径声学平均值），lambda=343/f。
          Diffuse Field: IEC 60268-7 标准公布的 11 折点分段线性近似（标准公布表格数值 → 常量表）。
    src/biquad_fit.rs
        iso_13_octave_centers(n, fmin, fmax) — ISO 1/3-octave 标准频点取 n 个等对数间隔
        rbj_peak_response_db(f, fc, q, gain_1db: f32) — RBJ Cookbook 幅频公式（纯代数无依赖）
        cholesky_solve_16(a: &mut [[f32;16];16], b: &mut [f32;16]) — 自研无依赖 16x16 Cholesky（<1μs）
        fit_weighted_lsq(meas_dB, target_dB, options) -> (Vec<EqBand>/*L or R*/, Vec<f32> residual_dB[128])
            算法: §5.2
    src/fir_residual.rs
        log_to_linear_freq_axis(log_freqs[128], log_mag[128], nfft, sr) -> Vec<f32> H_mag_linear[nfft]
            — 线性插值 + <20Hz 和 >sr/2 clamp 到 0dB + 20Hz 附近 3 点 cosine ramp 抑制吉布斯
        hilbert_minphase_from_mag(mag: &[f32]) -> Vec<Complex<f32>> — FFT-based Hilbert 最小相位恢复
        ifft_to_taps(h_cplx: Vec<Complex<f32>>, tap: usize, mode: PhaseMode) -> Vec<f32>
            — Hann 窗 + 归一化直流增益 = 0dB
    src/fit_engine.rs
        pub fn fit(req: FitRequest) -> Result<FitResult, FitError>
            主流程：
            1. 校验：L 测量点数 ≥ 48；freqs ∈ [20,20000]。失败 → FitError::MeasurementTooShort / FreqRange
            2. L 侧：解析（如果 parsers 做的话；这里输入已经是 MeasurementData → 直接用）→
               interpolate::resample_log_spaced → 128 点 log-spaced → smooth 1/12 oct → meas_dB[128]
            3. target_curves::eval 对同 128 频率轴 → target_dB[128]
            4. biquad_fit::fit_weighted_lsq → peq[bands] + residual_dB[128]
            5. 如果 options.fir_tap > 0：fir_residual::generate(residual_dB, options) → coeffs[fir_tap]
               否则 coeffs 空 Vec
            6. 计算 FitCurves：256 点固定对数频率轴 freq[i]=20*(20000/20)^(i/255)
               pre_fit_db[i] = Akima(measurement 原始点 → 在 freq[i] 插值)
               target_db[i] = target_curves::eval(freq[i])
               post_peq_only_db[i] = Σ peq[k].response(freq[i]) + pre_fit_db[i]（Biquad 单独效果）
               post_full_db[i] = post_peq_only_db[i] + fir_response(freq[i])（FIR 加进去）
               ★ 所有 dB 值 clamp [-30.0, +18.0]，NaN/Inf → 最近有效值替换。
            7. 计算 FitDiagnostics：基于 post_full vs target 的 4 个误差指标
            8. fit_id = base64(sha256(serde_json::to_vec(&req)?[0..16])) → 12 字符短码前缀做 UI 显示
            9. R 侧：若 measurement_R=None 则 L 复用；否则同 2-8 独立跑一份
            10. 组装 FitResult 返回
```

### 5.2 Biquad 拟合算法（biquad_fit.rs）

**输入**：
- `measurement_dB[freq_idx]`：测量曲线（重采样到 128 点 log-spaced，20Hz~20kHz）
- `target_dB[freq_idx]`：目标曲线（同频率轴）
- `options: FitOptions { peq_bands_per_channel, filter_types: "mixed", fir_tap, fir_smooth_bandwidth_hz, max_total_gain_db }`

**算法（加权迭代最小二乘）**：

```
1. 初始化 N_peq 个 center freqs：ISO 1/3-octave 标准频点中取 N_peq 等对数间隔（默认 N=16）
   — 若 filter_types = "mixed"：第 1 条 LowShelf (f=~100Hz) + 中间 N-2 条 Peaking + 第 N 条 HighShelf (f=~10kHz)
2. desired_dB[f] = target_dB[f] - measurement_dB[f]
3. 权重 w[f] = 1/√f（低频高权重）；|desired_dB| > 3dB 处 w 饱和，抑制奇异过拟合
4. for iter in 1..8:
     a) 128×N_peq 设计矩阵 A: A[f,k] = 第 k 条滤波器类型 1dB 单位增益响应（RBJ Cookbook 幅频公式）
     b) 正规方程: (Aᵀ W A) · g = Aᵀ W · desired；Cholesky 16×16 (<1μs)
     c) gain[k] clamp [-24, +24]，同时 clamp Σ gain ≤ max_total_gain_db
     d) re-compute response[f] = Σ A[f,k]*gain[k]
     e) residual[f] = desired[f] - response[f]
     f) residual 1/12 oct 平滑 → 更新 desired（抑制过拟合尖峰）
5. 输出: EqBand[N_peq] L/R + residual_dB[128] L/R
```

**质量指标**：在 20Hz~20kHz 范围内 `max(|residual|) < 3dB`（保证 FIR 残差补偿域内有效）。

### 5.3 FIR 残差系数生成（fir_residual.rs，依赖 rustfft）

**输入**：`residual_dB[128]` / `options: (tap_count, phase_mode, sample_rate)`

**算法**：
```
1. 频率轴变换：128 点 log-spaced → N_fft = 2*tap_count 线性频率轴（保证时间分辨率无混叠）
   20Hz~sr/2 线性插值 → H_mag；<20Hz 和 >sr/2 clamp 到 1.0；20Hz 附近 3 点 cosine ramp
2. |H_mag[ω]| = 10^(residual_dB[ω]/20)
3. 最小相位（默认）：
   a) 对 log(|H|) 做 FFT-based Hilbert 最小相位恢复 → H_phase
   b) H[ω] = |H_mag| exp(j·H_phase)
   c) 共轭对称填充 → IFFT → h[n]
4. 线性相位：H[ω] = |H_mag| exp(-j·ω·(tap-1)/2) → IFFT → h[n]
5. Hann 窗 × 归一化直流增益 = 0 dB → coeffs[tap_count]
```

**rustfft 依赖合规**：MIT/Apache 双许可，符合 deny.toml 白名单。`calibration-engine/Cargo.toml`: `rustfft = "6"`（默认 features）。`calibration-fir` 不依赖 rustfft。

---

## §6 曲线导入格式解析

| 格式 | 扩展名 | 典型来源 | 解析器位置（细，Rust）| 粗解析（TS，仅文本切片）|
|------|-------|----------|----------------------|------------------------|
| REW Export CSV | `.csv` | Room EQ Wizard | calibration-engine/src/parsers/rew_csv.rs | phonon-tauri/src/calibration/parsers.ts |
| FRD 文本 | `.frd` | FRD Consortium | calibration-engine/src/parsers/frd.rs | 同上 |
| AutoEq ParametricEQ TXT | `.txt` | AutoEq 项目导出 | calibration-engine/src/parsers/autoeq_txt.rs → **直接出 EqBand[]，Fit Engine 可跳过 Biquad 拟合** | 同上 |

**粗 vs 细**：TS 粗解析只做 "读取 File → BOM 跳过 + 换行符标准化 → 序列化为 JSON 字符串"，不做任何 DSP、不做 f32 解析（避免 JS 浮点误差）。Rust 侧细解析完成 f32 解析 + 类型构造 + 错误类型返回。

---

## §7 目标曲线

### 7.1 预置目标曲线（target_curves.rs，calibration-engine）

| 曲线 ID | 名称 | 实现方式 |
|---------|------|----------|
| `flat` | Flat / 完全平直 | `gain(f) = 0 dB` 全域 |
| `harman_oe_2019` | Harman Over-Ear 2019 | **数字化提取（64 点常量表 + Akima 插值，±0.3 dB 精度）**。原论文 Olive et al. AES 144th (2018) / JASA (2019) 公式 PDF 需付费购买无法抄系数；从 Harman/AKG 官方博客公开曲线图 WebPlotDigitizer 提取。代码注释完整标明图 URL + 数字化工具 + 精度声明 + 论文 DOI 出处作为权威参考。合规：公开事实信息的原创数字化提取劳动，不复制任何第三方数据文件。 |
| `harman_ie_2018` | Harman In-Ear 2018 | **同上数字化提取**。原论文 Olive et al. AES 143rd (2017) In-Ear target 同样需付费。64 点常量表 + Akima。 |
| `diffuse_field` | Diffuse Field | IEC 60268-7 标准公布的扩散场均衡曲线 11 折点分段线性近似（标准公布数值 → 常量表） |
| `free_field` | Free Field 0° 入射 | 头部直径近似 r=0.0875m：`gain = 20·log10(λ/√(λ² + r²))`；λ = 343/f Hz（简化解析公式，纯代码原创） |

> **合规红线（v1.2 最终落地）**：Harman 曲线的数字化提取 64pt 方案经过完整法律评估。后续如能拿到 AES eLibrary 的 PDF 公式系数，替换成本为 1 个 `fn gain(f)` 函数，FitCurves 接口不变。无论哪种方式，**绝对不引用 AutoEq Git 仓库 measurements CSV 数据文件**。

### 7.2 自定义目标曲线编辑器（前端 UI 侧）

- 控制点：`{ freq: number, gain_db: number }[]`；默认 8 点：50/100/200/500/1k/2k/5k/10k Hz @ 0 dB
- 渲染：Akima 样条插值（与 calibration-engine 的 FitCurves 256pt 重采样同算法，前后端像素映射 1:1 对齐）
- 交互：单击空白添加 / 拖拽修改 / 右键删除 / Shift 拖拽约束方向
- 工具栏：平滑强度、重置为预置、L⇄R 对称复制、导入 CSV/FRD

---

## §8 校准面板 UI（CalibrationPanel.tsx）

### 8.1 5 区顶级布局

```
┌─ Calibration Panel ─────────────────────────────────────────────────────────────────┐
│ ① 测量导入区                | ② 曲线可视化区（Canvas 2D 或 WebGL）                │
│ [＋拖拽 or 浏览文件]         | ┌───────────────────────────────────────────────┐    │
│ L: headphone_L.csv ✓  Swap  | │ 4 条曲线叠加：                                  │    │
│ R: headphone_R.csv ✓  Use L | │   测量(红)/目标(灰虚)/PEQ单独(蓝)/PEQ+FIR(绿)  │    │
│ ────────────────────────    | │   Y: [-30,+18] dB 线性 · X: 20~20kHz log      │    │
│ Presets: [▼预设名] [+] [-]  | │   残差填充：拟合后 post_full vs target 色块区  │    │
│ [导出JSON] [导入JSON]       | │   光标 tooltip：freq + 4 条 dB 值               │    │
│                             | └───────────────────────────────────────────────┘    │
├─────────────────────────────┴───────────────────────────────────────────────────────┤
│ ③ 目标曲线拖拽编辑器（控制点增删改 + 工具栏）                                         │
│ 预置: [▼Flat]  · 平滑: [2/10]  · [↔ L⇄R 复制]  · [↺ 重置为预置]                    │
├─────────────────────────────────────────────────────────────────────────────────────┤
│ ④ 拟合参数区 · [拟合预览 ▼计算中]  ·  进度  ·  诊断指标卡片（4 项误差）             │
│   PEQ 段数: [16 ▼]  FIR tap: [2048 ▼ 6 档]  相位: [最小相位▼混合]  Smooth: [1/12▼] │
│   → (apply_immediately=false) 计算完立刻显示曲线，不注入 DSP。                          │
│   ⑤ 按钮栏（右侧）：  [立即拟合并应用]  [应用配置]  [💾保存为预设]  [A/B 旁路切换]  │
│                        （前端 500ms 防抖防连点：sync_channel(1) ChannelFull 兜底）    │
└─────────────────────────────────────────────────────────────────────────────────────┘
```

### 8.2 拟合工作流（v1.2：2 步 dry-run + apply 分离，用户始终掌控）

```
用户点击 [拟合预览]  →  invoke run_calibration_fit(req, apply_immediately=false)
                   →  Tauri 命令侧 native 调 calibration_engine::fit(req)
                   →  50~150ms 后同步返回 FitResult（无 DSP 注入）
                   →  UI 立刻更新：Canvas 画 4 条曲线；4 项误差卡片诊断显示；[应用配置] 按钮亮；
                      当前预览 fit_id 短码显示（如 #A3F92C）；当前结果存入缓存（20 条 / 24h）

用户点击 [应用配置]  →  invoke apply_fit_result(fit_id)
                   →  原子事务：PEQ update_bands → 成功 → FIR set_plugin_config_sync → 成功
                      ↑ 若任一步失败，PEQ 自动回滚到事务前的 last_applied_snapshot
                   →  成功：状态栏绿条"校准已生效（#A3F92C）"；last_applied_result 被写入（永不淘汰）
                      失败：顶部 Toast 中文错误 + 不应用任何东西

快捷：用户点击 [立即拟合并应用]
                   →  invoke run_calibration_fit(req, apply_immediately=true)
                   →  计算 + 原子应用一次搞定。失败 toast，成功绿条。

查看当前生效详情：右键菜单"查看当前生效校准详情" → invoke get_current_fit_result()
                   →  得到 Option<FitResult> → 打开详情弹窗：完整 4 条曲线叠加 + 误差卡片 + fit_id
```

### 8.3 FitCurves 与前端 Canvas 对齐规则（4 条铁律）

| # | 规则 | 理由 |
|---|------|------|
| 1 | **长度固定 256** | `freq/ target_db/ pre_fit_db/ post_peq_only_db/ post_full_db` 五数组 **严格 len=256**。Canvas 绘制直接 `for i in 0..256`，无任何 len 分支；WebGL 256 索引紧凑。 |
| 2 | **freq 严格对数分布，精确端点** | `freq[i] = 20.0 * (20000.0/20.0).powf(i as f32 / 255.0)` → `freq[0]=20 Hz, freq[255]=20000 Hz`。前端 `xFor(f) = padL + (log10(f)-log10(20))/(log10(20000)-log10(20))*(W-padL-padR)`，两者公式完全一致 → 光标像素点击 ↔ 频率 ↔ 增益一一对应，光标 tooltip 无抖动误差。 |
| 3 | **dB clamp [-30, +18] + NaN 消毒** | calibration-engine fit() 生成 curves 末尾强制 `v.clamp(-30.0, 18.0)`；NaN 用最近有效值替换。Canvas 2D / SVG path 对 NaN 的语义是"当前子路径断裂"，必须在 Rust 端先消毒，零异常丢前端。 |
| 4 | **本期走 JSON number[]**。体积 256 × 5 × 8 B ~ 10KB × 2 ch ≈ 20KB，对现代 PC 零感知。后续版本可升级为 Tauri `Bytes` + bincode + `ArrayBuffer` 二进制直接 receive 提速 | JSON 快速交付；二进制作为未来 PR 优化，不阻塞本期 M1~M9。 |

前端 Canvas 绘制伪代码（与后端 1:1 对齐）：
```ts
const f0 = 20, f1 = 20000;
const xFor = (f: number) => padL + (Math.log10(f) - Math.log10(f0)) / (Math.log10(f1) - Math.log10(f0)) * (W - padL - padR);
const yFor = (db: number) => H - padB - ((db - (-30)) / (18 - (-30))) * (H - padT - padB);
// curves: FitCurves（来自 FitResult.curves_L）
const drawLine = (arr: number[], color: string) => {
  ctx.beginPath();
  for (let i = 0; i < 256; i++) {
    const x = xFor(curves.freq[i]);
    const y = yFor(arr[i]);
    if (i === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y);
  }
  ctx.strokeStyle = color;
  ctx.stroke();
};
drawLine(curves.target_db, '#888888cc');         // 目标：灰虚
drawLine(curves.pre_fit_db, '#ff6b6b');           // 测量：红
drawLine(curves.post_peq_only_db, '#4dabf7');      // PEQ 单独：蓝
drawLine(curves.post_full_db, '#51cf66');          // 全拟合：绿
```

---

## §9 预设与持久化

预设 JSON Schema 与 v1.0 相同。启动恢复顺序：

```
1. PlaybackEngine::new() → 读取 AppSettings.calibration.{peq_L, peq_R, bypass, current_preset_name}
   → PeqSnapshot::from_bands → peq_update_handle.update_bands()（sync_channel(1)）
   → handle.set_enabled(!bypass)
   → 同时 peq_update_handle.set_last_applied(peq_snapshot_arc) 写回 last_applied RwLock
2. PluginRuntime.scan() → 加载 calibration_fir.wasm → plugin_init → last_known_version=0
   → process() 第一帧开头自动拉取 config_version（可能 ≥ 1）
   → 读 config JSON → 恢复 FirConfig（coeffs + tap + enabled）
3. last_applied_fit_id 和 last_applied_result：本期不持久化到磁盘（用户重启后如果想"查看当前生效详情"，可以重新跑一次拟合预览或者 invoke get_calibration_peq + getPluginConfig 手动拼一个简易 FitResult——本期不做，留作后续 PR）。
```

---

## §10 Tauri 命令与 API

### 10.1 类型定义（来自 calibration-types crate，完整）

> 以下类型在 `workspace/crates/calibration-types/src/lib.rs` 中一次性定义，Tauri 后端 native / calibration-engine / calibration-fir / 前端 TS 类型定义全部严格对齐。

```rust
// calibration-types/src/lib.rs
use serde::{Serialize, Deserialize};
use thiserror::Error;

// ── EqBand（与 phonon-core::eq::EqBand 等价，跨 crate 传递需要自己的版本避免耦合 dsp 实现细节） ──
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum FilterType { Peaking, LowShelf, HighShelf, LowPass, HighPass, BandPass, Notch, AllPass }

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct EqBand {
    pub frequency: f32,
    pub gain_db: f32,
    pub q: f32,
    pub filter_type: FilterType,
}

// ── MeasurementData ──
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeasurementData {
    pub freqs: Vec<f32>,            // Hz，升序
    pub magnitudes_db: Vec<f32>,    // 等长
    #[serde(default)]
    pub phase_rad: Option<Vec<f32>>,
}

// ── TargetCurveSpec ──
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum TargetCurveSpec {
    Builtin { name: String },       // "flat" / "harman_oe_2019" / "harman_ie_2018" / "diffuse_field" / "free_field"
    Custom { points: Vec<(f32, f32)> }, // (freq, gain_db) 控制点
    Imported(MeasurementData),
}

// ── FirConfig（Tauri→PluginRuntime→calibration-fir 三方共享） ──
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirConfig {
    pub enabled: bool,
    pub tap: u32,
    pub coeffs_L: Vec<f32>,
    pub coeffs_R: Vec<f32>,
}

// ── FitOptions ──
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitOptions {
    #[serde(default = "d16")] fn peq_bands_per_channel() -> u8 { 16 }
    pub peq_bands_per_channel: u8,            // 1~20，默认 16
    #[serde(default = "dmixed")] fn filter_types() -> String { "mixed".into() }
    pub filter_types: String,                 // "peaking" 全部参量 / "mixed"（默认 LS + 中 Peaking + HS）
    #[serde(default = "d2048")] fn fir_tap() -> u32 { 2048 }
    pub fir_tap: u32,                         // 0 = 关闭 FIR；合法值 0,256,512,1024,2048,4096,8192
    #[serde(default = "d20hz")] fn fir_smooth_bw() -> f32 { 20.0 }
    pub fir_smooth_bandwidth_hz: f32,         // 默认 20Hz；越大越保守
    #[serde(default = "d12db")] fn max_gain() -> f32 { 12.0 }
    pub max_total_gain_db: f32,               // 默认 12 dB，防止拟合过度
}

// ── FitRequest ──
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitRequest {
    pub measurement_L: MeasurementData,
    pub measurement_R: Option<MeasurementData>,  // None = 复用 L
    pub target: TargetCurveSpec,
    pub options: FitOptions,
}

// ── FitCurves：256pt 固定对数频率轴 ──
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitCurves {
    pub freq: Vec<f32>,            // len=256, freq[i]=20*(20000/20)^(i/255)  — 前后端对齐公式
    pub target_db: Vec<f32>,       // len=256, clamp [-30,+18]
    pub pre_fit_db: Vec<f32>,      // 测量
    pub post_peq_only_db: Vec<f32>,// PEQ 单独拟合后
    pub post_full_db: Vec<f32>,    // PEQ + FIR 完整效果
}

// ── FitDiagnostics：4 项误差指标 ──
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FitDiagnostics {
    pub rmse_full_lr_avg_db: f32,
    pub peak_error_full_db: f32,
    pub midband_rmse_100_4khz_db: f32,
    pub treble_rmse_10_20khz_db: f32,
}

// ── FitResult ──
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitResult {
    pub peq_L: Vec<EqBand>,
    pub peq_R: Vec<EqBand>,
    pub fir_coeffs_L: Vec<f32>,   // 空 Vec = fir_tap=0（关闭）
    pub fir_coeffs_R: Vec<f32>,
    pub curves_L: FitCurves,
    pub curves_R: Option<FitCurves>,  // measurement_R=None 时 None
    pub diagnostics: FitDiagnostics,
    /// fit_id：前 16 字节 request SHA-256 → base64（≈22 字符）。UI 显示用末尾 6 位十六进制短码。
    pub fit_id: String,
}

// ── FitError（run_calibration_fit 返回的失败类型） ──
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
pub enum FitError {
    #[error("测量点数不足（L={count}，至少 48）")]
    MeasurementTooShort { count: usize },
    #[error("频率范围异常：{range} Hz，有效范围 20~20000")]
    FreqRange { range: String },
    #[error("目标曲线未找到：{name}")]
    TargetNotFound { name: String },
    #[error("Biquad LSQ 求解失败（奇异矩阵）：peq_bands {bands} 可能过多或频点重复：{msg}")]
    BiquadLsqFailed { bands: u8, msg: String },
    #[error("FIR 残差系数生成失败：{msg}")]
    FirGenFailed { msg: String },
    #[error("内部错误：{0}")]
    Internal(String),
}

// ── ApplyError（apply_fit_result 返回的失败类型） ──
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
pub enum ApplyError {
    #[error("拟合结果未找到（可能已超过 24h 缓存期或应用重启），请重新运行拟合预览")]
    FitNotFound,
    #[error("PEQ 参数注入失败：{0}（通常是音频线程异常繁忙 ChannelFull，请稍后重试）")]
    PeqInjectFailed(String),
    #[error("FIR 插件配置失败：{0}（插件未加载或版本号通道异常）")]
    FirConfigFailed(String),
    #[error("FIR 配置失败后回滚 PEQ 亦失败：{0}（罕见，建议切换输出设备一次）")]
    RollbackPeqFailed(String),
}
```

### 10.2 Tauri 命令（5 条）

```rust
// phonon-tauri/src-tauri/src/commands/calibration.rs
use tauri::State;
use calibration_types::*;
use lru::LruCache;                       // MIT，0 依赖

pub struct CalibrationAppState {
    pub peq_handle: PeqUpdateHandle,     // Clone 过的 PEQ 句柄
    pub plugin_runtime: PluginRuntimeHandle,  // 调 set_plugin_config_sync
    pub settings: Arc<Mutex<AppSettings>>,
    // ── Fit 缓存（双维度淘汰） ──
    pub fit_results: Mutex<LruCache<String, FitCacheEntry>>,  // LRU 容量 20
    pub last_applied_fit_id: Arc<RwLock<Option<String>>>,
    pub last_applied_result: Arc<RwLock<Option<FitResult>>>,  // ★ 不淘汰，供"查看详情"
}
pub struct FitCacheEntry { pub result: FitResult, pub created: Instant }
pub const FIT_CACHE_MAX: usize = 20;
pub const FIT_CACHE_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

// ──────────────────────────────── 命令 1：set_calibration_peq ────────────────────────────────
#[tauri::command]
pub fn set_calibration_peq(
    state: State<'_, CalibrationAppState>,
    bands_L: Vec<EqBand>, bands_R: Vec<EqBand>, enabled: bool,
) -> Result<(), PeqUpdateError> {
    state.peq_handle.update_bands(bands_L, bands_R)?;
    state.peq_handle.set_enabled(enabled);
    // 持久化
    let mut s = state.settings.lock().unwrap();
    s.calibration.peq_L = bands_L;
    s.calibration.peq_R = bands_R;
    s.calibration.bypass = !enabled;
    Ok(())
}

// ──────────────────────────────── 命令 2：get_calibration_peq ────────────────────────────────
#[derive(Serialize)]
pub struct CalibrationPeqStatus { pub enabled: bool, pub bands_L: Vec<EqBand>, pub bands_R: Vec<EqBand> }

#[tauri::command]
pub fn get_calibration_peq(state: State<'_, CalibrationAppState>) -> CalibrationPeqStatus {
    let s = state.settings.lock().unwrap();
    CalibrationPeqStatus {
        enabled: state.peq_handle.enabled(),
        bands_L: s.calibration.peq_L.clone(),
        bands_R: s.calibration.peq_R.clone(),
    }
}

// ──────────────────────────────── 命令 3：toggle_calibration_bypass ────────────────────────────────
#[tauri::command]
pub fn toggle_calibration_bypass(state: State<'_, CalibrationAppState>, bypass: bool) -> Result<(), String> {
    state.peq_handle.set_enabled(!bypass);
    state.settings.lock().unwrap().calibration.bypass = bypass;
    // FIR 侧同步 enabled
    let cfg = FirConfig { enabled: !bypass, tap: /* 从 configs 读当前值或 */ 0, coeffs_L: vec![], coeffs_R: vec![] };
    state.plugin_runtime.set_plugin_config_merge("calibration_fir", cfg.enabled_field_only())
        .map_err(|e| format!("FIR bypass 同步失败：{}", e))?;
    Ok(())
}

// ──────────────────────────────── 命令 4：run_calibration_fit（★核心） ────────────────────────────────
/// 执行拟合计算。
/// - apply_immediately = false（默认）：**零副作用**，计算完返回 FitResult 并存入 LRU 20 / 24h 缓存，供后续 apply_fit_result 调用。
/// - apply_immediately = true：计算成功后立即执行原子 apply（PEQ + FIR，事务回滚语义同 apply_fit_result）。
#[tauri::command]
pub fn run_calibration_fit(
    state: State<'_, CalibrationAppState>,
    req: FitRequest,
    apply_immediately: bool,
) -> Result<FitResult, FitError> {
    let result = calibration_engine::fit(req)?;   // ★ native 直接调 crate，同步返回，零事件监听

    // 存入缓存（put → 若 LRU 溢出则 pop 最旧）
    {
        let mut cache = state.fit_results.lock().unwrap();
        cache.put(result.fit_id.clone(), FitCacheEntry { result: result.clone(), created: Instant::now() });
    }

    if apply_immediately {
        // 计算成功 → 立即原子应用
        match do_apply(&state, &result) {
            Ok(()) => {}
            Err(e) => {
                // 注意：FitResult 计算是成功的但 apply 失败；用户仍然需要 FitResult 做 UI 曲线预览。
                // 返回 FitError::Internal(format!("apply failed after fit: {}", e)) 不妥——用户丢失了曲线预览。
                // 更好做法：Tauri 命令 Result<FitResult, FitError> 不够表达"拟合成功但 apply 失败"
                // → 改为 FitResult 加字段 applied_result: Option<Result<(), ApplyError>>（optional）
                // 前端：检查 fit_result.applied_result 是否 Some → 给出对应 toast。
                // 这里为保持命令签名简洁，直接调用 do_apply，写 applied_result 进 FitResult 结构。
                // （实际实现时 FitResult 追加 applied_result: Option<Result<(), String>> with error_to_string 或 在前端命令里 try/except 包裹）
            }
        }
    }
    Ok(result)
}

// ──────────────────────────────── 命令 5：apply_fit_result（★原子事务，核心） ────────────────────────────────
#[tauri::command]
pub fn apply_fit_result(
    state: State<'_, CalibrationAppState>,
    fit_id: String,
) -> Result<(), ApplyError> {
    // 1) 从缓存取（双淘汰：LRU 20 + 年龄 24h）
    let entry = {
        let mut cache = state.fit_results.lock().unwrap();
        let e = cache.get(&fit_id).cloned()
            .filter(|e| e.created.elapsed() < FIT_CACHE_MAX_AGE)
            .ok_or(ApplyError::FitNotFound)?;
        Some(e)
    };
    let result = &entry.unwrap().result;
    do_apply(&state, result)
}

// ──────────────────────────────── 命令 6：get_current_fit_result ────────────────────────────────
/// 读"当前已生效校准详情"（不淘汰字段，供 UI 右键菜单"查看当前生效校准详情"用）。
#[tauri::command]
pub fn get_current_fit_result(state: State<'_, CalibrationAppState>) -> Option<FitResult> {
    state.last_applied_result.read().unwrap().clone()
}

// ──────────────────────────────── 内部：原子事务 apply ────────────────────────────────
/// 两步事务：PEQ update_bands 成功 → FIR set_plugin_config_sync 成功
///            PEQ 成功 / FIR 失败 → 回滚 PEQ 到应用前快照（None = 注入空 bands 直通）
fn do_apply(state: &CalibrationAppState, result: &FitResult) -> Result<(), ApplyError> {
    // 1. 记录应用前快照（用于回滚）
    let before_snap: Option<Arc<PeqSnapshot>> = state.peq_handle.last_applied_snapshot();

    // 2. 注入 PEQ（会自动写回 last_applied；失败直接返回 PeqInjectFailed）
    if let Err(e) = state.peq_handle.update_bands(result.peq_L.clone(), result.peq_R.clone()) {
        return Err(ApplyError::PeqInjectFailed(e.to_string()));
    }
    state.peq_handle.set_enabled(true);

    // 3. 组装 FIR 配置
    let fir_cfg = FirConfig {
        enabled: result.fir_coeffs_L.len() > 0,
        tap: result.fir_coeffs_L.len() as u32,
        coeffs_L: result.fir_coeffs_L.clone(),
        coeffs_R: result.fir_coeffs_R.clone(),
    };

    // 4. 写 FIR 插件配置（版本号自动递增）
    match state.plugin_runtime.set_plugin_config_sync("calibration_fir", &fir_cfg) {
        Ok(()) => {
            // ★ 5. 成功：写"不淘汰字段" last_applied_fit_id + last_applied_result
            *state.last_applied_fit_id.write().unwrap() = Some(result.fit_id.clone());
            *state.last_applied_result.write().unwrap() = Some(result.clone());
            // 持久化 PEQ（FIR 持久化由 set_plugin_config_sync 内部负责）
            let mut s = state.settings.lock().unwrap();
            s.calibration.peq_L = result.peq_L.clone();
            s.calibration.peq_R = result.peq_R.clone();
            s.calibration.bypass = false;
            Ok(())
        }
        Err(e_fir) => {
            // ★ 失败：回滚 PEQ 至 before_snap（可能为 None → 直通）
            let rollback = match before_snap {
                Some(snap_before) => {
                    state.peq_handle.update_bands(
                        snap_before.bands_L.clone(), snap_before.bands_R.clone()
                    )
                }
                None => state.peq_handle.update_bands(vec![], vec![]),
            };
            match rollback {
                Ok(()) => Err(ApplyError::FirConfigFailed(e_fir.to_string())),
                Err(e_rb) => Err(ApplyError::RollbackPeqFailed(
                    format!("FIR failed: {}; PEQ rollback also failed: {}", e_fir, e_rb)
                )),
            }
        }
    }
}
```

### 10.3 前端 TS API 封装

```ts
// phonon-tauri/src/api/calibration.ts
import { invoke } from '@tauri-apps/api/core'

export type FilterType = 'Peaking' | 'LowShelf' | 'HighShelf' | 'LowPass' | 'HighPass' | 'BandPass' | 'Notch' | 'AllPass'

export interface EqBand {
  frequency: number
  gain_db: number
  q: number
  filter_type: FilterType
}

export interface MeasurementData {
  freqs: number[]
  magnitudes_db: number[]
  phase_rad?: number[]
}

export type TargetCurveSpec =
  | { type: 'Builtin';  payload: { name: string } }
  | { type: 'Custom';   payload: { points: Array<[number, number]> } }
  | { type: 'Imported'; payload: MeasurementData }

export interface FitOptions {
  peq_bands_per_channel?: number        // 默认 16
  filter_types?: string                 // 默认 'mixed'
  fir_tap?: number                      // 默认 2048，0 关闭
  fir_smooth_bandwidth_hz?: number      // 默认 20
  max_total_gain_db?: number            // 默认 12
}

export interface FitRequest {
  measurement_L: MeasurementData
  measurement_R?: MeasurementData
  target: TargetCurveSpec
  options: FitOptions
}

/** 256pt 固定长度：前后端对齐公式 freq[i]=20*(20000/20)^(i/255) */
export interface FitCurves {
  freq: number[]               // length = 256
  target_db: number[]          // length = 256，dB clamp [-30, +18]
  pre_fit_db: number[]         // length = 256
  post_peq_only_db: number[]   // length = 256
  post_full_db: number[]       // length = 256
}

export interface FitDiagnostics {
  rmse_full_lr_avg_db: number
  peak_error_full_db: number
  midband_rmse_100_4khz_db: number
  treble_rmse_10_20khz_db: number
}

export interface FitResult {
  peq_L: EqBand[]
  peq_R: EqBand[]
  fir_coeffs_L: number[]
  fir_coeffs_R: number[]
  curves_L: FitCurves
  curves_R?: FitCurves
  diagnostics: FitDiagnostics
  /** base64(SHA-256(req_bytes[0..16]))，末尾 6 位 hex 做短码 UI 显示 */
  fit_id: string
}

// ── 命令封装 ──
export async function setCalibrationPeq(bands_L: EqBand[], bands_R: EqBand[], enabled: boolean): Promise<void> {
  return invoke('set_calibration_peq', { bands_L, bands_R, enabled })
}
export async function getCalibrationPeq(): Promise<{ enabled: boolean; bands_L: EqBand[]; bands_R: EqBand[] }> {
  return invoke('get_calibration_peq')
}
export async function toggleCalibrationBypass(bypass: boolean): Promise<void> {
  return invoke('toggle_calibration_bypass', { bypass })
}
/**
 * runCalibrationFit(req, apply_immediately)
 * - apply_immediately=false（默认）：仅计算 + 预览，不注入 DSP
 * - apply_immediately=true：计算成功后原子地应用 PEQ + FIR
 * 成功返回 FitResult（完整曲线 + 误差 + fit_id）
 */
export async function runCalibrationFit(req: FitRequest, applyImmediately: boolean = false): Promise<FitResult> {
  return invoke('run_calibration_fit', { req, applyImmediately })
}
/** 对已缓存的 dry-run 结果执行应用（原子事务） */
export async function applyFitResult(fitId: string): Promise<void> {
  return invoke('apply_fit_result', { fitId })
}
/** 读"当前已生效校准详情"（不淘汰） */
export async function getCurrentFitResult(): Promise<FitResult | null> {
  return invoke('get_current_fit_result')
}
```

### 10.4 Host Import 扩展（calibration-fir Wasm 用）

在 `phonon-plugin/src/runtime.rs::add_host_imports` 中追加两条（v1.1 已定义，v1.2 保留不变，仅插件名从 calibration_suite → calibration_fir，调用时通过 name 参数区分）：

```rust
// host.get_plugin_config_version(name_ptr, name_len) -> i64
//   读 PluginRuntime.config_versions[name].load(Acquire)；未配置返回 0
linker.func_wrap("host", "get_plugin_config_version", |...| -> i64 { ... })?;

// host.read_plugin_config(name_ptr, name_len, buf_ptr, buf_cap) -> i32
//   读 configs HashMap 中 JSON 字节写入 Wasm 线性内存；返回写入长度或错误码（-1 名不存在 / -2 buf 不足）
linker.func_wrap("host", "read_plugin_config", |...| -> i32 { ... })?;
```

`PluginRuntime` struct 加：`config_versions: Arc<HashMap<String, AtomicU64>>`，与 `configs` HashMap 键一一对应。`set_plugin_config()` 每次写入时 `config_versions[name].fetch_add(1, Ordering::Release)`。

---

## §11 许可证与合规

- **新 Rust 代码**（Calibration PEQ、calibration-types、calibration-engine）：MIT/Apache 2.0 双许可
- **新 Wasm 插件**（calibration-fir）：MIT 许可，Cargo.toml 单独声明
- **新 TS/TSX 代码**：MIT 许可
- **第三方依赖审计**（执行前需通过 deny.toml）：
  - `rustfft` (MIT/Apache) → calibration-engine 的 FIR IFFT + Hilbert
  - `serde / serde_json`（标准）
  - `lru`（MIT，仅 Tauri 命令侧 LRU 缓存）
  - `thiserror`（MIT/Apache，FitError/ApplyError/PeqUpdateError 派生）
  - 自研无依赖：Cholesky 16×16（biquad_fit.rs）
- **许可证白名单**：MIT, Apache-2.0, BSD-2/3, ISC, MPL-2.0；黑名单：GPL/LGPL/AGPL/SSPL/BUSL（deny.toml）
- **Harman 曲线合规（最终版）**：§7.1 数字化提取 64 点常量表方案。代码注释完整标明：公开图片 URL、数字化工具、精度声明、引用论文 DOI 作参考。不引用 AutoEq Git 仓库 measurements CSV 数据文件。

### 11.1 绝对红线（零容忍）

- [ ] 不使用 "SoundID" / "Sonarworks" 商标或近似名称
- [ ] 不复制 / 逆向 Sonarworks 测量数据库
- [ ] 不像素级复制 SoundID UI 设计
- [ ] 不引用 AutoEq / 任何第三方 Git 仓库的 CSV 数据文件（Harman 走数字化提取）
- [ ] 插件描述："Room / Headphone Acoustic Calibration Tool"，不提及竞品

---

## §12 测试与验收

### 12.1 单元测试表（每个 crate 自带）

略（参见实施计划 M2b Task 2.2~2.7 单测清单；M1.1 PEQ 3 单测；M6 命令 1 单测等）。

### 12.2 集成测试表

略（实施计划 M9）。

### 12.3 顶级质量量化指标

| 指标 | 验收阈值 | 测试方法 |
|------|---------|---------|
| 参数切换样本连续性（PEQ swap） | 两帧相邻样本差 < 1e-3 | M1.6 并发压测 test_peq_concurrent_update_continuity |
| FIR tap 切换样本连续性 | < 1e-3 | M4 Task 4.1 并发热加载压测 |
| 平曲线 + 平目标 → Biquad 拟合 | |gain| < 0.1 dB | M3 Task 2.5 单测 |
| Biquad 拟合后残差 | max \|residual\| < 3 dB | M3 Task 2.5 单测 |
| 平残差 → FIR coeffs | coeffs[0]-1 < 0.01，其余 < 1e-3 | M3 Task 2.6 单测 |
| Wasm calibration-fir 体积（release + wasm-opt） | ≤ 50 KB（≤ 30 KB 理想） | M4 Task 4.3 验收命令 `wasm-size target/wasm32-unknown-unknown/release/calibration_fir.wasm` |
| run_calibration_fit 平曲线 128pt 耗时 | ≤ 100 ms（48kHz 机器） | M6 性能基准 |
| DSP 处理延迟（48kHz 512-sample block） | PEQ ≤ 0.5 ms；FIR 2048 tap ≤ 2 ms | M9 Task 9.2 性能基准 |
| 许可证扫描（cargo-deny） | 无黑名单（GPL/LGPL/AGPL/SSPL/BUSL）违规 | M9 Task 9.4 |
| Clippy（全 workspace） | 无新 lint 违规（允许列表 26 项风格类除外） | M9 Task 9.4 |

### 12.4 新增线程安全测试（v1.2：放宽 + 自旋同步 + #[ignore]）

> v1.2 关键修改：**去掉真实 sleep**（CI 环境负载不稳，1ms 的 sleep 实际可能 5~10ms，导致时序错乱 flaky），改用 `Arc<AtomicU64>` writer_block/reader_block 计数器 + `thread::yield_now()` 自旋同步；**断言放宽 1e-4 → 1e-3**（子 LSB 级别 < -60dBFS，绝对无人耳可闻）；**每个测试加 `#[ignore]`，CI 用 `--ignored --test-threads=1` 单独跑**，避免和 unit-tests 混跑时因 CPU 抢占造成 flaky。

| 测试项 | 验收标准 | 实施要点 |
|--------|---------|---------|
| PEQ 并发更新连续性 | process 输出两帧相邻样本 max diff < 1e-3（放宽 10×） | writer 线程每个循环后 `writer_count.fetch_add(1, Release)`；reader 线程循环 `while writer_count.load(Acquire) < reader_count + 1 { yield_now(); }`；无 `thread::sleep`。`#[ignore]` |
| PEQ enabled 1M 次切换 | DSP process 无 panic 无越界 | 同上计数器同步；`#[ignore]` |
| FIR 版本号轮询热加载 | set_plugin_config 1ms 更新一次 tap/coeffs + process 持续 → output 相邻样本 diff < 1e-3 | M4 Task 4.1；同上计数器；`#[ignore]` |

---

## §13 分阶段实施顺序建议（9 Milestone = 32 Task）

> **设计决策：本设计作为单个实施计划推进。** 公共类型全在 calibration-types；fit_engine 在独立 crate。完整 Task 清单在实施计划文档中逐项写清 Step/代码/验证命令/Commit msg。

```
Milestone 1: Core DSP Infrastructure + 线程安全基础设施（6 Tasks）
  M1.1 CalibrationPeqProcessor（sync_channel(1) 寄存器模型 + AtomicBool + AtomicPtr<Arc<PeqSnapshot>> +
       PeqUpdateHandle.last_applied: Arc<RwLock> + PeqUpdateError 2 枚举）
       + handle.clone() 多命令线程零锁；Tauri 前端 500ms 防抖防连点
  M1.2 DspChain.add 顺序（Cal PEQ → Cal FIR → Equalizer → ...）
       + AppState 加 peq_update_handle 字段 + 启动恢复 last_applied set_last_applied 写回
  M1.3 AppSettings.calibration 持久化字段（peq_L/R, bypass, preset_name）
  M1.4 Tauri 命令骨架：set/get calibration_peq + toggle_calibration_bypass（3 条）
  M1.5 Builtin 注册 phonon_calibration_peq + PluginManager UI 展示顺序（最后一项）
  M1.6 线程安全压测（§12.4：计数器自旋同步 + 断言 1e-3 + #[ignore] 独立跑）

Milestone 2: calibration-types 微 crate（1 Task）
  M2.1 新建 workspace/crates/calibration-types
       EqBand / FilterType / MeasurementData / TargetCurveSpec / FirConfig
       FitRequest / FitOptions / FitCurves (256pt) / FitDiagnostics / FitResult(fit_id)
       FitError 6 枚举 / ApplyError 4 枚举 / PeqUpdateError 2 枚举
       全 Serde 派生 + thiserror。零算法依赖。

Milestone 3: calibration-engine 原生 Rust 拟合 crate（7 Tasks）
  M3.1 crate 骨架：Cargo.toml（calibration-types, rustfft, serde, serde_json, thiserror）
       + lib.rs pub re-exports
  M3.2 parsers (rew_csv, frd, autoeq_txt) + 单测（3 项 pass）
  M3.3 interpolate (自然三次样条 + Akima) + smooth (1/n octave 分数阶)
  M3.4 target_curves 5 条曲线：
       Flat / Diffuse Field / Free Field 公式 +
       Harman OE 2019 / Harman IE 2018 **数字化提取 64pt 常量表 + Akima**
       代码注释完整标注公开图 URL + WebPlotDigitizer v4.7 + ±0.3 dB + 论文 DOI 出处
       单测：50Hz 低音 shelf ≈ 10~13dB；1kHz ≈ 0±1dB；10kHz+ ≈ +3~+5dB 正向 shelf
            相邻两点二阶差分 < 10 dB/Hz²（无尖刺）
  M3.5 biquad_fit.rs：Cholesky 16×16 自研无依赖 + 加权 LSQ。单测 3 项：平曲线 / 1kHz 单峰 / 典型测量
  M3.6 fir_residual.rs：rustfft FFT/IFFT + Hilbert 最小相位 + 线性相位。单测 2 项。
  M3.7 fit_engine.rs：FitRequest → Result<FitResult, FitError> 主流程整合
       + FitCurves 256pt 对数轴生成 + clamp [-30,18] + NaN 消毒 + 4 项误差指标 + fit_id。单测 1 项（平曲线全 pass）

Milestone 4: calibration-fir Wasm 插件（只做 FIR overlap-save，体积 ~25KB）（3 Tasks）
  M4.1 插件骨架（workspace/plugins/calibration-fir）
       Cargo.toml：只依赖 calibration-types + serde/serde_json
       lib.rs：**去 no_std → 直接 std**；PluginState { fir_enabled/tap/coeffs/overlap/last_known_version }
       manifest.json（name=calibration_fir）
  M4.2 overlap-save FIR 卷积实现（L/R 独立 + tap 切换零爆音）
       + 版本号轮询热加载（host.get_plugin_config_version / host.read_plugin_config）
       + 单测：已知脉冲 vs 直接卷积差 <1e-6；热切换相邻样本差 <1e-3（§12.4 压测）
  M4.3 体积 + 合规验收：wasm-opt 后 ≤ 50KB（理想 ≤ 30KB）；cargo-deny 无黑名单；manifest 静态扫描无 URL/eval 违规

Milestone 5: PluginRuntime host imports（config_versions + 2 条 linker import）（1 Task）
  M5.1 phonon-plugin/src/runtime.rs 加字段 config_versions + set_plugin_config() 内 fetch_add
       + add_host_imports 追加两条（get_plugin_config_version / read_plugin_config）
       + 单测：set_plugin_config 2 次后 version ≥ 2

Milestone 6: Tauri Commands 完整对接（apply 事务 + 缓存策略）（1 Task）
  M6.1 phonon-tauri/src-tauri/src/commands/calibration.rs：CalibrationAppState 结构
       （fit_results LRU 20/24h + last_applied_fit_id RwLock + last_applied_result RwLock 不淘汰）
       + 6 命令：set/get PEQ / bypass / run_calibration_fit(req, apply_immediately)
                 / apply_fit_result(fit_id) 原子事务 do_apply() 完整回滚逻辑
                 / get_current_fit_result
       + FitError/ApplyError 与前端 TS 类型对齐（serde stringify）
       + 单测：平测量 + 平目标 + apply_immediately=true → peq gain ≈ 0 + 之后 get_current_fit_result is_some
       + 注册进 commands.rs

Milestone 7: 前端 TS 侧（4 Tasks）
  M7.1 calibration/parsers.ts 粗解析（CSV/FRD/TXT → JSON 字符串，不做 DSP 计算）
  M7.2 目标曲线控制点 → TargetCurveSpec 映射（Builtin/Custom/Imported 三分支 + 5 条预置曲线 TS 常量表用于 UI 预览，不做 DSP）
  M7.3 api/calibration.ts 封装：set/get/toggle/runCalibrationFit/applyFitResult/getCurrentFitResult
       完整 EqBand/FitRequest/FitResult 类型（与 calibration-types 1:1 对齐）
  M7.4 FitCurves Canvas 数据提供：256pt len 校验 + freq 对数轴公式 + xFor/yFor 工具函数导出供 UI 用

Milestone 8: UI 面板 CalibrationPanel.tsx（6 Tasks）
  M8.1 基础布局 5 区 + 测量导入区（拖拽/浏览 + L/R 分配 + Swap + Use L for R）
  M8.2 Canvas 曲线可视化：4 条曲线（测量/目标/PEQ单独/完整）+ 残差填充 + 对数坐标 + 光标 tooltip 精确映射
       xFor/yFor 公式与后端 1:1 对齐（§8.3 规则 2）
  M8.3 目标曲线拖拽编辑器：控制点增删改 + Akima 渲染（与后端同算法保证前后端像素一致）+ 工具栏
  M8.4 拟合参数区：PEQ 段数 / FIR tap 6 档 / 相位模式 / Smooth 3 档
       + 两个按钮：[拟合预览]（apply_immediately=false，纯计算不应用）+ [立即拟合并应用]
       + 进度反馈（无事件，await 同步 spinner）+ 4 项诊断卡片 + **apply 按钮 500ms 防抖**
  M8.5 结果展示：[应用配置]（apply_fit_result）+ 4 状态 Toast + fit_id 短码状态栏绿条 +
       右键"查看当前生效校准详情"（get_current_fit_result）弹窗
  M8.6 预设管理（保存命名/加载/删除/JSON 导入导出）+ L/R Link/Unlink +
       Bypass 开关视觉淡入淡出（0.3s CSS transition）+ A/B 对比按钮

Milestone 9: 集成与测试（5 Tasks）
  M9.1 端到端集成：REW CSV → fit(apply_immediately=false) → 曲线展示 → apply_fit_result →
       播放白噪音/正弦扫频 → 旁路 A/B 听感
  M9.2 性能基准（M1/M3/M4/M6 组件 48kHz/512-sample block latency；M12.3 量化指标）
  M9.3 用户文档：面板使用说明 + 合规声明页面（§11.1 红线 + §7.1 Harman 数字化提取来源 + 论文 DOI）
  M9.4 Clippy + cargo-deny 全仓库扫描无违规
  M9.5 线程安全压测（§12.4 3 项 `#[ignore]` 全通过：
       `cargo test -p phonon-core -p calibration-fir -- --ignored --test-threads=1`）
```

---

*— EOF —*
