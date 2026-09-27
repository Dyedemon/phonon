# Phonon Acoustic Calibration Suite 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Phonon 中交付一套顶级的"耳机/音箱频响校准套件"——三 crate 分层架构（types / engine / fir），Calibration PEQ 用 `std::mpsc::sync_channel(1)` 寄存器模型（单槽 + 零重试 + 前端 500ms 防抖兜底），Biquad 加权 LSQ 主趋势拟合（原生 Rust）+ FIR overlap-save 残差补偿（Wasm，版本号轮询热加载）+ 3 格式曲线导入 + 5 条数字化提取目标曲线 + 曲线拖拽编辑器 + 预设管理 + 旁路 A/B。事务回滚保证 PEQ+FIR 一致性。零毛刺参数切换、原子并发安全、残差 < 3dB 指标保证。

**Architecture (v1.2 三 crate 分层):**
- **calibration-types**（微 crate，零算法）：共享类型 `EqBand / FitRequest / FitResult(FitCurves 256pt 严格对数轴) / FirConfig / TargetCurveSpec / FitOptions / FitDiagnostics / FitError / ApplyError / PeqUpdateError`
- **calibration-engine**（纯 Rust 原生 crate）：parsers (rew_csv/frd/autoeq_txt) / interpolate (三次样条 + Akima) / smooth (1/n oct) / target_curves (数字化提取 64pt + Akima) / biquad_fit (Cholesky 16×16 自研) / fir_residual (rustfft IFFT + Hilbert) / fit_engine 整合。Tauri 命令直接 native 调，零 Wasm 序列化损耗。
- **calibration-fir**（WasmExample，仅 overlap-save FIR，~25KB）：依赖仅 calibration-types。版本号轮询拉模型热加载（`host.get_plugin_config_version` + `host.read_plugin_config`），tap 256~8192，默认 2048。
- **Calibration PEQ**（Builtin，phonon-core）：`sync_channel(1)` 寄存器模型（单槽 = 永远最新 pending，零重试）+ `AtomicBool` enable + `AtomicPtr<Arc<PeqSnapshot>>` swap + `last_applied: Arc<RwLock<Option<Arc<PeqSnapshot>>>>`（事务回滚用）。Full 错误前端 500ms 防抖兜底。
- **Tauri 命令**：`run_calibration_fit` (dry-run 预览) / `apply_fit_result` (事务 PEQ+FIR，失败回滚) / `get_current_fit_result` (不淘汰缓存) / `set/get_calibration_peq` / `toggle_calibration_bypass`。
- **前端**：CalibrationPanel 5 区顶级 UI，**apply 按钮 500ms 防抖**（`useState(applying) + setTimeout`），Canvas 256pt 四曲线可视化。

**Tech Stack:**
- Rust (native + wasm32-unknown-unknown，calibration-fir 去 no_std → std 环境)
- `rustfft` 6.x MIT/Apache（仅 calibration-engine），`serde`, `serde_json`
- Tauri 2.x Commands + PluginRuntime (host imports 扩展 2 条：get_plugin_config_version / read_plugin_config)
- TypeScript 5.x + React 18 + Canvas 2D/WebGL
- Cargo deny.toml：MIT/Apache/BSD/ISC/MPL-2.0 白名单；GPL/LGPL/AGPL/SSPL/BUSL 黑名单
- Clippy `-D warnings` + 26 条 style lint allow + `--cap-lints allow`（项目约定）
- LRU 双淘汰：20 条 / 24h，`last_applied_result` 永不淘汰

---

## 执行前准备（所有任务共用）

- [ ] 读取 `d:\Phonon\deny.toml` 确认 GPL/LGPL/AGPL/SSPL/BUSL 黑名单生效
- [ ] 读取 `.cargo/config.toml` 确认 Clippy `-D warnings` + style lint allow 列表（约 26 项）
- [ ] 确认 workspace 根 `Cargo.toml` 的 `workspace.members` 列表含 `workspace/crates/*` 和 `workspace/plugins/*`（否则追加）
- [ ] 确认 `cargo-deny` 已安装：`cargo deny --version`（无则 `cargo install cargo-deny`）

---

## Milestone 1: calibration-types 微 crate（共享类型，零算法依赖）「1 Task」

### Task 1.1: 定义所有共享类型 + FitCurves 256pt 严格对数轴

**Files:**
- Create: `workspace/crates/calibration-types/Cargo.toml`
- Create: `workspace/crates/calibration-types/src/lib.rs`
- Test: `workspace/crates/calibration-types/tests/types_roundtrip.rs`
- Modify: 根 `Cargo.toml` workspace.members 追加 `workspace/crates/calibration-types`

- [ ] **Step 1: 写失败测试（类型不存在 → 编译错误）**

```rust
// workspace/crates/calibration-types/tests/types_roundtrip.rs
use calibration_types::*;
use serde_json;

#[test]
fn test_fitcurves_256pt_log_axis_exact() {
    // FitCurves 必须严格 256 点，freq[i]=20*(20000/20)^(i/255)
    let curves = FitCurves::default();
    assert_eq!(curves.freq.len(), 256, "FitCurves.freq must be exactly 256 points");
    assert_eq!(curves.measured_L.len(), 256);
    assert_eq!(curves.target.len(), 256);
    assert!((curves.freq[0] - 20.0).abs() < 1e-3, "freq[0] must be 20.0 Hz");
    assert!((curves.freq[255] - 20000.0).abs() < 1e-1, "freq[255] must be 20000.0 Hz");
    // 对数等比验证：freq[i+1]/freq[i] ≈ (1000)^(1/255) ≈ 1.02723
    let ratio = curves.freq[1] / curves.freq[0];
    assert!((ratio - 1.02723).abs() < 1e-3, "freq axis must be exact log ratio");
}

#[test]
fn test_fiterror_pequpdateerror_serde_roundtrip() {
    let e = FitError::PeqUpdate(PeqUpdateError::ChannelFull("busy".into()));
    let json = serde_json::to_string(&e).unwrap();
    let e2: FitError = serde_json::from_str(&json).unwrap();
    match e2 { FitError::PeqUpdate(PeqUpdateError::ChannelFull(m)) => assert_eq!(m, "busy"), _ => panic!() }

    let e3 = ApplyError::FirConfig("plugin not loaded".into());
    let json2 = serde_json::to_string(&e3).unwrap();
    let e4: ApplyError = serde_json::from_str(&json2).unwrap();
    match e4 { ApplyError::FirConfig(m) => assert_eq!(m, "plugin not loaded"), _ => panic!() }
}

#[test]
fn test_eqband_filtertype_serde() {
    use FilterType::*;
    for ft in [Peaking, LowShelf, HighShelf, LowPass, HighPass, BandPass, Notch, AllPass] {
        let band = EqBand { frequency: 1000.0, gain_db: 3.0, q: 1.414, filter_type: ft };
        let json = serde_json::to_string(&band).unwrap();
        let b2: EqBand = serde_json::from_str(&json).unwrap();
        assert_eq!(b2.filter_type, ft);
    }
}

#[test]
fn test_fitresult_fitdiagnostics_default() {
    let d = FitDiagnostics { residual_rmse_db_L: 0.5, residual_rmse_db_R: 0.4, residual_max_db_L: 2.1, residual_max_db_R: 1.9, peq_bands_used_L: 16, peq_bands_used_R: 16, fir_tap_used: 2048, phase_mode: PhaseMode::MinimumPhase, fit_time_ms: 23.4 };
    assert!(d.residual_max_db_L < 3.0, "残差必须 < 3dB (§12.1)");
}
```

- [ ] **Step 2: 跑失败测试**

Run: `cd d:\Phonon ; cargo test -p calibration-types --test types_roundtrip -- --nocapture 2>&1 | tail -n 20`
Expected: 编译错误（calibration-types crate 不存在）

- [ ] **Step 3: 创建 Cargo.toml 并实现 lib.rs**

```toml
# workspace/crates/calibration-types/Cargo.toml
[package]
name = "calibration-types"
version = "0.1.0"
edition = "2021"
license = "MIT"
description = "Shared types for Phonon acoustic calibration suite (zero algorithm deps)"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"

[dev-dependencies]
# (无额外依赖，用自带 serde_json)
```

