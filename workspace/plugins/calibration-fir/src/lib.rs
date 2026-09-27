#![allow(non_snake_case, dead_code)]
//! Calibration FIR plugin (Wasm).
//!
//! # Responsibility
//!
//! Run overlap-save convolution with per-channel FIR coefficients pulled
//! from the host on every DSP frame. This is the **B方案 FIR 补偿支路**:
//!
//!   ┌───────────┐   L+R samples    ┌────────────────────────────┐
//!   │ Host DSP  │ ───────────────► │ calibration-fir (this)     │
//!   │ Chain     │                  │                            │
//!   │           │ ◄─────────────── │ 1. poll host.get_config_v()│
//!   └───────────┘   convolved      │ 2. changed → read + decode│
//!                   audio          │ 3. overlap-save per ch.   │
//!                                  └────────────────────────────┘
//!
//! The Biquad correction has already been applied by the native
//! `CalibrationPeqProcessor` *upstream* of us (plugin ordering is
//! deterministic based on user config in the mixer).
//!
//! # Host ABI
//!
//! The plugin imports two functions from module `host`:
//!
//! ```wat
//! (import "host" "get_plugin_config_version"
//!   (func (param i32 i32) (result i64)))
//! ;; plugin_name_ptr/len → u64 version; monotonic per plugin name
//!
//! (import "host" "read_plugin_config"
//!   (func (param i32 i32 i32 i32) (result i32)))
//! ;; plugin_name_ptr/len dst_ptr/len → bytes read; 0 = not found
//! ```
//!
//! And exports the standard DSP entry points (`plugin_init`,
//! `plugin_reset`, `plugin_process_frame`).

#![cfg_attr(all(not(feature = "std"), target_arch = "wasm32"), no_std)]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::RefCell;

use calibration_types::FitError;
use serde::Deserialize;

// ── Host imports ──────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "host")]
extern "C" {
    /// Returns a monotonic version counter for the config blob named by
    /// the UTF-8 buffer (plugin_name_ptr, plugin_name_len). `0` = no
    /// config known for this plugin.
    fn get_plugin_config_version(plugin_name_ptr: i32, plugin_name_len: i32) -> i64;

    /// Copy UTF-8 JSON config for `name` into the plugin linear memory at
    /// dst_ptr, maximum dst_len bytes. Returns total bytes copied, or 0
    /// if no config is registered for that name. Caller uses the return
    /// value to re-allocate and retry on truncation.
    fn read_plugin_config(
        plugin_name_ptr: i32,
        plugin_name_len: i32,
        dst_ptr: i32,
        dst_len: i32,
    ) -> i32;
}

// Non-wasm / native test shims — keep the exact same signature so the
// rest of the crate compiles for `cargo test` on host.
#[cfg(not(target_arch = "wasm32"))]
unsafe fn get_plugin_config_version(_p: i32, _l: i32) -> i64 {
    0
}
#[cfg(not(target_arch = "wasm32"))]
unsafe fn read_plugin_config(_p: i32, _pl: i32, _d: i32, _dl: i32) -> i32 {
    0
}

// ── On-the-wire JSON config schema ────────────────────────────────────────

/// JSON payload pushed by the host (from `apply_fit_result` →
/// `set_plugin_config`) and pulled back here once per DSP frame.
///
/// All fields default when absent so a bypass-only payload `{"bypass":true}`
/// deserialises without error.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct FirConfig {
    pub sample_rate: u32,
    pub taps: usize,
    pub fir_L: Vec<f32>,
    pub fir_R: Vec<f32>,
    #[serde(default)]
    pub bypass: bool,
}

// ── Overlap-save convolution engine ───────────────────────────────────────

/// Single-channel state. Taps are constant during a pull cycle (we
/// rebuild the whole block if the config version bumps).
struct FirChannel {
    taps: Vec<f32>,
    /// Previous block's input tail. Length = taps - 1.
    history: Vec<f32>,
}

impl FirChannel {
    fn new(taps: Vec<f32>) -> Self {
        let hist = vec![0.0f32; taps.len().saturating_sub(1)];
        Self {
            taps,
            history: hist,
        }
    }

    fn reset(&mut self) {
        for x in self.history.iter_mut() {
            *x = 0.0;
        }
    }

