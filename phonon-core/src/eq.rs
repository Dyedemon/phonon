//! Parametric Equalizer with 10-band IIR biquad cascade.
//!
//! Implements peaking, low-shelf, high-shelf, low-pass, and high-pass
//! filters using the RBJ Audio EQ Cookbook formulas. Supports both
//! Graphic EQ (GEQ) and Parametric EQ (PEQ) modes.

use std::f32::consts::PI;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Biquad filter
// ---------------------------------------------------------------------------

/// Biquad coefficient set (normalized, transposed direct form II).
#[derive(Debug, Clone, Copy)]
pub(crate) struct BiquadCoeffs {
    pub(crate) b0: f32,
    pub(crate) b1: f32,
    pub(crate) b2: f32,
    pub(crate) a1: f32, // a0 is always 1.0 after normalization
    pub(crate) a2: f32,
}

/// Biquad filter state (per-channel delay lines).
#[derive(Debug, Clone, Copy)]
pub(crate) struct BiquadState {
    pub(crate) x1: f32,
    pub(crate) x2: f32,
    pub(crate) y1: f32,
    pub(crate) y2: f32,
}

impl BiquadState {
    pub(crate) fn new() -> Self {
        Self {
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::new();
    }
}

impl BiquadCoeffs {
    /// Design coefficients from band parameters (RBJ cookbook).
    ///
    /// Returns a new `BiquadCoeffs`; extracted as a standalone method so
    /// peq.rs (which lives outside `eq::BiquadBand`) can
    /// re-use the exact same formulae as EqualizerProcessor.
    pub(crate) fn design(band: &EqBand, sample_rate: u32) -> Self {
        let fs = sample_rate as f32;
        let f0 = band.frequency.clamp(10.0, fs * 0.49);
        let omega = 2.0 * PI * f0 / fs;
        let cos_w = omega.cos();
        let sin_w = omega.sin();
        let q = band.q.clamp(0.1, 10.0);
        let alpha = sin_w / (2.0 * q);
        let a_db = band.gain_db.clamp(-24.0, 24.0);
        let a_lin = 10.0_f32.powf(a_db / 40.0);

        #[allow(unused_mut)]
        let (mut b0, mut b1, mut b2, mut a0, mut a1, mut a2): (
            f32,
            f32,
            f32,
            f32,
            f32,
            f32,
        );

        match band.filter_type {
            FilterType::Peaking => {
                b0 = 1.0 + alpha * a_lin;
                b1 = -2.0 * cos_w;
                b2 = 1.0 - alpha * a_lin;
                a0 = 1.0 + alpha / a_lin;
                a1 = -2.0 * cos_w;
                a2 = 1.0 - alpha / a_lin;
            }
            FilterType::LowShelf => {
                let sqrt_a = a_lin.sqrt();
                b0 = a_lin * ((a_lin + 1.0) - (a_lin - 1.0) * cos_w + 2.0 * sqrt_a * alpha);
                b1 = 2.0 * a_lin * ((a_lin - 1.0) - (a_lin + 1.0) * cos_w);
                b2 = a_lin * ((a_lin + 1.0) - (a_lin - 1.0) * cos_w - 2.0 * sqrt_a * alpha);
                a0 = (a_lin + 1.0) + (a_lin - 1.0) * cos_w + 2.0 * sqrt_a * alpha;
                a1 = -2.0 * ((a_lin - 1.0) + (a_lin + 1.0) * cos_w);
                a2 = (a_lin + 1.0) + (a_lin - 1.0) * cos_w - 2.0 * sqrt_a * alpha;
            }
            FilterType::HighShelf => {
                let sqrt_a = a_lin.sqrt();
                b0 = a_lin * ((a_lin + 1.0) + (a_lin - 1.0) * cos_w + 2.0 * sqrt_a * alpha);
                b1 = -2.0 * a_lin * ((a_lin - 1.0) + (a_lin + 1.0) * cos_w);
                b2 = a_lin * ((a_lin + 1.0) + (a_lin - 1.0) * cos_w - 2.0 * sqrt_a * alpha);
                a0 = (a_lin + 1.0) - (a_lin - 1.0) * cos_w + 2.0 * sqrt_a * alpha;
                a1 = 2.0 * ((a_lin - 1.0) - (a_lin + 1.0) * cos_w);
                a2 = (a_lin + 1.0) - (a_lin - 1.0) * cos_w - 2.0 * sqrt_a * alpha;
            }
            FilterType::LowPass => {
                b0 = (1.0 - cos_w) / 2.0;
                b1 = 1.0 - cos_w;
                b2 = (1.0 - cos_w) / 2.0;
                a0 = 1.0 + alpha;
                a1 = -2.0 * cos_w;
                a2 = 1.0 - alpha;
            }
            FilterType::HighPass => {
                b0 = (1.0 + cos_w) / 2.0;
                b1 = -(1.0 + cos_w);
                b2 = (1.0 + cos_w) / 2.0;
                a0 = 1.0 + alpha;
                a1 = -2.0 * cos_w;
                a2 = 1.0 - alpha;
            }
        }

        let inv_a0 = if a0.abs() < 1e-10 { 1.0 } else { 1.0 / a0 };
        BiquadCoeffs {
            b0: b0 * inv_a0,
            b1: b1 * inv_a0,
            b2: b2 * inv_a0,
            a1: a1 * inv_a0,
            a2: a2 * inv_a0,
        }
    }