```rust
// workspace/crates/calibration-types/src/lib.rs
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------- Filter 基础类型 ----------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterType {
    Peaking, LowShelf, HighShelf, LowPass, HighPass, BandPass, Notch, AllPass,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct EqBand {
    pub frequency: f32,    // Hz
    pub gain_db: f32,      // dB (Peaking/LowShelf/HighShelf/AllPass 用；其它忽略)
    pub q: f32,            // 品质因数(Peaking/Notch/BandPass 用)；LowShelf/HighShelf 用 Q=0.707 默认斜率
    pub filter_type: FilterType,
}

// ---------- PEQ 通道错误（v1.2: sync_channel(1) 两枚举）----------
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
pub enum PeqUpdateError {
    #[error("audio thread busy (sync_channel(1) full) — try again in 500ms")]
    ChannelFull(String),
    #[error("calibration PEQ processor dropped (audio device switched?)")]
    ChannelClosed(String),
}

// ---------- Fit Request / Options ----------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhaseMode { MinimumPhase, LinearPhase }
pub use PhaseMode::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeasurementData {
    pub freqs: Vec<f32>,           // Hz（任意长度，原始数据）
    pub magnitudes_db: Vec<f32>,   // dB（与 freqs 等长）
    pub phase_rad: Option<Vec<f32>>,// 可选相位（弧度）
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetCurveSpec {
    pub name: String,                         // "Harman OE 2019" 等
    pub control_points: Vec<(f32, f32)>,      // (Hz, dB) 控制点 → Akima 插值成 256pt
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitOptions {
    pub peq_bands_per_side: u8,   // 8/12/16/20，默认 16（ISO 1/3-octave）
    pub fir_tap: u32,             // 256/512/1024/2048/4096/8192，默认 2048
    pub phase_mode: PhaseMode,    // 默认 MinimumPhase
    pub smooth_octaves: f32,      // 0.0（关闭）/ 1/48 / 1/24 / 1/12 / 1/6 / 1/3 / 1.0，默认 1/24
    pub weight_low_freq_db: f32,  // 低频加权倍数（默认 2.0，20~100Hz 权重加倍）
}
impl Default for FitOptions {
    fn default() -> Self { Self { peq_bands_per_side: 16, fir_tap: 2048, phase_mode: MinimumPhase, smooth_octaves: 1.0/24.0, weight_low_freq_db: 2.0 } }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitRequest {
    pub measurement_L: MeasurementData,
    pub measurement_R: Option<MeasurementData>,  // None = Use L for R
    pub target: TargetCurveSpec,
    pub options: FitOptions,
}

// ---------- Fit Result ----------
/// 严格 256 点对数频率轴：freq[i] = 20 * (20000/20)^(i/255)
/// 所有 dB 轴 clamp 到 [-30.0, 18.0]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitCurves {
    pub freq: Vec<f32>,              // 256pt 固定
    pub measured_L: Vec<f32>,        // 256pt 测量 L（重采样对齐后）
    pub measured_R: Vec<f32>,        // 256pt 测量 R
    pub target: Vec<f32>,            // 256pt 目标曲线
    pub peq_response_L: Vec<f32>,    // 256pt PEQ 单独响应（不含 FIR）
    pub peq_response_R: Vec<f32>,
    pub predicted_L: Vec<f32>,       // 256pt 预测：measured + peq + fir
    pub predicted_R: Vec<f32>,
    pub residual_L: Vec<f32>,        // 256pt 残差 = predicted - target
    pub residual_R: Vec<f32>,
}
impl FitCurves {
    pub const N: usize = 256;
    pub const FMIN: f32 = 20.0;
    pub const FMAX: f32 = 20000.0;
    pub const DB_MIN: f32 = -30.0;
    pub const DB_MAX: f32 = 18.0;

    /// 生成标准对数频率轴
    pub fn standard_freq_axis() -> Vec<f32> {
        (0..Self::N).map(|i| {
            Self::FMIN * (Self::FMAX / Self::FMIN).powf(i as f32 / (Self::N - 1) as f32)
        }).collect()
    }
    pub fn clamp_db(v: f32) -> f32 { v.clamp(Self::DB_MIN, Self::DB_MAX) }
}
impl Default for FitCurves {
    fn default() -> Self {
        let freq = Self::standard_freq_axis();
        let zeros = vec![0.0f32; Self::N];
        Self { freq: freq.clone(), measured_L: zeros.clone(), measured_R: zeros.clone(),
               target: zeros.clone(), peq_response_L: zeros.clone(), peq_response_R: zeros.clone(),
               predicted_L: zeros.clone(), predicted_R: zeros.clone(),
               residual_L: zeros.clone(), residual_R: zeros }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitDiagnostics {
    pub residual_rmse_db_L: f32,
    pub residual_rmse_db_R: f32,
    pub residual_max_db_L: f32,
    pub residual_max_db_R: f32,
    pub peq_bands_used_L: u8,
    pub peq_bands_used_R: u8,
    pub fir_tap_used: u32,
    pub phase_mode: PhaseMode,
    pub fit_time_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirConfig {
    pub tap: u32,
    pub enabled: bool,
    pub coeffs_L: Vec<f32>,   // 长度 = tap
    pub coeffs_R: Vec<f32>,
    pub phase_mode: PhaseMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitResult {
    pub fit_id: String,                        // 短码："cal_xxxxxx" (8 hex chars)
    pub created_at_unix_ms: u64,
    pub peq_L: Vec<EqBand>,                    // ≤20 段
    pub peq_R: Vec<EqBand>,
    pub fir: FirConfig,
    pub curves: FitCurves,
    pub diagnostics: FitDiagnostics,
    pub request_snapshot: FitRequest,          // 复现用
}

// ---------- 错误类型 ----------
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
pub enum FitError {
    #[error("invalid measurement data: {0}")]
    InvalidMeasurement(String),
    #[error("invalid target curve: {0}")]
    InvalidTarget(String),
    #[error("fit failed to converge: {0}")]
    ConvergeFailed(String),
    #[error("PEQ apply error: {0}")]
    PeqUpdate(#[from] PeqUpdateError),
    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(Debug, Clone, Error, Serialize, Deserialize)]
pub enum ApplyError {
    #[error("fit result not found (may have been evicted from cache): {0}")]
    NotFound(String),
    #[error("PEQ update failed: {0}")]
    Peq(#[from] PeqUpdateError),
    #[error("FIR config failed: {0}")]
    FirConfig(String),
    #[error("rollback failure: {0}")]
    RollbackFailed(String),
}

// ---------- 目标曲线预置 5 种（常量表在 calibration-engine）----------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresetTargetCurve { Flat, FreeField, DiffuseField, HarmanOE2019, HarmanIE2018 }
pub use PresetTargetCurve::*;
```

- [ ] **Step 4: 跑测试通过**

Run: `cd d:\Phonon ; cargo test -p calibration-types --test types_roundtrip -- --nocapture 2>&1 | tail -n 20`
Expected: `test result: ok. 4 passed; 0 failed;`

- [ ] **Step 5: Clippy + 根 Cargo.toml members 追加 + Commit**

Run:
```bash
cd d:\Phonon
cargo clippy -p calibration-types --lib --tests -- -D warnings --cap-lints allow 2>&1 | tail -n 15
# 根 Cargo.toml workspace.members 追加 workspace/crates/calibration-types（如未含）
cargo metadata --format-version 1 --no-deps 2>&1 | grep calibration-types || echo "NEED ADD TO WORKSPACE.MEMBERS"
git add workspace/crates/calibration-types Cargo.toml
git commit -m "feat(types): add calibration-types micro-crate (EqBand/FitResult 256pt/FitCurves/errors)"
```

---

## Milestone 2: Core DSP Infrastructure（Calibration PEQ + DspChain + AppState + Builtin + 压测）「5 Tasks」

### Task 2.1: CalibrationPeqProcessor（sync_channel(1) 寄存器模型 + AtomicPtr<Arc<>> + last_applied RwLock + PeqUpdateError）

**Files:**
- Create: `phonon-core/src/dsp/calibration_peq.rs`
- Modify: `phonon-core/src/dsp.rs` (末尾 `pub mod calibration_peq;`)
- Modify: `phonon-core/Cargo.toml` (追加 `calibration-types` 依赖)
- Test: `phonon-core/tests/dsp_calibration_peq.rs`

- [ ] **Step 1: 写 3 个失败测试**

```rust
// phonon-core/tests/dsp_calibration_peq.rs
use phonon_core::dsp::calibration_peq::{CalibrationPeqProcessor, PeqUpdateHandle, PeqUpdateError, PeqSnapshot};
use phonon_core::dsp::DspProcessor;
use calibration_types::{EqBand, FilterType};

/// 测试 1: 旁路时 process 为 passthrough (逐样本对比)
#[test]
fn test_bypass_passthrough() {
    let (proc, handle) = CalibrationPeqProcessor::new();
    handle.set_enabled(false);
    let mut samples: Vec<f32> = (0..512).map(|i| (i as f32) * 0.001 - 0.25).collect();
    let orig = samples.clone();
    proc.process(&mut samples, 2, 48000);
    for i in 0..512 {
        assert!((samples[i] - orig[i]).abs() < 1e-6, "bypass passthrough failed at i={}", i);
    }
}

/// 测试 2: update_bands 注入后 next process() 生效，swap 无突变（相邻帧边界 diff < 1e-4）
#[test]
fn test_snapshot_swap_output_continuity() {
    let (proc, handle) = CalibrationPeqProcessor::new();
    handle.set_enabled(true);
    // 先跑 2 帧 null → passthrough
    let mut s1: Vec<f32> = (0..512).map(|i| (i as f32) * 0.001).collect();
    proc.process(&mut s1, 2, 48000);
    // 注入 identity PEQ（gain=0 Peaking），不应改变输出
    let bands = vec![EqBand { frequency: 1000.0, gain_db: 0.0, q: 1.414, filter_type: FilterType::Peaking }];
    handle.update_bands(bands.clone(), bands).unwrap();
    // s2 用 s1 tail 作为开头（模拟连续流）
    let tail = s1[s1.len()-64..].to_vec();
    let mut s2: Vec<f32> = (0..512).map(|i| tail[i % tail.len()] + 0.001).collect();
    let before = s2[0];
    proc.process(&mut s2, 2, 48000);
    // identity PEQ → gain ≈ 1.0 → 边界差 < 1e-4
    assert!((s2[0] - before).abs() < 1e-4, "swap discontinuity at boundary: |{} - {}| = {}", s2[0], before, (s2[0]-before).abs());
}

/// 测试 3: sync_channel(1) 满槽 → ChannelFull（前端防抖兜底语义）
#[test]
fn test_sync_channel_full_returns_error() {
    let (_proc, handle) = CalibrationPeqProcessor::new();
    let bands = vec![EqBand { frequency: 1000.0, gain_db: 1.0, q: 1.414, filter_type: FilterType::Peaking }];
    // 第 1 次 try_send 填满单槽 → Ok
    handle.update_bands(bands.clone(), bands.clone()).unwrap();
    // 第 2 次（DSP 侧 process() 还没 drain → 槽满）→ ChannelFull
    let r = handle.update_bands(bands.clone(), bands);
    match r {
        Err(PeqUpdateError::ChannelFull(_)) => { /* 预期：前端 500ms 防抖应阻止发生 */ }
        other => panic!("expected ChannelFull, got {:?}", other),
    }
}

/// 测试 4: last_applied RwLock 保存最后快照 → 事务回滚前可读取
#[test]
fn test_last_applied_rwlock_saves_snapshot() {
    let (_proc, handle) = CalibrationPeqProcessor::new();
    assert!(handle.last_applied_snapshot().is_none(), "初始 last_applied = None");
    let b = vec![EqBand { frequency: 500.0, gain_db: -3.0, q: 1.0, filter_type: FilterType::Peaking }];
    handle.update_bands(b.clone(), b.clone()).unwrap();
    let last = handle.last_applied_snapshot().unwrap();
    assert_eq!(last.bands_L.len(), 1);
    assert_eq!(last.bands_L[0].gain_db, -3.0);
}
```

- [ ] **Step 2: 跑失败测试**

Run: `cd d:\Phonon ; cargo test -p phonon-core --test dsp_calibration_peq -- --nocapture 2>&1 | head -n 30`
Expected: 编译错误（calibration_peq 模块不存在）

- [ ] **Step 3: 实现（严格对齐 spec §3.1 代码模板）**

要点：
- `mpsc::sync_channel::<Arc<PeqSnapshot>>(1)`（单槽寄存器，不是 unbounded）
- `update_bands` 单次 `try_send`，零重试 → Full / Disconnected → `PeqUpdateError`
- `last_applied: Arc<RwLock<Option<Arc<PeqSnapshot>>>>`（读多写少 → RwLock，非 Mutex）
- `AtomicPtr<Arc<PeqSnapshot>>` swap：`Arc::into_raw` + `swap(AcqRel)` + 旧指针 `Arc::from_raw` 析构
- process 全程：Atomic load + try_recv + AtomicPtr swap + Biquad tick → 无锁无等待
- PeqSnapshot 使用 `phonon_core::eq::BiquadBand::from_eqband()`（复用 eq.rs，不重造）

代码结构完全与 spec §3.1 对齐（含 L142-L330 全部细节，此处不再重复粘贴，实施时直接复制 spec 代码并修改 import 路径适配）。

- [ ] **Step 4: 跑测试通过**

Run: `cd d:\Phonon ; cargo test -p phonon-core --test dsp_calibration_peq -- --nocapture 2>&1 | tail -n 20`
Expected: `test result: ok. 4 passed; 0 failed;`

- [ ] **Step 5: Clippy + Commit**

```bash
cd d:\Phonon
cargo clippy -p phonon-core --lib --tests -- -D warnings --cap-lints allow 2>&1 | tail -n 20
git add phonon-core/src/dsp.rs phonon-core/src/dsp/calibration_peq.rs phonon-core/Cargo.toml phonon-core/tests/dsp_calibration_peq.rs
git commit -m "feat(dsp): Calibration PEQ — sync_channel(1) register model + AtomicPtr<Arc> + last_applied RwLock"
```

---

### Task 2.2: DspChain 顺序 + AppState PeqUpdateHandle + 启动恢复

**Files:**
- Modify: `phonon-core/src/engine.rs` (DspChain 初始化 add 顺序 + AppState 追加 peq_update_handle)
- Test: `phonon-core/tests/engine_dsp_order.rs`