    /// Process block of samples. `out.len()` MUST equal `inp.len()`.
    ///
    /// We use scalar overlap-save; taps are typically ≤ 2048 so SIMD
    /// auto-vectorisation beats an FFT-based overlap-add for these
    /// lengths. Revisit if `taps > 8192` ever.
    fn process(&mut self, inp: &[f32], out: &mut [f32]) {
        let ntap = self.taps.len();
        if ntap == 0 {
            out.copy_from_slice(inp);
            return;
        }
        // Prepend `history` to logical input buffer. We copy it plus
        // input into a scratch `buf`; scratch length = block + ntap - 1.
        // This writes into a temporary; but we keep history outside to
        // avoid reallocating per call.
        let hist_len = self.history.len();
        // For performance, use a small inline scratch. Allocate once
        // through a TLS thread-local (or in native test, just vec).
        let mut buf = Vec::with_capacity(hist_len + inp.len());
        buf.extend_from_slice(&self.history);
        buf.extend_from_slice(inp);

        for (i, o) in out.iter_mut().enumerate() {
            // output sample i = sum(buf[i + t] * taps[ntap - 1 - t])
            let mut acc = 0.0f32;
            let base = i + hist_len;
            for t in 0..ntap {
                acc += buf[base + 1 + t - ntap] * self.taps[t];
            }
            *o = acc;
        }
        // Rotate history = last (ntap-1) samples of input.
        if hist_len > 0 && inp.len() >= hist_len {
            let tail = &inp[inp.len() - hist_len..];
            self.history.copy_from_slice(tail);
        } else if hist_len > 0 {
            // block smaller than history: shift history left by inp.len()
            // and append new tail.
            let shift = inp.len().min(hist_len);
            self.history.copy_within(shift.., 0);
            self.history[hist_len - shift..].copy_from_slice(&inp[..shift]);
        }
    }
}

// ── Plugin state (thread-local, Wasm is single-threaded per spec) ─────────

#[derive(Default)]
struct PluginState {
    channels: u16,
    sample_rate: u32,
    known_version: i64,
    left: Option<FirChannel>,
    right: Option<FirChannel>,
    /// Scratch buffer for interleaved frame processing.
    buf: Vec<f32>,
}

thread_local! {
    static STATE: RefCell<PluginState> = RefCell::new(PluginState::default());
}

const PLUGIN_NAME: &[u8] = b"calibration-fir";

// ── Exported ABI ──────────────────────────────────────────────────────────

/// Initialise plugin once per audio stream.
#[no_mangle]
pub extern "C" fn plugin_init(channels: i32, sample_rate: i32, _block_size: i32) -> i32 {
    if !(1..=2).contains(&channels) || sample_rate <= 0 {
        return -1;
    }
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.channels = channels as u16;
        s.sample_rate = sample_rate as u32;
        s.known_version = 0; // force refresh next process frame
        s.left = None;
        s.right = None;
    });
    0
}

/// Reset all delay lines (called after seek / device swap).
#[no_mangle]
pub extern "C" fn plugin_reset() {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if let Some(l) = s.left.as_mut() {
            l.reset();
        }
        if let Some(r) = s.right.as_mut() {
            r.reset();
        }
    });
}

/// Standard Phonon frame-processing entry. `ptr/len` is an interleaved
/// f32 buffer, frame count = len / channels. Processes in place.
///
/// Returns 0 on success, negative on error (-1 = config incompatible,
/// -2 = memory allocation failed, -3 = host returned empty config).
#[no_mangle]
pub extern "C" fn plugin_process_frame(ptr: i32, len: i32) -> i32 {
    if ptr <= 0 || len <= 0 {
        return -1;
    }
    let result: Result<i32, ProcessError> = STATE.with(|s| {
        let mut state = s.borrow_mut();
        // ── 1. Poll version, refresh on change ────────────────────────
        let new_ver = unsafe {
            get_plugin_config_version(PLUGIN_NAME.as_ptr() as i32, PLUGIN_NAME.len() as i32)
        };
        if new_ver != state.known_version {
            let cfg = pull_config()?;
            if cfg.bypass {
                // Bypass mode: clear channels so process() passes audio through.
                state.left = None;
                state.right = None;
                state.known_version = new_ver;
            } else {
                if cfg.sample_rate != state.sample_rate {
                    return Err(FitError::InvalidConfig(format!(
                        "FIR config sample_rate {} != plugin init rate {}",
                        cfg.sample_rate, state.sample_rate
                    )).into());
                }
                if cfg.fir_L.len() != cfg.taps || cfg.fir_R.len() != cfg.taps {
                    return Err(FitError::InvalidConfig(
                        "FIR taps mismatch payload".into(),
                    ).into());
                }
                state.left = Some(FirChannel::new(cfg.fir_L));
                if state.channels >= 2 {
                    state.right = Some(FirChannel::new(cfg.fir_R));
                } else {
                    state.right = None;
                }
                state.known_version = new_ver;
            }
        }

        // ── 2. Validate frame layout ──────────────────────────────────
        let channels = state.channels as usize;
        let samples = len as usize;
        if !samples.is_multiple_of(channels) {
            return Err(ProcessError::Layout);
        }
        let frames = samples / channels;
        if frames == 0 {
            return Ok(0);
        }
        // Borrow linear memory (guaranteed valid by host runtime).
        let slice: &mut [f32] = unsafe {
            core::slice::from_raw_parts_mut(ptr as *mut f32, samples)
        };

        // ── 3. Deinterleave into left/right scratch, convolve, reinterleave ──
        state.buf.resize(samples, 0.0);
        // Take channels out of state to avoid double-mutable-borrow with buf.
        let left = state.left.take();
        let right = state.right.take();
        let (buf_l, rest) = state.buf.split_at_mut(frames);
        let buf_r = &mut rest[..frames];
        match channels {
            1 => {
                buf_l[..frames].copy_from_slice(&slice[..frames]);
                let mut left = left.ok_or(ProcessError::NoConfig)?;
                left.process(buf_l, &mut slice[..frames]);
                state.left = Some(left);
            }
            2 => {
                for (i, pair) in slice.chunks_exact(2).take(frames).enumerate() {
                    buf_l[i] = pair[0];
                    buf_r[i] = pair[1];
                }
                let mut left = left.ok_or(ProcessError::NoConfig)?;
                let mut right = right.ok_or(ProcessError::NoConfig)?;
                // use an output buffer for each (overwrite slice only
                // after both pass — avoids read-while-writing aliasing).
                let mut out_l = alloc::vec![0.0f32; frames];
                let mut out_r = alloc::vec![0.0f32; frames];
                left.process(buf_l, &mut out_l);
                right.process(buf_r, &mut out_r);
                for (i, pair) in slice.chunks_exact_mut(2).take(frames).enumerate() {
                    pair[0] = out_l[i];
                    pair[1] = out_r[i];
                }
                state.left = Some(left);
                state.right = Some(right);
            }
            _ => return Err(ProcessError::Layout),
        }
        Ok(0)
    });
    match result {
        Ok(code) => code,
        Err(e) => e as i32,
    }
}

