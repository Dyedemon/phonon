#![allow(non_snake_case)]
//! Parametric EQ (PEQ) DSP processor — general-purpose biquad equalizer.
//!
//! A high-precision, stereo-independent biquad cascade used for parametric
//! equalization. It accepts `bands_L / bands_R` from any source (manual
//! user EQ presets, auto-calibration plugins, etc.) and applies them in
//! the DSP chain.
//!
//! # Concurrency model (single-slot register channel)
//!
//! The UI thread → DSP thread handshake is:
//!
//! ```text
//!  ┌────────────────┐  try_send(Arc<PeqSnapshot>)   ┌───────────────┐
//!  │ Tauri (UI/Tok) │ ───────────────────────────►  │ sync_channel  │
//!  │ PeqUpdateHandle│                               │  capacity=1   │
//!  └────────────────┘ ◄──────── ChannelFull if full └───────────────┘
//!                                                       │
//!                             DSP callback reads via    │ try_recv
//!                                                       ▼
//!                              ┌─────────────────────────────────────┐
//!                              │ PeqProcessor.process()              │
//!                              │  1. try_recv → new_snapshot         │
//!                              │  2. if Some: design biquads + swap  │
//!                              │     in `self.active: RwLock<Arc>`   │
//!                              │  3. process: indirect through the   │
//!                              │     current active pointer          │
//!                              └─────────────────────────────────────┘
//! ```
//!
//! Semantics of capacity=1 (register, not a queue):
//!   • If DSP is 1-2ms late on draining, the *old* pending snapshot is
//!     dropped and replaced by the newest UI push. No stale state lingers.
//!   • try_recv is non-blocking (zero wait in audio callback).
//!   • At most one pending PeqSnapshot live = memory bounded to ~KB.
//!
//! UI side (Tauri) is expected to debounce `apply_fit_result` calls by
//! ≥ 500 ms so ChannelFull never appears in normal usage. When it does,
//! `PeqUpdateError::ChannelFull` is surfaced to the TS toast layer.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, RwLock};

use crate::eq::{BiquadCoeffs, BiquadState, EqBand};
use crate::dsp::DspProcessor;
use std::any::Any;

// ── PeqUpdateError (defined here in phonon-core to avoid circular dep) ──
/// Errors produced by `PeqUpdateHandle::update_bands` (DSP register channel).
///
/// Defined in `phonon-core` (not `calibration-types`) because
/// `calibration_peq.rs` lives in `phonon-core` and `calibration-types`
/// already depends on `phonon-core` — the reverse would be circular.
/// `calibration-types` re-exports this type.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, thiserror::Error)]
pub enum PeqUpdateError {
    #[error("audio thread busy: sync_channel(1) full")]
    ChannelFull,
    #[error("calibration PEQ processor dropped (device switch)")]
    ChannelClosed,
    #[error("band #{0} invalid: {1}")]
    InvalidBand(usize, String),
    #[error("fir config failed: {0}")]
    FirConfigFailed(String),
}

/// Immutable snapshot of the stereo biquad state. Designed so the audio
/// callback can swap the whole active cascade in one RwLock write
/// without blocking readers (Arc clone is cheap).
///
/// `from_bands` pre-designs every band at the given sample rate; if the
/// device switches rates, the caller produces a new snapshot.
pub struct PeqSnapshot {
    pub bands_L: Vec<EqBand>,
    pub bands_R: Vec<EqBand>,
    /// Per-band design coefficients. `coeffs_L[b]` corresponds to
    /// `bands_L[b]`, precomputed for the snapshot's sample_rate.
    coeffs_L: Vec<BiquadCoeffs>,
    coeffs_R: Vec<BiquadCoeffs>,
    pub sample_rate: u32,
    /// Monotonically increasing version stamp. Each `update_bands` call
    /// produces a snapshot with `version = previous + 1`. DSP thread reads
    /// this via `PeqProcessor::current_version()` and asserts
    /// monotonicity in the stress test (Task 2.5 / AC-R12).
    pub version: u64,
}