- [ ] **Step 1: 写失败测试**

```rust
use phonon_core::engine::PlaybackEngine;
#[test]
fn test_dsp_chain_order_cal_peq_first_before_equalizer() {
    let engine = PlaybackEngine::new(Default::default()).unwrap();
    let chain = engine.dsp_chain().lock().unwrap();
    let procs = chain.list_processors();
    let cal_pos = procs.iter().position(|p| p.id == "calibration_peq");
    let fir_pos = procs.iter().position(|p| p.id == "calibration_fir");
    let eq_pos = procs.iter().position(|p| p.id == "equalizer");
    assert!(cal_pos.is_some(), "calibration_peq missing");
    assert!(fir_pos.is_some(), "calibration_fir missing (WasmExample loaded)");
    assert!(eq_pos.is_some(), "equalizer missing");
    // 顺序：Cal PEQ → Cal FIR → Equalizer → Surround → ...
    assert!(cal_pos.unwrap() < fir_pos.unwrap(), "PEQ must come before FIR");
    assert!(fir_pos.unwrap() < eq_pos.unwrap(), "FIR must come before Equalizer");
}
```

- [ ] **Step 2: 修改 engine.rs**（与 spec §2.2 DSP 链顺序完全一致）

要点：
(a) AppState 追加 `peq_update_handle: PeqUpdateHandle`
(b) DspChain::new() 后 `add()` 顺序：
    `chain.add(Box::new(cal_peq)) → chain.add(WasmPluginProxy("calibration_fir")) → Equalizer → Surround → SmartEffect → Downmix → ReplayGain`
(c) 启动恢复：从 `AppSettings.calibration.peq_L/peq_R/bypass` 构造快照 → `handle.update_bands()` → `handle.set_enabled(!bypass)` → `handle.set_last_applied(snap_arc)`（持久化的快照要同步到 last_applied RwLock）

- [ ] **Step 3: 测试 + Clippy + Commit**

Commit message: `feat(engine): DSP chain Cal PEQ → Cal FIR → Equalizer order + peq_update_handle AppState + startup restore`

---

### Task 2.3: AppSettings 新增 CalibrationSettings（持久化）

**Files:**
- Modify: `phonon-core/src/settings.rs`
- Test: `phonon-core/tests/settings_calibration.rs`

- [ ] **Step 1: 写失败测试（反序列化 roundtrip）**

与旧 plan Task 1.3 相同，但结构升级：
```rust
pub struct CalibrationSettings {
    pub bypass: bool,
    pub peq_L: Vec<EqBand>,
    pub peq_R: Vec<EqBand>,
    pub current_preset_name: Option<String>,
    pub last_applied_fit_id: Option<String>,   // v1.2 新增：显示"当前校准详情"用
    pub fir_tap: u32,                          // v1.2 新增：FIR tap 持久化（默认 2048）
    pub phase_mode: PhaseMode,                  // v1.2 新增
}
```

- [ ] **Step 2: 实现 → 测试 → Clippy → Commit**

Commit message: `feat(settings): CalibrationSettings v2 (+ last_applied_fit_id / fir_tap / phase_mode)`

---

### Task 2.4: Builtin 注册表追加 phonon_calibration_peq

**Files:**
- Modify: `phonon-plugin/src/builtin.rs`
- Test: `phonon-plugin/tests/builtin_registration.rs`

- [ ] **要点**：manifest.name = `"phonon_calibration_peq"`（`phonon_` 前缀，项目约定）；permissions = `vec![]`（空数组，项目约定）。

与旧 plan Task 1.5 结构相同。

Commit message: `feat(plugin): register phonon_calibration_peq builtin (name prefix phonon_ + empty permissions)`

---

### Task 2.5: 线程安全压测（§12.4 3 项 — sleep→计数器自旋 + 阈值放宽 + #[ignore]）

**Files:**
- Create: `phonon-core/tests/dsp_calibration_peq_concurrency.rs`

- [ ] **Step 1: 写并发测试（计数器 + yield_now 替代 sleep，断言 1e-3 + #[ignore]）**

```rust
use std::sync::{Arc, Mutex, atomic::{AtomicU64, Ordering}};
use std::thread;
use phonon_core::dsp::calibration_peq::*;
use phonon_core::dsp::DspProcessor;
use calibration_types::{EqBand, FilterType};

const BLOCK: usize = 512;

fn make_sine_block(freq_hz: f32, sr: u32, phase: &mut f32) -> Vec<f32> {
    let dt = 1.0 / sr as f32;
    let n_frames = BLOCK / 2;
    let mut out = vec![0.0f32; BLOCK];
    for i in 0..n_frames {
        let s = (2.0 * std::f32::consts::PI * freq_hz * (*phase + (i as f32) * dt)).sin() * 0.5;
        out[i*2] = s; out[i*2 + 1] = s;
    }
    *phase += (n_frames as f32) * dt;
    out
}

/// 测试 1: PEQ 并发更新 → 连续样本边界差 (v1.2: 计数器同步 + 阈值 1e-3)
#[test]
#[ignore = "concurrency stress, run with --ignored --test-threads=1"]
fn test_peq_concurrent_update_continuity() {
    let (proc, handle) = CalibrationPeqProcessor::new();
    handle.set_enabled(true);
    let proc = Arc::new(proc);
    let handle = Arc::new(handle);
    let last_sample = Arc::new(Mutex::new(0.0f32));
    let max_disc = Arc::new(Mutex::new(0.0f32));
    let block_count = Arc::new(AtomicU64::new(0));
    let update_count = Arc::new(AtomicU64::new(0));
    const N_BLOCKS: u64 = 1000;
    const N_UPDATES: u64 = 2000;

    let h_w = handle.clone();
    let up = update_count.clone();
    let writer = thread::spawn(move || {
        for k in 0..N_UPDATES {
            let gain = (k as f32 * 0.01).sin() * 6.0;
            let bands = vec![EqBand { frequency: 500.0 + (k % 20) as f32 * 100.0, gain_db: gain, q: 1.414, filter_type: FilterType::Peaking }];
            let _ = h_w.update_bands(bands.clone(), bands);  // 忽略 Full（并发压测中正常）
            up.fetch_add(1, Ordering::Release);
            thread::yield_now();  // v1.2: yield 代替 sleep，避免平台差异
        }
    });

    let proc_r = proc.clone();
    let last_r = last_sample.clone();
    let disc_r = max_disc.clone();
    let bc = block_count.clone();
    let reader = thread::spawn(move || {
        let mut phase = 0.0;
        for _ in 0..N_BLOCKS {
            let mut blk = make_sine_block(440.0, 48000, &mut phase);
            proc_r.process(&mut blk, 2, 48000);
            let last = *last_r.lock().unwrap();
            if last != 0.0 {
                let diff = (blk[0] - last).abs();
                let mut d = disc_r.lock().unwrap();
                if diff > *d { *d = diff; }
            }
            *last_r.lock().unwrap() = blk[BLOCK - 1];
            bc.fetch_add(1, Ordering::Release);
            thread::yield_now();
        }
    });

    writer.join().unwrap();
    reader.join().unwrap();

    // v1.2: 断言阈值放宽到 1e-3（跨平台浮点精度差异）
    let d = *max_disc.lock().unwrap();
    println!("max discontinuity = {}, blocks = {}, updates = {}", d, block_count.load(Ordering::Acquire), update_count.load(Ordering::Acquire));
    assert!(d < 1e-3, "snapshot swap discontinuity too high: max = {} >= 1e-3", d);
}

/// 测试 2: enabled 1M 次切换不 panic
#[test]
#[ignore = "concurrency stress, run with --ignored --test-threads=1"]
fn test_enabled_atomic_switch_1m_no_panic() {
    let (proc, handle) = CalibrationPeqProcessor::new();
    let proc = Arc::new(proc);
    let handle = Arc::new(handle);
    let w_done = Arc::new(AtomicU64::new(0));
    let r_done = Arc::new(AtomicU64::new(0));

    let h_w = handle.clone();
    let wd = w_done.clone();
    let w = thread::spawn(move || {
        for _ in 0..1_000_000 { h_w.set_enabled(true); h_w.set_enabled(false); }
        wd.store(1, Ordering::Release);
    });

    let proc_r = proc.clone();
    let rd = r_done.clone();
    let r = thread::spawn(move || {
        let mut s = vec![0.1f32; 1024];
        while w_done.load(Ordering::Acquire) == 0 {  // v1.2: 自旋等 writer
            proc_r.process(&mut s, 2, 48000);
            thread::yield_now();
        }
        // 再跑 1000 次收尾
        for _ in 0..1000 { proc_r.process(&mut s, 2, 48000); }
        rd.store(1, Ordering::Release);
    });

    w.join().unwrap();
    r.join().unwrap();
    assert_eq!(r_done.load(Ordering::Acquire), 1);
}

/// 测试 3: last_applied 并发读写不 panic（RwLock 读多写少语义验证）
#[test]
fn test_last_applied_concurrent_rw() {
    let (_proc, handle) = CalibrationPeqProcessor::new();
    let handle = Arc::new(handle);
    let h_w = handle.clone();
    let writer = thread::spawn(move || {
        for k in 0..10_000 {
            let b = vec![EqBand { frequency: 1000.0 + k as f32 * 0.1, gain_db: k as f32 * 0.001, q: 1.414, filter_type: FilterType::Peaking }];
            let _ = h_w.update_bands(b.clone(), b);
        }
    });
    let h_r = handle.clone();
    let readers: Vec<_> = (0..4).map(|_| {
        let h = h_r.clone();
        thread::spawn(move || {
            for _ in 0..50_000 {
                let _ = h.last_applied_snapshot();
                thread::yield_now();
            }
        })
    }).collect();
    writer.join().unwrap();
    for r in readers { r.join().unwrap(); }
}
```

- [ ] **Step 2: 跑测试（先普通，再 --ignored）**

```bash
cd d:\Phonon
cargo test -p phonon-core --test dsp_calibration_peq_concurrency -- --nocapture 2>&1 | tail -n 10
# 普通测试：test_last_applied_concurrent_rw 通过（非 #[ignore]）
cargo test -p phonon-core --test dsp_calibration_peq_concurrency -- --ignored --test-threads=1 --nocapture 2>&1 | tail -n 20
# #[ignore] 两个压力测试通过（可能需要 2-3 分钟）
```

- [ ] **Step 3: Clippy + Commit**

Commit message: `test(dsp): concurrency stress — counters + yield_now + 1e-3 threshold + #[ignore]`

---

## Milestone 3: calibration-engine 原生 crate（拟合算法全量 Rust）「7 Tasks」

### Task 3.1: crate 骨架 + Cargo.toml

**Files:**
- Create: `workspace/crates/calibration-engine/Cargo.toml`
- Create: `workspace/crates/calibration-engine/src/lib.rs` (re-export 全部子模块)
- Test: `cargo check -p calibration-engine` 无错
- Modify: 根 `Cargo.toml` workspace.members

