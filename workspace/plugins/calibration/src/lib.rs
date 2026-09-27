#![allow(non_snake_case, dead_code)]
//! Calibration WASM plugin — full acoustic calibration suite.
//!
//! # Capabilities
//!
//! 1. **DSP (realtime)**: FIR residual convolution via overlap-save.
//!    Pulls FIR tap configuration from the host via version-polled config.
//!
//! 2. **Commands (non-realtime)**: Biquad + FIR fit computation via
//!    `plugin_command("run_fit", json_request)`. Returns `FitResult` as JSON.
//!
//! # Host ABI
//!
//! Standard Phonon WASM plugin ABI:
//! - `plugin_init(channels, sample_rate, block_size) -> i32`
//! - `plugin_reset()`
//! - `plugin_process_frame(ptr, len) -> i32`
//! - `plugin_command(cmd_ptr, cmd_len, input_ptr, input_len) -> i64`
//!
//! Host imports:
//! - `host.get_plugin_config_version(name_ptr, name_len) -> i64`
//! - `host.read_plugin_config(name_ptr, name_len, dst_ptr, dst_len) -> i32`
//! - `host.emit_event(name_ptr, name_len, payload_ptr, payload_len)`

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
    fn get_plugin_config_version(plugin_name_ptr: i32, plugin_name_len: i32) -> i64;
    fn read_plugin_config(
        plugin_name_ptr: i32,
        plugin_name_len: i32,
        dst_ptr: i32,
        dst_len: i32,
    ) -> i32;
}

#[cfg(not(target_arch = "wasm32"))]
unsafe fn get_plugin_config_version(_p: i32, _l: i32) -> i64 {
    0
}
#[cfg(not(target_arch = "wasm32"))]
unsafe fn read_plugin_config(_p: i32, _pl: i32, _d: i32, _dl: i32) -> i32 {
    0
}

// ── FIR Config ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Default)]
pub struct FirConfig {
    pub sample_rate: u32,
    pub taps: usize,
    pub fir_L: Vec<f32>,
    pub fir_R: Vec<f32>,
    #[serde(default)]
    pub bypass: bool,
}

// ── Overlap-save FIR engine ───────────────────────────────────────────────

struct FirChannel {
    taps: Vec<f32>,
    history: Vec<f32>,
}

impl FirChannel {
    fn new(taps: Vec<f32>) -> Self {
        let hist = vec![0.0f32; taps.len().saturating_sub(1)];
        Self { taps, history: hist }
    }

    fn reset(&mut self) {
        for x in self.history.iter_mut() {
            *x = 0.0;
        }
    }

    fn process(&mut self, inp: &[f32], out: &mut [f32]) {
        let ntap = self.taps.len();
        if ntap == 0 {
            out.copy_from_slice(inp);
            return;
        }
        let hist_len = self.history.len();
        let mut buf = Vec::with_capacity(hist_len + inp.len());
        buf.extend_from_slice(&self.history);
        buf.extend_from_slice(inp);

        for (i, o) in out.iter_mut().enumerate() {
            let mut acc = 0.0f32;
            let base = i + hist_len;
            for t in 0..ntap {
                acc += buf[base + 1 + t - ntap] * self.taps[t];
            }
            *o = acc;
        }
        if hist_len > 0 && inp.len() >= hist_len {
            let tail = &inp[inp.len() - hist_len..];
            self.history.copy_from_slice(tail);
        } else if hist_len > 0 {
            let shift = inp.len().min(hist_len);
            self.history.copy_within(shift.., 0);
            self.history[hist_len - shift..].copy_from_slice(&inp[..shift]);
        }
    }
}

// ── Plugin state ──────────────────────────────────────────────────────────

#[derive(Default)]
struct PluginState {
    channels: u16,
    sample_rate: u32,
    known_version: i64,
    left: Option<FirChannel>,
    right: Option<FirChannel>,
    buf: Vec<f32>,
}

thread_local! {
    static STATE: RefCell<PluginState> = RefCell::new(PluginState::default());
}

const PLUGIN_NAME: &[u8] = b"calibration";

// ── DSP: init / reset / process ───────────────────────────────────────────

#[no_mangle]
pub extern "C" fn plugin_init(channels: i32, sample_rate: i32, _block_size: i32) -> i32 {
    if !(1..=2).contains(&channels) || sample_rate <= 0 {
        return -1;
    }
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.channels = channels as u16;
        s.sample_rate = sample_rate as u32;
        s.known_version = 0;
        s.left = None;
        s.right = None;
    });
    0
}

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

#[no_mangle]
pub extern "C" fn plugin_process(ptr: i32, len: i32, channels: i32, sample_rate: i32) {
    if ptr <= 0 || len <= 0 || channels <= 0 || sample_rate <= 0 {
        return;
    }
    let _ = process_impl(ptr, len, channels as u16, sample_rate as u32);
}

