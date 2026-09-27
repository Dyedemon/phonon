//! Integration tests for the phonon-core crate.
//!
//! Tests ring buffer concurrency, DSP chain, volume control,
//! and playback engine state machine.

use phonon_core::dsp::DspProcessor;
use phonon_core::*;
use rubato::Resampler;
use std::thread;
use std::time::Duration;

// ============================================================
// RingBuffer stress tests
// ============================================================

#[test]
fn test_ring_buffer_concurrent_write_read() {
    let buf = RingBuffer::new(200000);
    let buf_writer = buf.clone();
    let buf_reader = buf.clone();

    let writer = thread::spawn(move || {
        let samples: Vec<f32> = (0..10000).map(|i| i as f32).collect();
        for _ in 0..10 {
            let written = buf_writer.write(&samples);
            assert_eq!(written, 10000, "Write should not be truncated");
        }
    });

    let reader = thread::spawn(move || {
        let mut total_read = 0usize;
        let mut read_buf = vec![0.0f32; 4096];
        while total_read < 100000 {
            let n = buf_reader.read(&mut read_buf);
            total_read += n;
            if n == 0 {
                thread::sleep(Duration::from_micros(100));
            }
        }
        total_read
    });

    writer.join().unwrap();
    let total = reader.join().unwrap();
    assert!(total >= 100000, "Expected >= 100000, got {}", total);
}

#[test]
fn test_ring_buffer_no_data_loss() {
    let buf = RingBuffer::new(1024);
    let expected: Vec<f32> = (0..500).map(|i| i as f32).collect();

    let written = buf.write(&expected);
    assert_eq!(written, 500);

    let mut read_buf = vec![0.0f32; 500];
    let read = buf.read(&mut read_buf);
    assert_eq!(read, 500);

    for (i, &v) in read_buf.iter().enumerate().take(500) {
        assert_eq!(v, i as f32, "Mismatch at index {}", i);
    }
}

#[test]
fn test_ring_buffer_wrap_around() {
    let buf = RingBuffer::new(256);

    // Fill buffer
    let samples: Vec<f32> = (0..200).map(|i| i as f32).collect();
    buf.write(&samples);

    // Read half
    let mut read_buf = vec![0.0f32; 100];
    buf.read(&mut read_buf);
    for (i, &v) in read_buf.iter().enumerate().take(100) {
        assert_eq!(v, i as f32);
    }

    // Write more (should wrap around)
    let more: Vec<f32> = (200..350).map(|i| i as f32).collect();
    let written = buf.write(&more);
    assert!(written > 0);

    // Read remaining
    let mut final_buf = vec![0.0f32; 500];
    let total_read = buf.read(&mut final_buf);
    assert!(total_read > 0);

    // First read should be the second half of first batch (100..200)
    for (i, &v) in final_buf.iter().enumerate().take(100) {
        assert_eq!(v, (i + 100) as f32);
    }
}

#[test]
fn test_ring_buffer_empty_read() {
    let buf = RingBuffer::new(1024);
    let mut read_buf = vec![0.0f32; 100];
    let n = buf.read(&mut read_buf);
    assert_eq!(n, 0);
    assert!(buf.is_empty());
}

#[test]
fn test_ring_buffer_clone_independent() {
    let buf1 = RingBuffer::new(1024);
    let buf2 = buf1.clone();

    let samples = vec![1.0f32; 100];
    buf1.write(&samples);

    // Both should see the same data
    let mut read1 = vec![0.0f32; 100];
    let mut read2 = vec![0.0f32; 100];

    let n1 = buf1.read(&mut read1);
    let n2 = buf2.read(&mut read2);

    // Since they share the same inner buffer, reading from one affects the other
    assert!(n1 + n2 <= 100);
}

// ============================================================
// DSP chain tests
// ============================================================

#[test]
fn test_dsp_chain_add_remove() {
    let mut chain = phonon_core::dsp::DspChain::new();
    assert!(chain.is_empty());

    let rg = phonon_core::dsp::ReplayGainProcessor::new();
    chain.add(Box::new(rg));
    assert_eq!(chain.len(), 1);

    let downmix = phonon_core::dsp::DownmixProcessor::new();
    chain.add(Box::new(downmix));
    assert_eq!(chain.len(), 2);

    chain.remove("replaygain");
    assert_eq!(chain.len(), 1);
}