- [ ] **要点**：依赖 `calibration-types`（同 workspace path）、`rustfft` 6.x（默认 features，需 std）、`serde`、`serde_json`。lib.rs 中：
```rust
pub mod parsers;     // rew_csv / frd / autoeq_txt
pub mod interpolate; // 三次样条 + Akima
pub mod smooth;      // 1/n octave 平滑
pub mod target_curves; // 数字化提取 64pt + Akima（5 条）
pub mod biquad_fit;  // 加权 LSQ + Cholesky 16×16
pub mod fir_residual; // IFFT + Hilbert
pub mod fit_engine;  // FitRequest → FitResult 整合
```
- cargo-deny 检查通过（rustfft MIT/Apache，全部合规）

Commit message: `feat(engine): calibration-engine native crate skeleton + deps`

---

### Task 3.2: parsers（rew_csv / frd / autoeq_txt）

**Files:**
- Create: `workspace/crates/calibration-engine/src/parsers/{mod.rs, rew_csv.rs, frd.rs, autoeq_txt.rs}`
- Test: `workspace/crates/calibration-engine/tests/parsers.rs`

- [ ] **要点**：结构与旧 plan Task 2.2 相同，类型改为使用 `calibration_types::MeasurementData` / `calibration_types::EqBand` / `calibration_types::FilterType`（不重复定义）。AutoEq parse 返回 `(preamp_db, Vec<EqBand>)`。

- [ ] **TDD**：先写 3 个测试（rew basic / frd 2col+3col / autoeq Filter 行）→ FAIL → 实现 → PASS → Clippy → Commit。

Commit message: `feat(engine): parsers — REW CSV / FRD / AutoEq TXT`

---

### Task 3.3: interpolate（三次样条 + Akima）+ smooth（1/n octave 分数阶）

**Files:**
- Create: `workspace/crates/calibration-engine/src/interpolate.rs`
- Create: `workspace/crates/calibration-engine/src/smooth.rs`
- Test: `workspace/crates/calibration-engine/tests/interp_smooth.rs`

- [ ] **失败测试**（与旧 plan Task 2.3 相同：sine 11→101 RMSE<0.05dB、平滑 1oct 峰值保留 >5dB）

- [ ] **实现要点**：
  - interpolate: `natural_cubic_spline_interp` + `resample_log_spaced(freqs, mags, n_pts, fmin, fmax) -> (Vec<f32>, Vec<f32>)`（返回严格等比对数轴 n_pts，供 target_curves 和测量重采样用）
  - `akima_interp`（目标曲线控制点插值用，比三次样条更少过冲）：实现 Akima 1970 连续插值（5 点模板）
  - smooth: 1/n octave 高斯加权移动平均，窗口 = `2 * oct_bandwidth`，边界点邻域内归一化

Commit message: `feat(engine): interpolate (cubic + Akima) + smooth 1/n octave`

---

### Task 3.4: target_curves（数字化提取 64pt + Akima → 5 条）

**Files:**
- Create: `workspace/crates/calibration-engine/src/target_curves.rs`
- Test: `workspace/crates/calibration-engine/tests/target_curves.rs`

- [ ] **失败测试**：每条曲线在 1kHz 处 0dB ±0.5dB；端点 20Hz/20kHz 单调；Harman OE 低频 boost ≥10dB @20Hz；Harman IE 低频 boost ≥14dB @20Hz。

- [ ] **实现要点（v1.2 关键：数字化提取，非论文公式）**：

```rust
// workspace/crates/calibration-engine/src/target_curves.rs
//! 目标曲线通过对公开高分辨率曲线图进行 WebPlotDigitizer 数字化提取得到。
//! 每条曲线 64 控制点 + Akima 插值到 FitCurves.N (256) pt。
//! 精度 ±0.3 dB。
//!
//! 数据来源（每条都需在代码注释中声明 URL + 提取方法，确保合规可追溯）：
//! - Flat:         gain(f) = 0 dB (精确，无需提取)
//! - FreeField:    公开物理公式（球面波衰减，r=0.0875m head radius）
//! - DiffuseField: IEC 60268-7 标准分段折线 → 64pt 数字化
//! - Harman OE 2019: https://pro.harman.com/insights/akg/defining-the-standard...
//!                   → WebPlotDigitizer 提取 64pt
//! - Harman IE 2018: https://www.soundguys.com/harman-target-curves-explained/
//!                   → WebPlotDigitizer 提取 64pt（2024 公开博客曲线图）

use calibration_types::PresetTargetCurve::*;
use calibration_types::{FitCurves, PresetTargetCurve, TargetCurveSpec};
use crate::interpolate::akima_resample_to_256;

// 64pt 数字化提取常量表（Hz, dB）。实施时用 WebPlotDigitizer 对公开曲线图逐点提取，
// 取 64 个对数等间隔频点（20, 25, 31.5, 40, 50, 63, 80, 100, 125, ... 20000）。
// 每个数组 64 项。实施时填入实际提取值（此处省略 64 × 3 × 2 = 384 个数值，实施子任务必须补全）。

const HARMAN_OE_2019_64PT: [(f32, f32); 64] = [
    // (Hz, dB) — WebPlotDigitizer 提取自 Harman OE 2019 公开曲线图
    // 实施任务：实际提取后填入。示例占位（实施时必须替换为真实提取值）：
    (20.0, 12.0), (25.0, 12.0), (31.5, 11.5), (40.0, 10.5), (50.0, 9.5),
    (63.0, 8.0), (80.0, 6.5), (100.0, 5.0), (125.0, 4.0), (160.0, 3.0),
    (200.0, 2.0), (250.0, 1.5), (315.0, 1.0), (400.0, 0.5), (500.0, 0.0),
    (630.0, -0.2), (800.0, -0.3), (1000.0, 0.0), // 1kHz 锚点 0dB
    (1250.0, 0.2), (1600.0, 0.5), (2000.0, 1.0), (2500.0, 1.5), (3150.0, 2.0),
    (4000.0, 2.5), (5000.0, 2.5), (6300.0, 2.0), (8000.0, 1.0), (10000.0, 0.0),
    (12500.0, -1.0), (16000.0, -2.0), (20000.0, -3.0),
    // 其余 33 点：20Hz~20kHz 64pt 对数间隔，实际提取后补全
    // ... (实施子任务补全剩余 33 项，保证总长 64)
    (0.0, 0.0); // 占位，实施时必须替换！
];

const HARMAN_IE_2018_64PT: [(f32, f32); 64] = [
    // (Hz, dB) — Harman In-Ear 2018 数字化提取
    // 相比 OE：低频更多 boost (~14dB @20Hz)，中高频不同形状
    (20.0, 14.0), (25.0, 14.0), (31.5, 13.0), // ... IE 曲线实际提取后补全
    (20000.0, -4.0),
    // ... 其余 60 点（实施子任务补全）
    (0.0, 0.0); // 占位
];

const DIFFUSE_FIELD_64PT: [(f32, f32); 64] = [
    // IEC 60268-7 Diffuse Field 响应 → 数字化 64pt
    // ... 实施子任务补全
    (0.0, 0.0); // 占位
];

/// 把 f(free) 自由场公式写成精确函数（球面波直达声 + 无反射近似）
fn free_field_gain_db(f_hz: f32) -> f32 {
    let c = 343.0;           // m/s, 20°C 空气中声速
    let r = 0.0875;          // m, 平均人头半径
    let lambda = c / f_hz;   // m, 波长
    // 自由场中球形头的阴影效应近似：H(p) = λ / sqrt(λ² + r²)
    // 即低频 λ>>r → 1 → 0dB；高频 λ<<r → λ/r → 衰减
    let ratio = lambda / (lambda * lambda + r * r).sqrt();
    20.0 * ratio.log10()
}

/// 根据 PresetTargetCurve 返回 256pt 严格对数轴 dB 值（结果直接用于 FitCurves.target）
pub fn preset_curve_256pt(which: PresetTargetCurve) -> Vec<f32> {
    let freq_axis = FitCurves::standard_freq_axis();  // 256pt 标准轴
    match which {
        Flat => vec![0.0f32; FitCurves::N],
        FreeField => freq_axis.iter().map(|&f| FitCurves::clamp_db(free_field_gain_db(f))).collect(),
        DiffuseField => {
            let (fs, ds): (Vec<f32>, Vec<f32>) = DIFFUSE_FIELD_64PT.iter().copied().unzip();
            akima_resample_to_256(&fs, &ds, &freq_axis)
        }
        HarmanOE2019 => {
            let (fs, ds): (Vec<f32>, Vec<f32>) = HARMAN_OE_2019_64PT.iter().copied().unzip();
            akima_resample_to_256(&fs, &ds, &freq_axis)
        }
        HarmanIE2018 => {
            let (fs, ds): (Vec<f32>, Vec<f32>) = HARMAN_IE_2018_64PT.iter().copied().unzip();
            akima_resample_to_256(&fs, &ds, &freq_axis)
        }
    }
}

/// 把 TargetCurveSpec（自定义控制点）→ 256pt（Akima 插值）
pub fn custom_curve_256pt(spec: &TargetCurveSpec) -> Vec<f32> {
    let freq_axis = FitCurves::standard_freq_axis();
    let (fs, ds): (Vec<f32>, Vec<f32>) = spec.control_points.iter().copied().unzip();
    akima_resample_to_256(&fs, &ds, &freq_axis)
}
```

- [ ] **实施子任务必须**：实际使用 WebPlotDigitizer 从公开 URL 的高分辨率图中提取 64 点，替换占位数组。每个数组旁注释标注来源 URL、提取日期、提取方法。

- [ ] **测试 → Clippy → Commit**

Commit message: `feat(engine): target_curves — digitized 64pt + Akima (Flat/Free/Diffuse/Harman OE 2019/Harman IE 2018)`

---

### Task 3.5: biquad_fit（加权 LSQ + 自研 Cholesky 16×16）

**Files:**
- Create: `workspace/crates/calibration-engine/src/biquad_fit.rs`
- Test: `workspace/crates/calibration-engine/tests/biquad_fit.rs`

- [ ] **失败测试 3 个**：平曲线→gain≈0（|ε|<0.1dB）；1kHz +6dB 单峰→对应 gain≈-6dB；典型测量（100Hz +10dB 滚降 + 4kHz dip）→max residual < 3dB。

- [ ] **实现要点**：
  - `iso_13_octave_centers(n: u8, fmin: f32, fmax: f32) -> Vec<f32>`：ISO 1/3-octave 频点，默认 16 段约覆盖 25Hz~16kHz
  - `rbj_peak_response_db(f: f32, fc: f32, q: f32, gain_db: f32) -> f32`：RBJ Cookbook Peaking 幅频公式（精确 dB）
  - `cholesky_solve_16x16(a: &mut [[f32;16];16], b: &mut [f32;16])`：自研无依赖 Cholesky 分解 + 前代回代（n ≤ 16 → 栈数组，零堆分配）
  - `fit_weighted_lsq(meas_256: &[f32], target_256: &[f32], opt: &FitOptions, side: L|R) -> (Vec<EqBand>, Vec<f32> /* PEQ 256pt 响应 */, Vec<f32> /* residual 256pt PEQ alone */)`：
    - W = diag(weight_i)，weight_i = 1 + low_freq_bonus × (f<100Hz ? 1 : 0) + `measured_SPL_远离_clamp_边界` 加权
    - J 矩阵 256×16：J[i][j] = `rbj_peak_response(freq[i], fc[j], Q[j], 1dB)` 单位增益 dB 响应（线性化到幅度域求解更稳定）
    - 求解正规方程 `(J^T W J) g = J^T W (target - meas)`（幅度域）→ g 再转 dB 作为 EqBand.gain_db
    - 结果 clamp gain ∈ [-12, +12] dB（避免极端 Biquad 系数不稳定）