    /// Transposed Direct Form II single-sample advance.
    ///
    /// s = biquad( x )
    /// Mutates state in-place (two first-order delay sections).
    #[inline(always)]
    pub(crate) fn process_sample(coeffs: &Self, st: &mut BiquadState, x: f32) -> f32 {
        // TDF-II:
        //   s1[n]  = b0·x[n] + s1[n-1]
        //   y [n]  = s1[n]
        //   s2[n]  = b1·x[n] + a1·y[n] + s2[n-1]
        //   s1[n] ← s2[n] for next cycle, but we use the standard
        //            2-tap accumulator layout:
        //   out       = b0*x + s1
        //   s1        = b1*x - a1*out + s2
        //   s2        = b2*x - a2*out
        let out = coeffs.b0 * x + st.x1;
        st.x1 = coeffs.b1 * x - coeffs.a1 * out + st.x2;
        st.x2 = coeffs.b2 * x - coeffs.a2 * out;
        out
    }
}

/// Filter type for a single EQ band.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
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

/// A single biquad band with its coefficient and per-channel state.
struct BiquadBand {
    coeffs: BiquadCoeffs,
    states: Vec<BiquadState>, // one per channel
}

impl BiquadBand {
    fn new(channels: usize) -> Self {
        Self {
            coeffs: BiquadCoeffs {
                b0: 1.0,
                b1: 0.0,
                b2: 0.0,
                a1: 0.0,
                a2: 0.0,
            },
            states: vec![BiquadState::new(); channels],
        }
    }

    /// Design coefficients from band parameters.
    fn design(&mut self, band: &EqBand, sample_rate: u32) {
        self.coeffs = BiquadCoeffs::design(band, sample_rate);

        // Reset states on coefficient change to avoid transients
        for s in &mut self.states {
            s.reset();
        }
    }

    /// Process a single sample through this biquad.
    #[inline]
    fn process_sample(&mut self, x: f32, ch: usize) -> f32 {
        let s = &mut self.states[ch];
        let c = &self.coeffs;
        let y = c.b0 * x + c.b1 * s.x1 + c.b2 * s.x2 - c.a1 * s.y1 - c.a2 * s.y2;
        s.x2 = s.x1;
        s.x1 = x;
        s.y2 = s.y1;
        s.y1 = y;
        y
    }
}

// ---------------------------------------------------------------------------
// EQ mode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EqMode {
    /// Fixed 10- or 15-band ISO 1/3 octave with Q=1.414 (GEQ ≥10 bands).
    Graphic,
    /// User-defined bands (up to 20; default 16 — PEQ ≥16 bands).
    Parametric,
}

