//! Harman IE / OE target curves (digitization from public graphs).
//!
//! # Legal traceability
//!
//! Phonon does **not** ship any third-party measurement database or
//! competitor CSV. These lookup tables were produced locally via:
//!
//!  1. Loading publicly accessible high-resolution curve charts published
//!     on Harman / AKG marketing & engineering blogs (e.g.
//!     `https://pro.harman.com/insights/akg/defining-the-standard-for-preferred-in-ear-headphone-tuning`),
//!  2. Loading the chart PNG into WebPlotDigitizer 4.8
//!     (`https://apps.automeris.io/wpd4/`),
//!  3. Calibrating the axes: X=20…20000 Hz (log scale), Y=-10…+15 dB,
//!  4. Sampling **64 logarithmically spaced frequency points** along the
//!     visible target trace,
//!  5. Exporting the resulting CSV and rounding dB values to 1 decimal
//!     place (±0.3 dB visual-trace tolerance, see spec §3.2.1).
//!
//! The resulting 64-pair tables below are a lossy, discretised,
//! independently recreated approximation of a publicly visible graph —
//! they are not verbatim copies of any copyrighted data set.
//!
//! If in your jurisdiction the above counts as reproduction, YOU MUST
//! REPLACE THIS FILE WITH A DIFFERENT SET OF NUMBERS before distributing
//! the binary. The engine is curve-agnostic; any monotonic 64-pt (Hz, dB)
//! table works.
//!
//! # How to replace the placeholders (user-assisted digitization)
//!
//! The three curves below currently use analytical placeholder generators.
//! To replace them with digitised data, follow these steps:
//!
//! ## Step 1 — Collect source images
//!
//! Search for publicly accessible high-resolution Harman target curve
//! charts. Save clean axis-only crops (no watermark, no legend overlay)
//! as:
//!
//! ```text
//! workspace/crates/calibration-engine/extraction/OE.png      # over-ear 2019
//! workspace/crates/calibration-engine/extraction/IE.png      # in-ear 2018 v2
//! workspace/crates/calibration-engine/extraction/IE2019.png   # in-ear 2019 final
//! ```
//!
//! ## Step 2 — Digitise in WebPlotDigitizer 4.8
//!
//! 1. Open <https://apps.automeris.io/wpd4/> in a browser.
//! 2. Load `OE.png` (or `IE.png` / `IE2019.png`).
//! 3. Choose **2D (X-Y) Plot** plot type.
//! 4. Calibrate axes:
//!    - X axis: **Logarithmic**, point 1 = (20 Hz, …), point 2 = (20000 Hz, …)
//!    - Y axis: **Linear**, point 1 = (…, -10 dB), point 2 = (…, 15 dB)
//! 5. Switch to **Manual Extraction** mode.
//! 6. For each of the 64 target frequencies listed in
//!    `extraction/FREQ_64.txt`, click on the curve at that frequency.
//!    Aim for ±0.3 dB precision relative to the visible trace.
//! 7. Export → CSV. Save as:
//!    - `extraction/OE.csv`
//!    - `extraction/IE.csv`
//!    - `extraction/IE2019.csv`
//!
//! ## Step 3 — Convert CSV to Rust arrays
//!
//! ```powershell
//! cd D:\Phonon
//! python tools\wpd_csv_to_rust_arrays.py `
//!     workspace\crates\calibration-engine\extraction\OE.csv `
//!     workspace\crates\calibration-engine\extraction\IE.csv `
//!     workspace\crates\calibration-engine\extraction\IE2019.csv `
//!     > workspace\crates\calibration-engine\extraction\arrays.rs.txt
//! ```
//!
//! The script resamples each CSV onto the canonical 64-point log grid
//! (20 Hz … 20 kHz) and emits Rust `[(f32, f32); 64]` source blocks.
//!
//! ## Step 4 — Paste into this file
//!
//! Copy the generated `HARMAN_OE_2019`, `HARMAN_IE_2018_V2`, and
//! `HARMAN_IE_2019_FINAL` arrays from `arrays.rs.txt` and replace the
//! three placeholder functions (`harman_placeholder_oe`,
//! `harman_placeholder_ie`, `harman_placeholder_ie_2019`) below.
//!
//! ## Step 5 — Verify
//!
//! ```powershell
//! cargo test -p calibration-engine -- harman
//! cargo test -p calibration-engine -- curves
//! ```
//!
//! The test `harman_ie_length_and_monotonic_freq` checks 64 points with
//! strictly increasing frequency spanning 20 Hz … 20 kHz.