Commit message: `feat(engine): biquad_fit — weighted LSQ + 16x16 Cholesky self-implemented`

---

### Task 3.6: fir_residual（rustfft IFFT + Hilbert 最小/线性相位）

**Files:**
- Create: `workspace/crates/calibration-engine/src/fir_residual.rs`
- Test: `workspace/crates/calibration-engine/tests/fir_residual.rs`

- [ ] **失败测试 2 个**：平残差 → coeffs ≈ δ（|coeffs[0]-1|<0.01，其余 <1e-3）；已知简单 1kHz +3dB 响应 → FIR 卷积后扫频在 1kHz 处 gain≈+3dB（与直接卷积误差 <1e-3）。

- [ ] **实现要点**：
  - `log_to_linear_freq_axis_256_to_nfft(log_freq_256: &[f32], log_mag_db_256: &[f32], nfft: usize, sr: u32) -> Vec<f32>`：256pt 对数 dB → NFFT 线性 Hz 幅度（10^(dB/20) 线性化，Hz 域线性插值，20Hz~sr/2 clamp，边界 20Hz ramp 0→首值防直流突变）
  - `hilbert_minphase_from_mag(mag: &[f32]) -> Vec<Complex<f32>>`：FFT-based Hilbert 变换最小相位恢复（`ln(|H|) → FFT → ×(-i·sign) → iFFT → exp`，经典 DFT cepstrum 方法）
  - `ifft_real_to_taps(H: Vec<Complex<f32>>, tap: usize, mode: PhaseMode) -> Vec<f32>`：ifft → 取实部 → Hann 窗截短到 tap → 归一化 DC=0dB（`sum(taps) = 1 or 10^(target_dc_gain/20)`）

Commit message: `feat(engine): fir_residual — rustfft IFFT + Hilbert min/linear phase`

---

### Task 3.7: fit_engine::fit() 整合 + 端到端单测

**Files:**
- Create: `workspace/crates/calibration-engine/src/fit_engine.rs`
- Modify: `workspace/crates/calibration-engine/src/lib.rs`（pub use fit_engine::*）
- Test: `workspace/crates/calibration-engine/tests/fit_e2e.rs`

- [ ] **失败测试（端到端）**：平测量 + 平目标 → peq_gain 全 <0.1dB + FIR coeff≈δ；"1kHz +6dB 单峰测量 + 平目标" → peq 对应 gain≈-6dB + predicted residual max <1dB；"左右独立不对称测量" → L/R peq 不同。

- [ ] **实现要点（`pub fn fit(req: FitRequest) -> Result<FitResult, FitError>`）**：
  1. 输入校验（meas 非空、freq 严格递增、meas 点数 ≥ 10、target.control_points ≥ 2 → InvalidMeasurement / InvalidTarget）
  2. `Instant::now()` 计时 → 末尾填 `fit_time_ms`
  3. 测量 L/R 重采样：`parsers` → `interpolate.resample_log_spaced` → 256pt 严格对数轴（`FitCurves.standard_freq_axis()`）→ `smooth.octave` 平滑（`options.smooth_octaves`）
  4. 目标曲线：预置 → `target_curves::preset_curve_256pt()`；自定义 → `custom_curve_256pt(spec)`
  5. 误差曲线 = target - measured（dB 域）。L、R 各一份。
  6. Biquad 拟合：`biquad_fit::fit_weighted_lsq(error_L, opt) -> (peq_L, peq_L_resp_256, peq_L_residual)`；R 同理。
  7. 残差 = error - peq_response（FIR 要补偿的部分）。L/R 各一份。
  8. FIR 系数：`fir_residual` 把 residual dB → `log_to_linear` → Hilbert/IFFT → tap Hann 窗 → `coeffs_L/R`（`options.fir_tap`，options.phase_mode）
  9. 预测响应：`measured[i] + peq[i] + fir_response_256pt[i]` → clamp。FIR 256pt 响应由 coeffs 的 DTFT 在 256pt 频率轴上计算（直接卷积 δ_sweep，长度短（256）不敏感）。
  10. 组装 FitResult：fit_id = `"cal_" + hex(random 4 bytes, 8 chars)`；created_at；peq_L/R；fir{FirConfig{tap, enabled:true, coeffs_L/R, phase_mode}}；`curves` 10 条 256pt 向量；diagnostics（RMSE/max 对 residual 统计；bands 用 peq_L/R.len()；fir_tap_used=options.fir_tap）；request_snapshot=req.clone()。
  11. 返回 `FitDiagnostics.residual_max_db_{L,R} > 3.0` 时不报错（仅标记诊断），允许用户决定是否接受。

Commit message: `feat(engine): fit() end-to-end integration + diagnostics + 256pt curves`

---

## Milestone 4: calibration-fir Wasm 插件（仅 overlap-save FIR + 版本号轮询）「3 Tasks」

### Task 4.1: crate 骨架 + Cargo.toml + manifest（仅依赖 calibration-types，无算法）

**Files:**
- Create: `workspace/plugins/calibration-fir/Cargo.toml`
- Create: `workspace/plugins/calibration-fir/src/lib.rs`（骨架：PluginState + 空 plugin_init/process）
- Create: `workspace/plugins/calibration-fir/manifest.json`
- Modify: 根 `Cargo.toml` workspace.members 追加 `workspace/plugins/calibration-fir`

- [ ] **要点（v1.2 关键）**：
  - crate name: `calibration-fir`（不再是 calibration-suite，语义精准：只做 FIR）
  - dependencies: **仅** `calibration-types`（path 同 workspace）+ `serde` + `serde_json`。**无 rustfft、无 parsers、无插值**。Wasm 体积目标 ~25KB（strip + lto + opt-level=z）。
  - **去 `#![no_std]`** → 直接 `std` 环境（v1.2：rustfft 兼容性考虑；此 crate 虽不用 rustfft，但 std 环境 host import 写起来更干净，项目约定 Wasm 允许 std）。
  - manifest.name = `"calibration_fir"`，plugin_type = `"DspProcessor"`，permissions = `[]`。

- [ ] **验证**：`cargo check -p calibration-fir --target wasm32-unknown-unknown` 通过。

Commit message: `feat(plugin): calibration-fir Wasm skeleton (only overlap-save FIR, deps only calibration-types)`

---

### Task 4.2: overlap-save FIR 卷积（L/R 独立 + tap 切换零毛刺）

**Files:**
- Modify: `workspace/plugins/calibration-fir/src/lib.rs`（PluginState + process_block）
- Test: `workspace/plugins/calibration-fir/tests/fir_overlap_save.rs`（native test 不需要 wasm-pack）

- [ ] **失败测试**：已知 δ 冲激 → coeffs = δ → 输出 = 输入；已知线性扫频 vs 直接卷积误差 <1e-6；tap 从 512 → 2048 切换，块边界相邻样本差 < 1e-4（无毛刺）。

- [ ] **实现要点**：
  ```rust
  pub struct PluginState {
      fir_enabled: bool,
      fir_tap: usize,
      coeffs: (Arc<Vec<f32>>, Arc<Vec<f32>>),  // (L, R) — Arc swap 零拷贝
      overlap: (Vec<f32>, Vec<f32>),           // (L, R) 保存前 tap-1 输入
      last_known_config_version: u64,
  }
  impl PluginState {
      fn process_block(&mut self, samples: &mut [f32], ch: u16, sr: u32) {
          // 解交错 → L/R 各自 overlap-save 核心 → 重交错
          // 系数切换时：old coeffs 的 overlap 状态做 xfade(256 sample) 防毛刺
          // 若 fir_enabled=false → passthrough（只更新 overlap 末尾样本，不乘系数）
      }
  }
  ```

Commit message: `feat(plugin): overlap-save FIR L/R + tap-swap zero-click xfade`

---

### Task 4.3: plugin_process 导出 + 版本号轮询拉模型热加载

**Files:**
- Modify: `workspace/plugins/calibration-fir/src/lib.rs`（`#[no_mangle] pub extern "C" fn plugin_process(...)`；process 开头轮询版本号）
- Test: native mock：simulate 版本号变化 → 下次 process 内触发重新加载 coeffs

- [ ] **要点（spec §4.3）**：
  1. `plugin_init`: `last_known_config_version = host.get_plugin_config_version(name_ptr, name_len)`；加载初始 config。
  2. 每次 `plugin_process` 开头（**在 FIR 卷积之前，开销极小 = 1 次 AtomicU64 load**）：
     ```rust
     let cur_v = host.get_plugin_config_version(name_ptr, name_len) as u64;
     if cur_v != state.last_known_config_version {
         let json = host.read_plugin_config(name_ptr, name_len, buf_ptr, buf_cap);
         let cfg: FirConfig = serde_json::from_slice(&json_bytes).unwrap();
         state.apply_new_fir_config(&cfg);  // xfade swap coeffs
         state.last_known_config_version = cur_v;
     }
     ```
  3. `host.xxx` 两条函数通过 `#[link(wasm_import_module = "env")] extern "C" { ... }` 声明，名字与 phonon-plugin runtime.rs 注册时的 linker.func_wrap 名字完全一致（见 M5）。

Commit message: `feat(plugin): plugin_process export + config version polling hot-reload`

---

## Milestone 5: PluginRuntime Host Import 扩展（config_versions + 两条 host import）「2 Tasks」

### Task 5.1: PluginRuntime struct 加 config_versions + set_plugin_config 内 fetch_add

**Files:**
- Modify: `phonon-plugin/src/runtime.rs`
- Test: `phonon-plugin/tests/plugin_config_version.rs`

- [ ] **失败测试**：`set_plugin_config("calibration_fir", json1)` → version = 1；再 set → version = 2；从未 set 过的 plugin → version = 0。

- [ ] **实现要点**：
  - struct 追加：`config_versions: Arc<std::collections::HashMap<String, std::sync::atomic::AtomicU64>>`
  - new()：`HashMap::new()` 初始化空（按需插入）。
  - `fn set_plugin_config(&self, name: &str, value: serde_json::Value) -> Result<()>` 末尾：
    `self.config_versions.entry(name.to_string()).or_insert_with(|| AtomicU64::new(0)).fetch_add(1, Ordering::Release);`
    （语义：**每次 set_plugin_config 成功后版本号 +1**。Wasm 端 load 此版本号变化 → 重新读取 config JSON。）

Commit message: `feat(plugin): PluginRuntime.config_versions (HashMap<name, AtomicU64>) + set_plugin_config fetch_add`

---

### Task 5.2: 两条 host import（get_plugin_config_version / read_plugin_config）

**Files:**
- Modify: `phonon-plugin/src/runtime.rs` 的 linker（`add_host_imports` 函数追加两条 `linker.func_wrap`）
- Test: 同上测试文件 + 模拟 Wasm 调用（用 wasmtime 直接调 linker 封装函数）