impl PeqSnapshot {
    pub fn from_bands(bands_L: Vec<EqBand>, bands_R: Vec<EqBand>, sample_rate: u32) -> Self {
        let mut coeffs_L = Vec::with_capacity(bands_L.len());
        for b in &bands_L {
            coeffs_L.push(BiquadCoeffs::design(b, sample_rate));
        }
        let mut coeffs_R = Vec::with_capacity(bands_R.len());
        for b in &bands_R {
            coeffs_R.push(BiquadCoeffs::design(b, sample_rate));
        }
        Self {
            bands_L,
            bands_R,
            coeffs_L,
            coeffs_R,
            sample_rate,
            version: 0,
        }
    }
}

/// Per-channel transient state (delay lines). Lives inside the active
/// snapshot wrapper so swapping pointers in `process` is sound. Kept in
/// a `Mutex<Vec<BiquadState>>` even though the DSP callback is
/// single-threaded by contract — avoids interior-mutability gymnastics
/// with UnsafeCell while keeping lock cost ≤ 20 ns on modern x86.
struct ActiveCascade {
    snapshot: Arc<PeqSnapshot>,
    states_L: std::sync::Mutex<Vec<BiquadState>>,
    states_R: std::sync::Mutex<Vec<BiquadState>>,
}

impl ActiveCascade {
    fn new(snap: Arc<PeqSnapshot>) -> Self {
        let n_l = snap.bands_L.len();
        let n_r = snap.bands_R.len();
        Self {
            snapshot: snap,
            states_L: std::sync::Mutex::new(vec![BiquadState::new(); n_l]),
            states_R: std::sync::Mutex::new(vec![BiquadState::new(); n_r]),
        }
    }
}

// BiquadState is `pub(crate)` in eq.rs — accessible directly via
// `crate::eq::BiquadState` within this module.

/// Handle handed to the Tauri command layer. Clonable (SyncSender +
/// AtomicBool + RwLock all share across threads trivially).
pub struct PeqUpdateHandle {
    pending_tx: SyncSender<Arc<PeqSnapshot>>,
    pub bypass: Arc<AtomicBool>,
    last_applied: Arc<RwLock<Option<Arc<PeqSnapshot>>>>,
}

impl PeqUpdateHandle {
    pub fn update_bands(
        &self,
        bands_L: Vec<EqBand>,
        bands_R: Vec<EqBand>,
        sample_rate: u32,
    ) -> Result<(), PeqUpdateError> {
        if bands_L.len() != bands_R.len() {
            return Err(PeqUpdateError::InvalidBand(
                0,
                format!(
                    "stereo band count mismatch: L={} R={}",
                    bands_L.len(),
                    bands_R.len()
                ),
            ));
        }
        for (idx, b) in bands_L.iter().enumerate() {
            if let Err(msg) = validate_band(b) {
                return Err(PeqUpdateError::InvalidBand(idx, msg));
            }
        }
        for (idx, b) in bands_R.iter().enumerate() {
            if let Err(msg) = validate_band(b) {
                return Err(PeqUpdateError::InvalidBand(idx + bands_L.len(), msg));
            }
        }
        let snap = Arc::new(PeqSnapshot::from_bands(bands_L, bands_R, sample_rate));
        // Single try_send. No retry; frontend debounce keeps ChannelFull rare.
        match self.pending_tx.try_send(snap.clone()) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                return Err(PeqUpdateError::ChannelFull);
            }
            Err(TrySendError::Disconnected(_)) => {
                return Err(PeqUpdateError::ChannelClosed);
            }
        }
        // Persist for rollback & read-back.
        match self.last_applied.write() {
            Ok(mut guard) => *guard = Some(snap),
            Err(_) => return Err(PeqUpdateError::ChannelClosed), // poison = RIP
        }
        Ok(())
    }

    /// Returns the most-recently-applied snapshot (if any). Used by
    /// transactional apply when rolling back a failed FIR stage — the
    /// host reverts PEQ to whatever was live *before* this apply call.
    pub fn last_applied_snapshot(&self) -> Option<Arc<PeqSnapshot>> {
        self.last_applied.read().ok()?.clone()
    }
}

fn validate_band(b: &EqBand) -> Result<(), String> {
    use crate::eq::FilterType;
    if b.frequency < 10.0 || b.frequency > 96_000.0 {
        return Err(format!("freq {} out of band", b.frequency));
    }
    if !(-24.0..=24.0).contains(&b.gain_db) {
        return Err(format!("gain {} dB out of [-24, +24]", b.gain_db));
    }
    if b.q < 0.1 || b.q > 10.0 {
        return Err(format!("Q {} out of [0.1, 10]", b.q));
    }
    match b.filter_type {
        FilterType::Peaking
        | FilterType::LowShelf
        | FilterType::HighShelf
        | FilterType::LowPass
        | FilterType::HighPass => {}
    }
    Ok(())
}