#[repr(i32)]
#[derive(Debug)]
enum ProcessError {
    Layout = -1,
    NoConfig = -3,
    Host = -4,
    Decode = -5,
    Oom = -6,
}

impl From<FitError> for ProcessError {
    fn from(_: FitError) -> Self {
        ProcessError::Host
    }
}
impl From<serde_json::Error> for ProcessError {
    fn from(_: serde_json::Error) -> Self {
        ProcessError::Decode
    }
}

// ── Pull-decode helpers ───────────────────────────────────────────────────

fn pull_config() -> Result<FirConfig, ProcessError> {
    // Phase 1: ask how many bytes, allocate buffer. We do a 2-call dance:
    // first with empty dst → returns N we need; realloc + second call.
    // Many simpler implementations can just start with dst_len=4096 and
    // retry on overflow; we use the 2-call pattern since the host ABI
    // supports it cleanly.
    let n = unsafe {
        read_plugin_config(
            PLUGIN_NAME.as_ptr() as i32,
            PLUGIN_NAME.len() as i32,
            0, // nullptr → size query
            0,
        )
    };
    if n <= 0 {
        return Err(ProcessError::NoConfig);
    }
    let mut buf: Vec<u8> = vec![0u8; n as usize];
    let copied = unsafe {
        read_plugin_config(
            PLUGIN_NAME.as_ptr() as i32,
            PLUGIN_NAME.len() as i32,
            buf.as_mut_ptr() as i32,
            buf.len() as i32,
        )
    };
    if copied != n {
        return Err(ProcessError::Host);
    }
    let json = String::from_utf8(buf).map_err(|_| ProcessError::Decode)?;
    let cfg: FirConfig = serde_json::from_str(&json)?;
    Ok(cfg)
}

// ── Host-friendly debug logging (no-ops in no_std builds without wasi) ────
#[allow(dead_code)]
fn _dbg<S: ToString>(_msg: S) {
    #[cfg(feature = "std")]
    {
        log::debug!("calibration-fir: {}", _msg.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fir_channel_passthrough_with_zero_taps() {
        let mut c = FirChannel::new(vec![]);
        let inp = [1.0f32, 2.0, 3.0];
        let mut out = [0.0f32; 3];
        c.process(&inp, &mut out);
        assert_eq!(out, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn fir_channel_identity_tap_is_passthrough() {
        let mut c = FirChannel::new(vec![1.0f32]);
        let inp = [1.0f32, 2.0, 3.0, -1.0];
        let mut out = [0.0f32; 4];
        c.process(&inp, &mut out);
        assert_eq!(out, [1.0, 2.0, 3.0, -1.0]);
    }

    #[test]
    fn fir_channel_two_tap_average() {
        // h = [0.5, 0.5] → output[i] = 0.5*(x[i] + x[i-1])
        let mut c = FirChannel::new(vec![0.5, 0.5]);
        let inp = [0.0f32, 2.0, 4.0, 6.0];
        let mut out = [0.0f32; 4];
        c.process(&inp, &mut out);
        // out[0] = 0.5*(0 + 0) = 0  (history was zero)
        // out[1] = 0.5*(2 + 0) = 1
        // out[2] = 0.5*(4 + 2) = 3
        // out[3] = 0.5*(6 + 4) = 5
        assert!(
            (out[0] - 0.0).abs() < 1e-5,
            "out[0] = {}", out[0]
        );
        assert!((out[1] - 1.0).abs() < 1e-5, "out[1] = {}", out[1]);
        assert!((out[2] - 3.0).abs() < 1e-5, "out[2] = {}", out[2]);
        assert!((out[3] - 5.0).abs() < 1e-5, "out[3] = {}", out[3]);
    }
}
