//! §15-1 E2E: 解码层 golden file 测试
//!
//! 验证 1kHz sine 波的完整回环：
//!   生成 sine → 编码为 WAV (i16 PCM) → AudioDecoder 解码 → 收集 PCM
//!   → RMS 断言（信号能量合理） + FFT 频率 bin 断言（峰值落在 1kHz 附近）
//!
//! 这是 §15 E2E 测试策略中的第一项，确保解码链路对已知信号
//! 保持频率保真度。对应 tasks.md §15 第一条。

use phonon_codec::AudioDecoder;
use std::io::Cursor;

// ── WAV 编码辅助（与 codec_integration.rs 一致，独立定义避免跨文件依赖）──

/// 生成 sine 波样本（交错多声道）。
fn generate_sine(frequency: f64, sample_rate: u32, duration: f64, channels: u16) -> Vec<f32> {
    let num_samples = (sample_rate as f64 * duration) as usize * channels as usize;
    let mut samples = Vec::with_capacity(num_samples);
    for i in 0..num_samples / channels as usize {
        let t = i as f64 / sample_rate as f64;
        let value = (2.0 * std::f64::consts::PI * frequency * t).sin() as f32;
        for _ in 0..channels {
            samples.push(value);
        }
    }
    samples
}

/// 将 f32 样本编码为内存中的 WAV 字节流（16-bit PCM, 44 字节头）。
fn encode_wav_bytes(samples: &[f32], sample_rate: u32, channels: u16) -> Vec<u8> {
    let data_size = samples.len() * 2;
    let file_size = 44 + data_size;
    let mut wav = Vec::with_capacity(file_size);

    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(file_size as u32 - 8).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    let byte_rate = sample_rate * channels as u32 * 2;
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    let block_align = channels * 2;
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data_size as u32).to_le_bytes());

    for sample in samples {
        let clamped = (sample * 32767.0).clamp(-32768.0, 32767.0) as i16;
        wav.extend_from_slice(&clamped.to_le_bytes());
    }
    wav
}

// ── RMS 与 FFT 频率 bin 检测 ──

/// 计算信号 RMS（均方根）。
fn rms(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum_sq / samples.len() as f64).sqrt()
}

/// 简易 DFT：返回幅度谱（前半部分，对应 0..Nyquist）。
/// 仅用于测试断言频率 bin，不追求性能。
fn dft_magnitude(samples: &[f32], _sample_rate: u32) -> Vec<f64> {
    let n = samples.len();
    let mut mags = vec![0.0; n / 2];
    for k in 0..n / 2 {
        let mut re = 0.0_f64;
        let mut im = 0.0_f64;
        for t in 0..n {
            let angle = -2.0 * std::f64::consts::PI * (k as f64) * (t as f64) / (n as f64);
            re += samples[t] as f64 * angle.cos();
            im += samples[t] as f64 * angle.sin();
        }
        mags[k] = (re * re + im * im).sqrt() / (n as f64);
    }
    mags
}

/// 找到幅度谱中峰值对应的频率（Hz）。
fn peak_frequency(samples: &[f32], sample_rate: u32) -> f64 {
    let mags = dft_magnitude(samples, sample_rate);
    let (peak_idx, _) = mags
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
        .expect("DFT output should not be empty");
    // 峰值 bin k 对应频率 = k * sample_rate / N
    let n = samples.len();
    (peak_idx as f64) * (sample_rate as f64) / (n as f64)
}

// ── 测试用例 ──