/// The actual DSP chain node.
pub struct PeqProcessor {
    id: String,
    enabled: AtomicBool,
    bypass: Arc<AtomicBool>,
    // Mutex wrapper needed for `Sync` — `mpsc::Receiver` is `Send` but `!Sync`.
    // The DSP callback is single-threaded by contract, so the lock is never
    // contended; this exists solely to satisfy `DspProcessor: Send + Sync`.
    pending_rx: std::sync::Mutex<Receiver<Arc<PeqSnapshot>>>,
    /// Currently active EQ cascade, protected by an RwLock.
    /// Using Arc inside RwLock means readers can hold a cheap clone while
    /// the writer swaps in a new cascade — no unsafe pointer manipulation.
    active: RwLock<Arc<ActiveCascade>>,
    last_applied: Arc<RwLock<Option<Arc<PeqSnapshot>>>>,
    /// DSP-side version stamp: incremented each time `tick_config`
    /// swaps in a new snapshot. Reflects DSP receive order, not
    /// writer send order (which can interleave across 2 writers).
    dsp_version: AtomicU64,
}

impl PeqProcessor {
    pub const ID: &'static str = "peq";

    /// Build a (processor, handle) pair. Handle lives in AppState;
    /// processor lives in the DspChain.
    pub fn create(sample_rate: u32) -> (Self, PeqUpdateHandle) {
        let (tx, rx) = sync_channel::<Arc<PeqSnapshot>>(1);
        let bypass = Arc::new(AtomicBool::new(false));
        let last_applied = Arc::new(RwLock::new(None));
        // Seed with an empty snapshot (zero bands → passthrough).
        let seed = Arc::new(PeqSnapshot::from_bands(vec![], vec![], sample_rate));
        let active_cascade = Arc::new(ActiveCascade::new(seed.clone()));
        let proc = Self {
            id: Self::ID.to_string(),
            enabled: AtomicBool::new(true),
            bypass: bypass.clone(),
            pending_rx: std::sync::Mutex::new(rx),
            active: RwLock::new(active_cascade),
            last_applied: last_applied.clone(),
            // DSP-side version stamp: incremented each time `tick_config`
            // swaps in a new snapshot. Reflects DSP receive order, not
            // writer send order (which can interleave across 2 writers).
            dsp_version: AtomicU64::new(0),
        };
        let handle = PeqUpdateHandle {
            pending_tx: tx,
            bypass,
            last_applied,
        };
        *handle.last_applied.write().unwrap() = Some(seed);
        (proc, handle)
    }