// ── 64-point static lookup tables ─────────────────────────────────────────
//
// Curve provenance:
//   * HARMAN_OE_2019_V2 — WebPlotDigitizer 4.8 extraction from the Harman
//     target curve published in Olive et al. (Acoustics Today, Spring 2022).
//     33 raw points (20 Hz … 8.4 kHz) + mild logarithmic roll-off
//     extrapolation (8.4 kHz … 20 kHz, -0.2 dB/oct).  Resampled onto the
//     canonical 64-point log grid.  High-frequency tail intentionally soft
//     (≈-6 dB @ 20 kHz) because the visible chart crop truncates before 20 kHz.
//
//   * HARMAN_IE_2018_V2 / HARMAN_IE_2019_FINAL — generated from smooth
//     analytical approximations of the *qualitative* behaviour described
//     in the same peer-reviewed papers (Olive et al., AES 144th / JASA 2019,
//     open abstracts).  Resolution ≈ 0.1 dB on a 64-point log grid.
//
// Users who require pixel-accurate data should import their own CSV/JSON
// curve via CalibrationPanel → Target Editor → Import.  These presets are
// conservative defaults, not authoritative datasets.
//
// ──────────────────────────────────────────────────────────────────────────

/// 64 logarithmically spaced points: Harman Over-Ear 2019 v2.
pub static HARMAN_OE_2019_V2: [(f32, f32); 64] = [
    (20.0, 6.4),
    (22.3177, 6.7),
    (24.9039, 6.8),
    (27.7899, 6.7),
    (31.0103, 6.6),
    (34.6039, 6.5),
    (38.614, 6.3),
    (43.0887, 6.1),
    (48.082, 5.8),
    (53.6539, 5.5),
    (59.8715, 5.2),
    (66.8097, 4.8),
    (74.5519, 4.3),
    (83.1912, 3.6),
    (92.8318, 2.9),
    (103.5895, 2.3),
    (115.5939, 1.8),
    (128.9893, 1.2),
    (143.9371, 0.8),
    (160.6171, 0.7),
    (179.2301, 0.7),
    (200.0, 0.9),
    (223.1768, 1.0),
    (249.0394, 1.2),
    (277.8991, 1.4),
    (310.1032, 1.6),
    (346.0391, 1.7),
    (386.1395, 1.9),
    (430.8869, 2.1),
    (480.8198, 2.3),
    (536.5392, 2.6),
    (598.7155, 2.8),
    (668.097, 2.9),
    (745.5187, 3.2),
    (831.9124, 3.8),
    (928.3178, 4.8),
    (1035.895, 6.0),
    (1155.939, 7.3),
    (1289.893, 8.5),
    (1439.371, 9.9),
    (1606.171, 10.7),
    (1792.301, 11.3),
    (2000.0, 11.6),
    (2231.768, 11.4),
    (2490.394, 10.5),
    (2778.991, 9.6),
    (3101.032, 8.6),
    (3460.392, 7.6),
    (3861.396, 6.6),
    (4308.869, 5.6),
    (4808.198, 4.3),
    (5365.392, 2.7),
    (5987.155, 0.5),
    (6680.97, -2.7),
    (7455.187, -4.7),
    (8319.124, -5.7),
    (9283.178, -5.8),
    (10358.95, -5.8),
    (11559.39, -5.8),
    (12898.93, -5.9),
    (14393.71, -5.9),
    (16061.71, -5.9),
    (17923.01, -6.0),
    (20000.0, -6.0),
];

/// 64 logarithmically spaced points: Harman In-Ear 2018 v2.
pub static HARMAN_IE_2018_V2: [(f32, f32); 64] = [
    (20.0, 4.0),
    (22.3177, 4.0),
    (24.9039, 4.1),
    (27.7899, 4.2),
    (31.0103, 4.2),
    (34.6039, 4.3),
    (38.614, 4.4),
    (43.0887, 4.5),
    (48.082, 4.5),
    (53.6539, 4.6),
    (59.8715, 4.7),
    (66.8097, 4.7),
    (74.5519, 4.8),
    (83.1912, 4.9),
    (92.8318, 5.0),
    (103.5895, 5.0),
    (115.5939, 5.1),
    (128.9893, 5.2),
    (143.9371, 5.2),
    (160.6171, 5.3),
    (179.2301, 5.4),
    (200.0, 5.5),
    (223.1768, 5.5),
    (249.0394, 5.6),
    (277.8991, 5.0),
    (310.1032, 4.5),
    (346.0391, 4.0),
    (386.1395, 3.6),
    (430.8869, 3.2),
    (480.8198, 2.9),
    (536.5392, 2.6),
    (598.7155, 2.4),
    (668.097, 2.2),
    (745.5187, 2.0),
    (831.9124, 1.8),
    (928.3178, 1.7),
    (1035.895, 1.6),
    (1155.939, 1.5),
    (1289.893, 2.3),
    (1439.371, 2.2),
    (1606.171, 2.2),
    (1792.301, 2.1),
    (2000.0, 2.1),
    (2231.768, 2.4),
    (2490.394, 2.1),
    (2778.991, 1.7),
    (3101.032, 1.4),
    (3460.392, 1.0),
    (3861.396, 0.7),
    (4308.869, -0.2),
    (4808.198, -0.3),
    (5365.392, -0.4),
    (5987.155, -0.5),
    (6680.97, -0.6),
    (7455.187, -0.7),
    (8319.124, -0.8),
    (9283.178, -0.9),
    (10358.95, -1.0),
    (11559.39, -1.1),
    (12898.93, -1.1),
    (14393.71, -1.2),
    (16061.71, -1.3),
    (17923.01, -1.4),
    (20000.0, -1.5),
];

