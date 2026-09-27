#![allow(non_snake_case, clippy::needless_borrow)]

//! Calibration Engine (native only, runs inside phonon-tauri commands).
//!
//! # Architectural rules
//!
//! * This crate is explicitly NOT imported by `calibration-fir`. The Wasm
//!   plugin only does FIR convolution — curve fitting happens in the host
//!   via Tauri commands, which keeps the `.wasm` file small (≈100 KiB vs.
//!   1-2 MiB if rustfft + fitting maths were linked in).
//! * `pub type` re-exports from `calibration-types` so callers only need
//!   this crate's error-free `run_fit`/`compute_residual_fir` API.
//! * All algorithms are original Rust implementations (see each module for
//!   references). Curves derived from public graphs are explicitly tagged
//!   with the digitisation method to avoid licensing ambiguity.
//!
//! ```text
//!   ┌──────────────────────┐   FitRequest  ┌────────────────────┐
//!   │ phonon-tauri (host)  │ ────────────► │                    │
//!   │   run_calibration_   │               │  calibration-      │
//!   │        fit()         │ ◄──────────── │  engine            │
//!   └──────────────────────┘   FitResult   │  (native only)    │
//!             │                           │                    │
//!             │ fir_L, fir_R, bands_L/R   │  - Harman curves   │
//!             ▼                           │  - Biquad fit      │
//!   ┌──────────────────────┐              │  - residual FIR    │
//!   │ apply_fit_result()   │◄─────────────┤  - measurement     │
//!   │  - PEQ: PeqUpdate    │              │    preprocessing   │
//!   │  - FIR: host.set_    │              └────────────────────┘
//!   │        plugin_config │
//!   └──────────────────────┘
//! ```

pub mod error;
pub mod curves;
pub mod harman;
pub mod preprocess;
pub mod fit_biquad;
pub mod fit_fir;
pub mod akima;
pub mod cache;

pub use calibration_types::{
    EqBand, FitCurves, FitError, FitRequest, FitResult, FilterType,
};
#[cfg(feature = "native")]
pub use calibration_types::{ApplyError, PeqUpdateError};
pub use cache::{FitResultCache, FitResultCacheError, new_arc_cache};
pub use error::EngineError;

use std::sync::Arc;
use uuid::Uuid;

/// Entry point. Runs the full pipeline:
///
/// 1. preprocess the measurement (zero-fill, smooth, range-check)
/// 2. look up or compute the target curve (default: Harman IE 2018 v2)
/// 3. weighted non-linear least-squares over Biquads → `bands_L/R`
/// 4. compute the residual (target − biquad response) and flatten it into a
///    minimum-phase FIR using Hilbert transform via `rustfft`
/// 5. assemble the 256-pt visual curves (measurement / target / fitted / residual)
/// 6. insert into the global `FitResultCache` LRU for later `apply_fit_result`
pub fn run_fit(
    request: &FitRequest,
    cache: Option<&Arc<FitResultCache>>,
) -> Result<FitResult, EngineError> {
    // ── 1. input validation ────────────────────────────────────────────
    if request.measurement.is_empty() {
        return Err(FitError::EmptyMeasurement.into());
    }
    if request.sample_rate < 8_000 {
        return Err(FitError::InvalidConfig(format!(
            "sample_rate {} < 8 kHz",
            request.sample_rate
        ))
        .into());
    }
    if !(64..=8192).contains(&request.fir_taps) || !request.fir_taps.is_power_of_two() {
        return Err(FitError::InvalidConfig(format!(
            "fir_taps {} must be power-of-two in [64, 8192]",
            request.fir_taps
        ))
        .into());
    }
    if !(12..=20).contains(&request.max_bands) {
        return Err(FitError::InvalidConfig(format!(
            "max_bands {} must be in [12, 20]",
            request.max_bands
        ))
        .into());
    }

    // ── 2. preprocess measurement: stereo merge + smooth ───────────────
    let meas = preprocess::prepare_measurement(request)?;
    let meas_L = &meas.left;
    let meas_R = &meas.right;
    if meas_L.usable_bands() == 0 {
        return Err(FitError::NoUsableBand.into());
    }

    // ── 3. target lookup ───────────────────────────────────────────────
    let target_L = curves::resolve_target(request)?;
    let target_R = target_L.clone(); // target is stereo-identical by spec

    // ── 4. biquad fit (weighted LS on the standard 256-pt log grid) ────
    let (bands_L, response_L) = fit_biquad::fit_biquads(
        &meas_L,
        &target_L,
        request.max_bands as usize,
        request.sample_rate,
    )?;
    let (bands_R, response_R) = fit_biquad::fit_biquads(
        &meas_R,
        &target_R,
        request.max_bands as usize,
        request.sample_rate,
    )?;

    // ── 5. residual → minimum-phase FIR via rustfft + Hilbert ──────────
    let fir_L = fit_fir::residual_to_fir(
        &target_L,
        &response_L,
        request.fir_taps as usize,
        request.sample_rate,
    )?;
    let fir_R = fit_fir::residual_to_fir(
        &target_R,
        &response_R,
        request.fir_taps as usize,
        request.sample_rate,
    )?;

    // ── 6. visual curves (256-pt log axis, matches frontend Canvas) ────
    let curves_L = build_curves(&meas_L.curve, &target_L, &response_L)?;
    let curves_R = build_curves(&meas_R.curve, &target_R, &response_R)?;

    // ── 7. error metrics ───────────────────────────────────────────────
    let mut residual_sum = 0.0f32;
    let mut peak = 0.0f32;
    for i in 0..256 {
        let d = curves_L.mag_db[i] - target_L.mag_db[i];
        residual_sum += d * d;
        peak = peak.max(d.abs());
    }
    let rms = (residual_sum / 256.0).sqrt();

    let result = FitResult {
        fit_id: Uuid::new_v4().to_string(),
        target_curve_id: request.target_curve_id.clone(),
        bands_L,
        bands_R,
        fir_L,
        fir_R,
        curves_L,
        curves_R,
        rms_error_db: rms,
        peak_error_db: peak,
        applied_at: None,
    };

    // ── 8. LRU insert for later `apply_fit_result(fit_id)` ─────────────
    if let Some(c) = cache {
        let _ = c.insert(result.clone()); // failure here is not fatal to fit itself
    }

    Ok(result)
}

fn build_curves(
    measurement: &FitCurves,
    target: &FitCurves,
    biquad_response: &FitCurves,
) -> Result<FitCurves, EngineError> {
    // Calibration UI convention: curves hold the *compensated* response, i.e.
    // measurement + biquad_response + FIR-limit. We keep FIR contribution for
    // a later task (Task 3.7 / Task 7.3 fine-tuning); for now visual =
    // measurement shifted by the biquad response (close enough to eyeball
    // convergence while the DSP chain is being wired).
    let mut out = FitCurves::standard_log_grid();
    for i in 0..256 {
        let raw = measurement.mag_db[i];
        // NaN measurements → use target instead of blowing up
        let safe = if raw.is_finite() { raw } else { target.mag_db[i] };
        out.mag_db[i] = safe + biquad_response.mag_db[i];
    }
    out.validate().map_err(FitError::InvalidConfig)?;
    Ok(out)
}