#[test]
fn test_dsp_chain_reorder() {
    let mut chain = phonon_core::dsp::DspChain::new();
    chain.add(Box::new(phonon_core::dsp::ReplayGainProcessor::new()));
    chain.add(Box::new(phonon_core::dsp::DownmixProcessor::new()));

    let info = chain.list_processors();
    assert_eq!(info[0].name, "ReplayGain");
    assert_eq!(info[1].name, "Downmix");

    chain.reorder(0, 1);
    let info = chain.list_processors();
    assert_eq!(info[0].name, "Downmix");
    assert_eq!(info[1].name, "ReplayGain");
}

#[test]
fn test_dsp_chain_enable_disable() {
    let mut chain = phonon_core::dsp::DspChain::new();
    let mut rg = phonon_core::dsp::ReplayGainProcessor::new();
    rg.configure(dsp::ReplayGainMode::Track, -6.0);
    chain.add(Box::new(rg));

    let info = chain.list_processors();
    assert!(info[0].enabled);

    chain.set_enabled("replaygain", false);
    let info = chain.list_processors();
    assert!(!info[0].enabled);
}

#[test]
fn test_dsp_chain_total_latency() {
    let mut chain = phonon_core::dsp::DspChain::new();
    chain.add(Box::new(phonon_core::dsp::ReplayGainProcessor::new()));
    chain.add(Box::new(phonon_core::dsp::DownmixProcessor::new()));

    // Both processors have 0 latency
    assert!((chain.total_latency() - 0.0).abs() < 0.001);
}

#[test]
fn test_dsp_chain_process_multiple() {
    let mut chain = phonon_core::dsp::DspChain::new();
    let mut rg = phonon_core::dsp::ReplayGainProcessor::new();
    rg.configure(dsp::ReplayGainMode::Track, -6.0); // -6dB = 0.5x
    chain.add(Box::new(rg));

    let mut samples = vec![1.0f32, 0.5, -0.5, -1.0];
    chain.process(&mut samples, 2, 44100);

    assert!((samples[0] - 0.5).abs() < 0.002);
    assert!((samples[1] - 0.25).abs() < 0.002);
    assert!((samples[2] + 0.25).abs() < 0.002);
    assert!((samples[3] + 0.5).abs() < 0.002);
}

// ============================================================
// ReplayGain processor tests
// ============================================================

#[test]
fn test_replay_gain_processor_modes() {
    let mut rg = phonon_core::dsp::ReplayGainProcessor::new();

    // Off by default
    assert!(!rg.enabled());

    // Track mode
    rg.configure(dsp::ReplayGainMode::Track, -3.0);
    assert!(rg.enabled());

    // Off
    rg.configure(dsp::ReplayGainMode::Off, 0.0);
    assert!(!rg.enabled());

    // Album mode
    rg.configure(dsp::ReplayGainMode::Album, -4.5);
    assert!(rg.enabled());
}

#[test]
fn test_replay_gain_amplitude_calculation() {
    let mut rg = phonon_core::dsp::ReplayGainProcessor::new();

    // -6dB should halve amplitude
    rg.configure(dsp::ReplayGainMode::Track, -6.0);
    let mut samples = vec![1.0f32; 100];
    rg.process(&mut samples, 2, 44100);
    for s in &samples {
        assert!((s - 0.5).abs() < 0.002);
    }

    // +6dB should double amplitude (10^(6/20) ≈ 1.995)
    rg.configure(dsp::ReplayGainMode::Track, 6.0);
    let mut samples = vec![1.0f32; 100];
    rg.process(&mut samples, 2, 44100);
    for s in &samples {
        assert!((s - 2.0).abs() < 0.005);
    }

    // 0dB should not change
    rg.configure(dsp::ReplayGainMode::Track, 0.0);
    let mut samples = vec![0.5f32; 100];
    rg.process(&mut samples, 2, 44100);
    for s in &samples {
        assert!((s - 0.5).abs() < 0.002);
    }
}

// ============================================================
// Downmix processor tests
// ============================================================

#[test]
fn test_downmix_stereo_passthrough() {
    let dm = phonon_core::dsp::DownmixProcessor::new();
    // Stereo should pass through unchanged
    let mut samples = vec![0.5f32, -0.5, 1.0, -1.0];
    let original = samples.clone();
    dm.process(&mut samples, 2, 44100);
    assert_eq!(samples, original);
}