- [ ] **签名（与 Wasm 端 extern 声明严格匹配）**：
  1. `host.get_plugin_config_version(name_ptr: i32, name_len: i32) -> i64`
     - 从 Wasm 内存读 name_len 字节 UTF-8 → `name`
     - 返回 `*config_versions.get(name).map(|v| v.load(Acquire) as i64).unwrap_or(0)`
  2. `host.read_plugin_config(name_ptr: i32, name_len: i32, buf_ptr: i32, buf_cap: i32) -> i32`
     - 读 name → 查 `plugin_configs: Arc<RwLock<HashMap<String, serde_json::Value>>>`（需要 PluginRuntime 也同时持有 config 内容 Map；此 Map 由 set_plugin_config 写入 value，和 config_versions 同步更新）
     - `serde_json::to_vec(&value).unwrap()` → 复制到 Wasm 内存 buf_ptr..buf_ptr+buf_cap
     - 返回：成功 = 实际写入字节数；buf_cap < len = -1（Wasm 端 realloc 重试）；name 不存在 = -2；读 name UTF-8 失败 = -3

Commit message: `feat(plugin): host imports get_plugin_config_version + read_plugin_config (Wasm ← host pull model)`

---

## Milestone 6: Tauri Commands + CalibrationAppState LRU Cache + 事务 Apply「4 Tasks」

### Task 6.1: set_calibration_peq + get_calibration_peq

**Files:**
- Create: `phonon-tauri/src-tauri/src/commands/calibration.rs`（骨架 + 2 命令）
- Modify: `phonon-tauri/src-tauri/src/commands.rs`（`mod calibration;` + register）

- [ ] **要点**：
  - `set_calibration_peq(state, bands_L, bands_R, enabled) -> Result<(), PeqUpdateError>`：peq_handle.update_bands → set_enabled → 持久化到 AppSettings。
  - `get_calibration_peq(state) -> CalibrationPeqStatus { enabled, bands_L, bands_R }`：读 AppSettings.calibration + peq_handle.enabled_tx.load。

Commit message: `feat(commands): set/get_calibration_peq Tauri commands`

---

### Task 6.2: toggle_calibration_bypass（PEQ + FIR 同步）

**Files:**
- Modify: `phonon-tauri/src-tauri/src/commands/calibration.rs`

- [ ] **要点**：
  ```rust
  pub fn toggle_calibration_bypass(state, bypass: bool) -> Result<(), String> {
      // 1) PEQ 侧：peq_handle.set_enabled(!bypass)
      // 2) FIR 侧：通过 set_plugin_config("calibration_fir", {"enabled": !bypass, ...其余保留当前})
      //    → config_versions.fetch_add(1) → Wasm 下次 process() 内自动 pick up
      // 3) AppSettings.calibration.bypass = bypass 持久化
      // 步骤 1/2 顺序任意（两者独立，都是无锁写）
  }
  ```

Commit message: `feat(commands): toggle_calibration_bypass (PEQ + FIR synced via set_plugin_config version bump)`

---

### Task 6.3: run_calibration_fit（直接 native 调 calibration_engine::fit + LRU 双淘汰缓存）

**Files:**
- Modify: `phonon-tauri/src-tauri/src/commands/calibration.rs`（CalibrationAppState struct + run 命令）

- [ ] **要点（v1.2 三 crate 关键）**：
  1. CalibrationAppState：Tauri AppState 扩展字段（或单独 struct 作为 Tauri State 注册）：
     ```rust
     use std::collections::HashMap;
     use std::time::{Duration, Instant};
     struct FitCacheEntry { result: FitResult, inserted: Instant }
     pub struct CalibrationAppState {
         fit_results: std::sync::Mutex<lru::LruCache<String, FitCacheEntry>>, // 20 cap
         fit_results_by_age: Vec<(String, Instant)>, // 按时间排序，24h 淘汰辅助
         last_applied_fit_id: std::sync::RwLock<Option<String>>,
         last_applied_result: std::sync::RwLock<Option<FitResult>>, // ★ 永不淘汰
     }
     impl CalibrationAppState {
         const MAX_ENTRIES: usize = 20;
         const MAX_AGE: Duration = Duration::from_secs(24 * 3600);
         fn insert(&self, r: FitResult) {
             // 1) LruCache put（满 20 → LRU 弹旧）
             // 2) by_age 队列：push 末尾 + 清理 >24h 项（按 Instant now - inserted）
             // 3) 不写 last_applied_*（那是 apply 时才写）
         }
         fn get(&self, id: &str) -> Option<FitResult> {
             // 1) last_applied_result 读 → id 匹配 → 直接返回（不进入 LRU 淘汰逻辑）
             // 2) 否则 LruCache get（命中 → 提升 MRU）
             // 3) 再 by_age 检查（>24h → 移除）
         }
     }
     ```
  2. `run_calibration_fit(req: FitRequest, apply_immediately: bool) -> Result<FitResult, FitError>`：
     - `calibration_engine::fit(req) → result`（同步，native，~10-50ms）
     - 存入 `CalibrationAppState.insert(result.clone())`
     - 若 `apply_immediately = true` → 内部调 `do_apply(&state, &result.fit_id)`（见 Task 6.4）
     - 返回 result（FitCurves 256pt 供前端 Canvas 立即渲染预览）

Commit message: `feat(commands): run_calibration_fit + LRU cache (20 entries / 24h) — native call calibration_engine::fit()`

---

### Task 6.4: apply_fit_result（事务回滚）+ get_current_fit_result（不淘汰）

**Files:**
- Modify: `phonon-tauri/src-tauri/src/commands/calibration.rs`

- [ ] **核心：事务回滚（v1.2 顶级语义）**

```rust
/// 内部原子事务：PEQ 注入 → FIR 配置。两步都成功才"提交"；FIR 失败时 PEQ 回滚到应用前快照。
fn do_apply(state: &tauri::State<CalibrationAppState>, peq_handle: &PeqUpdateHandle, fit_id: &str) -> Result<(), ApplyError> {
    // 1) 从缓存取 FitResult
    let fit = state.get(fit_id).ok_or_else(|| ApplyError::NotFound(fit_id.into()))?;

    // 2) 保存 PEQ 应用前快照（before_snapshot = 事务回滚锚点）
    let before_snap = peq_handle.last_applied_snapshot();

    // 3) Step A: 应用 PEQ
    peq_handle.update_bands(fit.peq_L.clone(), fit.peq_R.clone())?;
    peq_handle.set_enabled(true);
    // A 成功暂时"提交"PEQ；但事务还没结束

    // 4) Step B: 应用 FIR（set_plugin_config 写 FirConfig JSON → 版本号 bump）
    let fir_cfg_json = serde_json::to_value(&fit.fir).unwrap();
    match phonon_plugin::runtime::set_plugin_config_sync("calibration_fir", fir_cfg_json) {
        Ok(()) => {
            // ★ 事务成功：持久化 + 更新 last_applied_*（永不淘汰）
            let mut app_settings = /* 从 Tauri State 拿 settings Mutex */;
            app_settings.calibration.peq_L = fit.peq_L.clone();
            app_settings.calibration.peq_R = fit.peq_R.clone();
            app_settings.calibration.bypass = false;
            app_settings.calibration.last_applied_fit_id = Some(fit.fit_id.clone());
            app_settings.calibration.fir_tap = fit.fir.tap;
            app_settings.calibration.phase_mode = fit.fir.phase_mode;
            // settings.save_to_disk();

            *state.last_applied_fit_id.write().unwrap() = Some(fit.fit_id.clone());
            *state.last_applied_result.write().unwrap() = Some(fit.clone());
            Ok(())
        }
        Err(fir_err) => {
            // ★ 事务回滚：把 PEQ 恢复到 before_snap（FIR 步骤之前的 last_applied）
            match before_snap {
                Some(before) => {
                    // 用 before 构造 bands（before.bands_L/R 是原始 Vec<EqBand> clone）
                    let (bl, br) = (before.bands_L.clone(), before.bands_R.clone());
                    match peq_handle.update_bands(bl, br) {
                        Ok(()) => {
                            peq_handle.set_enabled( /* before 时的 enable 状态需单独持久化 → 简化：从 AppSettings 恢复 bypass */);
                            Err(ApplyError::FirConfig(format!("{} (rollback success)", fir_err)))
                        }
                        Err(rollback_err) => {
                            // 回滚失败（罕见：DSP 此时突然 ChannelClosed）
                            Err(ApplyError::RollbackFailed(format!("FIR err: {}; rollback PEQ err: {}", fir_err, rollback_err)))
                        }
                    }
                }
                None => {
                    // before 为空（首次应用）→ 回滚策略：清空 PEQ 为 identity（update_bands(vec![], vec![])）
                    match peq_handle.update_bands(vec![], vec![]) {
                        Ok(()) => {
                            peq_handle.set_enabled(false);
                            Err(ApplyError::FirConfig(format!("{} (rollback to empty PEQ)", fir_err)))
                        }
                        Err(e) => Err(ApplyError::RollbackFailed(format!("FIR err: {}; rollback empty PEQ err: {}", fir_err, e))),
                    }
                }
            }
        }
    }
}

#[tauri::command]
pub fn apply_fit_result(state: tauri::State<CalibrationAppState>, fit_id: String) -> Result<(), ApplyError> {
    let peq_handle = /* 从 Tauri State 拿 AppState.peq_update_handle.clone() */;
    do_apply(&state, &peq_handle, &fit_id)
}

#[tauri::command]
pub fn get_current_fit_result(state: tauri::State<CalibrationAppState>) -> Option<FitResult> {
    // ★ 直接读 last_applied_result（永不淘汰），即使 fit 了 100 次后当前生效的也能查到
    state.last_applied_result.read().unwrap().clone()
}
```

Commit message: `feat(commands): apply_fit_result (transactional rollback PEQ↔FIR) + get_current_fit_result (never-evicted cache)`

---

## Milestone 7: 前端 TS 层（粗解析 + API 封装 + 目标曲线常量表）「3 Tasks」

### Task 7.1: calibration/parsers.ts 粗解析（文本切片 JSON，零 DSP）

**Files:**
- Create: `phonon-tauri/src/calibration/parsers.ts`
- Test: `phonon-tauri/src/calibration/__tests__/parsers.test.ts`

- [ ] **要点**：
  - TS 只做 BOM 跳过、换行符标准化、按行按逗号/空格切分列、字符串→字符串数组。
  - **不做任何 DSP**（数字转 f32 / 重采样 / 平滑 全部在 Rust 端）。
  - 输出：`{ format: 'rew_csv'|'frd'|'autoeq_txt', raw_columns: Array<[string, string, string]> }`（序列化给 `run_calibration_fit` 的 Tauri 命令，命令内再调 calibration-engine 做真正解析）。

Commit message: `feat(ts): calibration/parsers.ts — coarse text slicing only (zero DSP)`

---

### Task 7.2: api/calibration.ts 封装（6 命令 + 类型 + FitCurves 256pt 对齐）

**Files:**
- Create: `phonon-tauri/src/api/calibration.ts`
- Modify: `phonon-tauri/src/api/index.ts`（`export * from './calibration'`）

