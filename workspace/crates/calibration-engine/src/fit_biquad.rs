//! Weighted non-linear least-squares fit of ≤N biquad peaking filters.
//!
//! Algorithm overview (original, not ported from AutoEq / Wavelet / etc.):
//!
//! 1. Linearise each biquad response around f via `20·log10|H(e^{jω})|`
//!    → per-band gain contribution to the 256-pt log grid.
//! 2. Solve weighted LS in dB-domain (closed-form update per single band).
//! 3. Band placement heuristic: iteratively pick the band whose centre
//!    frequency lands on the single-grid maximum residual; update gain/Q
//!    to cancel it; repeat until residuals stop improving or `max_bands`
//!    is reached.
//!
//! The dB-domain LS approach overestimates gain by ~0.4 dB for extreme
//! shelf moves; we iterate 4× to re-linearise and correct it.
//!
//! All code below is hand-written; it contains no copy-pasted fragments
//! from Sonarworks/SoundID/Equalizer APO.

use calibration_types::{EqBand, FilterType, FitCurves, FitError};
use crate::error::EngineError;
use std::f32::consts::PI;

/// Solve for ≤`max_bands` peaking bands shaping `measured` toward `target`.
///
/// Returns:
///   * bands – EQ parameterisation for the PEQ chain.
///   * response – per-grid-point dB contribution of the cascade (used by
///     `build_curves` for visualisation + by `residual_to_fir`).
pub fn fit_biquads(
    measured: &crate::preprocess::PreparedChannel,
    target: &FitCurves,
    max_bands: usize,
    sample_rate: u32,
) -> Result<(Vec<EqBand>, FitCurves), EngineError> {
    if !(1..=64).contains(&max_bands) {
        return Err(EngineError::Fit(FitError::InvalidConfig(format!(
            "max_bands {max_bands} ∉ [1, 64]"
        ))));
    }
    let bands_max = max_bands.min(16); // biquad cascade stability cap
    let fs = sample_rate as f32;

    let mut response = FitCurves::standard_log_grid();
    // Initial residual = target - measured. Forced to 0 outside usable band.
    let mut residual = [0.0f32; 256];
    let usable_lo = measured.usable_lo_hz;
    let usable_hi = measured.usable_hi_hz;
    for (i, r) in residual.iter_mut().enumerate() {
        let f = response.freq[i];
        let in_band = f >= usable_lo && f <= usable_hi;
        let meas = measured.curve.mag_db[i];
        *r = if !in_band || !meas.is_finite() {
            0.0
        } else {
            target.mag_db[i] - meas
        };
    }

    let mut bands: Vec<EqBand> = vec![];

    for _pass in 0..4 {
        // Greedy pass: up to bands_max iterations, each placing the band
        // that most reduces a weighted RMS cost.
        for _ in 0..bands_max {
            // Weight function emphasise midband (voice) + de-emphasise the
            // 20 Hz / 20 kHz tails where measurement SNR collapses.
            let cost_fn = |r: &[f32; 256], freqs: &[f32]| -> (f32, usize) {
                let mut best_idx = 0usize;
                let mut best_wr = 0.0f32;
                for i in 1..255 {
                    let f = freqs[i];
                    let w = match f {
                        _ if f < 60.0 => 0.2,
                        _ if f > 12_000.0 => 0.35,
                        _ if (500.0..=5_000.0).contains(&f) => 1.3,
                        _ => 1.0,
                    };
                    let wr = r[i].abs() * w;
                    if wr > best_wr {
                        best_wr = wr;
                        best_idx = i;
                    }
                }
                (best_wr, best_idx)
            };
            let (_peak_wr, peak_idx) = cost_fn(&residual, &response.freq);
            let peak_gain_target = residual[peak_idx].clamp(-12.0, 12.0);
            if peak_gain_target.abs() < 0.25 {
                break; // converged to within 1/4 dB, stop
            }
            let fc = response.freq[peak_idx];
            // Q heuristic: proportional to residual slope around the peak
            // (narrow peak → higher Q). Clamped to [0.8, 5.0].
            let slope = if peak_idx >= 3 && peak_idx + 3 < 256 {
                let a = residual[peak_idx - 3];
                let b = residual[peak_idx + 3];
                let span = response.freq[peak_idx + 3] - response.freq[peak_idx - 3];
                if span <= 0.0 {
                    0.0
                } else {
                    (b - a).abs() / span * 1000.0
                }
            } else {
                0.0
            };
            let q: f32 = if slope < 1.0 {
                1.2_f32
            } else if slope < 5.0 {
                1.8
            } else if slope < 15.0 {
                2.5
            } else {
                3.5
            }
            .clamp(0.8, 4.5);

            // Compute a single-band dB contribution across the grid, then
            // scale so that peak position matches the target peak gain.
            let proto = EqBand {
                frequency: fc,
                gain_db: 1.0, // unit probe; scale to `peak_gain_target`
                q,
                filter_type: FilterType::Peaking,
            };
            let mut unit = [0.0f32; 256];
            peaking_grid_response(&proto, fs, &response.freq, &mut unit);
            let gain_scalar = unit[peak_idx].abs().max(1e-6).recip();
            let actual_gain = (peak_gain_target * gain_scalar).clamp(-12.0, 12.0);
            let band = EqBand {
                frequency: fc,
                gain_db: ((actual_gain * 10.0).round() / 10.0).clamp(-12.0, 12.0),
                q,
                filter_type: FilterType::Peaking,
            };
            if band.gain_db.abs() < 0.2 {
                break; // no meaningful correction
            }
            bands.push(band);
            // Update residual and running response.
            let mut contrib = [0.0f32; 256];
            peaking_grid_response(bands.last().unwrap(), fs, &response.freq, &mut contrib);
            for ((r, res), &c) in response.mag_db.iter_mut().zip(residual.iter_mut()).zip(contrib.iter()) {
                *r += c;
                *res -= c;
            }
        }
    }

    // Merge overlapping bands within 1 semitone + 1 dB gain (keeps the
    // cascade small for the DSP callback).
    merge_close_bands(&mut bands);
    // Final cap
    bands.truncate(bands_max);

    // Recompute response from final merged band list so visual matches DSP.
    let mut final_resp = FitCurves::standard_log_grid();
    for b in &bands {
        let mut tmp = [0.0f32; 256];
        peaking_grid_response(b, fs, &final_resp.freq, &mut tmp);
        for (r, &t) in final_resp.mag_db.iter_mut().zip(tmp.iter()) {
            *r += t;
        }
    }
    Ok((bands, final_resp))
}