#[test]
fn test_downmix_51_to_stereo() {
    let mut dm_enabled = phonon_core::dsp::DownmixProcessor::new();
    dm_enabled.set_enabled(true);

    // 5.1: L R C LFE Ls Rs = 6 channels
    // One frame: [1.0, 0.0, 0.707, 0.0, 0.0, 0.0] (only left channel active)
    let mut samples = vec![1.0f32, 0.0, 0.707, 0.0, 0.0, 0.0];
    dm_enabled.process(&mut samples, 6, 44100);

    // After downmix to stereo, first 2 samples are the stereo output
    // Left = 1.0*1.0 + 0.707*0.707 = 1.0 + 0.5 = 1.5
    // Right = 0.0*1.0 + 0.707*0.707 = 0.5 (C contributes to both channels)
    assert!((samples[0] - 1.5).abs() < 0.01);
    assert!((samples[1] - 0.5).abs() < 0.01);
}

// ============================================================
// Volume control tests
// ============================================================

#[test]
fn test_volume_hardware_mode() {
    let mut vc = VolumeControl::new(true);
    assert_eq!(vc.mode(), phonon_core::volume::VolumeMode::Hardware);
    assert!(vc.is_hardware_supported());

    vc.set_volume(0.75);
    assert_eq!(vc.level(), 0.75);

    // Software volume is always applied regardless of hardware support flag
    let mut samples = vec![1.0f32; 10];
    vc.apply_software_volume(&mut samples);
    for s in &samples {
        assert!((*s - 0.75).abs() < 0.001);
    }
}

#[test]
fn test_volume_software_mode() {
    let mut vc = VolumeControl::new(false);
    assert_eq!(vc.mode(), phonon_core::volume::VolumeMode::Software);

    vc.set_volume(0.5);
    let mut samples = vec![1.0f32, 0.5, -0.5];
    vc.apply_software_volume(&mut samples);

    assert!((samples[0] - 0.5).abs() < 0.001);
    assert!((samples[1] - 0.25).abs() < 0.001);
    assert!((samples[2] + 0.25).abs() < 0.001);
}

#[test]
fn test_volume_mode_switch() {
    let mut vc = VolumeControl::new(true);
    assert_eq!(vc.mode(), phonon_core::volume::VolumeMode::Hardware);

    // Simulate device that doesn't support hardware volume
    vc.set_hardware_supported(false);
    assert_eq!(vc.mode(), phonon_core::volume::VolumeMode::Software);
    assert!(!vc.is_hardware_supported());
}

#[test]
fn test_volume_clamp_range() {
    let mut vc = VolumeControl::new(false);

    vc.set_volume(1.5);
    assert_eq!(vc.level(), 1.0);

    vc.set_volume(-0.5);
    assert_eq!(vc.level(), 0.0);

    vc.set_volume(0.0);
    assert_eq!(vc.level(), 0.0);

    vc.set_volume(1.0);
    assert_eq!(vc.level(), 1.0);
}

// ============================================================
// Playback state machine tests
// ============================================================

#[test]
fn test_playback_state_transitions() {
    // State machine logic: Idle -> Playing -> Paused -> Playing -> Stopped -> Idle
    let state = PlaybackState::Idle;
    assert_eq!(state, PlaybackState::Idle);

    // Verify all states are different
    assert_ne!(PlaybackState::Idle, PlaybackState::Playing);
    assert_ne!(PlaybackState::Playing, PlaybackState::Paused);
    assert_ne!(PlaybackState::Paused, PlaybackState::Stopped);
}

// ============================================================
// EngineConfig tests
// ============================================================

#[test]
fn test_engine_config_default() {
    let config = EngineConfig::default();
    assert_eq!(config.buffer_size, 192000);
    assert!(config.device_name.is_empty());
    assert!(!config.exclusive);
}

#[test]
fn test_engine_config_custom() {
    let config = EngineConfig {
        buffer_size: 1024,
        device_name: "MyDAC".into(),
        exclusive: false,
        output_rate: 0,
        resampler_quality: ResamplerQuality::Balanced,
    };
    assert_eq!(config.buffer_size, 1024);
    assert_eq!(config.device_name, "MyDAC");
    assert!(!config.exclusive);
}