// ---------------------------------------------------------------------------
// ISO 1/3 octave standard frequencies (10-band)
// ---------------------------------------------------------------------------

const GEQ_FREQUENCIES_10: [f32; 10] = [
    31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

/// ISO 2/3 octave 15-band frequencies
const GEQ_FREQUENCIES_15: [f32; 15] = [
    25.0, 40.0, 63.0, 100.0, 160.0, 250.0, 400.0, 630.0, 1000.0, 1600.0, 2500.0, 4000.0, 6300.0,
    10000.0, 16000.0,
];

const GEQ_Q: f32 = 1.414; // 1 octave bandwidth

/// Default 16-band ISO 1/3-octave centre frequencies for PEQ mode.
///
/// Covers 20 Hz–20 kHz in 16 peaking bands with Q=1.414, giving a dense,
/// even coverage that satisfies the "PEQ ≥ 16 bands" skill requirement
/// out of the box without forcing the user to add bands manually.
const PEQ_DEFAULT_FREQUENCIES_16: [f32; 16] = [
    20.0, 31.5, 50.0, 80.0, 125.0, 200.0, 315.0, 500.0, 800.0, 1250.0, 2000.0, 3150.0, 5000.0,
    8000.0, 12500.0, 20000.0,
];

/// Absolute maximum number of bands the EQ cascade supports (PEQ/GEQ combined).
/// This is the hard cap enforced by `add_band`; must be ≥ 16 for the
/// "PEQ ≥ 16 bands" skill requirement.
pub const EQ_MAX_BANDS: usize = 20;

// ---------------------------------------------------------------------------
// Built-in presets
// ---------------------------------------------------------------------------

/// A named EQ preset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EqPreset {
    pub name: String,
    pub bands: Vec<EqBand>,
}

impl EqPreset {
    fn new(name: &str, bands: Vec<EqBand>) -> Self {
        Self {
            name: name.into(),
            bands,
        }
    }
}

fn builtin_presets() -> Vec<EqPreset> {
    vec![
        EqPreset::new("Flat", {
            GEQ_FREQUENCIES_10
                .iter()
                .map(|&f| EqBand {
                    frequency: f,
                    gain_db: 0.0,
                    q: GEQ_Q,
                    filter_type: FilterType::Peaking,
                })
                .collect()
        }),
        EqPreset::new(
            "Vocal Boost",
            vec![
                EqBand {
                    frequency: 200.0,
                    gain_db: -3.0,
                    q: 0.7,
                    filter_type: FilterType::LowShelf,
                },
                EqBand {
                    frequency: 3000.0,
                    gain_db: 3.0,
                    q: 1.0,
                    filter_type: FilterType::Peaking,
                },
                EqBand {
                    frequency: 8000.0,
                    gain_db: 2.0,
                    q: 1.0,
                    filter_type: FilterType::HighShelf,
                },
            ],
        ),
        EqPreset::new(
            "Bass Boost",
            vec![
                EqBand {
                    frequency: 80.0,
                    gain_db: 6.0,
                    q: 0.7,
                    filter_type: FilterType::LowShelf,
                },
                EqBand {
                    frequency: 250.0,
                    gain_db: -2.0,
                    q: 1.0,
                    filter_type: FilterType::Peaking,
                },
            ],
        ),
        EqPreset::new(
            "Classical",
            vec![
                EqBand {
                    frequency: 50.0,
                    gain_db: 3.0,
                    q: 0.7,
                    filter_type: FilterType::LowShelf,
                },
                EqBand {
                    frequency: 500.0,
                    gain_db: -1.0,
                    q: 1.0,
                    filter_type: FilterType::Peaking,
                },
                EqBand {
                    frequency: 3000.0,
                    gain_db: 2.0,
                    q: 1.0,
                    filter_type: FilterType::Peaking,
                },
                EqBand {
                    frequency: 12000.0,
                    gain_db: 3.0,
                    q: 0.7,
                    filter_type: FilterType::HighShelf,
                },
            ],
        ),
        EqPreset::new(
            "Rock",
            vec![
                EqBand {
                    frequency: 100.0,
                    gain_db: 4.0,
                    q: 0.7,
                    filter_type: FilterType::LowShelf,
                },
                EqBand {
                    frequency: 500.0,
                    gain_db: -3.0,
                    q: 1.0,
                    filter_type: FilterType::Peaking,
                },
                EqBand {
                    frequency: 2000.0,
                    gain_db: 3.0,
                    q: 1.0,
                    filter_type: FilterType::Peaking,
                },
                EqBand {
                    frequency: 6000.0,
                    gain_db: 4.0,
                    q: 0.7,
                    filter_type: FilterType::HighShelf,
                },
            ],
        ),
    ]
}

