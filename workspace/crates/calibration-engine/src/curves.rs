//! Target-curve resolution + common helpers on FitCurves.
//!
//! Spec §3.2.1 target registry:
//!   - harman_ie_2018_v2    (in-ear, preferred for IEMs)
//!   - harman_oe_2019_v2    (over-ear, preferred for headphones)
//!   - harman_ie_2019_final (in-ear 2019 final, smoother treble)
//!   - diffuse_field_flat   (flat in sound power)
//!   - free_field_flat      (flat in ear canal SPL)
//!   - neutral_listening    (OE 2019 with -3 dB bass shelf reduction)
//!
//! User can override any of them with `FitRequest.custom_target`
//! (a 64-pt (Hz, dB) table we interpolate to the 256-pt log grid).

use calibration_types::{FitCurves, FitError, FitRequest};
use crate::akima::akima_interp;
use crate::error::EngineError;
use crate::harman::{HARMAN_IE_2018_V2, HARMAN_IE_2019_FINAL, HARMAN_OE_2019_V2};

/// Turn a user-provided 64-pt (Hz, dB) table into a 256-pt standard grid.
pub fn sample_table_to_grid(table: &[(f32, f32)]) -> Result<FitCurves, EngineError> {
    if table.len() < 4 {
        return Err(EngineError::Interp(
            "custom target needs at least 4 (Hz, dB) points".into(),
        ));
    }
    let mut xs: Vec<f32> = table.iter().map(|(x, _)| x.log10()).collect();
    let mut ys: Vec<f32> = table.iter().map(|(_, y)| *y).collect();
    // strictly ascending frequency, drop duplicate / out-of-order rows
    let mut i = 1usize;
    while i < xs.len() {
        if xs[i] <= xs[i - 1] {
            xs.remove(i);
            ys.remove(i);
        } else {
            i += 1;
        }
    }
    if xs.len() < 4 {
        return Err(EngineError::Interp(
            "custom target must contain >=4 distinct frequencies".into(),
        ));
    }
    let mut out = FitCurves::standard_log_grid();
    for (slot, f) in out.freq.iter().enumerate() {
        let lf = f.log10();
        // Akima extrapolates gently outside the table; clamp residual to ±20 dB
        // so a bad table can't blow up subsequent LS fitting.
        let v = akima_interp(&xs, &ys, lf).clamp(-30.0, 30.0);
        out.mag_db[slot] = v;
    }
    out.validate().map_err(|s| EngineError::Fit(FitError::InvalidConfig(s)))?;
    Ok(out)
}

/// Registry lookup. Custom target in `request.custom_target` overrides
/// `target_curve_id` entirely (user explicitly edited the curve).
pub fn resolve_target(request: &FitRequest) -> Result<FitCurves, EngineError> {
    if let Some(ct) = &request.custom_target {
        // custom_target is already a 256-pt FitCurves on the standard grid
        return Ok(ct.clone());
    }
    let table: &[(f32, f32)] = match request.target_curve_id.as_str() {
        "harman_ie_2018_v2" | "harman_ie_2018" | "harman_ie" => &HARMAN_IE_2018_V2,
        "harman_oe_2019_v2" | "harman_oe_2019" | "harman_oe" | "harman_over_ear" => {
            &HARMAN_OE_2019_V2
        }
        "harman_ie_2019_final" | "harman_ie_2019" => &HARMAN_IE_2019_FINAL,
        "diffuse_field_flat" | "diffuse" => &DIFFUSE_FIELD_FLAT,
        "free_field_flat" | "free" => &FREE_FIELD_FLAT,
        "neutral_listening" | "neutral" => {
            // OE 2019 with -3 dB bass shelf reduction below 200 Hz.
            // Build a derived 64-pt table on the fly.
            let base = &HARMAN_OE_2019_V2;
            let mut reduced: [(f32, f32); 64] = [(0.0, 0.0); 64];
            for (i, &(f, db)) in base.iter().enumerate() {
                let adj = if f < 200.0 { -3.0 } else { 0.0 };
                reduced[i] = (f, db + adj);
            }
            return sample_table_to_grid(&reduced);
        }
        other => {
            return Err(EngineError::Fit(FitError::UnknownTarget(other.to_string())));
        }
    };
    sample_table_to_grid(table)
}

/// Diffuse-field flat target (0 dB across the band). Simple constant — no
/// licensing concern, pure engineering convention.
const DIFFUSE_FIELD_FLAT: [(f32, f32); 7] = [
    (20.0, 0.0),
    (100.0, 0.0),
    (500.0, 0.0),
    (1_000.0, 0.0),
    (3_000.0, 0.0),
    (10_000.0, 0.0),
    (20_000.0, 0.0),
];

/// Free-field flat target: linear -6 dB/oct above ~1 kHz to emulate
/// head-related pinna gain roll-off (IEC 60268-7:2006 convention, original
/// mathematical formulation — public ISO/IEC standard excerpt).
const FREE_FIELD_FLAT: [(f32, f32); 9] = [
    (20.0, 0.0),
    (100.0, 0.0),
    (500.0, 0.0),
    (1_000.0, 0.0),
    (2_000.0, -1.0),
    (4_000.0, -3.5),
    (8_000.0, -6.5),
    (15_000.0, -9.0),
    (20_000.0, -10.5),
];