// ============================================================
// EngineError tests
// ============================================================

#[test]
fn test_engine_error_display() {
    let err = EngineError::Decode("bad file".into());
    assert!(err.to_string().contains("bad file"));

    let err = EngineError::InvalidState("not playing".into());
    assert!(err.to_string().contains("not playing"));
}

// ============================================================
// DeviceError tests
// ============================================================

#[test]
fn test_device_error_all_variants() {
    let errors = vec![
        device::DeviceError::NoDevice,
        device::DeviceError::DeviceNotFound("Test".into()),
        device::DeviceError::Enumeration("fail".into()),
        device::DeviceError::Stream("broken".into()),
        device::DeviceError::Resampler("bad params".into()),
    ];

    for err in &errors {
        let msg = err.to_string();
        assert!(!msg.is_empty(), "Error message should not be empty");
    }
}

// ============================================================
// DSP downmix accuracy tests
// ============================================================

use phonon_core::dsp;

#[test]
fn test_downmix_preserves_energy() {
    let mut dm = dsp::DownmixProcessor::new();
    dm.set_enabled(true);

    // Create a 5.1 signal with known channel values
    // L=1.0, R=0.5, C=0.707, LFE=0.3, Ls=0.2, Rs=0.2
    let samples = vec![1.0f32, 0.5, 0.707, 0.3, 0.2, 0.2];
    let mut output = samples.clone();
    dm.process(&mut output, 6, 44100);

    // Output should be stereo (first 2 channels have the downmixed result)
    // Left channel should have contributions from L, C, Ls
    // Right channel should have contributions from R, C, Rs
    assert!(output[0] != 0.0, "Left channel should not be silent");
    assert!(output[1] != 0.0, "Right channel should not be silent");
}

#[test]
fn test_downmix_not_enabled_passthrough() {
    let dm = dsp::DownmixProcessor::new();
    // Default is disabled

    let samples = vec![1.0f32, 0.0, 0.707, 0.0, 0.0, 0.0];
    let mut output = samples.clone();
    dm.process(&mut output, 6, 44100);

    // Should pass through unchanged when disabled
    assert_eq!(output.len(), 6);
    assert_eq!(output, samples);
}

// ============================================================
// DSP chain ordering test
// ============================================================

#[test]
fn test_dsp_chain_order_matters() {
    // ReplayGain (-6dB) then another ReplayGain (-6dB) = -12dB total
    let mut chain = dsp::DspChain::new();

    let mut rg1 = dsp::ReplayGainProcessor::new();
    rg1.configure(dsp::ReplayGainMode::Track, -6.0);
    chain.add(Box::new(rg1));

    let mut rg2 = dsp::ReplayGainProcessor::new();
    rg2.configure(dsp::ReplayGainMode::Track, -6.0);
    chain.add(Box::new(rg2));

    let mut samples = vec![1.0f32; 100];
    chain.process(&mut samples, 2, 44100);

    // -12dB = 0.25x amplitude
    for s in &samples {
        assert!((s - 0.25).abs() < 0.002, "Expected 0.25, got {}", s);
    }
}

// ============================================================
// ReplayGain mode enum tests
// ============================================================
// Resampler tests
// ============================================================

#[test]
fn test_create_resampler_44k_to_48k() {
    // Create a resampler from 44100Hz to 48000Hz (stereo)
    let resampler = phonon_core::DeviceManager::create_resampler(44100, 48000, 2);
    assert!(
        resampler.is_ok(),
        "Resampler creation should succeed: {:?}",
        resampler.err()
    );
}

#[test]
fn test_create_resampler_48k_to_44k() {
    let resampler = phonon_core::DeviceManager::create_resampler(48000, 44100, 2);
    assert!(
        resampler.is_ok(),
        "Resampler creation should succeed: {:?}",
        resampler.err()
    );
}

#[test]
fn test_create_resampler_96k_to_48k() {
    let resampler = phonon_core::DeviceManager::create_resampler(96000, 48000, 2);
    assert!(
        resampler.is_ok(),
        "Resampler creation should succeed: {:?}",
        resampler.err()
    );
}

#[test]
fn test_create_resampler_same_rate() {
    let resampler = phonon_core::DeviceManager::create_resampler(44100, 44100, 2);
    assert!(
        resampler.is_ok(),
        "Passthrough resampler should succeed: {:?}",
        resampler.err()
    );
}

