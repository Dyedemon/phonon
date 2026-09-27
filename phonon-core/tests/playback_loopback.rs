//! §15-4 E2E: 回放回环测试
//!
//! 模拟 cpal 输出回调的完整数据路径：
//!   生成 sine → 写入 RingBuffer → 模拟回调多次调用 → 收集输出
//!   → 验证输入输出相关性 > 0.99（除延迟偏移外）
//!
//! 这个测试不依赖真实音频设备（无需 cpal DeviceManager），
//! 而是复用 PlaybackEngine 输出回调的核心逻辑：
//!   RingBuffer.read(output) → DSP.process → Volume.apply
//!
//! 对应 tasks.md §15 第三条："dummy 后端 + 回环 WAV →
//! 输入输出相关性 > 0.99（除延迟偏移）"

use phonon_core::{DspChain, RingBuffer, VolumeControl};
use std::f64::consts::PI;

// ── sine 生成 ──

fn generate_sine(frequency: f64, sample_rate: u32, duration_secs: f64, channels: u16) -> Vec<f32> {
    let frames = (sample_rate as f64 * duration_secs) as usize;
    let mut samples = Vec::with_capacity(frames * channels as usize);
    for i in 0..frames {
        let t = i as f64 / sample_rate as f64;
        let v = (2.0 * PI * frequency * t).sin() as f32;
        for _ in 0..channels {
            samples.push(v);
        }
    }
    samples
}

// ── 模拟 cpal 输出回调的核心逻辑（与 engine.rs::start_output 一致）──
//
// 从 RingBuffer 读 → 填充不足补零 → DSP 处理 → 音量处理
fn simulate_output_callback(
    rb: &RingBuffer,
    dsp: &DspChain,
    vol: &VolumeControl,
    output: &mut [f32],
    channels: u16,
) {
    let n_raw = rb.read(output);
    if n_raw < output.len() {
        output[n_raw..].fill(0.0);
    }
    dsp.process(output, channels, 48000);
    vol.apply_software_volume(output);
}

// ── 延迟对齐 + 相关性计算 ──

/// 找到使 cross-correlation 最大的 lag（样本数）。
/// 在 [-max_lag, +max_lag] 范围内搜索。
fn find_best_lag(input: &[f32], output: &[f32], max_lag: usize) -> isize {
    let mut best_lag = 0isize;
    let mut best_corr = -1.0f64;
    for lag in -(max_lag as isize)..=(max_lag as isize) {
        let corr = cross_correlation(input, output, lag);
        if corr > best_corr {
            best_corr = corr;
            best_lag = lag;
        }
    }
    best_lag
}

/// 计算 input 与 output 在指定 lag 下的归一化互相关。
fn cross_correlation(input: &[f32], output: &[f32], lag: isize) -> f64 {
    let (i_start, o_start, len) = if lag >= 0 {
        let lag_u = lag as usize;
        if lag_u >= input.len() {
            (0, 0, 0)
        } else {
            let len = (input.len() - lag_u).min(output.len());
            (0usize, lag_u, len)
        }
    } else {
        let abs_lag = (-lag) as usize;
        if abs_lag >= output.len() {
            (0, 0, 0)
        } else {
            let len = input.len().min(output.len() - abs_lag);
            (abs_lag, 0usize, len)
        }
    };
    if len == 0 {
        return 0.0;
    }
    let mut sum = 0.0f64;
    let mut i_sq = 0.0f64;
    let mut o_sq = 0.0f64;
    for k in 0..len {
        let i_val = input[i_start + k] as f64;
        let o_val = output[o_start + k] as f64;
        sum += i_val * o_val;
        i_sq += i_val * i_val;
        o_sq += o_val * o_val;
    }
    let denom = (i_sq * o_sq).sqrt();
    if denom < 1e-12 {
        0.0
    } else {
        sum / denom
    }
}

// ── 测试用例 ──

#[test]
fn test_playback_loopback_correlation() {
    let sample_rate = 48000u32;
    let channels = 2u16;
    let frequency = 440.0_f64;
    let duration = 1.0; // 1 秒

    // 1) 生成 sine 波
    let input = generate_sine(frequency, sample_rate, duration, channels);

    // 2) 创建 RingBuffer（容量 = 2 倍信号长度，避免溢出）
    let rb = Ring::new(input.len() * 2);
    let written = rb.write(&input);
    assert_eq!(
        written,
        input.len(),
        "all input should be written to ring buffer"
    );

    // 3) 创建空 DSP 链 + 音量 = 1.0（直通）
    let dsp = DspChain::new();
    let mut vol = VolumeControl::new(false); // 测试中不依赖硬件 volume
    vol.set_volume(1.0);

    // 4) 模拟 cpal 回调多次调用，每次 1024 帧
    let mut collected = Vec::with_capacity(input.len());
    let chunk_size = 1024 * channels as usize;
    let mut buf = vec![0.0f32; chunk_size];
    loop {
        simulate_output_callback(&rb, &dsp, &vol, &mut buf, channels);
        collected.extend_from_slice(&buf);
        if collected.len() >= input.len() {
            break;
        }
    }
    collected.truncate(input.len());

    // 5) 找到最佳对齐 lag（phase vocoder/DSP 可能有延迟）
    let max_lag = 2048;
    let best_lag = find_best_lag(&input, &collected, max_lag);
    let corr = cross_correlation(&input, &collected, best_lag);

    assert!(
        corr > 0.99,
        "loopback correlation {} (lag {}) below 0.99 threshold",
        corr,
        best_lag
    );
}

#[test]
fn test_playback_loopback_volume_attenuation() {
    // 验证音量衰减后信号仍保持形状（相关性高，幅值降低）
    let sample_rate = 48000u32;
    let channels = 2u16;
    let input = generate_sine(1000.0, sample_rate, 0.5, channels);

    let rb = Ring::new(input.len() * 2);
    rb.write(&input);

    let dsp = DspChain::new();
    let mut vol = VolumeControl::new(false);
    vol.set_volume(0.5); // 50% 音量

    let chunk_size = 512 * channels as usize;
    let mut collected = Vec::with_capacity(input.len());
    let mut buf = vec![0.0f32; chunk_size];
    loop {
        simulate_output_callback(&rb, &dsp, &vol, &mut buf, channels);
        collected.extend_from_slice(&buf);
        if collected.len() >= input.len() {
            break;
        }
    }
    collected.truncate(input.len());

    // 相关性应仍 > 0.99（形状不变）
    let best_lag = find_best_lag(&input, &collected, 2048);
    let corr = cross_correlation(&input, &collected, best_lag);
    assert!(
        corr > 0.99,
        "correlation {} should be > 0.99 at 50% volume",
        corr
    );

    // 幅值应约为输入的 50%
    let in_peak = input.iter().cloned().fold(0.0f32, f32::max).abs();
    let out_peak = collected.iter().cloned().fold(0.0f32, f32::max).abs();
    let ratio = out_peak / in_peak;
    assert!(
        ratio > 0.45 && ratio < 0.55,
        "output peak ratio {} should be ~0.5 at 50% volume",
        ratio
    );
}

// ── 简化别名：测试内部用 `Ring` 替代 `RingBuffer` 以保持独立 ──
// 由于 RingBuffer 已实现 Clone（共享内部 Arc），直接使用即可。
use phonon_core::RingBuffer as Ring;
