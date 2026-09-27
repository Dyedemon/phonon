//! Concurrency stress test for `PeqProcessor`.
//!
//! Spec Task 2.5 / AC-R12 / NFR1 / AC-U2.
//!
//! Runs 2 writer threads hammering `PeqUpdateHandle::update_bands` 4 M times
//! each while 1 DSP thread runs `process()` 4 M times. Verifies:
//!   - version monotonicity (DSP never observes a stale version)
//!   - P99.9 process() latency < 300 µs
//!   - zero panics
//!   - `Arc::strong_count` of last_applied ≤ 3
//!
//! Marked `#[ignore]` so default `cargo test` skips it; run on demand with:
//!   cargo test -p phonon-core --test calibration_stress -- --ignored --nocapture
//!
//! Expected duration: <2 min on Intel Ultra 7 155H.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use phonon_core::peq::{PeqProcessor, PeqUpdateError};
use phonon_core::dsp::DspProcessor;
use phonon_core::eq::{EqBand, FilterType};

/// Builds a valid `EqBand` peaking filter at a given centre frequency.
fn band(freq: f32, gain_db: f32) -> EqBand {
    EqBand {
        frequency: freq,
        gain_db,
        q: 1.0,
        filter_type: FilterType::Peaking,
    }
}

#[ignore]
#[test]
fn peq_write_and_dsp_spin_concurrent() {
    const WRITES: u64 = 4_000_000;
    const PROCS: u64 = 4_000_000;
    const FRAMES: usize = 128;
    const CH: u16 = 2;
    const SR: u32 = 48_000;

    let (proc, handle) = PeqProcessor::create(SR);
    let proc = Arc::new(proc);
    let handle = Arc::new(handle);

    let last_seen = Arc::new(AtomicU64::new(0));
    let nonmono = Arc::new(AtomicU64::new(0));
    let max_ns = Arc::new(AtomicU64::new(0));
    let p999_ns = Arc::new(AtomicU64::new(0));

    // Pre-build a small set of band lists. We rotate through these to give
    // the biquad design code something to do without allocating per call.
    let band_set_l: Vec<Vec<EqBand>> = (0..8)
        .map(|i| {
            vec![
                band(100.0, 3.0 + 0.1 * i as f32),
                band(1_000.0, -1.5 + 0.05 * i as f32),
                band(5_000.0, 2.0 - 0.1 * i as f32),
            ]
        })
        .collect();
    let band_set_r: Vec<Vec<EqBand>> = (0..8)
        .map(|i| {
            vec![
                band(120.0, 2.8 + 0.1 * i as f32),
                band(900.0, -1.2 + 0.05 * i as f32),
                band(6_000.0, 1.8 - 0.1 * i as f32),
            ]
        })
        .collect();

    // ── Writer 1: left-channel flavour ───────────────────────────────────
    let h1 = handle.clone();
    let band_set_l_w1 = band_set_l.clone();
    let band_set_r_w1 = band_set_r.clone();
    let w1 = std::thread::spawn(move || {
        let mut channel_full_count: u64 = 0;
        let mut ok_count: u64 = 0;
        for i in 0..WRITES {
            let idx = (i % 8) as usize;
            let res = h1.update_bands(
                band_set_l_w1[idx].clone(),
                band_set_r_w1[idx].clone(),
                SR,
            );
            match res {
                Ok(()) => ok_count += 1,
                Err(PeqUpdateError::ChannelFull) => channel_full_count += 1,
                Err(PeqUpdateError::ChannelClosed) => break,
                Err(other) => panic!("writer 1 unexpected error: {other:?}"),
            }
            // Occasional yield to avoid fully starving the DSP thread on
            // single-core CI runners. NOT a sleep — just a hint.
            if i % 1024 == 0 {
                std::thread::yield_now();
            }
        }
        (ok_count, channel_full_count)
    });

    // ── Writer 2: right-channel flavour (independent set) ────────────────
    let h2 = handle.clone();
    let band_set_l_w2 = band_set_l.clone();
    let band_set_r_w2 = band_set_r.clone();
    let w2 = std::thread::spawn(move || {
        let mut channel_full_count: u64 = 0;
        let mut ok_count: u64 = 0;
        for i in 0..WRITES {
            let idx = (i % 8) as usize;
            // rotate the bands slightly differently so snapshots differ
            let mut l = band_set_l_w2[(idx + 3) % 8].clone();
            let mut r = band_set_r_w2[(idx + 5) % 8].clone();
            for b in &mut l {
                b.gain_db += 0.01;
            }
            for b in &mut r {
                b.gain_db -= 0.01;
            }
            let res = h2.update_bands(l, r, SR);
            match res {
                Ok(()) => ok_count += 1,
                Err(PeqUpdateError::ChannelFull) => channel_full_count += 1,
                Err(PeqUpdateError::ChannelClosed) => break,
                Err(other) => panic!("writer 2 unexpected error: {other:?}"),
            }
            if i % 1024 == 0 {
                std::thread::yield_now();
            }
        }
        (ok_count, channel_full_count)
    });

    // ── DSP thread: hammer process() and measure latency ────────────────
    let proc_dsp = proc.clone();
    let last_seen_d = last_seen.clone();
    let nonmono_d = nonmono.clone();
    let max_ns_d = max_ns.clone();
    let p999_ns_d = p999_ns.clone();
    let dsp = std::thread::spawn(move || {
        let mut buf = vec![0.0f32; FRAMES * CH as usize];
        // Fill with a small test signal so the biquads actually do work.
        for (i, s) in buf.iter_mut().enumerate() {
            *s = ((i as f32) * 0.001).sin() * 0.5;
        }
        let mut hist: Vec<u64> = Vec::with_capacity(10_000);
        for i in 0..PROCS {
            let t = Instant::now();
            proc_dsp.process(&mut buf, CH, SR);
            let d = t.elapsed().as_nanos() as u64;
            if hist.len() < 10_000 {
                hist.push(d);
            }
            let prev_max = max_ns_d.load(Ordering::Relaxed);
            if d > prev_max {
                max_ns_d.store(d, Ordering::Relaxed);
            }
            // Read the version of the snapshot the DSP just used.
            // Must be monotonic non-decreasing across calls.
            let v = proc_dsp.current_version();
            let prev = last_seen_d.swap(v, Ordering::AcqRel);
            if v < prev {
                nonmono_d.fetch_add(1, Ordering::Relaxed);
            }
            // Occasional yield so writers can push.
            if i % 64 == 0 {
                std::thread::yield_now();
            }
        }
        // Compute P99.9.
        hist.sort_unstable();
        if !hist.is_empty() {
            let idx = ((hist.len() as f64) * 0.999) as usize;
            let idx = idx.min(hist.len() - 1);
            p999_ns_d.store(hist[idx], Ordering::Relaxed);
        }
    });

    let (w1_ok, w1_full) = w1.join().expect("writer 1 panicked");
    let (w2_ok, w2_full) = w2.join().expect("writer 2 panicked");
    dsp.join().expect("dsp thread panicked");

    // ── Assertions (AC-R12) ──────────────────────────────────────────────
    let nonmono_count = nonmono.load(Ordering::Relaxed);
    let max_ns_val = max_ns.load(Ordering::Relaxed);
    let p999_ns_val = p999_ns.load(Ordering::Relaxed);
    let p999_us = p999_ns_val as f64 / 1000.0;
    let max_us = max_ns_val as f64 / 1000.0;

    println!("\n=== calibration_stress results ===");
    println!("writer1: ok={w1_ok} full={w1_full} (total {WRITES})");
    println!("writer2: ok={w2_ok} full={w2_full} (total {WRITES})");
    println!("dsp process() calls: {PROCS}");
    println!("non-monotonic version reads: {nonmono_count}");
    println!("max process() latency: {max_us:.2} µs");
    println!("P99.9 process() latency: {p999_us:.2} µs");

    assert_eq!(nonmono_count, 0, "version non-monotonic {nonmono_count} times");
    assert!(p999_us < 300.0, "P99.9 {p999_us:.2} µs > 300 µs");

    // strong_count of last_applied should be small (≤3: handle.last_applied +
    // possibly proc.active + possibly a transient in the channel).
    if let Some(snap) = handle.last_applied_snapshot() {
        let count = Arc::strong_count(&snap);
        println!("Arc::strong_count(last_applied) = {count}");
        assert!(count <= 3, "Arc strong_count {count} > 3 (leak)");
    }
}