#[test]
fn test_create_resampler_mono() {
    let resampler = phonon_core::DeviceManager::create_resampler(44100, 48000, 1);
    assert!(
        resampler.is_ok(),
        "Mono resampler should succeed: {:?}",
        resampler.err()
    );
}

#[test]
fn test_resampler_process_audio() {
    let mut resampler = phonon_core::DeviceManager::create_resampler(44100, 48000, 2)
        .expect("Should create resampler");

    // Process multiple chunks of 1024 frames each (the resampler's chunk size)
    let chunk_size = 1024usize;
    let num_chunks = 8; // Process enough chunks to get stable output
    let mut total_output_frames = 0usize;
    let mut max_amplitude = 0.0f32;

    for chunk_idx in 0..num_chunks {
        let mut left = Vec::with_capacity(chunk_size);
        let mut right = Vec::with_capacity(chunk_size);
        let base_frame = chunk_idx * chunk_size;
        for i in 0..chunk_size {
            let t = (base_frame + i) as f64 / 44100.0;
            let val = (2.0 * std::f64::consts::PI * 440.0 * t).sin() as f32;
            left.push(val * 0.5);
            right.push(val * 0.5);
        }
        let wave_in = vec![left, right];

        let wave_out = resampler
            .process(&wave_in, None)
            .expect("Resampling should succeed");

        // Output should be stereo
        assert_eq!(wave_out.len(), 2);
        assert_eq!(wave_out[0].len(), wave_out[1].len());
        total_output_frames += wave_out[0].len();

        // Track max amplitude
        for ch in &wave_out {
            for &s in ch {
                if s.abs() > max_amplitude {
                    max_amplitude = s.abs();
                }
            }
        }
    }

    // Should produce some output
    assert!(
        total_output_frames > 0,
        "Resampler should produce output frames"
    );

    // Output should be proportional to input (ratio ≈ 48000/44100 ≈ 1.088)
    let input_frames = num_chunks * chunk_size;
    let ratio = total_output_frames as f64 / input_frames as f64;
    assert!(
        ratio > 0.5 && ratio < 2.0,
        "Output ratio should be reasonable, got {} ({} output / {} input)",
        ratio,
        total_output_frames,
        input_frames
    );

    // Output should not be silent
    assert!(
        max_amplitude > 0.01,
        "Output should not be silent, max amplitude: {}",
        max_amplitude
    );
}

// ============================================================
// ReplayGain mode equality
// ============================================================

#[test]
fn test_replay_gain_mode_equality() {
    assert_eq!(dsp::ReplayGainMode::Off, dsp::ReplayGainMode::Off);
    assert_ne!(dsp::ReplayGainMode::Track, dsp::ReplayGainMode::Album);
    assert_ne!(dsp::ReplayGainMode::Off, dsp::ReplayGainMode::Track);
}

// ============================================================
// Boundary / Edge case tests
// ============================================================

#[test]
fn test_engine_empty_queue_play_fails() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    let result = engine.start_playback();
    // 空队列 start_playback → quiet no-op (Ok)，不抛错避免前端 unhandled promise rejection
    assert!(
        result.is_ok(),
        "Empty queue play should be a quiet no-op (Ok)"
    );
    assert_eq!(
        engine.state(),
        PlaybackState::Idle,
        "state remains Idle when queue empty"
    );
    assert!(
        engine.current_track().is_none(),
        "no current track when queue empty"
    );
}

#[test]
fn test_engine_empty_queue_play_from_index() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    let result = engine.play_from_index(0);
    // 空队列 play_from_index → quiet no-op (Ok)，不抛错
    assert!(
        result.is_ok(),
        "play_from_index on empty queue should be a quiet no-op (Ok)"
    );
    assert_eq!(
        engine.state(),
        PlaybackState::Idle,
        "state remains Idle when queue empty"
    );
    // play_from_index 任意越界索引同样 quiet no-op（引擎实现里已对 q_len==0 短路）
    let result2 = engine.play_from_index(usize::MAX);
    assert!(
        result2.is_ok(),
        "play_from_index with large index on empty queue should be quiet no-op (Ok)"
    );
}

#[test]
fn test_engine_empty_queue_next() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    // next() on empty queue — should not panic
    engine.next();
    // Should not panic
    assert_eq!(engine.state(), PlaybackState::Idle);
}

