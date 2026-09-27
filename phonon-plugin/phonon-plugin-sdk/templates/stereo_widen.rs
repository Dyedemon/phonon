//! Example Phonon DSP Plugin: Stereo Widener
//!
//! A stereo widening effect using mid-side processing.
//! Increases the stereo width by adjusting the side channel gain.
//!
//! Build as a standalone plugin:
//!   1. Copy this file to your plugin project
//!   2. Add phonon-plugin-sdk to Cargo.toml dependencies
//!   3. cargo build --target wasm32-wasip1 --release
//!   4. Copy the .wasm to Phonon's plugins directory

use phonon_plugin_sdk::{declare_dsp_plugin, declare_plugin_config};

/// Width factor: 1.0 = normal, 2.0 = double width, 0.0 = mono
static mut WIDTH: f32 = 1.5;

declare_dsp_plugin! {
    name: "Stereo Widener",
    version: "1.0.0",
    init: || {
        // Nothing to initialize
    },
    process: |samples: &mut [f32], channels: u16, _sample_rate: u32| {
        if channels < 2 {
            return Ok(()); // Only works on stereo
        }
        let width = unsafe { WIDTH };
        let mid_gain = 1.0;
        let side_gain = width;

        for chunk in samples.chunks_exact_mut(channels as usize) {
            let left = chunk[0];
            let right = chunk[1];

            // Mid-Side encoding
            let mid = (left + right) * 0.5;
            let side = (left - right) * 0.5;

            // Apply width
            let mid_processed = mid * mid_gain;
            let side_processed = side * side_gain;

            // Mid-Side decoding
            chunk[0] = (mid_processed + side_processed).clamp(-1.0, 1.0);
            chunk[1] = (mid_processed - side_processed).clamp(-1.0, 1.0);
        }
        Ok(())
    },
}

declare_plugin_config!(r#"{"width": 1.5}"#);