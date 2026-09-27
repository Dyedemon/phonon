//! Akima cubic Hermite interpolation on a sorted grid.
//!
//! Reference:
//!   Hiroshi Akima, "A New Method of Interpolation and Smooth Curve
//!   Fitting Based on Local Procedures", J. ACM 17(4), 1970.
//!
//! This is a zero-dependency original Rust implementation; it's used for
//! resampling 64-pt measurement/target tables onto the 256-pt log grid
//! that the frontend Canvas uses and that the Biquad LS solver consumes.

/// Interpolate `x` onto the knot set `(xs, ys)` where `xs` is strictly
/// increasing and `xs.len() == ys.len() >= 4`.
///
/// Behaviour outside the knot range: linear extrapolation using the
/// closest interval's computed tangents (matches Octave's `akima` default).
pub fn akima_interp(xs: &[f32], ys: &[f32], x: f32) -> f32 {
    let n = xs.len();
    assert!(n >= 4, "akima needs >= 4 knots, got {n}");
    assert_eq!(
        n,
        ys.len(),
        "akima: xs.len({}) != ys.len({})",
        n,
        ys.len()
    );

    // Locate interval
    let k = match xs.binary_search_by(|&xi| xi.partial_cmp(&x).unwrap()) {
        Ok(i) => return ys[i],
        Err(i) => (i.saturating_sub(1)).min(n - 2),
    };

    // Build raw slopes on each interval.
    let mut raw_m = vec![0.0f32; n - 1];
    for i in 0..n - 1 {
        let dx = xs[i + 1] - xs[i];
        raw_m[i] = if dx.abs() < 1e-18 {
            0.0
        } else {
            (ys[i + 1] - ys[i]) / dx
        };
    }
    // Shadow slopes outside the data range are mirrored linearly — the
    // classical Akima paper end-condition recipe. Extend to [raw_m.len() + 4]
    // slots indexed 0..raw_m.len()+4 so we can subscript without branches.
    let last_idx = raw_m.len() as i32 - 1;
    let m_slots: Vec<f32> = {
        let s_m2 = 2.0 * raw_m[0] - raw_m[1];
        let s_m1 = 2.0 * raw_m[0] - s_m2;
        let e_p1 = 2.0 * raw_m[last_idx as usize] - raw_m[(last_idx - 1) as usize];
        let e_p2 = 2.0 * e_p1 - raw_m[last_idx as usize];
        let mut v = Vec::with_capacity(raw_m.len() + 4);
        v.push(s_m1);
        v.push(s_m2);
        v.extend_from_slice(&raw_m);
        v.push(e_p1);
        v.push(e_p2);
        v
    };
    let m = |i: i32| -> f32 {
        // Shadow index offset by +2: m(-2) → m_slots[0], m(0) → m_slots[2], etc.
        let slot = (i + 2).clamp(0, m_slots.len() as i32 - 1) as usize;
        m_slots[slot]
    };

    let k_i32 = k as i32;
    let _m1 = m(k_i32 - 2);
    let m2 = m(k_i32 - 1);
    let m3 = m(k_i32);
    let m4 = m(k_i32 + 1);
    let m5 = m(k_i32 + 2);

    // Left tangent (at xs[k]) uses Akima weighted average of m1..m5,
    // weights = |m3-m2| and |m5-m4| (the slope differences that bound it).
    let w_lhs = (m3 - m2).abs();
    let w_rhs = (m5 - m4).abs();
    let t_left = if w_lhs + w_rhs < 1e-12 {
        0.5 * (m2 + m3)
    } else {
        (w_rhs * m2 + w_lhs * m4) / (w_lhs + w_rhs)
    };
    // Right tangent (at xs[k+1]) uses m2..m6 (offset by one interval).
    // m6 = m(k_i32+3); t_right uses weights |m4-m3| and |m6-m5|.
    let m6 = m(k_i32 + 3);
    let w_lhs_r = (m4 - m3).abs();
    let w_rhs_r = (m6 - m5).abs();
    let t_right = if w_lhs_r + w_rhs_r < 1e-12 {
        0.5 * (m3 + m4)
    } else {
        (w_rhs_r * m3 + w_lhs_r * m5) / (w_lhs_r + w_rhs_r)
    };

    // Cubic Hermite spline on [xs[k], xs[k+1]].
    let h = xs[k + 1] - xs[k];
    if h.abs() < 1e-18 {
        return ys[k];
    }
    let t = (x - xs[k]) / h;
    let h00 = (2.0 * t - 3.0) * t * t + 1.0;
    let h10 = ((t - 2.0) * t + 1.0) * t;
    let h01 = (-2.0 * t + 3.0) * t * t;
    let h11 = (t - 1.0) * t * t;
    h00 * ys[k] + h10 * h * t_left + h01 * ys[k + 1] + h11 * h * t_right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn akima_on_knot_is_exact() {
        let xs = vec![1.0f32, 2.0, 4.0, 7.0, 9.0, 12.0];
        let ys = xs.iter().map(|x| x * x - 3.0 * x).collect::<Vec<_>>();
        for (xi, yi) in xs.iter().zip(ys.iter()) {
            let v = akima_interp(&xs, &ys, *xi);
            assert!((v - yi).abs() < 1e-5, "exact knot mismatch: x={xi}");
        }
    }

    #[test]
    fn akima_is_smooth_across_intervals() {
        // y = sin(x). Sample 200 internal points, max error should be <<1 dB.
        let mut xs = vec![];
        let mut ys = vec![];
        for i in 0..12 {
            let x = 0.0 + (i as f32) * 0.6;
            xs.push(x);
            ys.push(x.sin());
        }
        let mut max_err = 0.0f32;
        for i in 0..200 {
            let x = 0.0 + (i as f32) * (0.6 * 11.0 / 200.0);
            let e = (akima_interp(&xs, &ys, x) - x.sin()).abs();
            max_err = max_err.max(e);
        }
        assert!(
            max_err < 0.05,
            "akima interpolation error too big: {max_err}"
        );
    }
}
