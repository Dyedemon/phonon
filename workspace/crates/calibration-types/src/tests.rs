//! TDD tests for calibration-types: run with `cargo test -p calibration-types`.

use super::*;
use serde_json::{from_str, to_string};

// ── FitCurves.standard_log_grid invariants ──────────────────────────────────

#[test]
fn test_fit_curves_log_grid_exact_256_length() {
    let g = FitCurves::standard_log_grid();
    assert_eq!(g.freq.len(), 256, "freq must be 256");
    assert_eq!(g.mag_db.len(), 256, "mag_db must be 256");
    assert!((g.freq[0] - 20.0).abs() < 1e-6, "freq[0] must be 20 Hz");
    assert!((g.freq[255] - 20_000.0).abs() < 1e-3, "freq[255] must be 20 kHz");
    // Geometric midpoint check: freq[128] = 20 * 1000^(128/255)
    let mid_expected = 20.0 * 1000.0f32.powf(128.0 / 255.0);
    assert!(
        (g.freq[128] - mid_expected).abs() < 1e-3,
        "freq[128] = {:.4}, expected {:.4}",
        g.freq[128],
        mid_expected
    );
    // Strictly monotonic increasing.
    for i in 1..256 {
        assert!(
            g.freq[i] > g.freq[i - 1],
            "freq must be strictly monotonic; i={i} {:.3} <= prev {:.3}",
            g.freq[i],
            g.freq[i - 1]
        );
    }
    // validate() must be identity-ok.
    assert!(g.validate().is_ok(), "standard_log_grid must pass validate");
    // mag_db uniform 0 initialised
    for m in g.mag_db {
        assert!((m - 0.0).abs() < 1e-9, "initial mag must be zero");
    }
}

#[test]
fn test_fit_curves_validate_rejects_wrong_length() {
    let mut bad = FitCurves::standard_log_grid();
    bad.freq.truncate(255);
    let err = bad.validate().expect_err("255 freq must fail validate");
    assert!(err.contains("256"), "err must mention 256, got: {err}");
}

#[test]
fn test_fit_curves_validate_rejects_misaligned_freq_axis() {
    let mut bad = FitCurves::standard_log_grid();
    bad.freq[42] = 1234.567; // not on the standard log grid
    let err = bad.validate().expect_err("shifted point must fail");
    assert!(
        err.contains("strict 256-pt log axis"),
        "msg wrong: {err}"
    );
}

// ── FitRequest default matches spec defaults ───────────────────────────────

#[test]
fn test_fit_request_defaults() {
    let r = FitRequest::default();
    assert_eq!(r.target_curve_id, "harman_ie_2018_v2");
    assert_eq!(r.max_bands, 16);
    assert_eq!(r.fir_taps, 2048);
    assert_eq!(r.sample_rate, 48_000);
    assert!(r.measurement.is_empty());
    assert!(r.custom_target.is_none());
}

// ── Error types serde_json round-trip (critical for Tauri TS toast) ────────

#[test]
fn test_fit_error_serde_roundtrip_all_variants() {
    let cases: Vec<FitError> = vec![
        FitError::EmptyMeasurement,
        FitError::NoUsableBand,
        FitError::NoConverge(500),
        FitError::UnknownTarget("nonexistent".into()),
        FitError::InvalidConfig("oops".into()),
    ];
    for err in cases {
        let s = to_string(&err).expect("serialize");
        let back: FitError = from_str(&s).expect("deserialize");
        assert_eq!(back, err, "roundtrip mismatch for {err:?}\n  json={s}");
    }
}

#[test]
#[cfg(feature = "native")]
fn test_peq_update_error_serde_roundtrip() {
    let cases: Vec<PeqUpdateError> = vec![
        PeqUpdateError::ChannelFull,
        PeqUpdateError::ChannelClosed,
        PeqUpdateError::InvalidBand(3, "freq out of range".into()),
        PeqUpdateError::FirConfigFailed("bad json".into()),
    ];
    for err in cases {
        let s = to_string(&err).unwrap();
        let back: PeqUpdateError = from_str(&s).unwrap();
        assert_eq!(back, err, "roundtrip fail for {err:?}\n  json={s}");
    }
}

#[test]
#[cfg(feature = "native")]
fn test_apply_error_serde_roundtrip() {
    // Peq variant round-trips via the `#[from]` PeqUpdateError.
    let peq = ApplyError::Peq(PeqUpdateError::ChannelFull);
    let s = to_string(&peq).unwrap();
    let back: ApplyError = from_str(&s).unwrap();
    assert_eq!(back, peq, "Peq variant: json={s}");

    let rolled = ApplyError::FirRolledBack {
        peq_applied: true,
        detail: "plugin panic in decode".into(),
    };
    let s = to_string(&rolled).unwrap();
    let back: ApplyError = from_str(&s).unwrap();
    assert_eq!(back, rolled, "FirRolledBack variant: json={s}");

    let nf = ApplyError::FitNotFound;
    let s = to_string(&nf).unwrap();
    let back: ApplyError = from_str(&s).unwrap();
    assert_eq!(back, nf, "FitNotFound: json={s}");
}

// ── EqBand converts losslessly to/from phonon-core's EqBand ────────────────

#[test]
#[cfg(feature = "native")]
fn test_eqband_conversion_roundtrip() {
    // EqBand is a wrapper around phonon_core::eq::EqBand bridged by the
    // From impls — verify the conversion preserves every field both ways.
    let types_band = EqBand::default();
    let core_band: phonon_core::eq::EqBand = types_band.clone().into();
    assert_eq!(types_band.frequency, core_band.frequency);
    assert_eq!(types_band.gain_db, core_band.gain_db);
    assert_eq!(types_band.q, core_band.q);
    assert!(matches!(core_band.filter_type, phonon_core::eq::FilterType::Peaking));

    let back: EqBand = core_band.into();
    assert_eq!(back.frequency, types_band.frequency);
    assert_eq!(back.gain_db, types_band.gain_db);
    assert_eq!(back.q, types_band.q);
    assert!(matches!(back.filter_type, FilterType::Peaking));
}

// ── FitResult: applied_at serialized / deserialized usefully ──────────────

#[test]
#[cfg(feature = "native")]
fn test_fit_result_applied_at_roundtrip() {
    let mut fr = dummy_fit_result();
    fr.applied_at = Some(std::time::SystemTime::UNIX_EPOCH);
    let s = to_string(&fr).unwrap();
    let back: FitResult = from_str(&s).unwrap();
    assert!(back.applied_at.is_some());
}

#[cfg(feature = "native")]
fn dummy_fit_result() -> FitResult {
    FitResult {
        fit_id: "dummy".into(),
        target_curve_id: "diffuse_field_flat".into(),
        bands_L: vec![],
        bands_R: vec![],
        fir_L: vec![0.0; 8],
        fir_R: vec![0.0; 8],
        curves_L: FitCurves::standard_log_grid(),
        curves_R: FitCurves::standard_log_grid(),
        rms_error_db: 0.1,
        peak_error_db: 0.5,
        applied_at: None,
    }
}