/// 1kHz sine 在 44.1kHz 立体声下的完整回环 + RMS + 频率 bin 断言。
#[test]
fn test_golden_sine_1khz_44100_stereo_roundtrip() {
    let sample_rate = 44100u32;
    let channels = 2u16;
    let frequency = 1000.0_f64;
    let duration = 0.5; // 0.5 秒，足够 DFT 频率分辨率

    // 1) 生成 sine 波
    let original = generate_sine(frequency, sample_rate, duration, channels);

    // 2) 编码为 WAV
    let wav_bytes = encode_wav_bytes(&original, sample_rate, channels);

    // 3) 解码回来（in-memory Cursor，不落盘）
    let cursor = Cursor::new(wav_bytes);
    let mut decoder = AudioDecoder::open_reader(Box::new(cursor), "golden.wav")
        .expect("decoder should open in-memory WAV");

    let mut decoded = Vec::new();
    while let Some(frame) = decoder.next_frame().expect("decode should succeed") {
        decoded.extend_from_slice(&frame.samples);
    }

    // 4) 样本数应大致相等（允许 symphonia 帧对齐导致的尾部差异 ≤ 1 帧）
    let frame_samples = (sample_rate as f64 * duration) as usize * channels as usize;
    let diff = decoded.len() as isize - frame_samples as isize;
    assert!(
        diff.abs() < (channels as isize * 4096),
        "decoded sample count {} differs from expected {} by {}",
        decoded.len(),
        frame_samples,
        diff
    );

    // 5) RMS 断言：1kHz 满幅 sine 的 RMS 应接近 1/√2 ≈ 0.7071
    //    但经 i16 量化 + symphonia 32-bit float 还原后会有轻微损失，容忍 ±5%。
    let rms_val = rms(&decoded);
    let expected_rms = std::f64::consts::FRAC_1_SQRT_2; // 0.7071
    let rms_ratio = rms_val / expected_rms;
    assert!(
        rms_ratio > 0.93 && rms_ratio < 1.07,
        "RMS {} deviates from expected {}: ratio {}",
        rms_val,
        expected_rms,
        rms_ratio
    );

    // 6) FFT 频率 bin 断言：峰值频率应落在 1kHz 附近（容差 ±50Hz，DFT 分辨率限制）
    //    只取左声道前 8192 样本以加速 DFT（频率分辨率 = 44100/8192 ≈ 5.4Hz）
    let dft_window = 8192;
    let left_channel: Vec<f32> = decoded
        .chunks_exact(channels as usize)
        .take(dft_window)
        .map(|frame| frame[0])
        .collect();
    let peak_freq = peak_frequency(&left_channel, sample_rate);
    let freq_diff = (peak_freq - frequency).abs();
    assert!(
        freq_diff < 50.0,
        "peak frequency {} Hz deviates from expected {} Hz by {} Hz (tolerance 50 Hz)",
        peak_freq,
        frequency,
        freq_diff
    );
}

/// 440Hz sine 在 48kHz 单声道下的回环（覆盖不同频率/采样率/声道数）。
#[test]
fn test_golden_sine_440hz_48000_mono_roundtrip() {
    let sample_rate = 48000u32;
    let channels = 1u16;
    let frequency = 440.0_f64;
    let duration = 0.5;

    let original = generate_sine(frequency, sample_rate, duration, channels);
    let wav_bytes = encode_wav_bytes(&original, sample_rate, channels);

    let cursor = Cursor::new(wav_bytes);
    let mut decoder = AudioDecoder::open_reader(Box::new(cursor), "golden_mono.wav")
        .expect("decoder should open mono WAV");

    let mut decoded = Vec::new();
    while let Some(frame) = decoder.next_frame().expect("decode should succeed") {
        decoded.extend_from_slice(&frame.samples);
    }

    // RMS 断言
    let rms_val = rms(&decoded);
    let expected_rms = std::f64::consts::FRAC_1_SQRT_2;
    let rms_ratio = rms_val / expected_rms;
    assert!(
        rms_ratio > 0.93 && rms_ratio < 1.07,
        "RMS {} ratio {} out of tolerance for 440Hz mono",
        rms_val,
        rms_ratio
    );

    // 频率 bin 断言
    let dft_window = 8192.min(decoded.len());
    let peak_freq = peak_frequency(&decoded[..dft_window], sample_rate);
    let freq_diff = (peak_freq - frequency).abs();
    assert!(
        freq_diff < 50.0,
        "peak frequency {} Hz deviates from 440 Hz by {} Hz",
        peak_freq,
        freq_diff
    );
}