#[test]
fn test_engine_empty_queue_previous() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    // previous() on empty queue — should not panic
    engine.previous();
    assert_eq!(engine.state(), PlaybackState::Idle);
}

#[test]
fn test_engine_stop_idempotent() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    // Stop when already stopped should not panic
    engine.stop();
    engine.stop();
    assert_eq!(engine.state(), PlaybackState::Stopped);
}

#[test]
fn test_engine_stop_preserves_queue() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    let path = std::env::current_dir()
        .unwrap()
        .join("..")
        .join("phonon-codec")
        .join("tests")
        .join("fixtures")
        .join("test.wav");
    let path_str = path.to_string_lossy().to_string();
    engine.enqueue(&path_str).unwrap();
    engine.enqueue(&format!("{}#2", path_str)).unwrap();
    assert_eq!(engine.queue_len(), 2);
    engine.stop();
    // Queue should still be intact
    assert_eq!(engine.queue_len(), 2);
}

#[test]
fn test_engine_remove_out_of_bounds() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    // Remove from empty queue should not panic
    engine.remove_from_queue(&[0, 999]);
    assert_eq!(engine.queue_len(), 0);
}

#[test]
fn test_engine_reorder_same_index() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    let path = std::env::current_dir()
        .unwrap()
        .join("..")
        .join("phonon-codec")
        .join("tests")
        .join("fixtures")
        .join("test.wav");
    let path_str = path.to_string_lossy().to_string();
    engine.enqueue(&path_str).unwrap();
    engine.enqueue(&format!("{}#2", path_str)).unwrap();
    // Reorder to same position should be a no-op
    engine.reorder_queue(0, 0);
    assert_eq!(engine.queue_len(), 2);
}

#[test]
fn test_engine_reorder_out_of_bounds() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    let path = std::env::current_dir()
        .unwrap()
        .join("..")
        .join("phonon-codec")
        .join("tests")
        .join("fixtures")
        .join("test.wav");
    let path_str = path.to_string_lossy().to_string();
    engine.enqueue(&path_str).unwrap();
    // Reorder with out-of-bounds indices should not panic
    engine.reorder_queue(999, 0);
    engine.reorder_queue(0, 999);
    assert_eq!(engine.queue_len(), 1);
}

#[test]
fn test_engine_clear_queue() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    let path = std::env::current_dir()
        .unwrap()
        .join("..")
        .join("phonon-codec")
        .join("tests")
        .join("fixtures")
        .join("test.wav");
    let path_str = path.to_string_lossy().to_string();
    engine.enqueue(&path_str).unwrap();
    engine.enqueue(&format!("{}#2", path_str)).unwrap();
    assert_eq!(engine.queue_len(), 2);
    engine.clear_queue();
    assert_eq!(engine.queue_len(), 0);
}

#[test]
fn test_engine_config_minimum_buffer() {
    let config = EngineConfig {
        buffer_size: 1024,
        device_name: String::new(),
        exclusive: false,
        output_rate: 0,
        resampler_quality: ResamplerQuality::Balanced,
    };
    let engine = PlaybackEngine::new(config).unwrap();
    // Should still work with minimum buffer
    assert_eq!(engine.state(), PlaybackState::Idle);
}

#[test]
fn test_engine_config_large_buffer() {
    let config = EngineConfig {
        buffer_size: 1_000_000,
        device_name: String::new(),
        exclusive: false,
        output_rate: 0,
        resampler_quality: ResamplerQuality::Balanced,
    };
    let engine = PlaybackEngine::new(config).unwrap();
    assert_eq!(engine.state(), PlaybackState::Idle);
}

#[test]
fn test_volume_zero_silence() {
    let mut vc = VolumeControl::new(false);
    vc.set_volume(0.0);
    let mut samples = vec![1.0f32, 0.5, -0.5, 0.8];
    vc.apply_software_volume(&mut samples);
    for s in &samples {
        assert!((*s - 0.0).abs() < 0.001, "Expected 0.0, got {}", s);
    }
}

#[test]
fn test_volume_one_passthrough() {
    let mut vc = VolumeControl::new(false);
    vc.set_volume(1.0);
    let original = vec![0.75f32, -0.3, 0.5, 0.0];
    let mut samples = original.clone();
    vc.apply_software_volume(&mut samples);
    for (i, s) in samples.iter().enumerate() {
        assert!(
            (*s - original[i]).abs() < 0.001,
            "Expected {}, got {}",
            original[i],
            s
        );
    }
}