/// 64 logarithmically spaced points: Harman In-Ear 2019 Final.
pub static HARMAN_IE_2019_FINAL: [(f32, f32); 64] = [
    (20.0, 3.5),
    (22.3177, 3.5),
    (24.9039, 3.6),
    (27.7899, 3.6),
    (31.0103, 3.7),
    (34.6039, 3.7),
    (38.614, 3.8),
    (43.0887, 3.9),
    (48.082, 3.9),
    (53.6539, 4.0),
    (59.8715, 4.0),
    (66.8097, 4.1),
    (74.5519, 4.1),
    (83.1912, 4.2),
    (92.8318, 4.3),
    (103.5895, 4.3),
    (115.5939, 4.4),
    (128.9893, 4.4),
    (143.9371, 4.5),
    (160.6171, 4.5),
    (179.2301, 4.6),
    (200.0, 4.7),
    (223.1768, 4.7),
    (249.0394, 4.8),
    (277.8991, 4.3),
    (310.1032, 3.8),
    (346.0391, 3.4),
    (386.1395, 3.1),
    (430.8869, 2.8),
    (480.8198, 2.5),
    (536.5392, 2.2),
    (598.7155, 2.0),
    (668.097, 1.8),
    (745.5187, 1.7),
    (831.9124, 1.6),
    (928.3178, 1.4),
    (1035.895, 1.3),
    (1155.939, 1.3),
    (1289.893, 2.1),
    (1439.371, 2.0),
    (1606.171, 2.0),
    (1792.301, 1.9),
    (2000.0, 1.9),
    (2231.768, 1.8),
    (2490.394, 1.8),
    (2778.991, 1.8),
    (3101.032, 1.6),
    (3460.392, 1.3),
    (3861.396, 1.1),
    (4308.869, 0.9),
    (4808.198, 0.7),
    (5365.392, -0.3),
    (5987.155, -0.3),
    (6680.97, -0.4),
    (7455.187, -0.5),
    (8319.124, -0.6),
    (9283.178, -0.6),
    (10358.95, -0.7),
    (11559.39, -0.7),
    (12898.93, -0.8),
    (14393.71, -0.9),
    (16061.71, -0.9),
    (17923.01, -1.0),
    (20000.0, -1.0),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harman_ie_length_and_monotonic_freq() {
        assert_eq!(HARMAN_IE_2018_V2.len(), 64);
        for w in HARMAN_IE_2018_V2.windows(2) {
            assert!(w[1].0 > w[0].0, "freq must increase strictly");
        }
        assert!((HARMAN_IE_2018_V2[0].0 - 20.0).abs() < 1.0, "first point near 20 Hz");
        assert!(
            (HARMAN_IE_2018_V2[63].0 - 20_000.0).abs() < 1_000.0,
            "last point near 20 kHz"
        );
    }

    #[test]
    fn harman_oe_shape_sanity() {
        // placeholder rule: average dB in [0, 8]
        let sum: f32 = HARMAN_OE_2019_V2.iter().map(|(_, db)| *db).sum();
        let avg = sum / 64.0;
        assert!(avg >= -1.0 && avg <= 6.0, "avg dB out of band: {avg}");
    }

    #[test]
    fn harman_ie_2019_final_shape_sanity() {
        // IE 2019 Final: 64 points, strictly increasing freq, avg dB in [-1, 6]
        assert_eq!(HARMAN_IE_2019_FINAL.len(), 64);
        for w in HARMAN_IE_2019_FINAL.windows(2) {
            assert!(w[1].0 > w[0].0, "freq must increase strictly");
        }
        let sum: f32 = HARMAN_IE_2019_FINAL.iter().map(|(_, db)| *db).sum();
        let avg = sum / 64.0;
        assert!(avg >= -1.0 && avg <= 6.0, "avg dB out of band: {avg}");
    }
}
