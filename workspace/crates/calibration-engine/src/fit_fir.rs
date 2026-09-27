//! Residual → minimum-phase FIR using Hilbert transform.
//!
//! Process (original, textbook approach — none of this is proprietary):
//!
//!  1. Upsample the residual-curve (256 pts on log-axis) onto a linear
//!     frequency grid of length `fft_len ≥ 2·taps`.
//!  2. Convert magnitude dB → linear magnitude.
//!  3. Take `H_amp = ifftshift(exp(ln_mag))`; log-magnitude spectrum.
//!  4. Real FFT → Hilbert transform of the log-magnitude to get the
//!     minimum-phase response via the classic DFT-minimum-phase lemma:
//!
//!     ```text
//!     H_phase(w) = -Hilbert{ log |H| }(w)
//!     ```
//!
//!     where `Hilbert{·}` is the Hilbert transform computed by:
//!
//!     ```text
//!     H_fft = RFFT(log|H|)
//!     H_fft[0]         *= 0.0
//!     H_fft[N/2]       *= 0.0
//!     H_fft[1..N/2-1]  *= -j
//!     ```
//!
//!     (actual code uses +j for negative indices depending on the DFT
//!     convention used by rustfft; we use the standard approach — see
//!     Smith III, "Spectral Audio Signal Processing" online book, MIT
//!     OpenCourseWare, § Minimum-Phase FIR).
//!  5. Rebuild complex frequency response H_lin_mag·exp(j·phase), IFFT,
//!     window with Kaiser(β=6) to taps, normalise max(|h|) < 1 (so FIR is
//!     unconditionally L2 stable), then return as linear-phase truncated.
//!
//! Rustfft handles the RFFT/CFFT. The code below is hand-written.

use calibration_types::{FitCurves, FitError};
use crate::akima::akima_interp;
use crate::error::EngineError;
use rustfft::num_complex::Complex;
use rustfft::{FftPlanner, Fft};
use std::sync::Arc;