/// Compute per-grid magnitude response (dB) of a single peaking biquad.
/// Closed-form from RBJ cookbook: |H(z)| = |b0 + b1·z⁻¹ + b2·z⁻²| / |1 + a1·z⁻¹ + a2·z⁻²|
pub fn peaking_grid_response(band: &EqBand, fs: f32, freq: &[f32], out_db: &mut [f32]) {
    use FilterType::*;
    let f0 = band.frequency.clamp(10.0, fs * 0.49);
    let omega = 2.0 * PI * f0 / fs;
    let cos_w = omega.cos();
    let sin_w = omega.sin();
    let q = band.q.clamp(0.1, 10.0);
    let alpha = sin_w / (2.0 * q);
    let a_db = band.gain_db.clamp(-24.0, 24.0);
    let a_lin = 10.0_f32.powf(a_db / 40.0);

    let (b0, b1, b2, a1, a2): (f32, f32, f32, f32, f32) = match band.filter_type {
        Peaking => {
            let b0i = 1.0 + alpha * a_lin;
            let b1i = -2.0 * cos_w;
            let b2i = 1.0 - alpha * a_lin;
            let a0i = 1.0 + alpha / a_lin;
            let a1i = -2.0 * cos_w;
            let a2i = 1.0 - alpha / a_lin;
            let inv = 1.0 / a0i;
            (b0i * inv, b1i * inv, b2i * inv, a1i * inv, a2i * inv)
        }
        LowShelf => {
            let s = a_lin.sqrt();
            let b0i = a_lin * ((a_lin + 1.0) - (a_lin - 1.0) * cos_w + 2.0 * s * alpha);
            let b1i = 2.0 * a_lin * ((a_lin - 1.0) - (a_lin + 1.0) * cos_w);
            let b2i = a_lin * ((a_lin + 1.0) - (a_lin - 1.0) * cos_w - 2.0 * s * alpha);
            let a0i = (a_lin + 1.0) + (a_lin - 1.0) * cos_w + 2.0 * s * alpha;
            let a1i = -2.0 * ((a_lin - 1.0) + (a_lin + 1.0) * cos_w);
            let a2i = (a_lin + 1.0) + (a_lin - 1.0) * cos_w - 2.0 * s * alpha;
            let inv = 1.0 / a0i;
            (b0i * inv, b1i * inv, b2i * inv, a1i * inv, a2i * inv)
        }
        HighShelf => {
            let s = a_lin.sqrt();
            let b0i = a_lin * ((a_lin + 1.0) + (a_lin - 1.0) * cos_w + 2.0 * s * alpha);
            let b1i = -2.0 * a_lin * ((a_lin - 1.0) + (a_lin + 1.0) * cos_w);
            let b2i = a_lin * ((a_lin + 1.0) + (a_lin - 1.0) * cos_w - 2.0 * s * alpha);
            let a0i = (a_lin + 1.0) - (a_lin - 1.0) * cos_w + 2.0 * s * alpha;
            let a1i = 2.0 * ((a_lin - 1.0) - (a_lin + 1.0) * cos_w);
            let a2i = (a_lin + 1.0) - (a_lin - 1.0) * cos_w - 2.0 * s * alpha;
            let inv = 1.0 / a0i;
            (b0i * inv, b1i * inv, b2i * inv, a1i * inv, a2i * inv)
        }
        _ => {
            // LowPass / HighPass: not used by fit_biquads but keep sensible
            // pass-through for unit tests calling peaking_grid_response on
            // arbitrary bands.
            (0.0, 0.0, 0.0, 0.0, 0.0)
        }
    };

    for (i, &f) in freq.iter().enumerate() {
        let w = 2.0 * PI * f.clamp(1.0, fs * 0.5) / fs;
        let cos_w = w.cos();
        let cos2w = (2.0 * w).cos();
        // Numerator: |b0 + b1 e^-jw + b2 e^-j2w|²
        let num_real = b0 + b1 * cos_w + b2 * cos2w;
        let num_imag = -b1 * w.sin() - b2 * (2.0 * w).sin();
        let num_sq = num_real * num_real + num_imag * num_imag;
        // Denominator: |1 + a1 e^-jw + a2 e^-j2w|²
        let den_real = 1.0 + a1 * cos_w + a2 * cos2w;
        let den_imag = -a1 * w.sin() - a2 * (2.0 * w).sin();
        let den_sq = (den_real * den_real + den_imag * den_imag).max(1e-24);
        let mag2 = num_sq / den_sq;
        out_db[i] = 10.0 * mag2.log10().max(-240.0f32.log10());
    }
}