fn process_impl(ptr: i32, len: i32, channels: u16, sample_rate: u32) -> Result<(), ProcessError> {
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        // Sync state if needed (first call after init)
        if state.channels == 0 {
            state.channels = channels;
            state.sample_rate = sample_rate;
        }
        let new_ver = unsafe {
            get_plugin_config_version(PLUGIN_NAME.as_ptr() as i32, PLUGIN_NAME.len() as i32)
        };
        if new_ver != state.known_version {
            let cfg = pull_config()?;
            if cfg.bypass {
                state.left = None;
                state.right = None;
                state.known_version = new_ver;
            } else {
                if cfg.sample_rate != state.sample_rate {
                    return Err(ProcessError::Host);
                }
                if cfg.fir_L.len() != cfg.taps || cfg.fir_R.len() != cfg.taps {
                    return Err(ProcessError::Host);
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

        let ch = state.channels as usize;
        let samples = len as usize; // len is sample count (f32)
        if samples == 0 || !samples.is_multiple_of(ch) {
            return Err(ProcessError::Layout);
        }
        let frames = samples / ch;
        if frames == 0 {
            return Ok(());
        }
        let slice: &mut [f32] = unsafe {
            core::slice::from_raw_parts_mut(ptr as *mut f32, samples)
        };

        state.buf.resize(samples, 0.0);
        let left = state.left.take();
        let right = state.right.take();
        let (buf_l, rest) = state.buf.split_at_mut(frames);
        let buf_r = &mut rest[..frames];
        match ch {
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
        Ok(())
    })
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

fn pull_config() -> Result<FirConfig, ProcessError> {
    let n = unsafe {
        read_plugin_config(
            PLUGIN_NAME.as_ptr() as i32,
            PLUGIN_NAME.len() as i32,
            0,
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

// ── Commands: run_fit (non-realtime) ─────────────────────────────────────

use phonon_plugin_sdk::allocate_string;

/// Plugin command entry point.
///
/// Supported commands:
/// - `"run_fit"`: input = JSON FitRequest → output = JSON FitResult
/// - `"get_target"`: input = JSON {target_curve_id, custom_target?} → output = JSON FitCurves
///   (256-pt log grid), so the UI can draw the exact target the engine fits against
#[no_mangle]
pub extern "C" fn plugin_command(
    cmd_ptr: *const u8,
    cmd_len: i32,
    input_ptr: *const u8,
    input_len: i32,
) -> i64 {
    if cmd_ptr.is_null() || input_ptr.is_null() || cmd_len <= 0 || input_len < 0 {
        return 0;
    }
    let command = unsafe { phonon_plugin_sdk::read_string(cmd_ptr, cmd_len as usize) };
    let input_json = unsafe { phonon_plugin_sdk::read_string(input_ptr, input_len as usize) };

    let result = match command {
        "run_fit" => handle_run_fit(input_json),
        "get_target" => handle_get_target(input_json),
        _ => Err(format!("unknown command: {}", command)),
    };

    match result {
        Ok(output) => {
            let ptr = allocate_string(&output);
            let len = output.len() as u32;
            let ptr_u32 = ptr as u32;
            ((ptr_u32 as i64) << 32) | (len as i64)
        }
        Err(_) => 0,
    }
}

fn handle_run_fit(input_json: &str) -> Result<String, String> {
    use calibration_engine::run_fit;
    use calibration_types::FitRequest;

    let request: FitRequest = serde_json::from_str(input_json)
        .map_err(|e| format!("invalid FitRequest JSON: {}", e))?;

    let result = run_fit(&request, None)
        .map_err(|e| e.to_string())?;

    serde_json::to_string(&result)
        .map_err(|e| format!("serialization error: {}", e))
}

#[derive(serde::Deserialize)]
struct TargetRequest {
    #[serde(default)]
    target_curve_id: String,
    #[serde(default)]
    custom_target: Option<calibration_types::FitCurves>,
}

fn handle_get_target(input_json: &str) -> Result<String, String> {
    use calibration_engine::curves::resolve_target;
    use calibration_types::FitRequest;

    let req: TargetRequest = serde_json::from_str(input_json)
        .map_err(|e| format!("invalid TargetRequest JSON: {}", e))?;

    // resolve_target only reads target_curve_id / custom_target; the remaining
    // fields are filled with defaults so no measurement is needed here.
    let request = FitRequest {
        measurement: Vec::new(),
        target_curve_id: req.target_curve_id,
        custom_target: req.custom_target,
        ..FitRequest::default()
    };

    let curves = resolve_target(&request).map_err(|e| e.to_string())?;

    serde_json::to_string(&curves)
        .map_err(|e| format!("serialization error: {}", e))
}

// ── Plugin metadata ───────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn plugin_name() -> *mut u8 {
    allocate_string("Acoustic Calibration")
}

#[no_mangle]
pub extern "C" fn plugin_version() -> *mut u8 {
    allocate_string("1.0.0")
}

#[no_mangle]
pub extern "C" fn plugin_type() -> i32 {
    // DspProcessor = 1 (see phonon_plugin::types::PluginType)
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fir_channel_passthrough_zero_taps() {
        let mut c = FirChannel::new(vec![]);
        let inp = [1.0f32, 2.0, 3.0];
        let mut out = [0.0f32; 3];
        c.process(&inp, &mut out);
        assert_eq!(out, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn fir_channel_identity_tap() {
        let mut c = FirChannel::new(vec![1.0f32]);
        let inp = [1.0f32, 2.0, 3.0, -1.0];
        let mut out = [0.0f32; 4];
        c.process(&inp, &mut out);
        assert_eq!(out, [1.0, 2.0, 3.0, -1.0]);
    }
}