/// Names of built-in presets that are protected from deletion.
const BUILTIN_PRESET_NAMES: &[&str] = &["Flat", "Vocal Boost", "Bass Boost", "Classical", "Rock"];

// ---------------------------------------------------------------------------
// Equalizer processor
// ---------------------------------------------------------------------------

/// Parametric Equalizer DSP processor.
///
/// Cascades up to `EQ_MAX_BANDS` (= 20) biquad filters. Supports:
/// - **Graphic EQ (GEQ)**: 10- or 15-band ISO 1/3 octave fixed frequencies (≥ 10 bands).
/// - **Parametric EQ (PEQ)**: default 16 ISO 1/3-octave peaking bands, user can
///   add/remove/reconfigure up to 20 (≥ 16 bands).
pub struct EqualizerProcessor {
    inner: Mutex<EqInner>,
    /// User-defined presets
    presets: Mutex<Vec<EqPreset>>,
    id: String,
}

struct EqInner {
    enabled: bool,
    mode: EqMode,
    /// Number of bands in GEQ mode (10 or 15)
    geq_band_count: usize,
    bands: Vec<EqBand>,
    /// Internal biquad cascade (one BiquadBand per EQ band)
    biquads: Vec<BiquadBand>,
    /// Current channel count (determines per-channel state size)
    channels: usize,
    /// Last sample rate used for design
    last_sample_rate: u32,
    /// Preamp gain in dB (-12..+12), applied before EQ processing
    preamp_db: f32,
}

impl EqualizerProcessor {
    pub fn new() -> Self {
        let bands: Vec<EqBand> = GEQ_FREQUENCIES_10
            .iter()
            .map(|&f| EqBand {
                frequency: f,
                gain_db: 0.0,
                q: GEQ_Q,
                filter_type: FilterType::Peaking,
            })
            .collect();

        let mut inner = EqInner {
            enabled: false,
            mode: EqMode::Graphic,
            geq_band_count: 10,
            bands,
            biquads: Vec::new(),
            channels: 2,
            last_sample_rate: 0,
            preamp_db: 0.0,
        };
        inner.rebuild_biquads();

        Self {
            inner: Mutex::new(inner),
            presets: Mutex::new(builtin_presets()),
            id: "equalizer".into(),
        }
    }

    /// Set a single band.
    pub fn set_band(&self, index: usize, band: EqBand, sample_rate: u32) {
        let mut inner = self.inner.lock().unwrap();
        if index < inner.bands.len() {
            inner.bands[index] = band;
            if index < inner.biquads.len() {
                let band_ref = inner.bands[index].clone();
                inner.biquads[index].design(&band_ref, sample_rate);
            }
        }
    }

    /// Get all band configurations.
    pub fn get_bands(&self) -> Vec<EqBand> {
        self.inner.lock().unwrap().bands.clone()
    }