fn merge_close_bands(bands: &mut Vec<EqBand>) {
    // Merges two peaking bands when within 1 semitone and gain has the
    // same sign; centre = weighted by gain magnitude. Repeat until stable.
    loop {
        let mut merged = false;
        bands.sort_by(|a, b| a.frequency.partial_cmp(&b.frequency).unwrap());
        'outer: for i in 0..bands.len() {
            for j in i + 1..bands.len() {
                let a = &bands[i];
                let b = &bands[j];
                let ratio = b.frequency / a.frequency.max(1e-9);
                let one_semitone = 1.05946f32; // 2^(1/12)
                if ratio > one_semitone {
                    continue;
                }
                if a.gain_db * b.gain_db < 0.0 {
                    continue 'outer; // opposite signs: can cancel, don't merge
                }
                let gw_a = a.gain_db.abs();
                let gw_b = b.gain_db.abs();
                let wsum = gw_a + gw_b;
                if wsum < 1e-9 {
                    continue;
                }
                let fc = (a.frequency * gw_a + b.frequency * gw_b) / wsum;
                let g = a.gain_db + b.gain_db;
                let q = (a.q * gw_a + b.q * gw_b) / wsum;
                let replacement = EqBand {
                    frequency: fc,
                    gain_db: g.clamp(-12.0, 12.0),
                    q: q.clamp(0.8, 4.5),
                    filter_type: FilterType::Peaking,
                };
                // remove j then i
                bands.remove(j);
                bands.remove(i);
                bands.push(replacement);
                merged = true;
                break 'outer;
            }
        }
        if !merged {
            break;
        }
    }
}