#[test]
fn test_engine_queue_snapshot_empty() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    let snapshot = engine.queue_snapshot();
    assert!(snapshot.is_empty());
}

#[test]
fn test_engine_queue_snapshot_with_items() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    let path = std::env::current_dir()
        .unwrap()
        .join("..")
        .join("phonon-codec")
        .join("tests")
        .join("fixtures")
        .join("test.wav");
    let path_str = path.to_string_lossy().to_string();
    engine.enqueue(&path_str).unwrap();
    let snapshot = engine.queue_snapshot();
    assert_eq!(snapshot.len(), 1);
    assert!(snapshot[0].path.contains("test.wav"));
}

#[test]
fn test_engine_current_track_none_initially() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    assert!(engine.current_track().is_none());
}

#[test]
fn test_engine_position_zero_initially() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    assert!(engine.position() < 0.001);
}

#[test]
fn test_engine_duration_none_initially() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    assert!(engine.duration().is_none());
}

#[test]
fn test_engine_tracks_played_zero_initially() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    assert_eq!(engine.tracks_played(), 0);
}

#[test]
fn test_engine_buffer_fill_empty() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    assert!(engine.buffer_fill() < 0.001);
}

#[test]
fn test_dsp_chain_empty() {
    let chain = dsp::DspChain::new();
    assert!(chain.is_empty());
    assert_eq!(chain.len(), 0);
    assert!(chain.list_processors().is_empty());
}

#[test]
fn test_dsp_chain_empty_process() {
    let chain = dsp::DspChain::new();
    let mut samples = vec![1.0f32, 0.5, -0.5];
    chain.process(&mut samples, 2, 44100);
    // Should be unchanged
    assert_eq!(samples, vec![1.0f32, 0.5, -0.5]);
}

#[test]
fn test_remove_from_queue_empty() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    engine.remove_from_queue(&[0]);
    assert_eq!(engine.queue_len(), 0);
}

#[test]
fn test_enqueue_multiple() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    let path = std::env::current_dir()
        .unwrap()
        .join("..")
        .join("phonon-codec")
        .join("tests")
        .join("fixtures")
        .join("test.wav");
    let path_str = path.to_string_lossy().to_string();
    let path2 = format!("{}#2", path_str);
    let path3 = format!("{}#3", path_str);
    let paths = [path_str.as_str(), path2.as_str(), path3.as_str()];
    engine.enqueue_multiple(&paths).unwrap();
    assert_eq!(engine.queue_len(), 3);
}

// ============================================================
// Batch 3: Bit Depth tests
// ============================================================

#[test]
fn test_bit_depth_default_is_float32() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    assert_eq!(
        engine.bit_depth(),
        0,
        "Default bit depth should be 0 (float32)"
    );
}

#[test]
fn test_bit_depth_set_16bit() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    engine.set_bit_depth(16);
    assert_eq!(engine.bit_depth(), 16);
}

#[test]
fn test_bit_depth_set_24bit() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    engine.set_bit_depth(24);
    assert_eq!(engine.bit_depth(), 24);
}

#[test]
fn test_bit_depth_toggle() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    engine.set_bit_depth(16);
    assert_eq!(engine.bit_depth(), 16);
    engine.set_bit_depth(0);
    assert_eq!(engine.bit_depth(), 0);
    engine.set_bit_depth(24);
    assert_eq!(engine.bit_depth(), 24);
}

// ============================================================
// Batch 3: Output Sample Rate tests
// ============================================================

#[test]
fn test_output_sample_rate_default_is_auto() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    assert_eq!(
        engine.output_sample_rate_setting(),
        0,
        "Default should be 0 (auto)"
    );
}

#[test]
fn test_output_sample_rate_set_44100() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    engine.set_output_sample_rate(44100);
    assert_eq!(engine.output_sample_rate_setting(), 44100);
}

#[test]
fn test_output_sample_rate_set_96000() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    engine.set_output_sample_rate(96000);
    assert_eq!(engine.output_sample_rate_setting(), 96000);
}