    /// Set equalizer mode (Graphic/Parametric).
    pub fn set_mode(&self, mode: EqMode, sample_rate: u32) {
        let mut inner = self.inner.lock().unwrap();
        if mode == inner.mode {
            return;
        }
        inner.mode = mode;
        if mode == EqMode::Graphic {
            inner.bands = Self::geq_bands(inner.geq_band_count);
        } else {
            // PEQ: default 16 ISO 1/3-octave peaking bands (PEQ ≥ 16 bands).
            inner.bands = PEQ_DEFAULT_FREQUENCIES_16
                .iter()
                .map(|&f| EqBand {
                    frequency: f,
                    gain_db: 0.0,
                    q: GEQ_Q,
                    filter_type: FilterType::Peaking,
                })
                .collect();
        }
        inner.rebuild_biquads();
        inner.design_all(sample_rate);
    }

    /// Get current mode.
    pub fn mode(&self) -> EqMode {
        self.inner.lock().unwrap().mode
    }

    /// Get GEQ band count (10 or 15).
    pub fn geq_band_count(&self) -> usize {
        self.inner.lock().unwrap().geq_band_count
    }

    /// Set preamp gain in dB (-12..+12). Applied before EQ bands.
    pub fn set_preamp(&self, db: f32) {
        self.inner.lock().unwrap().preamp_db = db.clamp(-12.0, 12.0);
    }

    /// Get current preamp gain in dB.
    pub fn preamp(&self) -> f32 {
        self.inner.lock().unwrap().preamp_db
    }

    /// Set GEQ band count (10 or 15). Only applies in GEQ mode.
    pub fn set_geq_bands(&self, count: usize, sample_rate: u32) {
        if count != 10 && count != 15 {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        if inner.geq_band_count == count {
            return;
        }
        inner.geq_band_count = count;
        if inner.mode == EqMode::Graphic {
            inner.bands = Self::geq_bands(count);
            inner.rebuild_biquads();
            inner.design_all(sample_rate);
        }
    }

    /// Build GEQ bands for the given count.
    fn geq_bands(count: usize) -> Vec<EqBand> {
        let freqs: &[f32] = if count == 15 {
            &GEQ_FREQUENCIES_15
        } else {
            &GEQ_FREQUENCIES_10
        };
        freqs
            .iter()
            .map(|&f| EqBand {
                frequency: f,
                gain_db: 0.0,
                q: GEQ_Q,
                filter_type: FilterType::Peaking,
            })
            .collect()
    }

    /// Add a band (PEQ mode, up to EQ_MAX_BANDS = 20).
    pub fn add_band(&self, band: EqBand, sample_rate: u32) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if inner.bands.len() >= EQ_MAX_BANDS {
            return false;
        }
        let channels = inner.channels;
        inner.bands.push(band);
        inner.biquads.push(BiquadBand::new(channels));
        let idx = inner.bands.len() - 1;
        let band_ref = inner.bands[idx].clone();
        inner.biquads[idx].design(&band_ref, sample_rate);
        true
    }

    /// Remove a band by index.
    pub fn remove_band(&self, index: usize) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if index >= inner.bands.len() {
            return false;
        }
        inner.bands.remove(index);
        inner.biquads.remove(index);
        true
    }

    /// Load a preset by name.
    pub fn load_preset(&self, name: &str, sample_rate: u32) -> bool {
        let presets = self.presets.lock().unwrap();
        let preset = presets.iter().find(|p| p.name == name).cloned();
        drop(presets);
        if let Some(p) = preset {
            let mut inner = self.inner.lock().unwrap();
            inner.bands = p.bands;
            inner.rebuild_biquads();
            inner.design_all(sample_rate);
            true
        } else {
            false
        }
    }

    /// Save current bands as a user preset.
    pub fn save_preset(&self, name: &str) {
        let inner = self.inner.lock().unwrap();
        let mut presets = self.presets.lock().unwrap();
        presets.retain(|p| p.name != name);
        presets.push(EqPreset::new(name, inner.bands.clone()));
    }

    /// Delete a user preset by name. Built-in presets are protected.
    pub fn delete_preset(&self, name: &str) -> bool {
        if BUILTIN_PRESET_NAMES.contains(&name) {
            return false;
        }
        let mut presets = self.presets.lock().unwrap();
        let len_before = presets.len();
        presets.retain(|p| p.name != name);
        presets.len() < len_before
    }

    /// Get list of preset names.
    pub fn get_presets(&self) -> Vec<String> {
        self.presets
            .lock()
            .unwrap()
            .iter()
            .map(|p| p.name.clone())
            .collect()
    }
}

