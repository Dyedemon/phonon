//! Shared types for Phonon Acoustic Calibration plugin (Biquad PEQ + FIR residual).
//!
//! This crate is intentionally tiny — only `serde` + `thiserror` + `uuid`.
//! No `rustfft`, no `rand`.
//!
//! `EqBand` and `FilterType` are defined locally so the crate is usable in
//! both native and WASM builds. When the `native` feature is enabled,
//! conversion impls are provided to/from `phonon_core::eq` types.

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// EqBand / FilterType — shared EQ parameter structs
// ---------------------------------------------------------------------------

/// Filter type for an EQ band.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FilterType {
    #[default]
    Peaking,
    LowShelf,
    HighShelf,
    LowPass,
    HighPass,
}

/// Single EQ band configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EqBand {
    pub frequency: f32, // Hz
    pub gain_db: f32,   // -24..+24 dB
    pub q: f32,         // 0.1..10.0
    pub filter_type: FilterType,
}

impl Default for EqBand {
    fn default() -> Self {
        Self {
            frequency: 1000.0,
            gain_db: 0.0,
            q: 1.414,
            filter_type: FilterType::Peaking,
        }
    }
}

/// Native-only: convert between local EqBand and phonon_core::eq::EqBand
/// for type identity on the host side.
#[cfg(feature = "native")]
impl From<EqBand> for phonon_core::eq::EqBand {
    fn from(b: EqBand) -> Self {
        use phonon_core::eq::FilterType as CoreFt;
        Self {
            frequency: b.frequency,
            gain_db: b.gain_db,
            q: b.q,
            filter_type: match b.filter_type {
                FilterType::Peaking => CoreFt::Peaking,
                FilterType::LowShelf => CoreFt::LowShelf,
                FilterType::HighShelf => CoreFt::HighShelf,
                FilterType::LowPass => CoreFt::LowPass,
                FilterType::HighPass => CoreFt::HighPass,
            },
        }
    }
}

#[cfg(feature = "native")]
impl From<phonon_core::eq::EqBand> for EqBand {
    fn from(b: phonon_core::eq::EqBand) -> Self {
        use phonon_core::eq::FilterType as CoreFt;
        Self {
            frequency: b.frequency,
            gain_db: b.gain_db,
            q: b.q,
            filter_type: match b.filter_type {
                CoreFt::Peaking => FilterType::Peaking,
                CoreFt::LowShelf => FilterType::LowShelf,
                CoreFt::HighShelf => FilterType::HighShelf,
                CoreFt::LowPass => FilterType::LowPass,
                CoreFt::HighPass => FilterType::HighPass,
            },
        }
    }
}

/// Re-export `PeqUpdateError` from `phonon-core`.
/// Only available with the `native` feature (Wasm plugin does not apply PEQ).
#[cfg(feature = "native")]
pub use phonon_core::peq::PeqUpdateError;

// ---------------------------------------------------------------------------
// FitCurves — 256-point logarithmic grid shared between Rust and TS
// ---------------------------------------------------------------------------

/// 256-point (freq, dB) curves used for: target curve storage,
/// measurement resampled view, fitted response display, residual plots.
///
/// The X axis is a strictly logarithmic 256-point grid:
/// ```text
///   freq[i] = 20 * (20000/20)^(i/255)   for i ∈ 0..255
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FitCurves {
    /// Must be exactly 256 entries after `standard_log_grid` or deserialization
    /// that calls `validate()`.
    pub freq: Vec<f32>,
    /// Same length as `freq`. Magnitude in dB (post-fit response, target, ...).
    pub mag_db: Vec<f32>,
}

impl FitCurves {
    /// Build the standard 256pt logarithmic grid with uniform 0 dB magnitude.
    ///
    /// `freq[0] = 20.0 Hz`, `freq[255] = 20 000.0 Hz`.
    /// The middle point `i=128` lands at `sqrt(20 * 20000) ≈ 632.46 Hz`
    /// (geometric midpoint of the human audio band).
    pub fn standard_log_grid() -> Self {
        let n: usize = 256;
        let (flo, fhi): (f32, f32) = (20.0, 20_000.0);
        let ratio = fhi / flo;
        let mut freq = Vec::with_capacity(n);
        for i in 0..n {
            let t = i as f32 / ((n - 1) as f32);
            freq.push(flo * ratio.powf(t));
        }
        Self {
            freq,
            mag_db: vec![0.0; n],
        }
    }

