//! Playback engine - orchestrates decoding, buffering, DSP, and output.
//!
//! Manages the full audio pipeline from file to output device.
//! Supports gapless playback with a persistent decode thread and play queue.
//!
//! Architecture:
//!   Decode thread (per-track, with resampling) → RingBuffer → Output callback
//!   Output stream runs at a fixed device rate (e.g. 48kHz).
//!   Each track is resampled to the output rate before writing to the ring buffer.

use crate::dsd::{DeltaSigmaModulator, DsdMode};
use phonon_codec::*;
use rubato::{FftFixedIn, Resampler};
use rustfft::{num_complex::Complex, Fft, FftPlanner};
use std::cell::Cell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use timestretch::{EdmPreset, QualityMode, StreamProcessor, StretchParams};

use crate::device::{DeviceError, DeviceManager};
use crate::dsp::DspChain;
use crate::ringbuf::RingBuffer;
use crate::volume::VolumeControl;

// ── Unified event system ─────────────────────────────────────

/// Application-level events pushed to the frontend via broadcast channel.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type")]
pub enum AppEvent {
    /// Device hotplug (connect/disconnect).
    DeviceHotplug {
        event: String,
        device_name: String,
        is_default: bool,
    },
    /// Playback progress update.
    PlaybackProgress {
        position_secs: f64,
        duration_secs: Option<f64>,
    },
    /// Spectrum visualization data (FFT magnitudes).
    SpectrumData { values: Vec<f32> },
    /// Rich audio features for advanced visualizers (waveform, RMS, beat, chroma, …).
    /// Emitted alongside SpectrumData at the same cadence (~43 Hz for 1024-sample FFT at 44.1 kHz).
    AudioFeaturesData {
        /// FFT magnitude bands (normalized 0..1), same length as SpectrumData.values.
        spectrum: Vec<f32>,
        /// Down-mixed mono waveform (typically 512 samples, -1..1).
        waveform: Vec<f32>,
        /// RMS volume (0..1).
        rms: f32,
        /// Peak sample value (0..1).
        peak: f32,
        /// Spectral centroid in Hz (brightness).
        spectral_centroid_hz: f32,
        /// Onset confidence 0..1 (spikes on transients).
        onset: f32,
        /// True if a beat was detected in this analysis window.
        beat: bool,
        /// Estimated BPM (smoothed). 0 if unknown.
        bpm: f32,
        /// 12-bin chroma / pitch-class profile (C, C#, D, …, B). Sum-normalized.
        chroma: [f32; 12],
        /// Sample rate of the analyzed block (info only).
        sample_rate: u32,
        /// Channel count of the analyzed block (info only).
        channels: u16,
    },
    /// Plugin status change.
    PluginStatus {
        id: String,
        status: String,
        message: String,
    },
    /// Wasm 插件发起的自定义事件（经宿主 `host.emit_event` host import 转发）。
    /// 命名约定：Builtin 插件直接通过 EVENT_TX.send(AppEvent::PluginStatus 或其他专用变体)；
    /// 所有 Wasm 插件无论 WasmExample / WasmExternal，若想向 UI 发送任意 JSON 事件都必须走这条通用通道，
    /// 宿主在 runtime.rs 的 linker_init 中提供 `host::emit_event(name_ptr, name_len, payload_ptr, payload_len) -> i32`。
    PluginUserEvent {
        /// 事件名（由 Wasm 插件决定，例如 "gain-changed"、"custom-button-pressed"）。
        name: String,
        /// JSON 字符串载荷（宿主不假设 schema，直接透传给前端）。
        payload: String,
    },
    /// Playback state changed (Playing/Paused/Stopped/Idle).
    PlaybackStateChanged { state: String },
    /// Volume level changed (from external source like system volume).
    VolumeChanged { level: f32 },
    /// Library sort_key backfill progress (Plan A Task A2). Emitted by the
    /// background `spawn_sort_key_backfill` thread every 5% so the Settings
    /// UI card + EmptyStateGuide progress bar can animate. `phase` is a
    /// short lowercase slug like "sort_key" or "done".
    LibraryMigrationProgress {
        phase: String,
        processed: i64,
        total: u64,
        pct: u8,
    },
    /// Emitted once when MediaLibrary::open detects a corrupt SQLite DB and
    /// replaces it with a fresh empty one (Plan A Task A6 §5.10 #4). The
    /// frontend shows a red banner with the backup path so the user can
    /// recover their data.
    LibraryCorrupted { backup_path: String },
}

/// Global broadcast channel for app events.
/// Initialized by the Tauri setup, used by engine and hotplug monitor.
pub static EVENT_TX: std::sync::OnceLock<tokio::sync::broadcast::Sender<AppEvent>> =
    std::sync::OnceLock::new();

// ── Time-stretch trait ────────────────────────────────────────

/// Interface for pitch-preserving speed change.
/// Built-in implementation uses phase vocoder; plugin-based
/// implementations (e.g., Rubber Band, AI models) can be loaded
/// dynamically and automatically replace the built-in.
pub trait TimeStretch: Send {
    /// Push input samples, get stretched output.
    fn push(&mut self, input: &[f32], output: &mut Vec<f32>);
    /// Flush remaining samples at end of stream.
    fn flush(&mut self, output: &mut Vec<f32>);
    /// Reset internal state.
    fn reset(&mut self);
    /// Update speed in-place without resetting phase state.
    /// Default implementation recreates the stretcher; override
    /// for phase-preserving speed changes.
    fn set_speed(&mut self, _new_speed: f32) {}
}

/// Factory for creating TimeStretch instances per decode session.
pub type TimeStretchFactory = Arc<dyn Fn(f32, u16, u32) -> Box<dyn TimeStretch> + Send + Sync>;

/// 返回内置 phase vocoder 工厂（公开入口，供 Tauri 命令层与测试使用）。
///
/// 工厂签名：`(speed, channels, _sample_rate) -> Box<dyn TimeStretch>`。
/// `_sample_rate` 预留供未来基于采样率的窗长选择。
///
/// **Backwards-compat shim** — 等价于 `time_stretch_factory(TimeStretchMode::Auto)`。
pub fn default_time_stretch_factory() -> TimeStretchFactory {
    time_stretch_factory(TimeStretchMode::Auto)
}

// ── Time-stretch mode selector ────────────────────────────────

/// 用户可选的变速引擎算法。持久化在 `AppSettings.dsp.time_stretch_mode`。
///
/// - Auto（默认）：混合模式。0.8x ≤ speed ≤ 1.25x → timestretch crate（WSOLA hybrid，
///   瞬态最锐利，适合人声/打击乐）。范围外 → 内置 Phase Vocoder（极端变速抗涂抹）。
///   **Auto 永不失败** — 如果 timestretch crate 在构造时 panic，静默回退到内置 Phase Vocoder。
/// - Wsola：全速率强制 timestretch crate。init 失败时回退 Phase Vocoder。
/// - PhaseVocoder：全速率强制内置 Phase Vocoder。零外部依赖。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]   // ← TS 侧 'auto' | 'wsola' | 'phase_vocoder' 必须匹配
pub enum TimeStretchMode {
    #[default]
    Auto,
    Wsola,
    PhaseVocoder,
}

/// 返回统一工厂：(speed, channels, sample_rate) → Box<dyn TimeStretch>。
///
/// 选择规则：
/// ```text
/// Mode | 0.8 ≤ speed ≤ 1.25        | speed < 0.8 OR speed > 1.25
/// -----+---------------------------+----------------------------
/// Auto | TimestretchStretcher(WSOLA) | TimeStretcher(phase vocoder)
/// Wsola| TimestretchStretcher(失败回退 PhaseVocoder)
/// PV   | PhaseVocoder(零回退)
/// ```
///
/// 构造时回退：当 `TimestretchStretcher::new` 被选中但 panic 时，
/// 通过 `std::panic::catch_unwind` 捕获并静默替换为 `TimeStretcher::new`。
/// 此函数永不返回错误 / 永不 panic — 解码线程依赖此保证。
pub fn time_stretch_factory(mode: TimeStretchMode) -> TimeStretchFactory {
    Arc::new(move |speed, channels, sample_rate| {
        let use_wsola = match mode {
            TimeStretchMode::Auto => (0.8..=1.25).contains(&speed),
            TimeStretchMode::Wsola => true,
            TimeStretchMode::PhaseVocoder => false,
        };

        if use_wsola {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                TimestretchStretcher::new(speed, channels, sample_rate)
            }));
            match result {
                Ok(stretcher) => {
                    log::debug!(
                        "[TimeStretch] selected WSOLA hybrid (mode={:?}, speed={:.2}x, ch={}, sr={})",
                        mode, speed, channels, sample_rate
                    );
                    return Box::new(stretcher);
                }
                Err(payload) => {
                    let msg = payload
                        .downcast_ref::<&str>()
                        .copied()
                        .unwrap_or("<non-string panic payload>");
                    log::warn!(
                        "[TimeStretch] WSOLA init FAILED ({msg}), falling back to built-in Phase Vocoder"
                    );
                }
            }
        }
        // 回退 / 强制路径
        log::debug!(
            "[TimeStretch] selected built-in Phase Vocoder (mode={:?}, speed={:.2}x)",
            mode, speed
        );
        Box::new(TimeStretcher::new(speed, channels as usize))
    })
}

/// Plugin push callback: `(input, output) -> Result<(), String>`.
type PluginPushFn = Box<dyn FnMut(&[f32], &mut Vec<f32>) -> Result<(), String> + Send>;
/// Plugin flush callback: `(output) -> Result<(), String>`.
type PluginFlushFn = Box<dyn FnMut(&mut Vec<f32>) -> Result<(), String> + Send>;

/// A TimeStretch implementation backed by a WASM plugin.
/// Created by the Tauri command layer from plugin runtime callbacks.
/// Falls back to the built-in phase vocoder if the plugin errors at runtime.
pub struct PluginTimeStretcher {
    push_fn: PluginPushFn,
    flush_fn: PluginFlushFn,
    reset_fn: Box<dyn FnMut() -> Result<(), String> + Send>,
    set_speed_fn: Box<dyn FnMut(f32) -> Result<(), String> + Send>,
    fallback: TimeStretcher,
    use_fallback: bool,
}

impl PluginTimeStretcher {
    pub fn new(
        push_fn: PluginPushFn,
        flush_fn: PluginFlushFn,
        reset_fn: Box<dyn FnMut() -> Result<(), String> + Send>,
        set_speed_fn: Box<dyn FnMut(f32) -> Result<(), String> + Send>,
        speed: f32,
        channels: u16,
    ) -> Self {
        Self {
            push_fn,
            flush_fn,
            reset_fn,
            set_speed_fn,
            fallback: TimeStretcher::new(speed, channels as usize),
            use_fallback: false,
        }
    }
}

impl TimeStretch for PluginTimeStretcher {
    fn push(&mut self, input: &[f32], output: &mut Vec<f32>) {
        if self.use_fallback {
            self.fallback.push(input, output);
            return;
        }
        if let Err(e) = (self.push_fn)(input, output) {
            log::error!(
                "[PluginTimeStretch] push error: {}, falling back to built-in phase vocoder",
                e
            );
            self.use_fallback = true;
            self.fallback.push(input, output);
        }
    }

    fn flush(&mut self, output: &mut Vec<f32>) {
        if self.use_fallback {
            self.fallback.flush(output);
            return;
        }
        if let Err(e) = (self.flush_fn)(output) {
            log::error!(
                "[PluginTimeStretch] flush error: {}, falling back to built-in phase vocoder",
                e
            );
            self.use_fallback = true;
            self.fallback.flush(output);
        }
    }

    fn reset(&mut self) {
        if self.use_fallback {
            self.fallback.reset();
        } else {
            // Reset plugin state; if it fails, switch to fallback
            if let Err(e) = (self.reset_fn)() {
                log::error!(
                    "[PluginTimeStretch] reset error: {}, falling back to built-in phase vocoder",
                    e
                );
                self.use_fallback = true;
            }
        }
    }

    fn set_speed(&mut self, new_speed: f32) {
        self.fallback.set_speed(new_speed);
        if !self.use_fallback {
            if let Err(e) = (self.set_speed_fn)(new_speed) {
                log::error!(
                    "[PluginTimeStretch] set_speed error: {}, falling back to built-in phase vocoder",
                    e
                );
                self.use_fallback = true;
            }
        }
    }
}

// ── Rubber Band quality time-stretcher (via timestretch crate) ──

/// High-quality time-stretcher backed by the `timestretch` crate.
/// Uses hybrid phase vocoder + WSOLA with sub-bass phase locking.
/// Selected by `time_stretch_factory()` when mode=Auto (0.8x-1.25x) or Wsola.
struct TimestretchStretcher {
    processor: StreamProcessor,
    channels: usize,
    sample_rate: u32,
    speed: f32,
}

impl TimestretchStretcher {
    fn new(speed: f32, channels: u16, sample_rate: u32) -> Self {
        let ratio = (1.0 / speed) as f64;
        let params = StretchParams::new(ratio)
            .with_preset(EdmPreset::Ambient)
            .with_sample_rate(sample_rate)
            .with_channels(channels as u32)
            .with_quality_mode(QualityMode::Balanced);
        let processor = StreamProcessor::new(params);
        Self {
            processor,
            channels: channels as usize,
            sample_rate,
            speed,
        }
    }
}

impl TimeStretch for TimestretchStretcher {
    fn push(&mut self, input: &[f32], output: &mut Vec<f32>) {
        // Reserve capacity: output can be up to ~4x input for extreme slowdowns
        let estimate = (input.len() as f64 / self.speed as f64).ceil() as usize;
        output.reserve(estimate.saturating_sub(output.len()));
        if let Err(e) = self.processor.process_into(input, output) {
            log::error!("[Timestretch] push error: {}", e);
        }
    }

    fn flush(&mut self, output: &mut Vec<f32>) {
        output.reserve(4096);
        if let Err(e) = self.processor.flush_into(output) {
            log::error!("[Timestretch] flush error: {}", e);
        }
    }

    fn reset(&mut self) {
        let ratio = (1.0 / self.speed) as f64;
        let params = StretchParams::new(ratio)
            .with_preset(EdmPreset::Ambient)
            .with_sample_rate(self.sample_rate)
            .with_channels(self.channels as u32)
            .with_quality_mode(QualityMode::Balanced);
        self.processor = StreamProcessor::new(params);
    }

    fn set_speed(&mut self, new_speed: f32) {
        let ratio = (1.0 / new_speed) as f64;
        self.speed = new_speed;
        if let Err(e) = self.processor.set_stretch_ratio(ratio) {
            log::error!("[Timestretch] set_speed error: {}", e);
        }
    }
}

// ── Phase vocoder time-stretcher (built-in) ───────────────────