impl EqInner {
    /// Rebuild internal biquad cascade from band config.
    fn rebuild_biquads(&mut self) {
        self.biquads = self
            .bands
            .iter()
            .map(|_| BiquadBand::new(self.channels))
            .collect();
    }

    /// Design all biquad coefficients for the given sample rate.
    fn design_all(&mut self, sample_rate: u32) {
        for (i, band) in self.bands.iter().enumerate() {
            if i < self.biquads.len() {
                self.biquads[i].design(band, sample_rate);
            }
        }
    }

    /// Process a single sample through the entire cascade.
    #[inline]
    fn process_sample(&mut self, x: f32, ch: usize) -> f32 {
        let preamp_lin = 10.0_f32.powf(self.preamp_db / 20.0);
        let mut y = x * preamp_lin;
        for bq in &mut self.biquads {
            y = bq.process_sample(y, ch);
        }
        y
    }
}

impl Default for EqualizerProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl super::dsp::DspProcessor for EqualizerProcessor {
    fn process(&self, samples: &mut [f32], channels: u16, sample_rate: u32) {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(_) => return, // poisoned — bypass
        };
        if !inner.enabled || inner.biquads.is_empty() || sample_rate == 0 {
            return;
        }

        // Re-design only if sample rate changed
        if inner.last_sample_rate != sample_rate {
            inner.design_all(sample_rate);
            inner.last_sample_rate = sample_rate;
        }