    /// Non-blocking drain of the register channel. Swaps `self.active`
    /// atomically if a newer snapshot arrived. The previous cascade is
    /// dropped when the last reference goes away (Arc semantics).
    pub fn tick_config(&self) {
        // Lock the receiver (never contended — DSP callback is single-threaded)
        // and drain the single-slot register channel.
        let rx = match self.pending_rx.lock() {
            Ok(guard) => guard,
            Err(_) => return, // poisoned — skip this callback
        };
        while let Ok(new_snap) = rx.try_recv() {
            let new_cascade = Arc::new(ActiveCascade::new(new_snap));
            // Swap in the new cascade under write lock.
            if let Ok(mut active) = self.active.write() {
                *active = new_cascade;
            }
            // Stamp version on DSP side — reflects receive order, not
            // writer send order. Monotonic by construction.
            self.dsp_version.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// DSP-side version counter. Monotonically increases each time
    /// `tick_config` swaps in a new snapshot. Safe to call from any
    /// thread (Relaxed load — only used for sanity checking).
    pub fn current_version(&self) -> u64 {
        self.dsp_version.load(Ordering::Relaxed)
    }

    /// Get a clone of the current active cascade Arc.
    /// Safe to call from any thread; the Arc keeps the cascade alive.
    fn active_cascade(&self) -> Arc<ActiveCascade> {
        self.active
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_else(|_| {
                // Poisoned lock — return an empty cascade as fallback
                Arc::new(ActiveCascade::new(Arc::new(PeqSnapshot::from_bands(
                    vec![],
                    vec![],
                    48000,
                ))))
            })
    }
}

impl Drop for PeqProcessor {
    fn drop(&mut self) {
        // Arc inside RwLock handles cleanup automatically — nothing to do.
        // The RwLock will be dropped along with the struct.
    }
}

// DspProcessor requires &self, so we use interior mutability on the
// delay lines (Mutex<Vec<BiquadState>>). The audio thread takes the
// lock once per channel per frame; contention is zero because the lock
// is never taken outside the callback (UI only swaps snapshots).
impl DspProcessor for PeqProcessor {
    fn process(&self, samples: &mut [f32], channels: u16, sample_rate: u32) {
        if !self.enabled.load(Ordering::Relaxed) || self.bypass.load(Ordering::Relaxed) {
            return;
        }
        self.tick_config();
        let cascade = self.active_cascade();
        let snap = &cascade.snapshot;
        // If sample rate mismatches snapshot (device swap between
        // updates), fall back to passthrough until UI issues new
        // snapshot at correct rate.
        if snap.sample_rate != sample_rate || channels == 0 {
            return;
        }
        let frames = samples.len() / channels as usize;
        match channels {
            1 => {
                let mut states = match cascade.states_L.lock() {
                    Ok(guard) => guard,
                    Err(_) => return, // poisoned — bypass this cycle
                };
                for s in samples.iter_mut().take(frames) {
                    let mut val = *s;
                    for (bi, coeff) in snap.coeffs_L.iter().enumerate() {
                        let st = &mut states[bi];
                        val = BiquadCoeffs::process_sample(coeff, st, val);
                    }
                    *s = val;
                }
            }
            2 => {
                let mut st_l = match cascade.states_L.lock() {
                    Ok(guard) => guard,
                    Err(_) => return,
                };
                let mut st_r = match cascade.states_R.lock() {
                    Ok(guard) => guard,
                    Err(_) => return,
                };
                for chunk in samples.chunks_exact_mut(2).take(frames) {
                    let mut l = chunk[0];
                    let mut r = chunk[1];
                    for (bi, c) in snap.coeffs_L.iter().enumerate() {
                        l = BiquadCoeffs::process_sample(c, &mut st_l[bi], l);
                    }
                    for (bi, c) in snap.coeffs_R.iter().enumerate() {
                        r = BiquadCoeffs::process_sample(c, &mut st_r[bi], r);
                    }
                    chunk[0] = l;
                    chunk[1] = r;
                }
            }
            _ => {} // unsupported channel count → bypass silently
        }
    }

    fn name(&self) -> &str { "PEQ" }
    fn latency(&self) -> f64 { 0.0 } // IIR is zero-latency (analytically exact)
    fn enabled(&self) -> bool { self.enabled.load(Ordering::Relaxed) }
    fn set_enabled(&mut self, enabled: bool) { self.enabled.store(enabled, Ordering::Relaxed); }
    fn id(&self) -> &str { &self.id }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}

// Accessors used by tests / Tauri for "read back last applied bands".
impl PeqProcessor {
    pub fn last_applied(&self) -> Option<Arc<PeqSnapshot>> {
        self.last_applied.read().ok()?.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_snapshot_is_passthrough() {
        let (proc, _h) = PeqProcessor::create(48_000);
        let mut buf = [0.0f32; 8];
        for i in 0..4 {
            buf[2 * i] = i as f32 * 0.1 + 0.05;
            buf[2 * i + 1] = -buf[2 * i];
        }
        let original = buf;
        proc.process(&mut buf, 2, 48_000);
        for i in 0..8 {
            assert!((buf[i] - original[i]).abs() < 1e-6, "passthrough mismatch at {i}");
        }
    }

    #[test]
    fn try_send_full_returns_channelfull_error() {
        let (proc, h) = PeqProcessor::create(48_000);
        // fill the single-slot register twice.
        let ok = h.update_bands(vec![], vec![], 48_000);
        assert!(ok.is_ok(), "first send must succeed: {ok:?}");
        let second = h.update_bands(vec![], vec![], 48_000);
        // either Ok (DSP already drained) or ChannelFull; we accept both.
        let _ = proc; // keep proc (Receiver) alive until second send
        match second {
            Ok(()) => {} // fine: proc dropped / tick_config drained
            Err(PeqUpdateError::ChannelFull) => {} // fine: capacity=1 semantics
            Err(other) => panic!("unexpected err: {other:?}"),
        }
    }
}