/// Phase vocoder for pitch-preserving speed change.
/// Uses sqrt(Hann) window (w²=Hann satisfies COLA at all hop divisors),
/// fixed hop_out=N/4 for consistent 4x synthesis overlap,
/// frequency-dependent phase locking (wider radius for bass, narrower for treble),
/// transient detection with phase reset to prevent smearing on attacks,
/// 7-point magnitude-weighted delta smoothing, and -80dB noise threshold.
struct TimeStretcher {
    fft_size: usize,
    hop_in: usize,
    hop_out: usize,
    channels: usize,
    window: Vec<f32>,
    norm: f32,
    input_buf: Vec<f32>,
    out_buf: Vec<f32>,
    out_pos: usize,
    fft: Arc<dyn Fft<f32>>,
    ifft: Arc<dyn Fft<f32>>,
    prev_phase: Vec<Vec<f32>>,
    synth_phase: Vec<Vec<f32>>,
    first_frame: Vec<bool>,
    fft_in: Vec<Complex<f32>>,
    fft_out: Vec<Complex<f32>>,
    ifft_in: Vec<Complex<f32>>,
    ifft_out: Vec<Complex<f32>>,
    delta_buf: Vec<f32>,
    delta_tmp: Vec<f32>,
    /// Peak bin assigned to each bin (for correct center frequency in Pass 5).
    /// Unlocked bins store their own index.
    peak_bin: Vec<Vec<usize>>,
    /// Per-channel EMA of frame energy for transient detection.
    avg_energy: Vec<f32>,
}

impl TimeStretcher {
    fn new(speed: f32, channels: usize) -> Self {
        let fft_size = 4096;
        let divisors: [usize; 5] = [128, 256, 512, 1024, 2048];
        // Fixed hop_out = N/4 = 1024 gives consistent 4x synthesis overlap
        // at all speeds. Previously hop_out varied with speed, causing 2x
        // overlap at low speeds (stuttering) and 8x at high speeds (volume loss).
        let ideal = (fft_size as f32 / 4.0).clamp(64.0, fft_size as f32 / 2.0);
        let hop_out = divisors
            .iter()
            .min_by_key(|&&d| (d as f32 - ideal).abs() as i32)
            .copied()
            .unwrap_or(512);
        let hop_in = ((hop_out as f32 * speed).clamp(64.0, fft_size as f32)) as usize;
        let window: Vec<f32> = (0..fft_size)
            .map(|i| {
                (0.5 * (1.0
                    - (2.0 * std::f64::consts::PI * i as f64 / (fft_size - 1) as f64).cos()))
                .sqrt() as f32
            })
            .collect();
        let norm = (2.0 * hop_out as f32) / (fft_size * fft_size) as f32;
        let num_bins = fft_size / 2 + 1;
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(fft_size);
        let ifft = planner.plan_fft_inverse(fft_size);
        Self {
            fft_size,
            hop_in,
            hop_out,
            channels,
            window,
            norm,
            input_buf: Vec::new(),
            out_buf: Vec::new(),
            out_pos: 0,
            fft,
            ifft,
            prev_phase: vec![vec![0.0f32; num_bins]; channels],
            synth_phase: vec![vec![0.0f32; num_bins]; channels],
            first_frame: vec![true; channels],
            fft_in: vec![Complex::new(0.0, 0.0); fft_size],
            fft_out: vec![Complex::new(0.0, 0.0); fft_size],
            ifft_in: vec![Complex::new(0.0, 0.0); fft_size],
            ifft_out: vec![Complex::new(0.0, 0.0); fft_size],
            delta_buf: vec![0.0f32; num_bins],
            delta_tmp: vec![0.0f32; num_bins],
            peak_bin: vec![vec![0usize; num_bins]; channels],
            avg_energy: vec![0.0f32; channels],
        }
    }

    fn process_channel(&mut self, ch_idx: usize) {
        let n = self.fft_size;
        let num_bins = n / 2 + 1;
        let hop_in = self.hop_in;
        let hop_out = self.hop_out;
        let two_pi = 2.0 * std::f64::consts::PI as f32;

        // ── Transient detection with phase reset ──
        // Detect sharp energy increases to prevent attack smearing.
        // On transient frames, reset phase state so the transient is
        // re-synthesized at the correct position without phase propagation.
        let frame_energy: f32 = self.fft_in.iter().map(|c| c.re * c.re).sum::<f32>() / n as f32;
        if self.avg_energy[ch_idx] > 1e-10 {
            let ratio = frame_energy / self.avg_energy[ch_idx];
            if ratio > 3.0 {
                self.first_frame[ch_idx] = true;
            }
        }
        self.avg_energy[ch_idx] = 0.9 * self.avg_energy[ch_idx] + 0.1 * frame_energy;

        // FFT
        self.fft_out.copy_from_slice(&self.fft_in);
        self.fft.process(&mut self.fft_out);

        if self.first_frame[ch_idx] {
            self.first_frame[ch_idx] = false;
            for k in 0..num_bins {
                let re = self.fft_out[k].re;
                let im = self.fft_out[k].im;
                let mag = (re * re + im * im).sqrt();
                let phase = im.atan2(re);
                self.prev_phase[ch_idx][k] = phase;
                self.synth_phase[ch_idx][k] = phase;
                self.peak_bin[ch_idx][k] = k; // init: each bin is its own peak
                self.ifft_in[k] = Complex::new(mag * phase.cos(), mag * phase.sin());
            }
        } else {
            // Pass 1: compute raw phase deltas; store magnitudes in delta_tmp
            for k in 0..num_bins {
                let re = self.fft_out[k].re;
                let im = self.fft_out[k].im;
                let mag = (re * re + im * im).sqrt();
                let phase = im.atan2(re);
                let expected = two_pi * k as f32 * hop_in as f32 / n as f32;
                let mut delta = phase - self.prev_phase[ch_idx][k] - expected;
                delta -= two_pi * (delta / two_pi + 0.5).floor();
                self.delta_buf[k] = delta;
                self.delta_tmp[k] = mag;
                self.prev_phase[ch_idx][k] = phase;
            }

            // Pass 2: Ultra-light phase locking — only within 2 bins of peaks,
            // with very weak blend (max 0.5). Reduces phasiness without the
            // cross-harmonic frequency errors that cause metallic sound.
            // Does NOT use peak_bin — keeps bin's own center frequency.
            let orig_deltas = self.delta_buf.clone();
            let max_mag = self.delta_tmp[..num_bins]
                .iter()
                .cloned()
                .fold(0.0f32, f32::max);
            let peak_threshold = max_mag * 0.001; // -60 dB

            let mut peaks: Vec<usize> = Vec::new();
            for k in 2..num_bins - 2 {
                let m = self.delta_tmp[k];
                if m > peak_threshold
                    && m > self.delta_tmp[k - 1]
                    && m > self.delta_tmp[k - 2]
                    && m > self.delta_tmp[k + 1]
                    && m > self.delta_tmp[k + 2]
                {
                    peaks.push(k);
                }
            }

            if !peaks.is_empty() {
                for k in 0..num_bins {
                    let mut best_peak = peaks[0];
                    let mut best_dist = (k as isize - peaks[0] as isize).unsigned_abs();
                    for &p in &peaks[1..] {
                        let d = (k as isize - p as isize).unsigned_abs();
                        if d < best_dist {
                            best_dist = d;
                            best_peak = p;
                        }
                    }
                    let d = best_dist as f32;
                    // Frequency-dependent lock radius: bass bins get wider lock
                    // because low-frequency harmonics are more harmonically related.
                    // Treble bins get narrow lock to preserve transient detail.
                    let radius = if k < 40 {
                        4.0
                    } else if k < 160 {
                        3.0
                    } else if k < 500 {
                        2.0
                    } else {
                        1.0
                    };
                    let blend = if d <= radius {
                        let peak_strength =
                            (self.delta_tmp[best_peak] / (max_mag + 1e-10)).min(1.0);
                        (0.6 * peak_strength) / (1.0 + d * d * 0.3)
                    } else {
                        0.0
                    };
                    self.delta_buf[k] =
                        blend * orig_deltas[best_peak] + (1.0 - blend) * orig_deltas[k];
                }
            }

            // Pass 3: 7-point magnitude-weighted smoothing of deltas.
            // alpha=0.5: 50% locked, 50% smoothed.
            let locked_deltas = self.delta_buf.clone();
            let alpha: f32 = 0.5;
            for k in 3..num_bins - 3 {
                let m0 = self.delta_tmp[k - 3];
                let m1 = self.delta_tmp[k - 2];
                let m2 = self.delta_tmp[k - 1];
                let m3 = self.delta_tmp[k];
                let m4 = self.delta_tmp[k + 1];
                let m5 = self.delta_tmp[k + 2];
                let m6 = self.delta_tmp[k + 3];
                let total = m0 + m1 + m2 + m3 + m4 + m5 + m6;
                if total > 0.0 {
                    let smoothed = (m0 * locked_deltas[k - 3]
                        + m1 * locked_deltas[k - 2]
                        + m2 * locked_deltas[k - 1]
                        + m3 * locked_deltas[k]
                        + m4 * locked_deltas[k + 1]
                        + m5 * locked_deltas[k + 2]
                        + m6 * locked_deltas[k + 3])
                        / total;
                    self.delta_buf[k] = alpha * locked_deltas[k] + (1.0 - alpha) * smoothed;
                }
            }

            // Pass 4: -80dB amplitude threshold — noise-floor bins use
            // expected phase advance only (no delta), preventing artifacts.
            let noise_threshold = max_mag * 0.0001; // -80dB

            // Pass 5: apply deltas to synthesis phases.
            let phase_advance_base = two_pi * hop_out as f32 / n as f32;
            for k in 0..num_bins {
                let re = self.fft_out[k].re;
                let im = self.fft_out[k].im;
                let mag = (re * re + im * im).sqrt();

                if mag > noise_threshold {
                    let inst_freq = (two_pi * k as f32 * hop_in as f32 / n as f32
                        + self.delta_buf[k])
                        / hop_in as f32;
                    self.synth_phase[ch_idx][k] += inst_freq * hop_out as f32;
                } else {
                    self.synth_phase[ch_idx][k] += phase_advance_base * k as f32;
                }
                let sp = self.synth_phase[ch_idx][k];

                self.ifft_in[k] = Complex::new(mag * sp.cos(), mag * sp.sin());
            }
        }

        // Conjugate symmetric fill
        for k in num_bins..n {
            let conj_k = n - k;
            self.ifft_in[k] = Complex::new(self.ifft_in[conj_k].re, -self.ifft_in[conj_k].im);
        }

        // IFFT
        self.ifft_out.copy_from_slice(&self.ifft_in);
        self.ifft.process(&mut self.ifft_out);

        // Overlap-add (interleaved)
        let ch = self.channels;
        let needed = (self.out_pos + n) * ch;
        if self.out_buf.len() < needed {
            self.out_buf.resize(needed, 0.0);
        }
        for i in 0..n {
            self.out_buf[(self.out_pos + i) * ch + ch_idx] +=
                self.ifft_out[i].re * self.window[i] * self.norm;
        }
    }

    fn push(&mut self, input: &[f32], output: &mut Vec<f32>) {
        self.input_buf.extend_from_slice(input);
        let ch = self.channels;
        let n = self.fft_size;
        let hop_in = self.hop_in;
        let hop_out = self.hop_out;

        while self.input_buf.len() >= n * ch {
            let frame_data: Vec<f32> = self.input_buf[..n * ch].to_vec();
            for c in 0..ch {
                let c_offset = c;
                for i in 0..n {
                    self.fft_in[i] =
                        Complex::new(frame_data[i * ch + c_offset] * self.window[i], 0.0);
                }
                self.process_channel(c);
            }
            self.out_pos += hop_out;
            self.input_buf.drain(..hop_in * ch);
        }
        self.drain_output(output);
    }

    fn drain_output(&mut self, output: &mut Vec<f32>) {
        let ch = self.channels;
        let keep = self.fft_size * ch;
        let hop_samples = self.hop_out * ch;
        if self.out_buf.len() > keep {
            let drain_len = ((self.out_buf.len() - keep) / hop_samples) * hop_samples;
            if drain_len > 0 {
                output.extend_from_slice(&self.out_buf[..drain_len]);
                self.out_buf.drain(..drain_len);
                self.out_pos = self.out_pos.saturating_sub(drain_len / ch);
            }
        }
    }

    fn flush(&mut self, output: &mut Vec<f32>) {
        let ch = self.channels;
        let n = self.fft_size;
        if self.input_buf.len() >= ch {
            let needed = n * ch;
            self.input_buf.resize(needed, 0.0);
            let frame_data: Vec<f32> = self.input_buf[..needed].to_vec();
            for c in 0..ch {
                let c_offset = c;
                for i in 0..n {
                    self.fft_in[i] =
                        Complex::new(frame_data[i * ch + c_offset] * self.window[i], 0.0);
                }
                self.process_channel(c);
            }
            self.out_pos += self.hop_out;
            self.input_buf.clear();
        }
        if !self.out_buf.is_empty() {
            output.extend_from_slice(&self.out_buf);
            self.out_buf.clear();
            self.out_pos = 0;
        }
    }

    fn reset(&mut self) {
        self.input_buf.clear();
        self.out_buf.clear();
        self.out_pos = 0;
        let _num_bins = self.fft_size / 2 + 1;
        for ch in 0..self.channels {
            self.prev_phase[ch].fill(0.0);
            self.synth_phase[ch].fill(0.0);
            for k in 0..self.peak_bin[ch].len() {
                self.peak_bin[ch][k] = k;
            }
        }
        self.first_frame.fill(true);
        self.avg_energy.fill(0.0);
    }
}

impl TimeStretch for TimeStretcher {
    fn push(&mut self, input: &[f32], output: &mut Vec<f32>) {
        TimeStretcher::push(self, input, output);
    }
    fn flush(&mut self, output: &mut Vec<f32>) {
        TimeStretcher::flush(self, output);
    }
    fn reset(&mut self) {
        TimeStretcher::reset(self);
    }
    /// Update speed in-place without resetting phase state.
    /// Preserves phase continuity across speed changes for smooth transitions.
    /// Uses fixed hop_out = N/4 for consistent 4x synthesis overlap at all speeds.
    fn set_speed(&mut self, new_speed: f32) {
        let divisors: [usize; 5] = [128, 256, 512, 1024, 2048];
        let ideal = (self.fft_size as f32 / 4.0).clamp(64.0, self.fft_size as f32 / 2.0);
        self.hop_out = divisors
            .iter()
            .min_by_key(|&&d| (d as f32 - ideal).abs() as i32)
            .copied()
            .unwrap_or(512);
        self.hop_in =
            ((self.hop_out as f32 * new_speed).clamp(64.0, self.fft_size as f32)) as usize;
        self.norm = (2.0 * self.hop_out as f32) / (self.fft_size * self.fft_size) as f32;
    }
}

// ── Spectrum analyzer ─────────────────────────────────────────

const SPECTRUM_BANDS: usize = 32;
const SPECTRUM_FFT_SIZE: usize = 1024;

