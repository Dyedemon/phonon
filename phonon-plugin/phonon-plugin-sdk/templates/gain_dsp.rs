//! Example Phonon DSP Plugin: Gain Control
//!
//! A simple volume/gain plugin that demonstrates the DSP plugin interface.
//!
//! Build:
//!   cargo build --target wasm32-wasip1 --release -p gain-dsp
//!   (requires: rustup target add wasm32-wasip1)
//!
//! The resulting .wasm file can be loaded by Phonon.

use phonon_plugin_sdk::{declare_dsp_plugin, declare_plugin_config};

/// Default gain in dB
static mut GAIN_DB: f32 = 0.0;

declare_dsp_plugin! {
    name: "Gain DSP",
    version: "1.0.0",
    init: || {
        // Nothing to initialize
    },
    process: |samples: &mut [f32], _channels: u16, _sample_rate: u32| {
        let gain_db = unsafe { GAIN_DB };
        let gain_linear = 10.0_f32.powf(gain_db / 20.0);
        for sample in samples.iter_mut() {
            *sample *= gain_linear;
        }
        // Clamp to prevent clipping
        for sample in samples.iter_mut() {
            *sample = sample.clamp(-1.0, 1.0);
        }
        Ok(())
    },
}

declare_plugin_config!(r#"{"gain_db": 0.0}"#);