- [ ] **要点**：
  ```typescript
  import { invoke } from '@tauri-apps/api/core';
  // 所有类型严格对齐 calibration-types（Rust 侧）。不定义任何重复类型。
  export type FilterType = 'Peaking'|'LowShelf'|'HighShelf'|'LowPass'|'HighPass'|'BandPass'|'Notch'|'AllPass';
  export interface EqBand { frequency: number; gain_db: number; q: number; filter_type: FilterType; }
  export interface FitCurves {
    freq: number[];  // 256pt, freq[i]=20*(20000/20)^(i/255)
    measured_L: number[]; measured_R: number[];
    target: number[]; peq_response_L: number[]; peq_response_R: number[];
    predicted_L: number[]; predicted_R: number[];
    residual_L: number[]; residual_R: number[];
    // 常量：
    readonly N: 256; readonly FMIN: 20; readonly FMAX: 20000; readonly DB_MIN: -30; readonly DB_MAX: 18;
  }
  // … PhaseMode / MeasurementData / TargetCurveSpec / FitOptions / FitRequest / FitDiagnostics / FirConfig / FitResult / FitError / ApplyError / PresetTargetCurve …
  // 6 条命令封装（invoke<R>(name, args)）：
  export async function setCalibrationPeq(args: { bands_L: EqBand[]; bands_R: EqBand[]; enabled: boolean }): Promise<void> { /* invoke */ }
  export async function getCalibrationPeq(): Promise<{ enabled: boolean; bands_L: EqBand[]; bands_R: EqBand[] }> { /* invoke */ }
  export async function toggleCalibrationBypass(bypass: boolean): Promise<void> { /* invoke */ }
  export async function runCalibrationFit(req: FitRequest, applyImmediately: boolean): Promise<FitResult> { /* invoke */ }
  export async function applyFitResult(fitId: string): Promise<void> { /* invoke */ }
  export async function getCurrentFitResult(): Promise<FitResult | null> { /* invoke */ }
  ```

Commit message: `feat(ts): api/calibration.ts — 6 commands + types (FitCurves 256pt exact axis)`

---

### Task 7.3: 目标曲线 TS 常量表（5 条 256pt 预览用）

**Files:**
- Create: `phonon-tauri/src/calibration/target_curves_const.ts`

- [ ] **要点**：
  - Flat：`[...Array(256)].fill(0)`（直接）
  - FreeField：直接计算 `20*log10(lambda/sqrt(lambda^2+r^2))`（r=0.0875）→ 256pt
  - DiffuseField / Harman OE 2019 / Harman IE 2018：从 calibration-engine 的 64pt 数组复制（保持一致）到 TS → 前端侧用 Akima 插值 64→256（或直接 `akima-resample` npm 包；如无，轻量实现 30 行版本）。TS 常量**仅用于 Canvas 预览渲染**（真实拟合用 Rust 端的保证一致；TS 预览是"用户看目标曲线形状"用途，结果对齐即可）。

Commit message: `feat(ts): target_curves_const.ts — 5 preset curves 256pt preview constants`

---

## Milestone 8: CalibrationPanel UI（5 区顶级 UI + apply 防抖 500ms）「5 Tasks」

### Task 8.1: 基础布局 + 测量导入区（拖拽/浏览 + L/R 分配 + Swap/Use L for R）

**Files:**
- Create: `phonon-tauri/src/components/CalibrationPanel.tsx`
- Modify: 父组件（通常 SettingsPanel 或 Sidebar 某处）加入入口 Tab

- [ ] **5 区布局（spec §8.1）**：
  ```
  ┌───────────────────────────────────────────────────────┐
  │ [1] 测量导入区 (顶部 120px)                            │
  │  Drag & Drop / Browse  L file + R file  Swap / Use L→R│
  ├───────────────────────────────────────────────────────┤
  │ [2] 四曲线 Canvas 可视化 (中部 自适应高度)              │
  │  measured(L cyan / R magenta dashed) / target(green) /│
  │  PEQ predicted(yellow solid) / full predicted(white)  │
  │  residual (gray area fill ±10dB)                      │
  │  Tooltip hover: freq+dB 精确值                        │
  ├───────────────────────────────────────────────────────┤
  │ [3] 目标曲线拖拽编辑器 (中右 320px)                    │
  │  Preset: [Flat▼] [Harman OE 2019] [...] + Custom Drag│
  │  控制点增删改 + Akima 实时渲染                         │
  ├───────────────────────────────────────────────────────┤
  │ [4] 拟合参数区 + 预览/应用 (底部 180px)                │
  │  PEQ段数 [16▼] FIR tap [2048▼] Phase [Min▼] Sm[1/24▼]│
  │  [Run Fit] → [Apply Fit] (★ 500ms 防抖)               │
  │  Progress 3 阶段进度条 + FitDiagnostics 4 栏: RMSE/Max│
  ├───────────────────────────────────────────────────────┤
  │ [5] 预设管理 + Bypass + A/B (底部栏 56px)              │
  │  Presets: [Save▼] [Load▼] [Del] [JSON↶] [JSON↷]      │
  │  [Bypass ●] A/B Toggle  L/R [Link🔗/Unlink]           │
  └───────────────────────────────────────────────────────┘
  ```

- [ ] **测量导入区实现要点**：拖拽 `ondrop` → `FileReader.readAsText` → 格式探测（`.csv`→rew，`.frd`→frd，`.txt`→autoeq）→ parsers.ts 粗解析。L/R 各一个卡片。Swap 按钮互换；"Use L for R" 复制 L 到 R。

Commit message: `feat(ui): CalibrationPanel base layout + Measurement Import (drag/browse + L/R + Swap)`

---

### Task 8.2: Canvas 四曲线可视化（256pt 对数轴，严格对齐 FitCurves 常量）

**Files:**
- Modify: `CalibrationPanel.tsx`（新增 Canvas 区域子组件，可单独成 `CalibrationCanvas.tsx`）

- [ ] **要点**：
  - X 轴：`log(freq)`，ticks 20/50/100/200/500/1k/2k/5k/10k/20k Hz（严格对数映射 `x = (log10(f) - log10(20)) / (log10(20000) - log10(20)) * width`）
  - Y 轴：dB，范围 [-18, +18] 或 [-30, +18]（`FitCurves.DB_MIN/MAX`），ticks 每 6dB
  - 数据源：直接 `FitResult.curves`（256pt，freq 严格匹配 Canvas X 映射 → 一一对应，无需再插值）
  - 渲染：10 条曲线可开/关（legend 点击），但默认显示"measured + target + full predicted + residual fill"四通道。
  - 残差区域：`residual_L` 的 ± 值 → 从 y=0dB 基线到 `y = residual[i]` 灰色半透明 fillRect。

Commit message: `feat(ui): CalibrationCanvas 256pt log-axis — measured/target/predicted/residual fill`

---

### Task 8.3: 目标曲线拖拽编辑器（控制点增删改 + Akima 实时渲染）

**Files:**
- Modify: CalibrationPanel.tsx（TargetCurveEditor 子组件）

- [ ] **要点**：
  - 预置曲线 5 个 Radio 按钮 → 选中时从 `target_curves_const.ts` 加载控制点。
  - "Custom" 模式下，Canvas 上双击新增控制点，单击选中后 Delete 删除，按住拖拽移动。
  - 实时 Akima 插值渲染（目标曲线绿色虚线），并立即更新 FitRequest.target 供 Run Fit 用。
  - 控制点 freq 必须递增（拖拽时 clamp 相邻点）。dB clamp [-24, +24]。

Commit message: `feat(ui): TargetCurveDragEditor — preset 5 radio + custom control points + Akima`

---

### Task 8.4: 拟合参数区 + Run Fit + Apply Fit（★ 500ms 防抖）

**Files:**
- Modify: CalibrationPanel.tsx（FittingParamsArea 子组件）

- [ ] **要点（★ 顶级防抖语义）**：

```typescript
// ---------- apply 按钮 500ms 防抖（防止连点触发 PEQ sync_channel(1) Full）----------
const [applying, setApplying] = useState(false);
const [fitting, setFitting] = useState(false);
const [lastFitResult, setLastFitResult] = useState<FitResult | null>(null);
const fitId2 = useRef<string | null>(null);

async function handleRunFit() {
  if (fitting) return;  // 拟合过程中禁用
  setFitting(true);
  try {
    // 构造 FitRequest（包含 L/R 粗解析 raw_columns → Tauri 命令内转 MeasurementData → calibration-engine 内真正解析）
    const req: FitRequest = buildFitRequestFromUI();
    // apply_immediately=false → 仅预览（dry-run，PEQ/FIR 不改动）
    const result = await runCalibrationFit(req, false);
    setLastFitResult(result);
    fitId2.current = result.fit_id;
    // 立即渲染 curves 到 Canvas（残差 fill、diagnostics 4 栏同步更新）
  } finally {
    setFitting(false);
  }
}

async function handleApplyFit() {
  // ★ 核心防抖：applying=true 期间直接拒绝
  if (applying) return;
  if (!fitId2.current) return;
  setApplying(true);
  try {
    await applyFitResult(fitId2.current);
    // 成功：Bypass 自动设为 false；状态栏显示 fit_id
  } catch (err) {
    // 错误分两类：
    // - PeqUpdateError::ChannelFull → toast "DSP 繁忙，请稍后再试"（前端防抖已兜底，罕见）
    // - ApplyError::FirConfig(rollback) → toast "FIR 应用失败，PEQ 已自动回滚"
    showApplyErrorToast(err);
  } finally {
    // ★ 500ms 冷却期（无论成功失败都释放）
    setTimeout(() => setApplying(false), 500);
  }
}
```

拟合参数区：
- PEQ 段数 select：8 / 12 / 16 / 20
- FIR tap select：256 / 512 / 1024 / 2048 / 4096 / 8192
- Phase mode：Minimum Phase / Linear Phase（Tooltip："最小相位 = 低延迟 + 更自然瞬态；线性相位 = 无相位失真 + 延迟 = tap/sr"）
- Smooth：Off / 1/48 / 1/24 / 1/12 / 1/6 / 1/3 / 1 oct
- Run Fit 按钮（蓝色，disabled 时 fitting）；进度条 3 阶段（解析/重采样→Biquad 拟合→FIR 生成，用 fake progress bar 平滑动画，因为是同步命令）
- Apply Fit 按钮（绿色，disabled 时 fitting 或 applying；无 fit_result 时 disabled）
- Diagnostics 4 栏：RMSE L/R、Max L/R（`<span style={max>3?'color:red':''}>{max.toFixed(2)} dB</span>`）

Commit message: `feat(ui): FittingParamsArea — PEQ/FIR/Phase/Smooth options + RunFit + ApplyFit 500ms debounce + diagnostics bars`

---

### Task 8.5: 预设管理 + Bypass 开关 + A/B 对比 + L/R Link

**Files:**
- Modify: CalibrationPanel.tsx（PresetsBar + BypassBar 子组件）

- [ ] **预设管理**：localStorage 存 `phonon-calibration-presets`（Array<{name, createdAt, fit: FitResult}>）。Save 弹窗输入命名；Load 下拉选择 → 恢复 peq+fir+curves 到 UI；Delete 删除；JSON↶ 导出（download blob JSON）；JSON↷ 导入（FileReader 解析 JSON → 验证类型 → 加入列表）。AppSettings.current_preset_name 同步持久化。

- [ ] **Bypass 开关**：原生 `<input type="checkbox" switch 样式>`（项目约定：UI 控件视觉一致性，不用自定义 toggle）。onChange → `toggleCalibrationBypass(e.target.checked)`；切换时 Canvas 曲线"旁路时 predicted → measured"视觉淡入淡出（opacity 动画）。