/// Lightweight real-time spectrum analyzer.
/// Accumulates PCM samples and periodically computes FFT magnitudes.
struct SpectrumAnalyzer {
    buffer: Vec<f32>,
    write_pos: usize,
    fft_size: usize,
    bands: usize,
    planner: FftPlanner<f32>,
    // Reusable buffers for compute()
    windowed: Vec<Complex<f32>>,
    band_energies: Vec<f32>,
    band_counts: Vec<usize>,
}

impl SpectrumAnalyzer {
    fn new(fft_size: usize, bands: usize) -> Self {
        Self {
            buffer: vec![0.0f32; fft_size],
            write_pos: 0,
            fft_size,
            bands,
            planner: FftPlanner::new(),
            windowed: vec![Complex::new(0.0f32, 0.0f32); fft_size],
            band_energies: vec![0.0f32; bands],
            band_counts: vec![0usize; bands],
        }
    }

    /// Feed a chunk of interleaved stereo samples. Takes the left channel only.
    fn feed(&mut self, samples: &[f32], channels: u16) -> Option<[f32; SPECTRUM_BANDS]> {
        let ch = channels as usize;
        if ch == 0 {
            return None;
        }
        let stride = ch.max(1);
        for i in (0..samples.len()).step_by(stride) {
            self.buffer[self.write_pos] = samples[i];
            self.write_pos += 1;
            if self.write_pos >= self.fft_size {
                let result = self.compute();
                self.write_pos = 0;
                return Some(result);
            }
        }
        None
    }

    fn compute(&mut self) -> [f32; SPECTRUM_BANDS] {
        let n = self.fft_size;

        // Hann window — reuse self.windowed buffer
        for i in 0..n {
            let w = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (n - 1) as f32).cos());
            self.windowed[i].re = self.buffer[i] * w;
            self.windowed[i].im = 0.0;
        }

        // FFT
        let fft = self.planner.plan_fft_forward(n);
        fft.process(&mut self.windowed);

        // Log-spaced bands — reuse self.band_energies / self.band_counts
        self.band_energies.fill(0.0);
        self.band_counts.fill(0);

        let nyquist = n / 2;
        for bin in 1..nyquist {
            let magnitude = (self.windowed[bin].re * self.windowed[bin].re
                + self.windowed[bin].im * self.windowed[bin].im)
                .sqrt();
            let band = ((bin as f32).ln() / (nyquist as f32).ln() * self.bands as f32) as usize;
            let band = band.min(self.bands - 1);
            self.band_energies[band] += magnitude;
            self.band_counts[band] += 1;
        }

        // Fill empty bands by interpolating from neighbors.
        for b in 0..self.bands {
            if self.band_counts[b] == 0 {
                let left = (0..b)
                    .rev()
                    .find(|&i| self.band_counts[i] > 0)
                    .map(|i| self.band_energies[i] / self.band_counts[i] as f32);
                let right = ((b + 1)..self.bands)
                    .find(|&i| self.band_counts[i] > 0)
                    .map(|i| self.band_energies[i] / self.band_counts[i] as f32);
                match (left, right) {
                    (Some(l), Some(r)) => {
                        self.band_energies[b] = (l + r) / 2.0;
                        self.band_counts[b] = 1;
                    }
                    (Some(l), None) => {
                        self.band_energies[b] = l;
                        self.band_counts[b] = 1;
                    }
                    (None, Some(r)) => {
                        self.band_energies[b] = r;
                        self.band_counts[b] = 1;
                    }
                    (None, None) => {}
                }
            }
        }

        // Normalize: FFT full-scale reference = N/4 (Hann window)
        let ref_level = n as f32 / 4.0;
        let epsilon = 1e-6f32;
        let min_db: f32 = -96.0;
        for b in 0..self.bands {
            if self.band_counts[b] > 0 {
                self.band_energies[b] /= self.band_counts[b] as f32;
            }
            let normalized = (self.band_energies[b] / ref_level).max(epsilon);
            let db = 20.0 * normalized.log10();
            self.band_energies[b] = ((db - min_db) / (-min_db)).clamp(0.0, 1.0);
        }

        let mut out = [0.0f32; SPECTRUM_BANDS];
        for (o, &e) in out.iter_mut().zip(self.band_energies.iter()) {
            *o = e;
        }
        out
    }
}

/// Playback state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PlaybackState {
    Idle,
    Playing,
    Paused,
    Stopped,
}

/// Playback mode for repeat behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackMode {
    /// Play tracks in order, stop at the end.
    Normal,
    /// Repeat the current track.
    RepeatOne,
    /// Repeat the entire queue.
    RepeatAll,
}

/// Playback engine configuration.
pub struct EngineConfig {
    /// Ring buffer size in samples (default: 2 seconds at 48kHz stereo = 192000).
    pub buffer_size: usize,
    /// Device name (empty = default).
    pub device_name: String,
    /// Whether to use exclusive mode.
    pub exclusive: bool,
    /// Fixed output sample rate (0 = auto-detect from device).
    pub output_rate: u32,
    /// Resampling quality.
    pub resampler_quality: ResamplerQuality,
}

/// Resampling quality levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ResamplerQuality {
    /// Fast linear interpolation, lowest CPU.
    Fast = 0,
    /// Balanced Sinc with fixed input length (default).
    Balanced = 1,
    /// Highest quality Sinc, most CPU.
    High = 2,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            buffer_size: 192000,
            device_name: String::new(),
            exclusive: false,
            output_rate: 0,
            resampler_quality: ResamplerQuality::Balanced,
        }
    }
}

/// A public queue item for display.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueueItem {
    pub path: String,
    pub title: String,
    pub artist: Option<String>,
    pub duration: Option<f64>,
}

/// A queued track with its playback parameters.
#[derive(Debug, Clone)]
struct QueuedTrack {
    path: String,
    start_offset: f64,
    duration_limit: Option<f64>,
    display_name: String,
}

/// Hardware volume callback type: `Arc<Mutex<Option<Arc<dyn Fn(f32) + Send + Sync>>>>`.
type HwVolumeCallback = Arc<Mutex<Option<Arc<dyn Fn(f32) + Send + Sync>>>>;

/// The main playback engine.
pub struct PlaybackEngine {
    state: Arc<Mutex<PlaybackState>>,
    ring_buffer: RingBuffer,
    dsp_chain: Arc<Mutex<DspChain>>,
    volume: Arc<Mutex<VolumeControl>>,
    device_manager: Arc<Mutex<DeviceManager>>,
    config: Arc<Mutex<EngineConfig>>,
    /// Flag to signal the decode thread to stop.
    stop_signal: Arc<AtomicBool>,
    /// Flag to skip to the next track.
    skip_signal: Arc<AtomicBool>,
    /// Counter for skip requests — handles rapid double-click without losing skips.
    skip_count: Arc<AtomicUsize>,
    /// Flag to seek within current track, with target position in seconds.
    seek_signal: Arc<AtomicBool>,
    seek_target: Arc<Mutex<f64>>,
    /// Fixed output sample rate (set once when output starts).
    output_rate: Arc<Mutex<SampleRate>>,
    /// Current channel count.
    current_channels: Arc<Mutex<ChannelCount>>,
    /// Play queue for gapless playback.
    play_queue: Arc<Mutex<VecDeque<QueuedTrack>>>,
    /// Currently playing track path (for display).
    current_track: Arc<Mutex<Option<String>>>,
    /// Whether the output stream is running.
    output_running: Arc<AtomicBool>,
    /// Whether playback is paused (avoids locking state Mutex in output callback).
    paused: Arc<AtomicBool>,
    /// Total tracks played (for gapless tracking).
    tracks_played: Arc<Mutex<usize>>,
    /// Approximate current position in seconds.
    current_position: Arc<Mutex<f64>>,
    /// Duration of current track in seconds.
    current_duration: Arc<Mutex<Option<f64>>>,
    /// Current position in the play queue (index, not consuming).
    current_queue_index: Arc<Mutex<usize>>,
    /// Handle for the decode thread (so we can join before starting a new one).
    decode_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Playback speed multiplier (1.0 = normal).
    speed: Arc<Mutex<f32>>,
    /// Flag to signal that speed has changed (triggers re-decode).
    speed_changed: Arc<AtomicBool>,
    /// Hardware volume callback: called when volume changes in HW mode.
    hw_volume_cb: HwVolumeCallback,
    /// Playback mode (Normal, RepeatOne, RepeatAll).
    playback_mode: Arc<Mutex<PlaybackMode>>,
    /// Generation counter incremented on each start_playback/stop.
    /// Decode threads check this to know when they're stale.
    decode_generation: Arc<AtomicU64>,
    /// Optional TimeStretch plugin factory. When set, replaces the built-in
    /// phase vocoder for pitch-preserving speed change.
    time_stretch_factory: Arc<Mutex<Option<TimeStretchFactory>>>,
    /// Bit depth: 0 = float32 passthrough, 16, 24.
    bit_depth: Arc<Mutex<u16>>,
    /// DSD output mode.
    dsd_mode: Arc<Mutex<DsdMode>>,
    /// Sigma-delta modulators for DSD conversion (one per channel).
    dsd_modulators: Arc<Mutex<Vec<DeltaSigmaModulator>>>,
    /// Everything needed to (re)create the output stream at a given rate.
    /// Shared by the engine (initial start, device switch) and the decode
    /// thread (per-track auto sample-rate switching in auto mode).
    output_ctx: OutputStreamCtx,
}

/// Snapshot of one analysis window, produced by the audio callback and
/// consumed by the visualizer publisher thread. Fixed-size arrays keep the
/// real-time side allocation-free (~2 KB copy per window).
struct VisualizerFrame {
    bands: [f32; SPECTRUM_BANDS],
    waveform: [f32; 512],
    rms: f32,
    peak: f32,
    sample_rate: u32,
    channels: u16,
    /// Channel count of the SOURCE data (before mono→stereo upmix) — drives
    /// the publisher's playback clock so mono files advance at the right rate.
    data_channels: u16,
    frames_read: usize,
}

/// Latest-wins slot between the audio callback and the publisher thread.
type VisualizerHub = Arc<Mutex<Option<VisualizerFrame>>>;

/// Cross-callback state used to derive beat / onset / BPM / chroma features
/// directly from PCM + spectrum bands. Kept intentionally lightweight so the
/// audio output callback never blocks more than ~0.5 ms.
struct VisualizerAccum {
    /// Previous-frame RMS (used for onset energy flux).
    prev_rms: f32,
    /// Exponentially-smoothed onset confidence.
    onset_smooth: f32,
    /// Timestamp (monotonic seconds) of the last detected beat.
    last_beat_secs: f64,
    /// Monotonic clock reference (seconds accumulator from elapsed callback).
    clock_secs: f64,
    /// Circular buffer of recent beat intervals (seconds) — used for BPM estimate.
    beat_intervals: [f32; 8],
    beat_intervals_head: usize,
    beat_intervals_count: usize,
    /// Smoothed BPM output.
    bpm_smooth: f32,
    /// 12-bin running chroma accumulator (before normalization).
    chroma_accum: [f32; 12],
}

impl Default for VisualizerAccum {
    fn default() -> Self {
        Self {
            prev_rms: 0.0,
            onset_smooth: 0.0,
            last_beat_secs: 0.0,
            clock_secs: 0.0,
            beat_intervals: [0.0; 8],
            beat_intervals_head: 0,
            beat_intervals_count: 0,
            bpm_smooth: 120.0,
            chroma_accum: [0.0; 12],
        }
    }
}

/// Shared handles needed to (re)create the output stream at a given rate.
/// Held by the engine and cloned into the decode thread so playback can
/// follow each track's native sample rate (auto mode) without owning the
/// engine itself.
#[derive(Clone)]
struct OutputStreamCtx {
    rb: RingBuffer,
    dsp: Arc<Mutex<DspChain>>,
    volume: Arc<Mutex<VolumeControl>>,
    device_manager: Arc<Mutex<DeviceManager>>,
    config: Arc<Mutex<EngineConfig>>,
    output_rate: Arc<Mutex<SampleRate>>,
    hw_volume_cb: HwVolumeCallback,
    last_hw_volume: Arc<Mutex<f32>>,
    stop_signal: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    spectrum: Arc<Mutex<SpectrumAnalyzer>>,
    current_position: Arc<Mutex<f64>>,
    current_duration: Arc<Mutex<Option<f64>>>,
    speed: Arc<Mutex<f32>>,
    data_channels: Arc<Mutex<ChannelCount>>,
    bit_depth: Arc<Mutex<u16>>,
    src_bit_depth: Arc<Mutex<u16>>,
    dsd_mode: Arc<Mutex<DsdMode>>,
    dsd_modulators: Arc<Mutex<Vec<DeltaSigmaModulator>>>,
    visualizer_hub: VisualizerHub,
}

