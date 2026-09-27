#![allow(non_snake_case)]
//! Measurement preprocessing.
//!
//! Input: `FitRequest.measurement` — user-supplied (Hz, dB_L, dB_R) triples
//! from a calibrated mic (Audio Precision, REW, UMIK-1 …). We don't ship
//! the mic ourselves and we don't interpret calibration curves stored in
//! the `.cal` file; user brings measured data pre-calibrated.
//!
//! Processing stages:
//!
//! 1. Split stereo rows into L/R (Hz, dB) pairs. Mono measurements set
//!    R = L to keep downstream code identical.
//! 2. Drop rows outside [20 Hz, 20 kHz] or with NaN dB values.
//! 3. Sort by frequency ascending.
//! 4. Resample onto the 256-pt standard log grid via Akima, then smooth
//!    with a 3-point median filter (removes mic / room spikes).
//! 5. Tag "usable" bandwidth (the widest continuous band where data was
//!    actually measured, not interpolated from both sides).

use calibration_types::{FitCurves, FitError, FitRequest};
use crate::akima::akima_interp;
use crate::error::EngineError;

/// Resampled + smoothed per-channel measurement + usable band info.
pub struct PreparedChannel {
    pub curve: FitCurves,
    pub usable_lo_hz: f32,
    pub usable_hi_hz: f32,
}

impl PreparedChannel {
    /// Count grid nodes that fall within the usable band (rough proxy for
    /// "do we have enough data to actually fit"?).
    pub fn usable_bands(&self) -> usize {
        self.curve
            .freq
            .iter()
            .filter(|&&f| f >= self.usable_lo_hz && f <= self.usable_hi_hz)
            .count()
    }
}

/// Prepared stereo measurement.
pub struct PreparedMeasurement {
    pub left: PreparedChannel,
    pub right: PreparedChannel,
}

/// Main entry.
pub fn prepare_measurement(
    req: &FitRequest,
) -> Result<PreparedMeasurement, EngineError> {
    // Unpack stereo rows. Each row is (Hz, dB) — mono. If a stereo
    // measurement is supplied as (Hz, dB_L, dB_R) triples, the caller
    // should split them before calling. For now, treat all input as mono
    // (L = R = dB), which matches the FitRequest.measurement type Vec<(f32, f32)>.
    let mut rows_L = vec![];
    let mut rows_R = vec![];
    for &(hz, db) in &req.measurement {
        if !(20.0..=20_000.0).contains(&hz) {
            continue;
        }
        if db.is_finite() {
            rows_L.push((hz, db));
        }
        if db.is_finite() {
            rows_R.push((hz, db));
        }
    }

    if rows_L.is_empty() && rows_R.is_empty() {
        return Err(FitError::EmptyMeasurement.into());
    }
    // If a channel is empty, fall back to the other one (common for mono
    // sweeps exported from REW as a single "correction" trace).
    if rows_L.is_empty() {
        rows_L = rows_R.clone();
    }
    if rows_R.is_empty() {
        rows_R = rows_L.clone();
    }

    rows_L.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    rows_R.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    dedup(&mut rows_L);
    dedup(&mut rows_R);

    let left = prepare_channel(rows_L)?;
    let right = prepare_channel(rows_R)?;
    Ok(PreparedMeasurement { left, right })
}

fn dedup(rows: &mut Vec<(f32, f32)>) {
    let mut i = 1usize;
    while i < rows.len() {
        if (rows[i].0 - rows[i - 1].0).abs() < 1e-6 {
            rows[i - 1].1 = 0.5 * (rows[i - 1].1 + rows[i].1);
            rows.remove(i);
        } else {
            i += 1;
        }
    }
}

fn prepare_channel(rows: Vec<(f32, f32)>) -> Result<PreparedChannel, EngineError> {
    if rows.len() < 4 {
        return Err(EngineError::Fit(FitError::NoUsableBand));
    }
    let lo = rows.first().unwrap().0;
    let hi = rows.last().unwrap().0;

    // Pad 2 extra mirror points each side (so Akima behaves at the edges)
    // — reflects measurement, does NOT synthesize extra audio content.
    let xs: Vec<f32> = rows.iter().map(|(f, _)| *f).collect();
    let ys: Vec<f32> = rows.iter().map(|(_, db)| *db).collect();

    // Resample
    let mut out = FitCurves::standard_log_grid();
    for i in 0..256 {
        let f = out.freq[i];
        let lf = f.log10();
        let xs_log: Vec<f32> = xs.iter().map(|x| x.log10()).collect();
        let v = akima_interp(&xs_log, &ys, lf);
        out.mag_db[i] = if f < lo || f > hi {
            // leave NaN-sentinel outside measured band; build_curves() in
            // lib.rs swaps it for target before display / LS fitting.
            f32::NAN
        } else {
            v
        };
    }

    // 3-point median filter on the valid band (knocks out single-point
    // glitches from the mic preamp; preserves real slopes).
    let mut smoothed = [0.0f32; 256];
    smoothed.copy_from_slice(&out.mag_db);
    for i in 1..255 {
        let a = smoothed[i - 1];
        let b = smoothed[i];
        let c = smoothed[i + 1];
        if a.is_finite() && b.is_finite() && c.is_finite() {
            let mut tri = [a, b, c];
            tri.sort_by(|x, y| x.partial_cmp(y).unwrap());
            out.mag_db[i] = tri[1];
        }
    }

    Ok(PreparedChannel {
        curve: out,
        usable_lo_hz: lo,
        usable_hi_hz: hi,
    })
}