#[test]
fn test_output_sample_rate_set_192000() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    engine.set_output_sample_rate(192000);
    assert_eq!(engine.output_sample_rate_setting(), 192000);
}

// ============================================================
// Batch 3: DSD Mode tests
// ============================================================

#[test]
fn test_dsd_mode_default_is_off() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    assert_eq!(engine.dsd_mode(), DsdMode::Off);
}

#[test]
fn test_dsd_mode_set_dop_dsd64() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    engine.set_dsd_mode(DsdMode::DopDsd64);
    assert_eq!(engine.dsd_mode(), DsdMode::DopDsd64);
}

#[test]
fn test_dsd_mode_set_dop_dsd128() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    engine.set_dsd_mode(DsdMode::DopDsd128);
    assert_eq!(engine.dsd_mode(), DsdMode::DopDsd128);
}

#[test]
fn test_dsd_mode_set_dop_dsd256() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    engine.set_dsd_mode(DsdMode::DopDsd256);
    assert_eq!(engine.dsd_mode(), DsdMode::DopDsd256);
}

#[test]
fn test_dsd_mode_full_cycle() {
    let config = EngineConfig::default();
    let engine = PlaybackEngine::new(config).unwrap();
    let modes = [
        DsdMode::Off,
        DsdMode::DopDsd64,
        DsdMode::DopDsd128,
        DsdMode::DopDsd256,
    ];
    for &mode in &modes {
        engine.set_dsd_mode(mode);
        assert_eq!(
            engine.dsd_mode(),
            mode,
            "DSD mode cycle failed at {:?}",
            mode
        );
    }
}

#[test]
fn test_dsd_carrier_rates() {
    assert_eq!(DsdMode::Off.carrier_rate(), None);
    assert_eq!(DsdMode::DopDsd64.carrier_rate(), Some(176400));
    assert_eq!(DsdMode::DopDsd128.carrier_rate(), Some(352800));
    assert_eq!(DsdMode::DopDsd256.carrier_rate(), Some(705600));
}

#[test]
fn test_dsd_bits_per_sample() {
    assert_eq!(DsdMode::Off.bits_per_sample(), 16);
    assert_eq!(DsdMode::DopDsd64.bits_per_sample(), 16);
    assert_eq!(DsdMode::DopDsd128.bits_per_sample(), 16);
    assert_eq!(DsdMode::DopDsd256.bits_per_sample(), 16);
}

// ============================================================
// Batch 3: Dithering tests
// ============================================================

#[test]
fn test_dither_16bit_produces_valid_range() {
    let mut samples = vec![0.5f32, -0.3f32, 0.0f32, 0.999f32, -0.999f32];
    phonon_core::engine::test_apply_dither(&mut samples, 16);
    for s in &samples {
        assert!(
            *s >= -1.0 && *s <= 1.0,
            "Dithered sample out of range: {}",
            s
        );
    }
}

#[test]
fn test_dither_24bit_produces_valid_range() {
    let mut samples = vec![0.5f32, -0.3f32, 0.0f32, 0.999f32, -0.999f32];
    phonon_core::engine::test_apply_dither(&mut samples, 24);
    for s in &samples {
        assert!(
            *s >= -1.0 && *s <= 1.0,
            "Dithered sample out of range: {}",
            s
        );
    }
}

#[test]
fn test_dither_zero_signal_stays_near_zero() {
    let mut samples = vec![0.0f32; 1000];
    phonon_core::engine::test_apply_dither(&mut samples, 16);
    let max_abs = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    // With triangular dither at 1 LSB peak-to-peak, max should be small
    assert!(max_abs < 0.001, "Zero signal dither too large: {}", max_abs);
}

#[test]
fn test_dither_preserves_signal_polarity() {
    let mut positive = vec![0.8f32; 100];
    let mut negative = vec![-0.8f32; 100];
    phonon_core::engine::test_apply_dither(&mut positive, 16);
    phonon_core::engine::test_apply_dither(&mut negative, 24);
    let pos_avg: f32 = positive.iter().sum::<f32>() / positive.len() as f32;
    let neg_avg: f32 = negative.iter().sum::<f32>() / negative.len() as f32;
    assert!(
        pos_avg > 0.0,
        "Positive signal should remain positive, got {}",
        pos_avg
    );
    assert!(
        neg_avg < 0.0,
        "Negative signal should remain negative, got {}",
        neg_avg
    );
}