impl PlaybackEngine {
    /// Create a new playback engine.
    #[allow(clippy::arc_with_non_send_sync)]
    pub fn new(config: EngineConfig) -> Result<Self, DeviceError> {
        let device_manager = Arc::new(Mutex::new(DeviceManager::new()?));
        // Always use software volume for consistency; HW/SW mode toggle
        // is cosmetic but keeps independent level storage per mode.
        let volume = Arc::new(Mutex::new(VolumeControl::new(true)));

        let buffer_size = config.buffer_size;
        let config = Arc::new(Mutex::new(config));

        let state = Arc::new(Mutex::new(PlaybackState::Idle));
        let ring_buffer = RingBuffer::new(buffer_size);
        let dsp_chain = Arc::new(Mutex::new(DspChain::new()));
        let output_rate = Arc::new(Mutex::new(48000u32));
        let current_channels = Arc::new(Mutex::new(2u16));
        let stop_signal = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));
        let spectrum = Arc::new(Mutex::new(SpectrumAnalyzer::new(
            SPECTRUM_FFT_SIZE,
            SPECTRUM_BANDS,
        )));
        let current_position = Arc::new(Mutex::new(0.0f64));
        let current_duration = Arc::new(Mutex::new(None::<f64>));
        let speed = Arc::new(Mutex::new(1.0f32));
        let hw_volume_cb: HwVolumeCallback = Arc::new(Mutex::new(None));
        let last_hw_volume = Arc::new(Mutex::new(-1.0f32));
        let bit_depth = Arc::new(Mutex::new(0u16));
        let src_bit_depth = Arc::new(Mutex::new(0u16));
        let dsd_mode = Arc::new(Mutex::new(DsdMode::Off));
        let dsd_modulators = Arc::new(Mutex::new(vec![
            DeltaSigmaModulator::new(),
            DeltaSigmaModulator::new(),
        ]));
        let visualizer_hub: VisualizerHub = Arc::new(Mutex::new(None));

        // Visualizer publisher thread: the audio callback only snapshots the
        // latest analysis frame into `visualizer_hub` (try_lock, never
        // blocks); all feature math (chroma/onset/beat/BPM) and event
        // fan-out happen here, off the real-time thread.
        {
            let hub = visualizer_hub.clone();
            thread::spawn(move || {
                let mut accum = VisualizerAccum::default();
                loop {
                    thread::sleep(Duration::from_millis(20));
                    let frame = match hub.try_lock() {
                        Ok(mut g) => g.take(),
                        Err(_) => continue,
                    };
                    let Some(f) = frame else { continue };
                    let VisualizerFrame {
                        bands,
                        waveform,
                        rms,
                        peak,
                        sample_rate,
                        channels,
                        data_channels,
                        frames_read,
                    } = f;

                    let sr = sample_rate as f32;
                    let nyquist = sr * 0.5;

                    // Spectral centroid from 32 log-spaced bands (approximate Hz)
                    let mut centroid_num: f32 = 0.0;
                    let mut centroid_den: f32 = 0.0;
                    for (i, &b) in bands.iter().enumerate() {
                        let t = ((i as f32 + 0.5) / bands.len() as f32).max(0.0001);
                        let freq_hz = nyquist * t * t;
                        centroid_num += freq_hz * b;
                        centroid_den += b;
                    }
                    let spectral_centroid_hz = if centroid_den > 1e-6 {
                        centroid_num / centroid_den
                    } else {
                        0.0
                    };

                    // 12-bin approximate chroma: map each spectrum band to the
                    // closest pitch class assuming equal-temperament A440.
                    let a4_hz = 440.0f32;
                    let mut chroma = [0.0f32; 12];
                    for (i, &b) in bands.iter().enumerate() {
                        if b < 0.02 {
                            continue;
                        }
                        let t = ((i as f32 + 0.5) / bands.len() as f32).max(0.0005);
                        let freq = nyquist * t * t;
                        if freq < 55.0 {
                            continue;
                        }
                        let midi = 69.0 + 12.0 * (freq / a4_hz).log2();
                        let pc = (midi.round() as i32).rem_euclid(12) as usize;
                        chroma[pc] += b;
                    }
                    // Running-accumulate with decay so chroma doesn't flicker
                    let mut sum_c: f32 = 0.0;
                    for (va_chroma, &chroma_i) in accum.chroma_accum.iter_mut().zip(chroma.iter())
                    {
                        *va_chroma = *va_chroma * 0.75 + chroma_i * 0.25;
                        sum_c += *va_chroma;
                    }
                    if sum_c > 1e-6 {
                        for va_chroma in accum.chroma_accum.iter_mut() {
                            *va_chroma /= sum_c;
                        }
                    }
                    let chroma_out = accum.chroma_accum;

                    // Onset (energy flux) + beat detection
                    accum.clock_secs +=
                        frames_read as f64 / (data_channels as f64 * sample_rate as f64);
                    let raw_onset = (rms - accum.prev_rms).max(0.0) / (accum.prev_rms + 0.01);
                    accum.prev_rms = rms;
                    accum.onset_smooth = accum.onset_smooth * 0.82 + raw_onset.min(2.0) * 0.18;
                    let beat_spacing_min = 60.0 / 220.0; // ≥220 BPM
                    let time_since_beat = accum.clock_secs - accum.last_beat_secs;
                    let threshold = 0.22;
                    let mut beat_detected = false;
                    if accum.onset_smooth > threshold && time_since_beat > beat_spacing_min {
                        beat_detected = true;
                        let interval = time_since_beat as f32;
                        let head = accum.beat_intervals_head;
                        let cap = accum.beat_intervals.len();
                        accum.beat_intervals[head] = interval;
                        accum.beat_intervals_head = (head + 1) % cap;
                        if accum.beat_intervals_count < cap {
                            accum.beat_intervals_count += 1;
                        }
                        let n = accum.beat_intervals_count;
                        if n >= 2 {
                            let mut sum_int: f32 = 0.0;
                            for i in 0..n {
                                sum_int += accum.beat_intervals[i];
                            }
                            let avg_int = sum_int / (n as f32);
                            if avg_int > 0.0 {
                                let bpm_est = 60.0 / avg_int;
                                if (40.0..=220.0).contains(&bpm_est) {
                                    accum.bpm_smooth = accum.bpm_smooth * 0.7 + bpm_est * 0.3;
                                }
                            }
                        }
                        accum.last_beat_secs = accum.clock_secs;
                    }

                    if let Some(tx) = EVENT_TX.get() {
                        let _ = tx.send(AppEvent::SpectrumData {
                            values: bands.to_vec(),
                        });
                        let _ = tx.send(AppEvent::AudioFeaturesData {
                            spectrum: bands.to_vec(),
                            waveform: waveform.to_vec(),
                            rms,
                            peak,
                            spectral_centroid_hz,
                            onset: accum.onset_smooth,
                            beat: beat_detected,
                            bpm: accum.bpm_smooth,
                            chroma: chroma_out,
                            sample_rate,
                            channels,
                        });
                    }
                }
            });
        }

        let output_ctx = OutputStreamCtx {
            rb: ring_buffer.clone(),
            dsp: dsp_chain.clone(),
            volume: volume.clone(),
            device_manager: device_manager.clone(),
            config: config.clone(),
            output_rate: output_rate.clone(),
            hw_volume_cb: hw_volume_cb.clone(),
            last_hw_volume: last_hw_volume.clone(),
            stop_signal: stop_signal.clone(),
            paused: paused.clone(),
            spectrum: spectrum.clone(),
            current_position: current_position.clone(),
            current_duration: current_duration.clone(),
            speed: speed.clone(),
            data_channels: current_channels.clone(),
            bit_depth: bit_depth.clone(),
            src_bit_depth: src_bit_depth.clone(),
            dsd_mode: dsd_mode.clone(),
            dsd_modulators: dsd_modulators.clone(),
            visualizer_hub: visualizer_hub.clone(),
        };

        Ok(Self {
            state,
            ring_buffer,
            dsp_chain,
            volume,
            device_manager,
            config,
            stop_signal,
            skip_signal: Arc::new(AtomicBool::new(false)),
            skip_count: Arc::new(AtomicUsize::new(0)),
            seek_signal: Arc::new(AtomicBool::new(false)),
            seek_target: Arc::new(Mutex::new(0.0)),
            output_rate,
            current_channels,
            play_queue: Arc::new(Mutex::new(VecDeque::new())),
            current_track: Arc::new(Mutex::new(None)),
            output_running: Arc::new(AtomicBool::new(false)),
            paused,
            tracks_played: Arc::new(Mutex::new(0)),
            current_position,
            current_duration,
            current_queue_index: Arc::new(Mutex::new(0)),
            decode_handle: Arc::new(Mutex::new(None)),
            speed,
            speed_changed: Arc::new(AtomicBool::new(false)),
            hw_volume_cb,
            playback_mode: Arc::new(Mutex::new(PlaybackMode::Normal)),
            decode_generation: Arc::new(AtomicU64::new(0)),
            time_stretch_factory: Arc::new(Mutex::new(None)),
            bit_depth,
            dsd_mode,
            dsd_modulators,
            output_ctx,
        })
    }

    // ── Accessors ─────────────────────────────────────────────

    pub fn state(&self) -> PlaybackState {
        *self.state.lock().unwrap()
    }

    pub fn dsp_chain(&self) -> Arc<Mutex<DspChain>> {
        self.dsp_chain.clone()
    }

    pub fn volume(&self) -> Arc<Mutex<VolumeControl>> {
        self.volume.clone()
    }

    pub fn device_manager(&self) -> Arc<Mutex<DeviceManager>> {
        self.device_manager.clone()
    }

    /// Set a callback for hardware volume changes.
    /// When the volume mode is Hardware, this callback is called instead of
    /// applying software volume attenuation.
    pub fn set_hw_volume_callback<F: Fn(f32) + Send + Sync + 'static>(&self, cb: F) {
        *self.hw_volume_cb.lock().unwrap() = Some(Arc::new(cb));
    }

    /// Returns the name of the currently configured output device.
    pub fn current_device_name(&self) -> String {
        self.config.lock().unwrap().device_name.clone()
    }

    pub fn sample_rate(&self) -> SampleRate {
        *self.output_rate.lock().unwrap()
    }

    pub fn channels(&self) -> ChannelCount {
        *self.current_channels.lock().unwrap()
    }

    pub fn queue_len(&self) -> usize {
        self.play_queue.lock().unwrap().len()
    }

    pub fn current_track(&self) -> Option<String> {
        self.current_track.lock().unwrap().clone()
    }

    pub fn tracks_played(&self) -> usize {
        *self.tracks_played.lock().unwrap()
    }

    pub fn position(&self) -> f64 {
        *self.current_position.lock().unwrap()
    }

    pub fn duration(&self) -> Option<f64> {
        *self.current_duration.lock().unwrap()
    }

    pub fn queue_index(&self) -> usize {
        *self.current_queue_index.lock().unwrap()
    }

    pub fn set_queue_index(&self, index: usize) {
        *self.current_queue_index.lock().unwrap() = index;
    }

    /// Set playback speed. Clamps to 0.25–4.0. Dynamically recreates TimeStretcher without decoder restart.
    pub fn set_speed(&self, speed: f32) {
        let clamped = speed.clamp(0.25, 4.0);
        *self.speed.lock().unwrap() = clamped;
        self.speed_changed.store(true, Ordering::SeqCst);
        log::info!("Playback speed set to {:.2}x", clamped);
    }

    /// Get current playback speed.
    pub fn get_speed(&self) -> f32 {
        *self.speed.lock().unwrap()
    }

    /// Set a TimeStretch plugin factory. When set, the engine will use
    /// the plugin's time-stretcher instead of the built-in phase vocoder.
    pub fn set_time_stretch_factory(&self, factory: Option<TimeStretchFactory>) {
        *self.time_stretch_factory.lock().unwrap() = factory;
        // Trigger speed change flag so the decode thread will recreate the
        // stretcher with the new factory (or built-in) on the next frame.
        self.speed_changed.store(true, Ordering::SeqCst);
        log::info!(
            "TimeStretch plugin {}",
            if self.time_stretch_factory.lock().unwrap().is_some() {
                "enabled"
            } else {
                "disabled"
            }
        );
    }

    /// Get the current TimeStretch factory (if a plugin has registered one).
    /// Exposed `pub` so external crates (e.g. phonon-tauri `update_settings`)
    /// can detect whether to swap the built-in factory.
    pub fn get_time_stretch_factory(&self) -> Option<TimeStretchFactory> {
        self.time_stretch_factory.lock().unwrap().clone()
    }

    /// Set the playback mode.
    pub fn set_playback_mode(&self, mode: PlaybackMode) {
        *self.playback_mode.lock().unwrap() = mode;
    }

    /// Get the current playback mode.
    pub fn get_playback_mode(&self) -> PlaybackMode {
        *self.playback_mode.lock().unwrap()
    }

    /// Force output stream recreation on next start_playback() (e.g. for device switch).
    pub fn set_output_running(&self, running: bool) {
        self.output_running.store(running, Ordering::SeqCst);
    }

    /// Set exclusive mode (requires restarting playback to take effect).
    pub fn set_exclusive(&self, exclusive: bool) {
        self.config.lock().unwrap().exclusive = exclusive;
    }

    /// Set buffer size in samples. Takes effect on next playback start.
    pub fn set_buffer_size(&self, size: usize) {
        self.config.lock().unwrap().buffer_size = size;
    }

    /// Get current buffer size.
    pub fn buffer_size(&self) -> usize {
        self.config.lock().unwrap().buffer_size
    }

    /// Set resampling quality. Takes effect on next track decode.
    pub fn set_resampler_quality(&self, quality: ResamplerQuality) {
        self.config.lock().unwrap().resampler_quality = quality;
    }

    /// Get current resampling quality.
    pub fn resampler_quality(&self) -> ResamplerQuality {
        self.config.lock().unwrap().resampler_quality
    }

    /// Set bit depth: 0 = float32 passthrough, 16, 24.
    pub fn set_bit_depth(&self, depth: u16) {
        *self.bit_depth.lock().unwrap() = depth;
    }

    /// Get current bit depth.
    pub fn bit_depth(&self) -> u16 {
        *self.bit_depth.lock().unwrap()
    }

    /// Set DSD output mode. Takes effect on next output stream start.
    pub fn set_dsd_mode(&self, mode: DsdMode) {
        *self.dsd_mode.lock().unwrap() = mode;
        for m in self.dsd_modulators.lock().unwrap().iter_mut() {
            m.reset();
        }
    }

    /// Get current DSD output mode.
    pub fn dsd_mode(&self) -> DsdMode {
        *self.dsd_mode.lock().unwrap()
    }

    /// Set output sample rate. 0 = auto-detect from device.
    pub fn set_output_sample_rate(&self, rate: u32) {
        self.config.lock().unwrap().output_rate = rate;
    }

    /// Get current output sample rate setting.
    pub fn output_sample_rate_setting(&self) -> u32 {
        self.config.lock().unwrap().output_rate
    }

    /// Get the actual current output sample rate (from device).
    pub fn current_output_rate(&self) -> u32 {
        *self.output_rate.lock().unwrap()
    }

    // ── Queue management ────────────────────────────────────────

    /// Supported audio file extensions (kept in sync with phonon-source SUPPORTED_EXTENSIONS).
    const SUPPORTED_AUDIO_EXTENSIONS: &'static [&'static str] = &[
        "flac", "alac", "m4a", "wav", "wave", "mp3", "aac", "dsf", "dff",
    ];

    /// Lyrics / non-audio text extensions that should never be enqueued.
    const BLOCKED_EXTENSIONS: &'static [&'static str] = &[
        "lrc", "txt", "srt", "ass", "ssa", "vtt", "jpeg", "jpg", "png", "gif", "bmp", "pdf", "doc",
        "docx", "xls", "xlsx", "zip", "rar", "7z",
    ];

    /// Strip a trailing `#<digits>` CUE-track index suffix from `path`.
    ///
    /// CUE virtual tracks are referenced as `<audio_file>#<track_index>`
    /// (e.g. `"album.flac#2"`). The suffix is not part of the real file
    /// name, so extension checks and decoder opens must operate on the
    /// base path. Returns the base path slice if a suffix is present,
    /// otherwise returns the original slice unchanged.
    fn strip_cue_index(path: &str) -> &str {
        if let Some(hash_pos) = path.rfind('#') {
            let suffix = &path[hash_pos + 1..];
            if !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()) {
                // Make sure the `#` is after the last path separator so we
                // don't accidentally strip a `#` that's part of a directory
                // name.
                let base = &path[..hash_pos];
                let last_sep = base
                    .rfind(['/', '\\'])
                    .map(|p| p + 1)
                    .unwrap_or(0);
                if hash_pos >= last_sep {
                    return base;
                }
            }
        }
        path
    }

    /// Check whether a file looks like a valid audio track.
    fn is_valid_audio_path(path: &str) -> bool {
        let base = Self::strip_cue_index(path);
        let p = std::path::Path::new(base);
        let lower = match p.extension().and_then(|e| e.to_str()) {
            Some(ext) => ext.to_lowercase(),
            None => return false,
        };
        if Self::SUPPORTED_AUDIO_EXTENSIONS
            .iter()
            .any(|e| e == &lower.as_str())
        {
            return true;
        }
        // CUE references can pass through (they point to a real audio file).
        if Self::BLOCKED_EXTENSIONS
            .iter()
            .any(|e| e == &lower.as_str())
        {
            return false;
        }
        // Fallback: try decoding — any path with Unknown extension will fail here.
        phonon_codec::AudioFormat::from_extension(&lower) != phonon_codec::AudioFormat::Unknown
    }

    /// Probe audio file duration by briefly opening the decoder.
    /// Returns None if the file can't be opened or decoded.
    fn probe_duration(path: &str) -> Option<f64> {
        let base = Self::strip_cue_index(path);
        phonon_codec::AudioDecoder::open(base)
            .ok()
            .and_then(|d| d.duration())
    }

    pub fn enqueue(&self, path: &str) -> Result<bool, EngineError> {
        if !Self::is_valid_audio_path(path) {
            log::debug!("Skipping non-audio file: {}", path);
            return Ok(false);
        }
        // Deduplicate: skip if path already in queue
        {
            let queue = self.play_queue.lock().unwrap();
            if queue.iter().any(|t| t.path == path) {
                log::debug!("Skipping duplicate: {}", path);
                return Ok(false);
            }
        }
        let display = Self::display_name(path);
        let duration_limit = Self::probe_duration(path);
        self.play_queue.lock().unwrap().push_back(QueuedTrack {
            path: path.to_string(),
            start_offset: 0.0,
            duration_limit,
            display_name: display,
        });
        log::info!("Enqueued: {}", path);
        Ok(true)
    }

    /// Enqueue a track at the front of the queue (new tracks appear at top).
    pub fn enqueue_front(&self, path: &str) -> Result<bool, EngineError> {
        if !Self::is_valid_audio_path(path) {
            log::debug!("Skipping non-audio file (front): {}", path);
            return Ok(false);
        }
        {
            let queue = self.play_queue.lock().unwrap();
            if queue.iter().any(|t| t.path == path) {
                log::debug!("Skipping duplicate (front): {}", path);
                return Ok(false);
            }
        }
        let display = Self::display_name(path);
        let duration_limit = Self::probe_duration(path);
        self.play_queue.lock().unwrap().push_front(QueuedTrack {
            path: path.to_string(),
            start_offset: 0.0,
            duration_limit,
            display_name: display,
        });
        log::info!("Enqueued (front): {}", path);
        Ok(true)
    }

    /// Parse a CUE file and enqueue all its tracks at the front.
    /// Returns (added, duplicates) counts.
    pub fn enqueue_cue_front(&self, cue_path: &str) -> Result<(usize, usize), EngineError> {
        let cue_content = std::fs::read_to_string(cue_path)
            .map_err(|e| EngineError::Decode(format!("Cannot read CUE file: {}", e)))?;
        let sheet = phonon_codec::cue::parse_cue(&cue_content)
            .map_err(|e| EngineError::Decode(format!("CUE parse error: {}", e)))?;

        let audio_file = sheet
            .file
            .as_ref()
            .ok_or_else(|| EngineError::Decode("CUE file has no FILE entry".into()))?;

        let cue_dir = std::path::Path::new(cue_path)
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        let audio_path = cue_dir.join(audio_file);
        let audio_path_str = audio_path
            .to_str()
            .ok_or_else(|| EngineError::Decode("Invalid audio file path in CUE".into()))?;

        let mut added = 0usize;
        let mut duplicates = 0usize;

        // Add tracks in reverse order so first track is at the top
        for track in sheet.tracks.iter().rev() {
            let display = format!(
                "{} - {}",
                track.performer.as_deref().unwrap_or("Unknown"),
                track.title.as_deref().unwrap_or("Unknown")
            );

            // Check for duplicate
            {
                let queue = self.play_queue.lock().unwrap();
                if queue
                    .iter()
                    .any(|t| t.path == audio_path_str && t.display_name == display)
                {
                    duplicates += 1;
                    continue;
                }
            }

            self.play_queue.lock().unwrap().push_front(QueuedTrack {
                path: audio_path_str.to_string(),
                start_offset: track.start_time,
                duration_limit: track.duration,
                display_name: display,
            });
            added += 1;
        }

        log::info!("Enqueued {} tracks from CUE: {}", added, cue_path);
        Ok((added, duplicates))
    }

    pub fn enqueue_multiple(&self, paths: &[&str]) -> Result<usize, EngineError> {
        let mut count = 0;
        for path in paths {
            if self.enqueue(path)? {
                count += 1;
            }
        }
        Ok(count)
    }

    pub fn clear_queue(&self) {
        self.play_queue.lock().unwrap().clear();
    }

    pub fn remove_from_queue(&self, indices: &[usize]) {
        let mut queue = self.play_queue.lock().unwrap();
        let mut sorted: Vec<usize> = indices.to_vec();
        sorted.sort_unstable();
        for &idx in sorted.iter().rev() {
            if idx < queue.len() {
                queue.remove(idx);
            }
        }
        // Adjust current queue index if tracks before it were removed
        let mut cur = self.current_queue_index.lock().unwrap();
        let removed_before = sorted.iter().filter(|&&i| i < *cur).count();
        *cur = cur.saturating_sub(removed_before);
    }

    pub fn reorder_queue(&self, from: usize, to: usize) {
        let mut queue = self.play_queue.lock().unwrap();
        if from < queue.len() && to < queue.len() {
            if let Some(item) = queue.remove(from) {
                if to <= queue.len() {
                    queue.insert(to, item);
                } else {
                    queue.push_back(item);
                }
                // Adjust current_queue_index if the playing track was moved
                let mut cur = self.current_queue_index.lock().unwrap();
                let old = *cur;
                if from == old {
                    *cur = to;
                } else if from < old && to >= old {
                    *cur = old - 1;
                } else if from > old && to <= old {
                    *cur = old + 1;
                }
            }
        }
    }

    pub fn queue_snapshot(&self) -> Vec<QueueItem> {
        self.play_queue
            .lock()
            .unwrap()
            .iter()
            .map(|t| QueueItem {
                path: t.path.clone(),
                title: t.display_name.clone(),
                artist: None,
                duration: t.duration_limit,
            })
            .collect()
    }

    pub fn next(&self) {
        if let Ok(mut idx) = self.current_queue_index.lock() {
            *idx += 1;
        }

        // If engine is stopped, start playback directly
        let is_playing = self
            .state
            .lock()
            .map(|s| *s == PlaybackState::Playing)
            .unwrap_or(false);
        if !is_playing {
            let _ = self.start_playback();
            return;
        }

        self.skip_signal.store(true, Ordering::SeqCst);
        self.ring_buffer.clear();
    }

    pub fn previous(&self) {
        if let Ok(mut idx) = self.current_queue_index.lock() {
            if *idx > 0 {
                *idx -= 1;
            }
        }

        // If engine is stopped/paused, start playback directly — mirrors
        // next() behavior so prev/next are symmetric.
        let is_playing = self
            .state
            .lock()
            .map(|s| *s == PlaybackState::Playing)
            .unwrap_or(false);
        if !is_playing {
            let _ = self.start_playback();
            return;
        }

        self.skip_signal.store(true, Ordering::SeqCst);
        self.ring_buffer.clear();
    }

    pub fn seek(&self, position_secs: f64) {
        let target = position_secs.max(0.0);
        *self.seek_target.lock().unwrap() = target;
        *self.current_position.lock().unwrap() = target;
        self.seek_signal.store(true, Ordering::SeqCst);
        self.skip_signal.store(true, Ordering::SeqCst); // stop current decode immediately
        self.ring_buffer.clear();
    }

    // ── Playback control ────────────────────────────────────────

    pub fn play_file(&self, path: &str) -> Result<(), EngineError> {
        self.stop();

        if path.to_lowercase().ends_with(".cue") {
            return self.play_cue(path, 0);
        }

        self.clear_queue();
        self.enqueue(path)?;
        self.start_playback()
    }

    pub fn play_cue(&self, cue_path: &str, track_index: usize) -> Result<(), EngineError> {
        let cue_content = std::fs::read_to_string(cue_path)
            .map_err(|e| EngineError::Decode(format!("Cannot read CUE file: {}", e)))?;
        let sheet = phonon_codec::cue::parse_cue(&cue_content)
            .map_err(|e| EngineError::Decode(format!("CUE parse error: {}", e)))?;

        let audio_file = sheet
            .file
            .as_ref()
            .ok_or_else(|| EngineError::Decode("CUE file has no FILE entry".into()))?;

        let cue_dir = std::path::Path::new(cue_path)
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        let audio_path = cue_dir.join(audio_file);

        let track = sheet.tracks.get(track_index).ok_or_else(|| {
            EngineError::Decode(format!(
                "Track {} not found ({} tracks total)",
                track_index + 1,
                sheet.tracks.len()
            ))
        })?;

        let display = format!(
            "{} - {}",
            track.performer.as_deref().unwrap_or("Unknown"),
            track.title.as_deref().unwrap_or("Unknown")
        );

        log::info!(
            "Playing CUE track {}/{}: {} (offset: {:.1}s)",
            track_index + 1,
            sheet.tracks.len(),
            display,
            track.start_time,
        );

        let audio_path_str = audio_path
            .to_str()
            .ok_or_else(|| EngineError::Decode("Invalid audio file path in CUE".into()))?;

        self.stop();
        self.clear_queue();

        self.play_queue.lock().unwrap().push_back(QueuedTrack {
            path: audio_path_str.to_string(),
            start_offset: track.start_time,
            duration_limit: track.duration,
            display_name: display,
        });

        self.start_playback()
    }

    /// Start playback from the queue.
    /// Ensures any previous decode thread has finished before starting a new one.
    pub fn start_playback(&self) -> Result<(), EngineError> {
        // Clamp the index to the valid range BEFORE checking emptiness,
        // so next()/previous()-style overshoots never produce an InvalidState
        // error. An empty queue is treated as a quiet no-op (Ok) rather than
        // a hard error — the frontend still has nothing to play, but we
        // avoid spamming "Uncaught (in promise) Invalid state" into the
        // JS console on every accidental empty-queue play click.
        let q_len = self.play_queue.lock().unwrap().len();
        if q_len == 0 {
            return Ok(());
        }
        let mut idx_guard = self.current_queue_index.lock().unwrap();
        if *idx_guard >= q_len {
            *idx_guard = q_len - 1;
        }
        drop(idx_guard);

        // Stop any existing decode thread — stop the output stream first
        // so the decode thread doesn't block in write_to_rb waiting for
        // ring buffer space that the output callback is consuming.
        self.stop_signal.store(true, Ordering::SeqCst);
        self.output_running.store(false, Ordering::SeqCst);
        self.device_manager.lock().unwrap().stop_output();
        // Bump generation so the old decode thread exits, then join with timeout.
        // Joining prevents thread/resource accumulation during rapid track switches.
        self.decode_generation.fetch_add(1, Ordering::SeqCst);
        if let Some(handle) = self.decode_handle.lock().unwrap().take() {
            log::info!("Joining old decode thread (generation bumped)");
            // Join with timeout using a channel-based rendezvous.
            // std::thread::JoinHandle::join_timeout is not yet stable on MSRV.
            let (tx, rx) = mpsc::channel();
            thread::spawn(move || {
                let _ = handle.join();
                let _ = tx.send(());
            });
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(_) => log::info!("Old decode thread joined successfully"),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    log::warn!("Old decode thread join timed out, detaching")
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    log::warn!("Old decode thread join channel disconnected")
                }
            }
        }
        self.stop_signal.store(false, Ordering::SeqCst);
        self.skip_signal.store(false, Ordering::SeqCst);
        self.skip_count.store(0, Ordering::SeqCst);
        self.seek_signal.store(false, Ordering::SeqCst);
        *self.seek_target.lock().unwrap() = 0.0;
        self.ring_buffer.clear();

        // Determine the initial output rate. output_rate = 0 (auto) means
        // "follow the source": seed the stream with the first track's native
        // rate when possible; the decode loop keeps switching per track.
        let out_rate = {
            let dm = self.device_manager.lock().unwrap();
            let cfg = self.config.lock().unwrap();
            if cfg.output_rate > 0 {
                cfg.output_rate
            } else {
                let peek = {
                    let idx = *self.current_queue_index.lock().unwrap();
                    self.play_queue
                        .lock()
                        .unwrap()
                        .get(idx)
                        .and_then(|t| {
                            phonon_codec::AudioDecoder::open(Self::strip_cue_index(&t.path))
                                .ok()
                                .map(|d| d.sample_rate())
                        })
                };
                match peek {
                    // start_output falls back to the best supported rate if
                    // the device can't do the native rate.
                    Some(r) => r,
                    None => dm.best_output_rate(48000),
                }
            }
        };
        *self.output_rate.lock().unwrap() = out_rate;

        let channels = 2u16; // Stereo output
        *self.current_channels.lock().unwrap() = channels;

        // Always start a fresh output stream to ensure it's alive
        log::info!("Starting output stream: {}Hz {}ch...", out_rate, channels);
        self.start_output(out_rate, channels)?;
        self.output_running.store(true, Ordering::SeqCst);
        log::info!("Output stream started successfully");

        *self.state.lock().unwrap() = PlaybackState::Playing;
        self.paused.store(false, Ordering::SeqCst);
        if let Some(tx) = EVENT_TX.get() {
            let _ = tx.send(AppEvent::PlaybackStateChanged {
                state: "Playing".to_string(),
            });
        }

        // Clone Arcs for the decode thread
        let rb = self.ring_buffer.clone();
        let out_ctx = self.output_ctx.clone();
        let queue = self.play_queue.clone();
        let queue_idx = self.current_queue_index.clone();
        let stop = self.stop_signal.clone();
        let skip = self.skip_signal.clone();
        let skip_count = self.skip_count.clone();
        let seek_sig = self.seek_signal.clone();
        let seek_tgt = self.seek_target.clone();
        let state = self.state.clone();
        let current_track = self.current_track.clone();
        let tracks_played = self.tracks_played.clone();
        let current_pos = self.current_position.clone();
        let current_dur = self.current_duration.clone();
        let output_rate = self.output_rate.clone();
        let output_ch = self.current_channels.clone();
        let speed = self.speed.clone();
        let speed_changed = self.speed_changed.clone();
        let output_running = self.output_running.clone();
        let playback_mode = self.playback_mode.clone();
        let generation = self.decode_generation.clone();
        let quality = self.config.lock().unwrap().resampler_quality;
        let time_stretch_factory = self.get_time_stretch_factory();

        let handle = {
            let output_running_guard = output_running.clone();
            let state_guard = state.clone();
            thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    Self::decode_loop(
                        rb,
                        out_ctx,
                        queue,
                        queue_idx,
                        stop,
                        skip,
                        skip_count,
                        seek_sig,
                        seek_tgt,
                        state,
                        current_track,
                        tracks_played,
                        output_rate,
                        output_ch,
                        current_pos,
                        current_dur,
                        speed,
                        speed_changed,
                        output_running,
                        playback_mode,
                        generation,
                        quality,
                        time_stretch_factory,
                    );
                }));
                if let Err(e) = result {
                    let msg = if let Some(s) = e.downcast_ref::<String>() {
                        s.clone()
                    } else if let Some(s) = e.downcast_ref::<&str>() {
                        s.to_string()
                    } else {
                        "unknown panic".to_string()
                    };
                    log::error!("Decode thread panicked: {}", msg);
                    output_running_guard.store(false, Ordering::SeqCst);
                    if let Ok(mut s) = state_guard.lock() {
                        *s = PlaybackState::Stopped;
                    }
                }
            })
        };

        *self.decode_handle.lock().unwrap() = Some(handle);

        Ok(())
    }

    /// Play from a specific index in the queue without removing tracks.
    /// Sets the current queue index and starts playback from that position.
    pub fn play_from_index(&self, index: usize) -> Result<(), EngineError> {
        let q_len = {
            let queue = self.play_queue.lock().unwrap();
            queue.len()
        };
        // Treat out-of-range index the same way an empty queue is treated:
        // a quiet no-op rather than a user-visible error. Decoding nothing
        // and staying in Idle is always safer than throwing an exception
        // that propagates all the way to the JS console as an unhandled
        // promise rejection.
        if q_len == 0 || index >= q_len {
            return Ok(());
        }
        self.stop();
        // Set index AFTER stop() since stop() resets current_queue_index to 0
        *self.current_queue_index.lock().unwrap() = index;
        self.start_playback()
    }

    /// The persistent decode loop: iterates through the queue by index,
    /// decodes AND resamples each track to the fixed output rate,
    /// then feeds the ring buffer. The queue is never consumed.
    #[allow(clippy::too_many_arguments)]
    fn decode_loop(
        rb: RingBuffer,
        ctx: OutputStreamCtx,
        queue: Arc<Mutex<VecDeque<QueuedTrack>>>,
        queue_idx: Arc<Mutex<usize>>,
        stop: Arc<AtomicBool>,
        skip: Arc<AtomicBool>,
        skip_count: Arc<AtomicUsize>,
        seek_sig: Arc<AtomicBool>,
        seek_tgt: Arc<Mutex<f64>>,
        state: Arc<Mutex<PlaybackState>>,
        current_track: Arc<Mutex<Option<String>>>,
        tracks_played: Arc<Mutex<usize>>,
        output_rate: Arc<Mutex<SampleRate>>,
        output_ch: Arc<Mutex<ChannelCount>>,
        current_pos: Arc<Mutex<f64>>,
        current_dur: Arc<Mutex<Option<f64>>>,
        speed: Arc<Mutex<f32>>,
        speed_changed: Arc<AtomicBool>,
        output_running: Arc<AtomicBool>,
        playback_mode: Arc<Mutex<PlaybackMode>>,
        generation: Arc<AtomicU64>,
        quality: ResamplerQuality,
        time_stretch_factory: Option<TimeStretchFactory>,
    ) {
        let my_gen = generation.load(Ordering::SeqCst);
        let mut current: Option<QueuedTrack> = None;

        loop {
            // Exit if we're an old decode thread
            if generation.load(Ordering::SeqCst) != my_gen {
                log::debug!("Decode thread exiting: generation mismatch");
                break;
            }
            if stop.load(Ordering::SeqCst) {
                break;
            }

            // Handle pending skip at the top of the loop to prevent stalls.
            if skip.load(Ordering::SeqCst) {
                // queue_idx was already incremented by next()/previous()
                skip_count.store(0, Ordering::SeqCst);
                skip.store(false, Ordering::SeqCst);
                rb.clear();
                // Fall through to decode the next track
            }

            // Handle seek: re-open current track at new position
            if seek_sig.load(Ordering::SeqCst) {
                if let Some(ref track) = current {
                    seek_sig.store(false, Ordering::SeqCst);
                    skip.store(false, Ordering::SeqCst); // reset skip so we can decode from seek position
                    let target = *seek_tgt.lock().unwrap();
                    log::info!("Seeking to {:.1}s in {}", target, track.display_name);
                    rb.clear();
                    let seek_offset = track.start_offset + target;
                    let decoder_path = Self::strip_cue_index(&track.path);
                    let mut decoder = match phonon_codec::AudioDecoder::open(decoder_path) {
                        Ok(d) => d,
                        Err(e) => {
                            log::error!("Failed to reopen {} for seek: {}", track.path, e);
                            continue;
                        }
                    };
                    let src_rate = decoder.sample_rate();
                    let channels = decoder.channels();
                    let out_rate = *output_rate.lock().unwrap();
                    // >2ch sources are folded to stereo before the ring buffer
                    // (the device is opened stereo), so the effective data
                    // channel count never exceeds 2.
                    *output_ch.lock().unwrap() = channels.min(2);
                    // Use decoder-reported duration, falling back to track limit
                    let dur = track.duration_limit.or_else(|| decoder.duration());
                    *current_dur.lock().unwrap() = dur;

                    let resampler = Self::make_resampler(src_rate, out_rate, channels.min(2), quality);

                    // Try native seek first; fall back to frame-skipping
                    let mut total_time = 0.0;
                    let mut skipping = seek_offset > 0.0;
                    if seek_offset > 0.0 && decoder.seek(seek_offset).is_ok() {
                        // Native seek succeeded — start decoding from seek position
                        total_time = target;
                        skipping = false;
                    }
                    // else: fall through to frame-skipping below
                    let success = Self::decode_one_track_resampled(
                        &rb,
                        decoder,
                        &stop,
                        &skip,
                        &generation,
                        my_gen,
                        src_rate,
                        out_rate,
                        channels,
                        &resampler,
                        seek_offset,
                        track.duration_limit,
                        &mut total_time,
                        &mut skipping,
                        speed.clone(),
                        speed_changed.clone(),
                        &time_stretch_factory,
                    );

                    *tracks_played.lock().unwrap() += 1;

                    if !success && stop.load(Ordering::SeqCst) {
                        break;
                    }

                    if !success && skip.load(Ordering::SeqCst) {
                        skip_count.store(0, Ordering::SeqCst);
                        skip.store(false, Ordering::SeqCst);
                        continue;
                    }

                    // Only advance on normal completion (not RepeatOne which seeks back)
                    if success {
                        let mode = *playback_mode.lock().unwrap();
                        if mode != PlaybackMode::RepeatOne {
                            *queue_idx.lock().unwrap() += 1;
                        }
                    }
                    // For RepeatOne: fall through to loop top — current still holds
                    // the track, and the normal path will reopen the decoder.
                    continue;
                }
            }

            // Get track at current index (does NOT consume from queue)
            let idx = *queue_idx.lock().unwrap();
            let track = {
                let q = queue.lock().unwrap();
                q.get(idx).cloned()
            };

            let track = match track {
                Some(t) => t,
                None => {
                    // Past end of queue — restart if looping, else stop
                    // Re-read mode every iteration so mode changes take effect immediately
                    let mode = *playback_mode.lock().unwrap();
                    if mode == PlaybackMode::RepeatAll {
                        log::info!("Queue exhausted, repeating ({:?})", mode);
                        rb.clear();
                        *queue_idx.lock().unwrap() = 0;
                        continue;
                    }
                    if rb.is_empty() {
                        break;
                    }
                    if skip.load(Ordering::SeqCst) {
                        continue; // go to top where skip is handled
                    }
                    thread::sleep(Duration::from_millis(50));
                    continue;
                }
            };

            current = Some(track.clone());
            *current_track.lock().unwrap() = Some(track.path.clone());
            skip.store(false, Ordering::SeqCst);

            // Check for seek requested before playback started (e.g. device switch)
            let seek_target = if seek_sig.load(Ordering::SeqCst) {
                seek_sig.store(false, Ordering::SeqCst);
                let target = *seek_tgt.lock().unwrap();
                log::info!("Seeking to {:.1}s in {}", target, track.display_name);
                rb.clear();
                Some(target)
            } else {
                None
            };
            let start_offset = match seek_target {
                Some(t) => track.start_offset + t,
                None => track.start_offset,
            };
            *current_pos.lock().unwrap() = seek_target.unwrap_or(0.0);

            log::info!("Now playing [{}]: {}", idx, track.display_name);

            let decoder_path = Self::strip_cue_index(&track.path);
            let mut decoder = match phonon_codec::AudioDecoder::open(decoder_path) {
                Ok(d) => d,
                Err(e) => {
                    log::error!("Failed to open {}: {}", track.path, e);
                    *tracks_played.lock().unwrap() += 1;
                    *queue_idx.lock().unwrap() += 1;
                    current = None;
                    continue;
                }
            };

            // Use decoder-reported duration, falling back to track limit
            let dur = track.duration_limit.or_else(|| decoder.duration());
            *current_dur.lock().unwrap() = dur;

            let src_rate = decoder.sample_rate();
            let channels = decoder.channels();

            // Source bit depth (0 = unknown / float source) — gates dithering
            // in the output callback to real depth reductions only.
            *ctx.src_bit_depth.lock().unwrap() =
                decoder.metadata().bit_depth.unwrap_or(0);

            // Auto sample-rate mode ("follow source"): when the track's native
            // rate differs from the live stream and the device supports it,
            // re-create the stream at the track boundary. This is the
            // bit-perfect path — no resampler is inserted when rates match.
            {
                let auto_rate = ctx.config.lock().unwrap().output_rate == 0;
                // DoP output pins the stream to the carrier rate — never
                // follow the source rate while DSD is active.
                let dsd_on = *ctx.dsd_mode.lock().unwrap() != DsdMode::Off;
                let cur_rate = *output_rate.lock().unwrap();
                if auto_rate && !dsd_on && src_rate != cur_rate {
                    let supported =
                        ctx.device_manager.lock().unwrap().supports_rate(src_rate);
                    if supported {
                        log::info!(
                            "Auto output rate: {}Hz → {}Hz (following source)",
                            cur_rate,
                            src_rate
                        );
                        rb.clear();
                        match Self::start_output_stream(&ctx, src_rate, 2) {
                            Ok(actual) => *output_rate.lock().unwrap() = actual,
                            Err(e) => log::warn!(
                                "Auto rate switch to {}Hz failed, keeping {}Hz: {}",
                                src_rate,
                                cur_rate,
                                e
                            ),
                        }
                    }
                }
            }

            let out_rate = *output_rate.lock().unwrap();
            *output_ch.lock().unwrap() = channels.min(2);

            let resampler = Self::make_resampler(src_rate, out_rate, channels.min(2), quality);

            // Try native seek for device switch; fall back to frame-skipping
            let mut total_time = 0.0;
            let mut skipping = start_offset > 0.0;
            if start_offset > 0.0 && decoder.seek(start_offset).is_ok() {
                total_time = seek_target.unwrap_or(0.0);
                skipping = false;
            }

            let success = Self::decode_one_track_resampled(
                &rb,
                decoder,
                &stop,
                &skip,
                &generation,
                my_gen,
                src_rate,
                out_rate,
                channels,
                &resampler,
                start_offset,
                track.duration_limit,
                &mut total_time,
                &mut skipping,
                speed.clone(),
                speed_changed.clone(),
                &time_stretch_factory,
            );

            *tracks_played.lock().unwrap() += 1;

            if !success && stop.load(Ordering::SeqCst) {
                break;
            }

            // Only advance on normal completion — next()/previous()
            // already updated the index when they set skip_signal.
            if success {
                let mode = *playback_mode.lock().unwrap();
                match mode {
                    PlaybackMode::RepeatOne => {
                        // RepeatOne: don't advance queue, replay the same track from start
                        current = None;
                        rb.clear();
                        log::info!("RepeatOne: replaying track from start");
                        continue;
                    }
                    PlaybackMode::Normal | PlaybackMode::RepeatAll => {
                        *queue_idx.lock().unwrap() += 1;
                    }
                }
            } else if skip.load(Ordering::SeqCst) {
                // Skipped via next/previous — queue_idx was already incremented
                skip_count.store(0, Ordering::SeqCst);
                skip.store(false, Ordering::SeqCst);
            } else if !skip.load(Ordering::SeqCst) {
                // Decode error (not stop/skip) — skip this broken track
                log::error!("Decode failed, skipping to next track");
                *queue_idx.lock().unwrap() += 1;
            }
        }

        // Drain remaining buffer before marking as stopped, with generation guard
        let drain_start = std::time::Instant::now();
        while !rb.is_empty() && !stop.load(Ordering::SeqCst) {
            // Exit if we're an old thread (generation changed by start_playback)
            if generation.load(Ordering::SeqCst) != my_gen {
                break;
            }
            // Safety timeout: 5 seconds max
            if drain_start.elapsed() > std::time::Duration::from_secs(5) {
                log::warn!("Decode loop drain timed out after 5s, forcing exit");
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }

        // Don't touch state if we're an old thread
        if generation.load(Ordering::SeqCst) != my_gen {
            return;
        }
        output_running.store(false, Ordering::SeqCst);
        *state.lock().unwrap() = PlaybackState::Stopped;
        *current_track.lock().unwrap() = None;
        *current_pos.lock().unwrap() = 0.0;
        log::info!("Playback finished");
    }

    /// Create a resampler for the given source → output rate conversion.
    fn make_resampler(
        src_rate: SampleRate,
        out_rate: SampleRate,
        channels: ChannelCount,
        quality: ResamplerQuality,
    ) -> Option<Arc<Mutex<FftFixedIn<f32>>>> {
        if src_rate == out_rate {
            return None;
        }
        let (chunk_size, sub_chunks) = match quality {
            ResamplerQuality::Fast => (256, 1),
            ResamplerQuality::Balanced => (1024, 1),
            ResamplerQuality::High => (2048, 2),
        };
        match FftFixedIn::new(
            src_rate as usize,
            out_rate as usize,
            chunk_size,
            sub_chunks,
            channels as usize,
        ) {
            Ok(r) => {
                log::info!(
                    "Resampling: {}Hz -> {}Hz ({}ch, {:?})",
                    src_rate,
                    out_rate,
                    channels,
                    quality
                );
                Some(Arc::new(Mutex::new(r)))
            }
            Err(e) => {
                log::error!("Failed to create resampler: {}", e);
                None
            }
        }
    }

    /// Decode a single track → [downmix >2ch] → TimeStretcher → Resample → Write.
    /// Pipeline: Decoder → (stereo fold) → TimeStretch (speed change, pitch
    /// preserved) → Rubato (src→out rate) → RingBuffer
    /// When a TimeStretch plugin is loaded, it replaces the built-in phase vocoder.
    /// Supports dynamic speed changes by recreating the TimeStretcher in-place without decoder restart.
    #[allow(clippy::too_many_arguments)]
    fn decode_one_track_resampled(
        rb: &RingBuffer,
        mut decoder: phonon_codec::AudioDecoder,
        stop: &Arc<AtomicBool>,
        skip: &Arc<AtomicBool>,
        generation: &Arc<AtomicU64>,
        my_gen: u64,
        src_rate: SampleRate,
        _out_rate: SampleRate,
        channels: ChannelCount,
        resampler: &Option<Arc<Mutex<FftFixedIn<f32>>>>,
        start_offset: f64,
        _duration_limit: Option<f64>,
        total_time: &mut f64,
        skipping: &mut bool,
        speed: Arc<Mutex<f32>>,
        speed_changed: Arc<AtomicBool>,
        time_stretch_factory: &Option<TimeStretchFactory>,
    ) -> bool {
        // Everything downstream of the decoder is mono/stereo: the output
        // device is opened stereo and >2ch sources are folded down on
        // ingestion (see `downmix_to_stereo`).
        let eff_ch = channels.min(2);
        // After time stretch, process exactly chunk_frames for rubato.
        // Use the resampler's required input size to match the quality setting
        // (Fast=256, Balanced=1024, High=2048). Fallback to 1024 if no resampler.
        let chunk_frames = if let Some(ref r) = resampler {
            r.lock().unwrap().input_frames_max()
        } else {
            1024
        };
        let chunk_samples = chunk_frames * eff_ch as usize;

        let mut frame_buffer: Vec<f32> = Vec::new();
        let mut stretched_buf: Vec<f32> = Vec::new();

        // Time-stretcher (pitch-preserving speed change)
        // If a plugin factory is provided, use it; else fall back to built-in phase vocoder
        let cur_speed = *speed.lock().unwrap();
        let use_stretch = (cur_speed - 1.0).abs() > 0.001;
        let mut stretcher: Option<Box<dyn TimeStretch>> = if use_stretch {
            if let Some(factory) = time_stretch_factory {
                Some(factory(cur_speed, eff_ch, src_rate))
            } else {
                Some(Box::new(TimeStretcher::new(cur_speed, eff_ch as usize)))
            }
        } else {
            None
        };

        // Gain protection: -2dB (×0.8) to prevent clipping after time stretch
        const GAIN_SAFE: f32 = 0.95;

        loop {
            if generation.load(Ordering::SeqCst) != my_gen {
                return false;
            }
            if stop.load(Ordering::SeqCst) {
                return false;
            }
            if skip.load(Ordering::SeqCst) {
                return false;
            }

            // Dynamic speed change: use set_speed() to preserve phase state
            // when switching between non-1.0x speeds. Recreate only when
            // crossing 1.0x boundary (phase reset unavoidable).
            if speed_changed.load(Ordering::SeqCst) {
                speed_changed.store(false, Ordering::SeqCst);
                let new_speed = *speed.lock().unwrap();
                log::info!(
                    "Dynamic speed change to {:.2}x (no decoder restart)",
                    new_speed
                );
                let new_use_stretch = (new_speed - 1.0).abs() > 0.001;

                let (needs_drain, drain_amount) = match (&mut stretcher, new_use_stretch) {
                    (Some(ref mut s), true) => {
                        // Speed change between non-1.0x values: preserve phase state
                        s.set_speed(new_speed);
                        // Drain ~50ms of buffered audio so new speed takes effect quickly
                        (true, 4800)
                    }
                    (Some(ref mut s), false) => {
                        // Switching to 1.0x: flush and discard
                        s.flush(&mut stretched_buf);
                        stretcher = None;
                        // Drain ~50ms
                        (true, 4800)
                    }
                    (None, true) => {
                        // Switching from 1.0x: create new stretcher
                        if let Some(factory) = time_stretch_factory {
                            stretcher = Some(factory(new_speed, eff_ch, src_rate));
                        } else {
                            stretcher =
                                Some(Box::new(TimeStretcher::new(new_speed, eff_ch as usize)));
                        }
                        (true, 4800)
                    }
                    (None, false) => {
                        // 1.0x → 1.0x: no change
                        (false, 0)
                    }
                };
                if needs_drain {
                    rb.drain_to(drain_amount);
                }
            }

            let frame_opt = match decoder.next_frame() {
                Ok(Some(f)) => Some(f),
                Ok(None) => None,
                Err(e) => {
                    log::error!("Decode error: {}", e);
                    return false;
                }
            };

            if let Some(ref frame) = frame_opt {
                let frame_duration =
                    frame.samples.len() as f64 / (src_rate as f64 * channels as f64);

                if *skipping {
                    *total_time += frame_duration;
                    if *total_time >= start_offset {
                        *skipping = false;
                        *total_time = start_offset;
                    }
                    continue;
                }

                if channels > 2 {
                    frame_buffer.extend(Self::downmix_to_stereo(&frame.samples, channels));
                } else {
                    frame_buffer.extend_from_slice(&frame.samples);
                }
            } else {
                // EOF: feed any remaining buffered frames into the stretcher
                // (zero-padded so the tail is stretched too), then flush.
                // Previously only the stretcher's internal state was flushed
                // here, silently dropping up to 4096 frames (~93 ms) of tail.
                if let Some(ref mut s) = stretcher {
                    let frame_samples = 4096 * eff_ch as usize;
                    while frame_buffer.len() >= frame_samples {
                        let frame: Vec<f32> = frame_buffer.drain(..frame_samples).collect();
                        s.push(&frame, &mut stretched_buf);
                    }
                    if !frame_buffer.is_empty() {
                        let need = frame_samples - frame_buffer.len();
                        frame_buffer.extend(std::iter::repeat(0.0).take(need));
                        let frame: Vec<f32> = frame_buffer.drain(..frame_samples).collect();
                        s.push(&frame, &mut stretched_buf);
                    }
                    s.flush(&mut stretched_buf);
                }
                if frame_buffer.is_empty() && stretched_buf.is_empty() {
                    return true;
                }
            }

            // Feed to time stretcher if needed
            if let Some(ref mut s) = stretcher {
                while frame_buffer.len() >= 4096 * eff_ch as usize {
                    let frame: Vec<f32> = frame_buffer.drain(..4096 * eff_ch as usize).collect();
                    s.push(&frame, &mut stretched_buf);
                }
            } else {
                // 1.0x, direct to stretched_buf
                stretched_buf.extend_from_slice(&frame_buffer);
                frame_buffer.clear();
            }

            // Process full chunks from stretched_buf through resampler
            let cur_speed = *speed.lock().unwrap();
            while stretched_buf.len() >= chunk_samples {
                let chunk: Vec<f32> = stretched_buf.drain(0..chunk_samples).collect();

                let resampled = Self::resample_chunk(&chunk, eff_ch, chunk_frames, resampler);
                // Bit-transparency: at 1.0x with no active stretcher the samples
                // reach the device untouched (no gain, no clamp). The safety gain
                // + hard limit only apply to the time-stretched path, where the
                // waveform is being rebuilt and may overshoot.
                let out: Vec<f32> = if stretcher.is_some() {
                    if cur_speed > 1.0 {
                        resampled
                            .iter()
                            .map(|s| (s * GAIN_SAFE).clamp(-1.0, 1.0))
                            .collect()
                    } else {
                        resampled.iter().map(|s| s.clamp(-1.0, 1.0)).collect()
                    }
                } else {
                    resampled
                };
                Self::write_to_rb(rb, &out, stop, skip, generation, my_gen, &speed_changed);

                if stop.load(Ordering::SeqCst) || skip.load(Ordering::SeqCst) {
                    return false;
                }
                // If speed changed mid-write, discard remaining old-speed samples
                // and loop back to handle the speed change at the top
                if speed_changed.load(Ordering::SeqCst) {
                    stretched_buf.clear();
                    frame_buffer.clear();
                    continue;
                }
            }

            if frame_opt.is_none() {
                // Process remaining stretched samples
                if !stretched_buf.is_empty() {
                    let padded_len = stretched_buf.len().div_ceil(chunk_samples) * chunk_samples;
                    stretched_buf.resize(padded_len, 0.0);
                    while stretched_buf.len() >= chunk_samples {
                        let chunk: Vec<f32> = stretched_buf.drain(0..chunk_samples).collect();
                        let resampled =
                            Self::resample_chunk(&chunk, channels, chunk_frames, resampler);
                        // Tail flush: same rule as the main path — only the
                        // time-stretched output gets the safety gain/limit.
                        let out: Vec<f32> = if stretcher.is_some() {
                            resampled
                                .iter()
                                .map(|s| (s * GAIN_SAFE).clamp(-1.0, 1.0))
                                .collect()
                        } else {
                            resampled
                        };
                        Self::write_to_rb(
                            rb,
                            &out,
                            stop,
                            skip,
                            generation,
                            my_gen,
                            &speed_changed,
                        );
                        if stop.load(Ordering::SeqCst) || skip.load(Ordering::SeqCst) {
                            return false;
                        }
                    }
                }
                return true;
            }
        }
    }

    /// Fold an interleaved frame with >2 channels down to stereo.
    ///
    /// Channel order is assumed WAVE-standard (FL FR FC LFE BL BR SL SR …);
    /// the first two channels pass through at unity gain and every remaining
    /// channel (center, LFE, surrounds) is mixed into both at −3 dB. This is
    /// a best-effort fold for playback on a stereo device — before this
    /// existed, >2ch files played as channel-garbled noise.
    fn downmix_to_stereo(samples: &[f32], channels: ChannelCount) -> Vec<f32> {
        let ch = channels as usize;
        let side_gain = std::f32::consts::FRAC_1_SQRT_2;
        let frames = samples.len() / ch;
        let mut out = Vec::with_capacity(frames * 2);
        for f in 0..frames {
            let s = &samples[f * ch..f * ch + ch];
            let mut l = s[0];
            let mut r = s[1];
            for &v in &s[2..] {
                let att = v * side_gain;
                l += att;
                r += att;
            }
            out.push(l.clamp(-1.0, 1.0));
            out.push(r.clamp(-1.0, 1.0));
        }
        out
    }

    /// Resample a chunk of interleaved audio samples.
    fn resample_chunk(
        chunk: &[f32],
        channels: ChannelCount,
        chunk_frames: usize,
        resampler: &Option<Arc<Mutex<FftFixedIn<f32>>>>,
    ) -> Vec<f32> {
        if let Some(ref r) = resampler {
            let mut r = r.lock().unwrap();
            let wave_in: Vec<Vec<f32>> = (0..channels as usize)
                .map(|ch| {
                    (0..chunk_frames.min(chunk.len() / channels as usize))
                        .map(|i| chunk[i * channels as usize + ch])
                        .collect()
                })
                .collect();
            match r.process(&wave_in, None) {
                Ok(wave_out) => {
                    let out_frames = wave_out[0].len();
                    let mut interleaved = Vec::with_capacity(out_frames * channels as usize);
                    for i in 0..out_frames {
                        for ch in &wave_out {
                            if i < ch.len() {
                                interleaved.push(ch[i]);
                            }
                        }
                    }
                    interleaved
                }
                Err(e) => {
                    log::error!("Resampler error: {}", e);
                    Vec::new()
                }
            }
        } else {
            chunk.to_vec()
        }
    }

    /// Write samples to ring buffer with backpressure.
    /// Checks speed_changed to break out early when speed is modified,
    /// so the decode thread can handle the speed change promptly.
    fn write_to_rb(
        rb: &RingBuffer,
        samples: &[f32],
        stop: &Arc<AtomicBool>,
        skip: &Arc<AtomicBool>,
        generation: &Arc<AtomicU64>,
        my_gen: u64,
        speed_changed: &Arc<AtomicBool>,
    ) {
        let mut offset = 0;
        while offset < samples.len() {
            if generation.load(Ordering::SeqCst) != my_gen
                || stop.load(Ordering::SeqCst)
                || skip.load(Ordering::SeqCst)
                || speed_changed.load(Ordering::SeqCst)
            {
                return;
            }
            let free = rb.free_space();
            if free < 4096 {
                std::thread::sleep(std::time::Duration::from_millis(1));
                continue;
            }
            let to_write = (samples.len() - offset).min(free);
            let written = rb.write(&samples[offset..offset + to_write]);
            if written == 0 {
                std::thread::sleep(std::time::Duration::from_millis(1));
            } else {
                offset += written;
            }
        }
    }

    /// Restart the output stream without touching the decode thread.
    /// Used for device switching — stops the old output stream and starts a new
    /// one on the current device, while the decode thread continues running.
    /// The caller should set seek_signal/seek_target before calling this.
    pub fn restart_output_stream(&self) -> Result<(), EngineError> {
        log::info!("Restarting output stream (device switch)...");
        self.device_manager.lock().unwrap().stop_output();
        let out_rate = *self.output_rate.lock().unwrap();
        let channels = *self.current_channels.lock().unwrap();
        self.start_output(out_rate, channels)?;
        self.output_running.store(true, Ordering::SeqCst);
        log::info!("Output stream restarted successfully");
        Ok(())
    }

    /// Start the audio output stream at a fixed rate.
    /// The output callback simply reads from the ring buffer (data is already
    /// at the output rate) and applies DSP + volume.
    fn start_output(
        &self,
        output_rate: SampleRate,
        channels: ChannelCount,
    ) -> Result<(), EngineError> {
        Self::start_output_stream(&self.output_ctx, output_rate, channels).map(|_| ())
    }

    /// Create (or re-create) the audio output stream at the requested rate.
    /// Returns the actual rate used (after device-capability fallback).
    /// Not a `&self` method so the decode thread can call it for per-track
    /// auto sample-rate switching.
    fn start_output_stream(
        ctx: &OutputStreamCtx,
        output_rate: SampleRate,
        channels: ChannelCount,
    ) -> Result<SampleRate, EngineError> {
        let rb = ctx.rb.clone();
        let dsp = ctx.dsp.clone();
        let vol = ctx.volume.clone();
        let hw_cb = ctx.hw_volume_cb.clone();
        let last_hw = ctx.last_hw_volume.clone();
        let stop = ctx.stop_signal.clone();
        let paused = ctx.paused.clone();
        let spectrum = ctx.spectrum.clone();
        let current_pos = ctx.current_position.clone();
        let current_dur = ctx.current_duration.clone();
        let speed = ctx.speed.clone();
        let data_ch = ctx.data_channels.clone();
        let bit_depth = ctx.bit_depth.clone();
        let src_bit_depth = ctx.src_bit_depth.clone();
        let dsd_modulators = ctx.dsd_modulators.clone();
        let visualizer_hub = ctx.visualizer_hub.clone();

        let mut device_manager = ctx.device_manager.lock().unwrap();
        let config = ctx.config.lock().unwrap();

        let device_name = device_manager
            .current_device_name()
            .or_else(|| device_manager.default_device().ok().map(|d| d.name))
            .ok_or(EngineError::InvalidState(
                "No output device available".into(),
            ))?;

        // ── DSD/DoP output requirements ────────────────────────────
        // DoP carries DSD bitstream inside ordinary 24-bit PCM frames at a
        // fixed carrier rate, so the stream must run exclusively at
        // `carrier_rate` over a 24-bit PCM transport. DoP cannot survive the
        // shared-mode mixer, and DSP/volume/dither must never touch the
        // encoded words — the callback bypasses all of them when `use_pcm24`.
        let mut target_rate = output_rate;
        let mut use_pcm24 = false;
        if *ctx.dsd_mode.lock().unwrap() != DsdMode::Off {
            if !config.exclusive {
                log::warn!(
                    "DSD/DoP output requires WASAPI exclusive mode — disabling DSD for this stream"
                );
                *ctx.dsd_mode.lock().unwrap() = DsdMode::Off;
            } else {
                target_rate = ctx
                    .dsd_mode
                    .lock()
                    .unwrap()
                    .carrier_rate()
                    .unwrap_or(output_rate);
                use_pcm24 = true;
            }
        }

        device_manager.open_device(&device_name, target_rate, channels, config.exclusive)?;

        // Verify the requested rate is actually supported by the device.
        // If not (e.g. device only supports 48000 but we requested 44100),
        // fall back to the best supported rate and update output_rate so
        // the decode thread's resampler targets the correct rate. A DSD
        // stream whose carrier rate is unsupported is unusable — disable
        // DSD and fall back to the PCM path instead.
        let actual_rate = if device_manager.supports_rate(target_rate) {
            target_rate
        } else if use_pcm24 {
            log::warn!(
                "DoP carrier rate {}Hz not supported by device — disabling DSD for this stream",
                target_rate
            );
            *ctx.dsd_mode.lock().unwrap() = DsdMode::Off;
            use_pcm24 = false;
            let best = device_manager.best_output_rate(output_rate);
            log::warn!(
                "Requested {}Hz not supported by device, falling back to {}Hz",
                output_rate,
                best
            );
            best
        } else {
            let best = device_manager.best_output_rate(target_rate);
            log::warn!(
                "Requested {}Hz not supported by device, falling back to {}Hz",
                target_rate,
                best
            );
            best
        };

        if actual_rate != output_rate {
            *ctx.output_rate.lock().unwrap() = actual_rate;
        }

        log::info!(
            "Output stream: {}Hz {}ch (exclusive: {}, requested: {}Hz, pcm24: {})",
            actual_rate,
            channels,
            config.exclusive,
            output_rate,
            use_pcm24
        );

        // Shadow the parameter with the actual rate so the closure below uses
        // the real device sample rate for DSP processing and position tracking.
        // Without this, elapsed/pos would be computed against the wrong rate and
        // drift out of sync with real playback time.
        let output_rate = actual_rate;
        let progress_acc = Cell::new(0.0f64);

        device_manager.start_playback(actual_rate, channels, use_pcm24, move |output: &mut [f32]| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if stop.load(Ordering::SeqCst) {
                    output.fill(0.0);
                    return;
                }

                if paused.load(Ordering::Relaxed) {
                    output.fill(0.0);
                    return;
                }

                let data_channels = data_ch.lock().map(|g| *g).unwrap_or(2);
                let n_raw = rb.read(output);

                // Convert mono → stereo if source channel count differs from device
                if data_channels == 1 && channels == 2 {
                    let mono_count = n_raw.min(output.len() / 2);
                    for i in (0..mono_count).rev() {
                        let sample = output[i];
                        output[i * 2] = sample;
                        output[i * 2 + 1] = sample;
                    }
                    let n_stereo = mono_count * 2;
                    if n_stereo < output.len() {
                        output[n_stereo..].fill(0.0);
                    }
                } else if n_raw < output.len() {
                    output[n_raw..].fill(0.0);
                }

                // DSP processing — skipped entirely for DoP output: any
                // processing would corrupt the DSD-over-PCM words.
                if !use_pcm24 {
                    if let Ok(chain) = dsp.lock() {
                        chain.process(output, channels, output_rate);
                    }
                }

                // Volume processing — also bypassed for DoP (scaling DoP
                // words corrupts the marker/data bits).
                if !use_pcm24 {
                    if let Ok(vc) = vol.lock() {
                        if vc.mode() == crate::volume::VolumeMode::Hardware {
                            if let Ok(mut last) = last_hw.lock() {
                                let current = vc.level();
                                let mut use_sw_fallback = !vc.has_hw_controller();
                                if (*last - current).abs() > 0.001 {
                                    *last = current;
                                    let hw_ok = vc.apply_hw_volume();
                                    if let Ok(hw_cb_guard) = hw_cb.lock() {
                                        if let Some(ref cb) = *hw_cb_guard {
                                            cb(current);
                                        }
                                    }
                                    if !hw_ok {
                                        log::warn!(
                                            "HW volume apply failed, falling back to software volume"
                                        );
                                        use_sw_fallback = true;
                                    }
                                }
                                if use_sw_fallback {
                                    vc.apply_software_volume(output);
                                }
                            } else {
                                vc.apply_software_volume(output);
                            }
                        } else {
                            vc.apply_software_volume(output);
                        }
                    }
                }

                // Apply dithering + quantization for non-float bit depths —
                // but only when actually reducing depth. A same-depth
                // passthrough (16-bit file → 16-bit output) must stay
                // bit-transparent. Skipped for DoP (words must stay raw).
                if !use_pcm24 {
                    if let Ok(bd) = bit_depth.lock() {
                        if *bd == 16 || *bd == 24 {
                            let src_bd = src_bit_depth.lock().map(|g| *g).unwrap_or(0);
                            if src_bd == 0 || src_bd > *bd {
                                apply_dither(output, *bd);
                            }
                        }
                    }
                }

                // Snapshot one analysis window for the visualizer publisher
                // thread. Fixed-size arrays and try_lock keep this section
                // allocation-free and non-blocking on the real-time thread;
                // all feature math + event sends happen on the publisher.
                if let Ok(mut sa) = spectrum.try_lock() {
                    if let Some(bands) = sa.feed(output, channels) {
                        // Output is always device-channel interleaved here
                        // (mono data was already upmixed above): mono data
                        // contributed n_raw frames, n-ch data n_raw/ch frames.
                        let n_frames = if data_channels == 1 {
                            n_raw
                        } else {
                            n_raw / (channels as usize).max(1)
                        };
                        let ch_usize = (channels as usize).max(1);
                        let wav_target: usize = 512;
                        let mut waveform = [0.0f32; 512];
                        let mut sum_sq: f64 = 0.0;
                        let mut peak_val: f32 = 0.0;
                        let stride = (n_frames / wav_target).max(1);
                        let mut frame_idx = 0usize;
                        while frame_idx < n_frames {
                            let idx = frame_idx * ch_usize;
                            let v = if ch_usize >= 2 && idx + 1 < output.len() {
                                (output[idx] + output[idx + 1]) * 0.5
                            } else if idx < output.len() {
                                output[idx]
                            } else {
                                break;
                            };
                            let slot = (frame_idx / stride).min(wav_target - 1);
                            waveform[slot] = v;
                            sum_sq += (v as f64) * (v as f64);
                            let av = v.abs();
                            if av > peak_val {
                                peak_val = av;
                            }
                            frame_idx += stride;
                        }
                        let rms = ((sum_sq / (n_frames as f64).max(1.0)).sqrt() as f32)
                            .clamp(0.0, 1.0);
                        let frame = VisualizerFrame {
                            bands,
                            waveform,
                            rms,
                            peak: peak_val.min(1.0),
                            sample_rate: output_rate,
                            channels,
                            data_channels: data_channels as u16,
                            frames_read: n_raw,
                        };
                        if let Ok(mut slot) = visualizer_hub.try_lock() {
                            *slot = Some(frame); // latest wins — never block
                        }
                    }
                }

                // DoP encoding (DSD-over-PCM): sigma-delta modulate each
                // channel to a 1-bit stream and pack 16 DSD bits into each
                // 24-bit output word. Runs last so the visualizer above sees
                // plain PCM, and only on the 24-bit PCM transport (the DAC
                // must receive the raw words, not floats).
                if use_pcm24 {
                    if let Ok(mut mods) = dsd_modulators.lock() {
                        // Ensure we have enough modulators for all channels
                        while mods.len() < channels as usize {
                            mods.push(DeltaSigmaModulator::new());
                        }
                        let ch_count = channels as usize;
                        let frames = output.len() / ch_count;
                        // DoP standard: the marker byte alternates 0x05/0xFA
                        // on every sample per channel — DACs use it to
                        // distinguish DoP from ordinary PCM.
                        let mut marker_hi = [false; 8];
                        for ch in 0..ch_count.min(8) {
                            let modulator = &mut mods[ch];
                            for frame in 0..frames {
                                let idx = frame * ch_count + ch;
                                let mut bits = [0u8; 16];
                                modulator.process(output[idx] as f64, &mut bits, 16);
                                let mut dsd_word: u16 = 0;
                                for (i, &b) in bits.iter().enumerate() {
                                    if b != 0 {
                                        dsd_word |= 1 << (15 - i);
                                    }
                                }
                                let marker: u32 = if marker_hi[ch] { 0xFA } else { 0x05 };
                                marker_hi[ch] = !marker_hi[ch];
                                let dop_val = (marker << 16) | (dsd_word as u32);
                                // Sign-extend the 24-bit word and store it as
                                // an exact-integer f32 (|v| < 2^24 is exact);
                                // the PCM24 transport shifts it left-aligned
                                // into the 32-bit container.
                                let v24 = if dop_val & 0x800000 != 0 {
                                    dop_val | 0xFF000000
                                } else {
                                    dop_val
                                };
                                output[idx] = v24 as i32 as f32;
                            }
                        }
                    }
                }

                // Update position every callback using actual data channel count
                let elapsed = n_raw as f64 / (data_channels as f64 * output_rate as f64);
                let speed_factor = speed.lock().map(|g| *g).unwrap_or(1.0) as f64;
                if let Ok(mut pos) = current_pos.lock() {
                    *pos += elapsed * speed_factor;
                }

                // Push playback progress event periodically (~10 Hz)
                let acc = progress_acc.get() + elapsed;
                if acc >= 0.1 {
                    if let Ok(pos) = current_pos.lock() {
                        let dur = current_dur.lock().map(|d| *d).unwrap_or(None);
                        if let Some(tx) = EVENT_TX.get() {
                            let _ = tx.send(AppEvent::PlaybackProgress {
                                position_secs: *pos,
                                duration_secs: dur,
                            });
                        }
                    }
                    progress_acc.set(0.0);
                } else {
                    progress_acc.set(acc);
                }
            }));
            if result.is_err() {
                // Output callback panicked — fill with silence to keep the stream alive
                output.fill(0.0);
            }
        })?;

        // `output_rate` is shadowed by the actual device rate above.
        Ok(output_rate)
    }

    // ── Playback control commands ───────────────────────────────

    pub fn pause(&self) {
        *self.state.lock().unwrap() = PlaybackState::Paused;
        self.paused.store(true, Ordering::SeqCst);
        if let Some(tx) = EVENT_TX.get() {
            let _ = tx.send(AppEvent::PlaybackStateChanged {
                state: "Paused".to_string(),
            });
        }
    }

    pub fn resume(&self) {
        *self.state.lock().unwrap() = PlaybackState::Playing;
        self.paused.store(false, Ordering::SeqCst);
        if let Some(tx) = EVENT_TX.get() {
            let _ = tx.send(AppEvent::PlaybackStateChanged {
                state: "Playing".to_string(),
            });
        }
    }

    /// Stop playback: signal decode thread to stop, clear buffer, reset state.
    /// Does NOT clear the queue — user can resume with play().
    /// Does NOT join the decode thread — let it exit on its own via stop_signal.
    /// start_playback() will join the old thread before starting a new one.
    pub fn stop(&self) {
        self.stop_signal.store(true, Ordering::SeqCst);
        self.seek_signal.store(false, Ordering::SeqCst);
        if let Ok(mut t) = self.seek_target.lock() {
            *t = 0.0;
        }
        self.ring_buffer.clear();
        self.output_running.store(false, Ordering::SeqCst);
        if let Ok(mut dm) = self.device_manager.lock() {
            dm.stop_output();
        }
        // Bump generation so the old decode thread exits, then join with timeout.
        self.decode_generation.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut handle) = self.decode_handle.lock() {
            if let Some(h) = handle.take() {
                let (tx, rx) = mpsc::channel();
                thread::spawn(move || {
                    let _ = h.join();
                    let _ = tx.send(());
                });
                match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(_) => log::info!("Decode thread joined on stop"),
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        log::warn!("Decode thread join timed out on stop, detaching")
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        log::warn!("Decode thread join channel disconnected on stop")
                    }
                }
            }
        }
        self.paused.store(false, Ordering::SeqCst);
        if let Ok(mut s) = self.state.lock() {
            *s = PlaybackState::Stopped;
        }
        if let Some(tx) = EVENT_TX.get() {
            let _ = tx.send(AppEvent::PlaybackStateChanged {
                state: "Stopped".to_string(),
            });
        }
        if let Ok(mut t) = self.current_track.lock() {
            *t = None;
        }
        if let Ok(mut p) = self.current_position.lock() {
            *p = 0.0;
        }
    }

    pub fn buffer_fill(&self) -> f64 {
        self.ring_buffer.available() as f64 / self.ring_buffer.capacity() as f64
    }

    fn display_name(path: &str) -> String {
        std::path::Path::new(path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string())
    }
}