    /// Validate shape invariants. Errors with a descriptive message string
    /// so callers can propagate into `FitError::InvalidConfig`.
    pub fn validate(&self) -> Result<(), String> {
        if self.freq.len() != 256 {
            return Err(format!("FitCurves freq.len = {}, want 256", self.freq.len()));
        }
        if self.mag_db.len() != 256 {
            return Err(format!("FitCurves mag_db.len = {}, want 256", self.mag_db.len()));
        }
        let expect = Self::standard_log_grid();
        for i in 0..256 {
            let a = self.freq[i];
            let b = expect.freq[i];
            // 1e-3 Hz tolerance — enough for f32 roundtrip through serde_json.
            if (a - b).abs() > 1e-3 {
                return Err(format!(
                    "FitCurves freq[{i}] = {a}, want {b} (strict 256-pt log axis)"
                ));
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// FitRequest / FitResult — run_calibration_fit + apply_fit_result payloads
// ---------------------------------------------------------------------------

/// Input to the calibration fitting engine.
///
/// `measurement` is a list of `(Hz, dB)` samples from a user-provided curve
/// file (REW `.frd`, CSV / TXT, AutoEq-style CSV). The engine itself performs
/// sorting, out-of-band clipping, and resampling to the 256-pt log grid;
/// callers do NOT need to pre-clean the measurement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitRequest {
    /// Raw measurement samples (Hz, dB). May be unordered and include
    /// points outside the 20 Hz–20 kHz passband.
    pub measurement: Vec<(f32, f32)>,
    /// Target curve id, e.g. `"harman_ie_2018_v2"`.
    pub target_curve_id: String,
    /// If `target_curve_id == "custom"` callers must supply the user-dragged
    /// curve here (validated as a proper 256-pt log FitCurves).
    pub custom_target: Option<FitCurves>,
    /// Maximum number of biquad bands to fit. Range 12..20. Default 16.
    pub max_bands: u8,
    /// FIR residual filter length in taps. Range 64..8192. Default 2048.
    pub fir_taps: u32,
    /// Running sample rate used to design biquad coefficients and
    /// normalize the FIR response.
    pub sample_rate: u32,
}

impl Default for FitRequest {
    fn default() -> Self {
        Self {
            measurement: Vec::new(),
            target_curve_id: "harman_ie_2018_v2".into(),
            custom_target: None,
            max_bands: 16,
            fir_taps: 2048,
            sample_rate: 48_000,
        }
    }
}

/// Output of the calibration fitting engine, and the payload that
/// `apply_fit_result` consumes to simultaneously update the PEQ and FIR
/// layers atomically (rollback-safe transaction semantics).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(non_snake_case)]
pub struct FitResult {
    /// Unique id of this result (UUID v4). Used as the LRU cache key and
    /// as the argument to `apply_fit_result`.
    pub fit_id: String,
    /// Echoes the `target_curve_id` used during fitting, so the UI can
    /// re-highlight it on re-open.
    pub target_curve_id: String,
    /// Biquad bands for left channel (length = `FitRequest.max_bands`).
    pub bands_L: Vec<EqBand>,
    /// Biquad bands for right channel. Equal to `bands_L` when `L/R Link=on`.
    pub bands_R: Vec<EqBand>,
    /// FIR residual convolution coefficients for left channel, length = `fir_taps`.
    pub fir_L: Vec<f32>,
    /// FIR residual convolution coefficients for right channel.
    pub fir_R: Vec<f32>,
    /// Fitted response on the standard 256-pt log grid (for UI Canvas).
    pub curves_L: FitCurves,
    /// Fitted response for right channel / linked copy.
    pub curves_R: FitCurves,
    /// Residual RMS error over 20 Hz–20 kHz after PEQ + FIR.
    pub rms_error_db: f32,
    /// Residual peak error over 20 Hz–20 kHz after PEQ + FIR.
    pub peak_error_db: f32,
    /// None for cached-but-unapplied results; `Some(t)` once `apply_fit_result`
    /// commits the PEQ+FIR transaction successfully.
    pub applied_at: Option<std::time::SystemTime>,
}

// ---------------------------------------------------------------------------
// Error types — all impl Serialize so Tauri commands can forward to TS toast
// ---------------------------------------------------------------------------

/// Errors produced by `calibration_engine::fit()`.
#[derive(Debug, Clone, Error, Serialize, Deserialize, PartialEq)]
pub enum FitError {
    #[error("measurement empty")]
    EmptyMeasurement,
    #[error("no samples fall inside the 10 Hz..24 kHz analysis band")]
    NoUsableBand,
    #[error("solver failed to converge after {0} iterations")]
    NoConverge(u32),
    #[error("unknown target curve: {0}")]
    UnknownTarget(String),
    #[error("band configuration invalid: {0}")]
    InvalidConfig(String),
}

/// Errors produced by `apply_fit_result` (the two-phase PEQ+FIR transaction).
#[derive(Debug, Clone, Error, Serialize, Deserialize, PartialEq)]
pub enum ApplyError {
    /// `fit_id` was not in the LRU cache and `last_applied_result` does not
    /// match it either — the result was evicted (LRU over 20 or older than
    /// 24 h) and must be regenerated by a fresh `run_calibration_fit` call.
    #[error("fit id not found in cache; re-run the fit")]
    FitNotFound,
    /// PEQ apply failed before any side effect — neither PEQ nor FIR changed.
    /// Only present in native builds (Wasm plugin doesn't apply PEQ).
    #[cfg(feature = "native")]
    #[error("peq apply failed: {0}")]
    Peq(#[from] PeqUpdateError),
    /// PEQ applied successfully but FIR config write failed (panic or
    /// deserialisation error on the plugin side). The PEQ side was already
    /// rolled back to the pre-call snapshot, but `peq_applied` is reported
    /// for diagnostics in the UI toast.
    #[error("fir apply failed: peq {peq_applied:?}, detail: {detail}")]
    FirRolledBack {
        /// True if the PEQ side committed before the failure (it will have
        /// been rolled back by the time the caller sees this error, but the
        /// information lets the UI differentiate rollback cases).
        peq_applied: bool,
        /// Human-readable detail produced by `set_plugin_config` / Wasm decode.
        detail: String,
    },
}