/// Produce a minimum-phase FIR of exactly `taps` samples at `fs` that,
/// when cascaded after the biquad PEQ, compensates the residual
/// `(target − biquad_response)` curve.
pub fn residual_to_fir(
    target: &FitCurves,
    biquad_response: &FitCurves,
    taps: usize,
    fs: u32,
) -> Result<Vec<f32>, EngineError> {
    if !taps.is_power_of_two() || !(64..=8192).contains(&taps) {
        return Err(FitError::InvalidConfig(format!(
            "residual_to_fir: taps={taps} not power-of-two in [64, 8192]"
        ))
        .into());
    }
    if !(8_000..=384_000).contains(&fs) {
        return Err(FitError::InvalidConfig(format!(
            "residual_to_fir: fs={fs} out of [8k, 384k]"
        ))
        .into());
    }

    // Build residual in dB on the 256-pt log grid.
    let mut residual_db = [0.0f32; 256];
    for (r, (&t, &b)) in residual_db.iter_mut().zip(target.mag_db.iter().zip(biquad_response.mag_db.iter())) {
        *r = t - b;
    }
    // Smooth (single 3-tap median — removes numerical ripple from the
    // Akima resampler crossing log/linear).
    for i in 1..255 {
        let mut tri = [residual_db[i - 1], residual_db[i], residual_db[i + 1]];
        tri.sort_by(|a, b| a.partial_cmp(b).unwrap());
        residual_db[i] = tri[1];
    }

    // Step 1 — upsample onto linear grid (0 … fs/2).
    let fft_len = (taps * 4).next_power_of_two().max(1024);
    let half = fft_len / 2 + 1; // RFFT output length
    let mut log_mag = vec![0.0f32; fft_len];
    {
        let xs_log: Vec<f32> = FitCurves::standard_log_grid()
            .freq
            .iter()
            .map(|f| f.log10())
            .collect();
        for (k, slot) in log_mag.iter_mut().enumerate().take(half) {
            let f_linear = (k as f32) * (fs as f32) / (fft_len as f32);
            let f_clamp = f_linear.clamp(20.0, 20_000.0);
            let v_log = f_clamp.log10();
            let rd = akima_interp(&xs_log, &residual_db, v_log);
            // fade in/out below 20 Hz / above 20 kHz to 0 dB (avoids
            // extrapolation artifacts turning into DC/ultrasonic junk).
            let fade = if f_linear < 20.0 {
                (f_linear / 20.0).clamp(0.0, 1.0)
            } else if f_linear > 20_000.0 {
                ((fs as f32 * 0.5 - f_linear) / (fs as f32 * 0.5 - 20_000.0)).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let db = rd * fade;
            // Convert dB → ln-magnitude.
            *slot = db * (10.0f32.ln() / 20.0);
        }
        // Mirror to negative frequencies (hermitian) — RFFT of a real signal
        // is hermitian; to take a real FFT of log_mag we don't need this;
        // we only fill half the log_mag vector and its hermitian mate
        // is inferred. Actually we use a full CFFT below so set mirror:
        // Mirror to negative frequencies (hermitian) — read-while-write on
        // same array; pre-collect to avoid borrow conflict with iter_mut.
        let mirrors: Vec<f32> = (half..fft_len)
            .map(|k| {
                let mirror_k = fft_len - k;
                if mirror_k < half { log_mag[mirror_k] } else { 0.0 }
            })
            .collect();
        for (k, &m) in (half..fft_len).zip(mirrors.iter()) {
            log_mag[k] = m;
        }
    }

    // Steps 2-4 — minimum phase via DFT Hilbert of log-mag.
    let mut planner = FftPlanner::<f32>::new();
    let fft_fwd: Arc<dyn Fft<f32>> = planner.plan_fft_forward(fft_len);
    let fft_inv: Arc<dyn Fft<f32>> = planner.plan_fft_inverse(fft_len);

    let mut buf: Vec<Complex<f32>> = log_mag.iter().map(|&x| Complex::new(x, 0.0)).collect();
    fft_fwd.process(&mut buf);
    // Hilbert by frequency-domain masking:
    //   h[n] ← IFFT( H·(U·2 - δ) )
    // where U is unit step on positive freqs, δ is DC impulse. This is the
    // standard "double positive frequencies and zero out negative ones"
    // recipe, modified so real-odd result is the Hilbert of log|H|.
    {
        // DC and Nyquist stay real (imag=0) and halved.
        buf[0] = Complex::new(0.0, 0.0);
        if fft_len >= 2 {
            buf[fft_len / 2] = Complex::new(0.0, 0.0);
        }
        for k in 1..fft_len / 2 {
            // positive k → multiply by -j (so imag takes Hilbert sign)
            let c = buf[k];
            buf[k] = Complex::new(c.im, -c.re) * 2.0; // actually *(-j)=swap-real/imag and sign
            let k_neg = fft_len - k;
            let d = buf[k_neg];
            buf[k_neg] = Complex::new(-d.im, d.re) * 2.0;
        }
    }
    fft_inv.process(&mut buf);
    // Normalisation by 1/N already applied by rustfft? No — rustfft does
    // NOT divide, so Hilbert result is in `buf[i].re / fft_len as f32`;
    // we keep imag (the Hilbert of log|H|) as the phase. Correct formula:
    //   phase[k] = -hilbert(log|H|)[k]
    let phase: Vec<f32> = (0..fft_len)
        .map(|k| -buf[k].re / (fft_len as f32))
        .collect();
    let _ = phase;

    // Step 5 — reconstruct complex H per bin: exp(log_mag + j*phase).
    let mut h_spec: Vec<Complex<f32>> = (0..fft_len)
        .map(|k| {
            let phi = phase[k]; // = -hilbert(log|H|)[k]
            Complex::new(phi.cos(), phi.sin()) * log_mag[k].exp()
        })
        .collect();
    fft_inv.process(&mut h_spec);
    // IFFT → h[n] is real (to float precision).
    let scale = 1.0 / (fft_len as f32);
    let impulse: Vec<f32> = h_spec.iter().map(|c| c.re * scale).collect();

    // Circular shift: minimum phase produced by this recipe lives on
    // h[0..taps/2] plus the wrap-around tail on h[fft_len-taps/2..fft_len].
    // Concatenate tail → head to yield a linear, causal impulse.
    let shift_len = taps / 2;
    let mut causal = Vec::with_capacity(taps);
    causal.extend_from_slice(&impulse[fft_len - shift_len..fft_len]);
    causal.extend_from_slice(&impulse[..taps - shift_len]);

    // Kaiser window, β=6 (80 dB stop-band, sharp transition).
    apply_kaiser(&mut causal, 6.0);
    // Normalise so that the max absolute tap ≤ 0.95; prevents wrap-around
    // clipping in any downstream fixed-point or integer plugin layer.
    let peak = causal.iter().fold(0.0f32, |a, &b| a.max(b.abs())).max(1e-9);
    let norm = 0.95 / peak;
    for x in causal.iter_mut() {
        *x *= norm;
    }
    Ok(causal)
}

fn apply_kaiser(xs: &mut [f32], beta: f64) {
    let n = xs.len();
    if n <= 2 {
        return;
    }
    // kaiser[i] = I0(β·sqrt(1 - (2i/(N-1)-1)^2)) / I0(β)
    let inv_denom = 1.0 / bessel_i0(beta);
    for (i, x) in xs.iter_mut().enumerate() {
        let t = (2.0 * i as f64) / ((n - 1) as f64) - 1.0;
        let arg = beta * (1.0 - t * t).sqrt().max(0.0);
        let w = (bessel_i0(arg) * inv_denom) as f32;
        *x *= w;
    }
}

/// Scalar approximation of the 0th-order modified Bessel I₀(x),
/// Horner'd Taylor series up to x^22; error < 1e-8 for x ≤ 15 which
/// comfortably covers Kaiser β ∈ [0, 10].
fn bessel_i0(x: f64) -> f64 {
    let x2 = x * x;
    let mut term = 1.0f64;
    let mut sum = 1.0f64;
    for k in 1..30 {
        term *= x2 / (4.0 * (k as f64) * (k as f64));
        sum += term;
        if term < 1e-15 {
            break;
        }
    }
    sum
}