/// Apply triangular PDF dithering + quantization for non-float bit depths.
/// Triangular dither minimizes distortion and noise modulation.
fn apply_dither(samples: &mut [f32], bits: u16) {
    // Full scale = 2^(bits-1): the negative rail maps exactly to -1.0
    // (16-bit: -32768..=32767 → -1.0..=0.999969), so decoded 16-bit sources
    // round-trip bit-exactly instead of losing the lowest LSB.
    let half = 1i32 << (bits - 1);
    let scale = half as f32;
    let min_val = -(half as f32);
    let max_val = (half - 1) as f32;

    // Use a simple xorshift-like PRNG for speed
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64;

    for sample in samples.iter_mut() {
        // Triangular PDF: sum of two uniform randoms → peak at 0
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let r1 = (seed >> 32) as f32 / (u32::MAX as f32);
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let r2 = (seed >> 32) as f32 / (u32::MAX as f32);
        let dither = (r1 + r2 - 1.0) / scale; // 1 LSB p-p triangular dither

        let scaled = *sample * scale + dither;
        let quantized = scaled.round().clamp(min_val, max_val);
        *sample = quantized / scale;
    }
}

/// Test-only wrapper for the private `apply_dither` function.
/// Not part of the public API; only exposed for integration tests.
#[doc(hidden)]
pub fn test_apply_dither(samples: &mut [f32], bits: u16) {
    apply_dither(samples, bits);
}

/// Errors that can occur in the playback engine.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("Device error: {0}")]
    Device(#[from] DeviceError),
    #[error("Decode error: {0}")]
    Decode(String),
    #[error("Invalid state: {0}")]
    InvalidState(String),
}