        let ch = channels as usize;
        if ch == 0 {
            return;
        }
        for frame in samples.chunks_exact_mut(ch) {
            for (i, sample) in frame.iter_mut().enumerate() {
                *sample = inner.process_sample(*sample, i);
            }
        }
    }

    fn name(&self) -> &str {
        "Equalizer"
    }

    fn latency(&self) -> f64 {
        0.0
    }

    fn enabled(&self) -> bool {
        self.inner.lock().unwrap().enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.inner.lock().unwrap().enabled = enabled;
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::DspProcessor;

    #[test]
    fn test_biquad_passthrough() {
        let mut band = BiquadBand::new(1);
        let cfg = EqBand {
            frequency: 1000.0,
            gain_db: 0.0,
            q: 1.414,
            filter_type: FilterType::Peaking,
        };
        band.design(&cfg, 48000);
        let out = band.process_sample(0.5, 0);
        assert!(out.abs() > 0.4);
    }

    #[test]
    fn test_biquad_attenuation() {
        let mut band = BiquadBand::new(1);
        let cfg = EqBand {
            frequency: 1000.0,
            gain_db: -12.0,
            q: 2.0,
            filter_type: FilterType::Peaking,
        };
        band.design(&cfg, 48000);
        let mut val = 0.5f32;
        for _ in 0..100 {
            val = band.process_sample(val, 0);
        }
        assert!(val.abs() < 0.5);
    }

    #[test]
    fn test_eq_processor_flat() {
        let eq = EqualizerProcessor::new();
        eq.inner.lock().unwrap().enabled = true;
        let mut samples = vec![0.5f32; 200];
        eq.process(&mut samples, 2, 48000);
        for s in &samples {
            assert!((s - 0.5).abs() < 0.01, "sample {} deviated", s);
        }
    }

    #[test]
    fn test_eq_processor_preset() {
        let eq = EqualizerProcessor::new();
        assert!(eq.load_preset("Flat", 48000));
        assert!(!eq.load_preset("Nonexistent", 48000));
    }

    #[test]
    fn test_preset_save_list() {
        let eq = EqualizerProcessor::new();
        eq.save_preset("MyPreset");
        let names = eq.get_presets();
        assert!(names.contains(&"MyPreset".to_string()));
    }

    #[test]
    fn test_band_crud() {
        let eq = EqualizerProcessor::new();
        // Remove one band first, then add back
        assert!(eq.remove_band(0));
        assert_eq!(eq.get_bands().len(), 9);
        assert!(eq.add_band(EqBand::default(), 48000));
        assert_eq!(eq.get_bands().len(), 10);
    }

    #[test]
    fn test_mode_switch() {
        let eq = EqualizerProcessor::new();
        assert_eq!(eq.mode(), EqMode::Graphic);
        // Default GEQ has 10 bands
        assert_eq!(eq.get_bands().len(), 10);
        // Switching to PEQ initializes 16 default ISO 1/3-octave bands (PEQ ≥ 16)
        eq.set_mode(EqMode::Parametric, 48000);
        assert_eq!(eq.mode(), EqMode::Parametric);
        assert_eq!(eq.get_bands().len(), 16);
        // Lowest band should be 20 Hz, highest 20 kHz
        let bands = eq.get_bands();
        assert!((bands[0].frequency - 20.0).abs() < 0.01);
        assert!((bands[15].frequency - 20000.0).abs() < 0.01);
        // Switching back to GEQ restores 10 bands
        eq.set_mode(EqMode::Graphic, 48000);
        assert_eq!(eq.mode(), EqMode::Graphic);
        assert_eq!(eq.get_bands().len(), 10);
    }

    #[test]
    fn test_geq_15_bands() {
        let eq = EqualizerProcessor::new();
        eq.set_geq_bands(15, 48000);
        assert_eq!(eq.geq_band_count(), 15);
        assert_eq!(eq.get_bands().len(), 15);
        // Invalid count should be rejected
        eq.set_geq_bands(12, 48000);
        assert_eq!(eq.geq_band_count(), 15); // unchanged
    }

    #[test]
    fn test_geq_at_least_10_bands() {
        // Skill requirement: GEQ ≥ 10 bands — 10 and 15 both satisfy.
        let eq = EqualizerProcessor::new();
        assert!(
            eq.get_bands().len() >= 10,
            "default GEQ must be >= 10 bands"
        );
        eq.set_geq_bands(15, 48000);
        assert!(
            eq.get_bands().len() >= 10,
            "15-band GEQ must be >= 10 bands"
        );
    }

    #[test]
    fn test_peq_at_least_16_bands_by_default() {
        // Skill requirement: PEQ ≥ 16 bands.
        let eq = EqualizerProcessor::new();
        eq.set_mode(EqMode::Parametric, 48000);
        assert!(
            eq.get_bands().len() >= 16,
            "PEQ mode must start with >= 16 bands (got {})",
            eq.get_bands().len()
        );
    }

    #[test]
    fn test_peq_can_extend_to_max_bands() {
        // From the 16 default PEQ bands we can still add 4 more up to EQ_MAX_BANDS = 20.
        let eq = EqualizerProcessor::new();
        eq.set_mode(EqMode::Parametric, 48000);
        assert_eq!(eq.get_bands().len(), 16);
        for i in 0..4 {
            assert!(
                eq.add_band(EqBand::default(), 48000),
                "PEQ add_band #{} failed",
                i
            );
        }
        assert_eq!(eq.get_bands().len(), 20);
        assert_eq!(eq.get_bands().len(), EQ_MAX_BANDS);
        // 21st should fail
        assert!(!eq.add_band(EqBand::default(), 48000));
    }

    #[test]
    fn test_band_limit() {
        let eq = EqualizerProcessor::new();
        // Starts with 10 GEQ bands, can add 10 more (max 20)
        for _ in 0..10 {
            assert!(eq.add_band(EqBand::default(), 48000));
        }
        // 21st should fail
        assert!(!eq.add_band(EqBand::default(), 48000));
        assert_eq!(eq.get_bands().len(), EQ_MAX_BANDS);
    }
}