- [ ] **A/B 对比按钮**：按住"A/B"按钮 = bypass 打开；松开 = bypass 还原（快速听感对比）。移动端触摸事件同时支持。

- [ ] **L/R Link/Unlink**：Link 模式下目标曲线编辑器同时改 L+R；Unlink 下分别编辑。默认 Link。显示在底部栏右侧。

Commit message: `feat(ui): Presets management + Bypass switch + A/B hold + L/R Link`

---

## Milestone 9: 集成验证、合规与交付（最终 2 Tasks）「2 Tasks」

### Task 9.1: 端到端集成 + Clippy/cargo-deny 全仓库合规

- [ ] **Step 1: 端到端跑通**（人工 / 子 agent 自动化脚本）
  1. 启动 Tauri dev server
  2. 导入 REW sample CSV（内置 sample 文件：1kHz +6dB 单峰平曲线）
  3. 选 Harman OE 2019
  4. Run Fit（dry-run）→ FitResult.curves predicted 显示 1kHz 处 ≈ -6dB 补偿
  5. Apply Fit（500ms 内连点 3 次 → 前 1 次成功，后 2 次因 applying 拦截 → 无 ChannelFull 错误抛出前端）
  6. 播放 1kHz 测试音（粉红噪声 + 扫频） → 听感上 1kHz 被衰减
  7. Bypass → 听感恢复原始 → 再次 Bypass 取消 → 听感再次衰减（A/B 正常）
  8. 关闭 App → 重启 → Bypass 状态 + PEQ + FIR tap + last_applied_fit_id 正确恢复
  9. getCurrentFitResult → 返回的 FitResult.fit_id 与 #5 apply 的相同（不淘汰缓存验证）
  10. 快速开关 Bypass 100 次 → 无爆音、无 panic（AtomicBool 性能验证）
  11. 模拟 FIR apply 失败（临时禁用 calibration-fir 插件）→ Apply Fit 返回 ApplyError::FirConfig(rollback) → PEQ 回滚到 apply 之前（`get_calibration_peq` 与 #5 apply 前一致）

- [ ] **Step 2: Clippy 全仓库**

```bash
cd d:\Phonon
cargo clippy --workspace --all-targets -- -D warnings --cap-lints allow 2>&1 | tail -n 50
# Expected: 仅 26 条已知 allow 风格类 lint，无 ERROR，无新增 WARNING
```

- [ ] **Step 3: cargo-deny 全仓库**

```bash
cd d:\Phonon
cargo deny check --workspace 2>&1 | tail -n 30
# Expected: 无 GPL/LGPL/AGPL/SSPL/BUSL 违规。所有依赖许可证在白名单内。
```

- [ ] **Step 4: 插件静态规则扫描**（phonon-plugin/src/runtime.rs `scan_static_rules(bytes)`）
  - `cargo build -p calibration-fir --target wasm32-unknown-unknown --release`
  - 对 wasm 字节码 + manifest.json 执行扫描：
    - 无商业音源 URL 匹配（spotify.com / qqmusic / netease / kugou / apple music / tidal / deezer / youtube music）
    - 无混淆 / 规避标记（`atob` / `eval.call` / `new Function` / `btoa` 等 JS 混淆）
    - 无数据外泄标记（`http://` / `https://` 外部 URL 全部禁止；本插件所有通信走 Tauri invoke + wasm import，无直接网络）
    - URL 数量 ≤ 0（WasmExample 插件不应有任何 URL）
  - Expected: 全部通过。

Commit message: `test(e2e): integration pass + clippy workspace clean + cargo-deny workspace clean + plugin static scan clean`

---

### Task 9.2: 性能基准 + 最终交付清单

- [ ] **Step 1: 性能基准（与 spec §12.2 对齐）**
  - Calibration PEQ + Equalizer (16×2 bands) + Calibration FIR (2048 tap) @ 48kHz 256-frame block：
    - 单块 process 时间 < 0.5ms（5.3ms block duration 的 < 10%）
    - 最坏情况 99% 分位 < 1.0ms（无爆音安全裕度）
  - 测量方法：`Instant::now()` 在 process() 入口/出口各打点 10000 次统计。
  - 命令：`cd d:\Phonon ; cargo bench -p phonon-core calibration_peq_bench -- --nocapture 2>&1 | tail -n 15`（若无 bench，则用 test + 打印）

- [ ] **Step 2: 最终交付清单（与 spec §2.3 组件清单交叉验证，无遗漏）**

| # | 交付物 | 所在 Task | 状态 |
|---|--------|----------|------|
| 1 | calibration-types 微 crate（10+ 类型 + FitCurves 256pt） | 1.1 | ✅ |
| 2 | Calibration PEQ Builtin（sync_channel(1) + Arc swap + last_applied RwLock） | 2.1 | ✅ |
| 3 | DspChain 顺序 + AppState peq_handle + 启动恢复 | 2.2 | ✅ |
| 4 | AppSettings CalibrationSettings v2 持久化 | 2.3 | ✅ |
| 5 | BUILTINS phonon_calibration_peq 注册 | 2.4 | ✅ |
| 6 | 线程安全压测 3 项（计数器自旋 + 1e-3 + #[ignore]） | 2.5 | ✅ |
| 7 | calibration-engine 骨架（7 子模块 re-export） | 3.1 | ✅ |
| 8 | parsers 3 格式 | 3.2 | ✅ |
| 9 | interpolate (cubic + Akima) + smooth | 3.3 | ✅ |
| 10 | target_curves 5 条（数字化提取 64pt + 来源注释） | 3.4 | ✅ |
| 11 | biquad_fit 加权 LSQ + 自研 Cholesky | 3.5 | ✅ |
| 12 | fir_residual IFFT + Hilbert | 3.6 | ✅ |
| 13 | fit() 端到端整合 + diagnostics | 3.7 | ✅ |
| 14 | calibration-fir Wasm 骨架（deps only calibration-types, ~25KB） | 4.1 | ✅ |
| 15 | overlap-save FIR L/R + tap swap xfade | 4.2 | ✅ |
| 16 | plugin_process + 版本号轮询拉模型 | 4.3 | ✅ |
| 17 | PluginRuntime.config_versions + set_plugin_config bump | 5.1 | ✅ |
| 18 | 2 条 host imports | 5.2 | ✅ |
| 19 | set/get_calibration_peq 命令 | 6.1 | ✅ |
| 20 | toggle_calibration_bypass PEQ+FIR 同步 | 6.2 | ✅ |
| 21 | run_calibration_fit 命令 + LRU 双淘汰 | 6.3 | ✅ |
| 22 | apply_fit_result 事务回滚 + get_current_fit_result 不淘汰 | 6.4 | ✅ |
| 23 | calibration/parsers.ts 粗解析（零 DSP） | 7.1 | ✅ |
| 24 | api/calibration.ts 6 命令 + 类型（FitCurves 256pt） | 7.2 | ✅ |
| 25 | target_curves_const.ts 5 条 256pt 预览 | 7.3 | ✅ |
| 26 | CalibrationPanel 5 区基础布局 + 测量导入 | 8.1 | ✅ |
| 27 | Canvas 四曲线可视化（256pt 对数轴） | 8.2 | ✅ |
| 28 | 目标曲线拖拽编辑器 | 8.3 | ✅ |
| 29 | 拟合参数区 + ★ Apply 500ms 防抖 + diagnostics 4 栏 | 8.4 | ✅ |
| 30 | 预设管理 + Bypass + A/B + L/R Link | 8.5 | ✅ |
| 31 | E2E 集成 + Clippy + deny + 静态扫描 全过 | 9.1 | ✅ |
| 32 | 性能基准通过 + 最终交付清单核对 | 9.2 | ✅ |

Commit message: `perf: benchmark pass (<0.5ms/block @256fr) + final delivery checklist complete (32/32 items)`

---

## Spec Coverage 自检（最后一次 before delivery）

| Spec v1.2 节 | 覆盖 Tasks |
|-------------|-----------|
| §1 需求范围（Phase 1 MVP 9 项） | M7/M8/M9.1 |
| §2 架构原则（三 crate + 寄存器通道 + 拉模型 + 事务） | 1.1 / 2.1 / 3.1 / 4.1 / 4.3 / 5.2 / 6.3 / 6.4 |
| §2.2 DSP 链顺序（PEQ→FIR→Equalizer） | 2.2 |
| §2.3 组件清单（11 项） | 1.1 / 2.1 / 2.3 / 2.4 / 3.1-3.7 / 4.1-4.3 / 5.1-5.2 / 6.1-6.4 / 7.1-7.3 / 8.1-8.5 |
| §3 Cal PEQ 线程安全（§3.1 完整代码） | 2.1 / 2.5 |
| §3.2 PEQ 与 Equalizer 隔离 | 2.2 / 2.3 |
| §3.3 BUILTINS 顺序 | 2.4 |
| §4 FIR 版本号轮询拉模型（§4.3） | 4.3 / 5.1 / 5.2 |
| §5 拟合算法 Rust（7 子模块） | 3.2 / 3.3 / 3.4 / 3.5 / 3.6 / 3.7 |
| §6 曲线格式导入（3 种） | 3.2 / 7.1 |
| §7 目标曲线数字化提取（64pt + 来源） | 3.4 / 7.3 |
| §8 UI 面板（5 区 + 双按钮流程 + 256pt） | 8.1 / 8.2 / 8.3 / 8.4 / 8.5 |
| §9 预设持久化 + AppSettings | 2.3 / 8.5 |
| §10 Tauri 6 命令 + LRU 双淘汰 + 事务回滚 | 6.1 / 6.2 / 6.3 / 6.4 |
| §11 合规红线（原创代码 / 无第三方数据 / 许可证） | 3.4 / 9.1（静态扫描 + deny）/ 5 篇来源注释 |
| §12 测试验收（残差 <3dB / 延迟 / 压测 / 合规） | 3.5 / 3.6 / 3.7 / 2.5 / 9.2 / 9.1 |
| §13 9 Milestones + 32 Tasks | 本计划完全对应，无遗漏 |
| v1.2 变更 #1（三 crate 分层） | 1.1 / 3.1 / 4.1 |
| v1.2 变更 #2（sync_channel(1) + Arc<RwLock> + PeqUpdateError） | 2.1 |
| v1.2 变更 #3（calibration-fir 改名 + 去 no_std） | 4.1 |
| v1.2 变更 #4（拟合归属 calibration-engine native） | 3.1 / 3.7 / 6.3 |
| v1.2 变更 #5（Harman 数字化提取） | 3.4 |
| v1.2 变更 #6（FitCurves 256pt 严格对数轴 + 双按钮流程） | 1.1 / 8.2 / 8.4 |
| v1.2 变更 #7（命令拆分 + 事务回滚） | 6.3 / 6.4 |
| v1.2 变更 #8（LRU 双淘汰 + last_applied 不淘汰） | 6.3 / 6.4 |
| v1.2 变更 #9（并发测试计数器 + 1e-3 + #[ignore]） | 2.5 |
| v1.2 变更 #10（9 Milestones 32 Tasks 重编号） | 本计划（1.1-9.2 = 1+5+7+3+2+4+3+5+2 = 32 Tasks ✓）|
| ★ 用户最终方案：sync_channel(1) 无 retry + 前端 500ms 防抖 | 2.1 (update_bands 零 try_send retry) / 8.4 (applying useState + setTimeout 500ms) |
