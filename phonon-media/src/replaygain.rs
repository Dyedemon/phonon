//! ReplayGain batch scanning — Mode A (sidecar SQLite, no file modification).
//!
//! Implements ITU-R BS.1770-4 loudness measurement (K-weighting + gated
//! block mean) to compute ReplayGain 2.0 track gain/peak. Results are stored
//! in a sidecar SQLite database (`.phonon_data/replaygains.sqlite`) so the
//! original audio files are never modified. Computed values are also mirrored
//! back into the main `tracks` table for convenient querying.
//!
//! # Algorithm
//! 1. Decode the entire file via `phonon_codec::AudioDecoder` (streamed, not
//!    buffered in full — K-weighting is applied per-frame).
//! 2. Apply the two-stage K-weighting filter (pre-filter high-shelf + RLB
//!    high-pass) to each channel independently. Coefficients are derived from
//!    the ITU-R BS.1770-4 anchor frequencies (1681.97 Hz / 38.14 Hz) using
//!    the RBJ Audio EQ Cookbook formulas, so they adapt to any sample rate.
//! 3. Partition the weighted samples into 400 ms blocks with 75% overlap
//!    (100 ms hop). Compute the mean square of each block across all channels.
//! 4. Gating: discard blocks below -70 LUFS (absolute gate), then discard
//!    blocks below (mean_of_surviving − 10) LUFS (relative gate).
//! 5. The gated mean square → LUFS = −0.691 + 10·log10(gated_ms).
//! 6. ReplayGain track gain = −18 − lufs (dB). Peak = max|sample| (0–1).
//!
//! # Compliance
//! No third-party metadata API is used. The audio is decoded locally and the
//! gain is computed purely from the PCM stream. Mode B (write-back to Vorbis
//! Comments) is deliberately NOT implemented here — it requires explicit user
//! confirmation in the UI and is deferred to a future task.

use std::path::Path;

use rusqlite::Connection;

use crate::error::Result;
use crate::schema;

/// ReplayGain 2.0 reference loudness (LUFS). Track gain is relative to this.
const REFERENCE_LUFS: f64 = -18.0;

/// Absolute gating threshold (LUFS). Blocks quieter than this are discarded.
const ABSOLUTE_GATE_LUFS: f64 = -70.0;

/// Relative gating threshold (LU). Blocks more than 10 LU below the
/// absolute-gated mean are discarded.
const RELATIVE_GATE_LU: f64 = -10.0;

/// Block length in milliseconds (ITU-R BS.1770-4 mandates 400 ms).
const BLOCK_MS: f64 = 400.0;

/// Block overlap: 75% → hop = 25% of block length = 100 ms.
const HOP_MS: f64 = 100.0;

/// Computed ReplayGain values for a single track.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReplayGainResult {
    /// Track gain in dB (relative to -18 LUFS).
    pub track_gain: f64,
    /// Track peak amplitude (0.0–1.0).
    pub track_peak: f64,
    /// Measured loudness in LUFS.
    pub lufs: f64,
}

// ---------------------------------------------------------------------------
// K-weighting biquad filter (ITU-R BS.1770-4, two cascaded stages).
// ---------------------------------------------------------------------------

/// A Direct-Form-II Transposed biquad with f64 precision.
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    /// RBJ Audio EQ Cookbook high-shelf coefficients.
    fn high_shelf(fs: f64, f0: f64, gain_db: f64, q: f64) -> Self {
        let a = 10.0_f64.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f64::consts::PI * f0 / fs;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q);
        let sq_a = a.sqrt();

        let b0 = a * ((a + 1.0) + (a - 1.0) * cos_w0 + 2.0 * sq_a * alpha);
        let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) + (a - 1.0) * cos_w0 - 2.0 * sq_a * alpha);
        let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + 2.0 * sq_a * alpha;
        let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) - (a - 1.0) * cos_w0 - 2.0 * sq_a * alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// RBJ Audio EQ Cookbook high-pass coefficients.
    fn high_pass(fs: f64, f0: f64, q: f64) -> Self {
        let w0 = 2.0 * std::f64::consts::PI * f0 / fs;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q);

        let b0 = (1.0 + cos_w0) / 2.0;
        let b1 = -(1.0 + cos_w0);
        let b2 = (1.0 + cos_w0) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// Process a single sample (DF-II Transposed).
    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Two-stage K-weighting filter (pre-filter + RLB) for one channel.
struct KWeight {
    stage1: Biquad,
    stage2: Biquad,
}

impl KWeight {
    /// Build the K-weighting filter pair for the given sample rate.
    ///
    /// Anchor frequencies per ITU-R BS.1770-4 (48 kHz reference, recomputed
    /// for the actual `fs` via the RBJ cookbook so any sample rate works).
    fn new(fs: f64) -> Self {
        Self {
            // Pre-filter: high-shelf, +4 dB at 1681.97 Hz.
            stage1: Biquad::high_shelf(
                fs,
                1681.974450955533,
                3.999843853973347,
                0.7071752369554196,
            ),
            // RLB: high-pass at 38.14 Hz, Q = 0.5.
            stage2: Biquad::high_pass(fs, 38.13547087602444, 0.500327037323877),
        }
    }

    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        self.stage2.process(self.stage1.process(x))
    }
}

// ---------------------------------------------------------------------------
// Gated block loudness accumulator.
// ---------------------------------------------------------------------------

/// Streaming accumulator for ITU-R BS.1770-4 gated loudness.
///
/// Feeds K-weighted samples per channel; internally partitions into 400 ms
/// blocks (75% overlap) and collects block mean-squares for two-stage gating.
struct LoudnessMeter {
    channels: usize,
    /// Samples per 400 ms block.
    block_len: usize,
    /// Samples per 100 ms hop.
    hop_len: usize,
    /// Ring buffer of weighted samples, per channel (length = block_len).
    blocks: Vec<Vec<f64>>,
    /// Current write position within the block.
    pos: usize,
    /// Number of complete blocks emitted.
    blocks_emitted: usize,
    /// Mean-square of each emitted block.
    block_ms: Vec<f64>,
}

impl LoudnessMeter {
    fn new(channels: usize, fs: f64) -> Self {
        // Guard against corrupt decoder metadata reporting fs = 0, which
        // would make block_len 0 and divide by zero in emit_block.
        let fs = if fs > 0.0 { fs } else { 48000.0 };
        let block_len = (fs * BLOCK_MS / 1000.0).round() as usize;
        let hop_len = (fs * HOP_MS / 1000.0).round() as usize;
        Self {
            channels,
            block_len,
            hop_len,
            blocks: vec![Vec::with_capacity(block_len); channels],
            pos: 0,
            blocks_emitted: 0,
            block_ms: Vec::new(),
        }
    }

    /// Push one deinterleaved frame (one sample per channel, already K-weighted).
    fn push_frame(&mut self, channel_samples: &[f64]) {
        debug_assert_eq!(channel_samples.len(), self.channels);
        for (ch, &s) in channel_samples.iter().enumerate() {
            self.blocks[ch].push(s);
        }
        self.pos += 1;

        // Emit a block when we've filled block_len samples.
        if self.pos >= self.block_len {
            self.emit_block();
        }
    }

    /// Compute the mean-square of the current block and emit it, then slide
    /// the window forward by `hop_len` samples.
    fn emit_block(&mut self) {
        // ITU-R BS.1770-4: block energy = Σ_ch (mean square of channel ch).
        // The channel energies are SUMMED, not averaged — dividing by the
        // channel count here under-reported stereo loudness by ~3 dB.
        // (Unknown layouts: every channel is weighted equally; BS.1770's
        // surround-surfaces/LFE-exclusion rules need positional metadata.)
        let mut sum_sq: f64 = 0.0;
        for block in &self.blocks {
            for &s in block {
                sum_sq += s * s;
            }
        }
        let ms = sum_sq / self.block_len as f64;
        self.block_ms.push(ms);
        self.blocks_emitted += 1;

        // Slide: drop the first `hop_len` samples from each channel.
        let drop = self.hop_len.min(self.pos);
        for block in &mut self.blocks {
            block.drain(0..drop);
        }
        self.pos -= drop;
    }

    /// Finalize and compute the gated LUFS. Must be called after all frames
    /// are pushed. Flushes any trailing partial block (only if ≥ 50% full,
    /// per the ITU tolerance for the final block).
    fn finalize(&mut self) -> f64 {
        // Flush trailing partial block if at least half-full.
        if self.pos >= self.block_len / 2 {
            self.emit_block();
        }

        if self.block_ms.is_empty() {
            return ABSOLUTE_GATE_LUFS;
        }

        // Stage 1: absolute gate at -70 LUFS.
        let absolute_gate_ms = 10.0_f64.powf((ABSOLUTE_GATE_LUFS + 0.691) / 10.0);
        let surviving: Vec<f64> = self
            .block_ms
            .iter()
            .copied()
            .filter(|&ms| ms > absolute_gate_ms)
            .collect();
        if surviving.is_empty() {
            return ABSOLUTE_GATE_LUFS;
        }

        // Relative gate = mean_of_surviving - 10 LU.
        let mean_ms: f64 = surviving.iter().sum::<f64>() / surviving.len() as f64;
        let mean_lufs = -0.691 + 10.0 * mean_ms.log10();
        let relative_gate_lufs = mean_lufs + RELATIVE_GATE_LU;
        let relative_gate_ms = 10.0_f64.powf((relative_gate_lufs + 0.691) / 10.0);

        // Stage 2: relative gate.
        let gated: Vec<f64> = surviving
            .into_iter()
            .filter(|ms| *ms > relative_gate_ms)
            .collect();
        if gated.is_empty() {
            return mean_lufs;
        }
        let gated_mean: f64 = gated.iter().sum::<f64>() / gated.len() as f64;
        -0.691 + 10.0 * gated_mean.log10()
    }
}

// ---------------------------------------------------------------------------
// Public API.
// ---------------------------------------------------------------------------

/// Compute ReplayGain track gain and peak for a single audio file.
///
/// Decodes the file via `phonon_codec`, applies K-weighting per channel, and
/// measures gated loudness. Returns `Err` if the file cannot be decoded.
pub fn calculate_replaygain(path: &Path) -> Result<ReplayGainResult> {
    use phonon_codec::decoder::AudioDecoder;

    let mut decoder = AudioDecoder::open(path)?;
    let channels = decoder.channels() as usize;
    let fs = decoder.sample_rate() as f64;

    // One K-weight filter per channel.
    let mut filters: Vec<KWeight> = (0..channels).map(|_| KWeight::new(fs)).collect();
    let mut meter = LoudnessMeter::new(channels, fs);
    let mut peak: f64 = 0.0;
    // Scratch buffer for the deinterleaved frame — hoisted out of the hot
    // loop (a per-frame heap alloc here used to dominate scan time).
    let mut weighted: Vec<f64> = vec![0.0; channels];

    #[allow(clippy::while_let_loop)]
    loop {
        let frame = match decoder.next_frame()? {
            Some(f) => f,
            None => break,
        };
        // `frame.samples` is interleaved f32: [ch0, ch1, ch2, ..., ch0, ch1, ...]
        let n_frames = frame.samples.len() / channels;
        for chunk in frame.samples.chunks_exact(channels).take(n_frames) {
            for (ch, &s_f32) in chunk.iter().enumerate() {
                let s = s_f32 as f64;
                let abs_s = s.abs();
                if abs_s > peak {
                    peak = abs_s;
                }
                weighted[ch] = filters[ch].process(s);
            }
            meter.push_frame(&weighted);
        }
    }

    let lufs = meter.finalize();
    let track_gain = REFERENCE_LUFS - lufs;

    Ok(ReplayGainResult {
        track_gain,
        track_peak: peak,
        lufs,
    })
}

// ---------------------------------------------------------------------------
// Sidecar SQLite storage (Mode A).
// ---------------------------------------------------------------------------

/// Open (or create) the sidecar ReplayGain database at `db_path`.
///
/// Schema:
/// ```sql
/// CREATE TABLE replaygains (
///     file_path   TEXT PRIMARY KEY,
///     track_gain  REAL NOT NULL,
///     track_peak  REAL NOT NULL,
///     lufs        REAL NOT NULL,
///     scanned_at  INTEGER NOT NULL
/// );
/// ```
pub fn open_sidecar(db_path: &Path) -> Result<Connection> {
    if let Some(parent) = db_path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let conn = Connection::open(db_path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS replaygains (
            file_path   TEXT PRIMARY KEY,
            track_gain  REAL NOT NULL,
            track_peak  REAL NOT NULL,
            lufs        REAL NOT NULL,
            scanned_at  INTEGER NOT NULL
        )",
    )?;
    Ok(conn)
}

/// Upsert a computed ReplayGain result into the sidecar DB.
pub fn store_sidecar(conn: &Connection, file_path: &str, r: &ReplayGainResult) -> Result<()> {
    let now = schema::now_ts();
    conn.execute(
        "INSERT INTO replaygains (file_path, track_gain, track_peak, lufs, scanned_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(file_path) DO UPDATE SET
            track_gain = excluded.track_gain,
            track_peak = excluded.track_peak,
            lufs = excluded.lufs,
            scanned_at = excluded.scanned_at",
        rusqlite::params![file_path, r.track_gain, r.track_peak, r.lufs, now],
    )?;
    Ok(())
}

/// Read a cached ReplayGain result from the sidecar DB.
pub fn read_sidecar(conn: &Connection, file_path: &str) -> Result<Option<ReplayGainResult>> {
    let row = conn
        .query_row(
            "SELECT track_gain, track_peak, lufs FROM replaygains WHERE file_path = ?1",
            rusqlite::params![file_path],
            |r| {
                Ok(ReplayGainResult {
                    track_gain: r.get(0)?,
                    track_peak: r.get(1)?,
                    lufs: r.get(2)?,
                })
            },
        )
        .ok();
    Ok(row)
}

// ---------------------------------------------------------------------------
// Unit tests.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// BS.1770-4 calibration: a 1 kHz sine at 0 dBFS in a single channel
    /// reads −3.01 LKFS, so a dual-mono stereo sine at −20 dBFS reads
    /// ≈ −20.0 LUFS (channel energies summed; ±0.5 covers the small RBJ
    /// vs. prototype K-filter response difference at 1 kHz). The pre-fix
    /// per-channel averaging read ≈ −23.0 for this signal.
    #[test]
    fn loudness_meter_stereo_sine_matches_bs1770_calibration() {
        let fs = 48000.0;
        let channels = 2;
        let mut meter = LoudnessMeter::new(channels, fs);
        let mut filters: Vec<KWeight> = (0..channels).map(|_| KWeight::new(fs)).collect();

        // 2 seconds of 1 kHz sine at -20 dBFS (amplitude 0.1), identical in
        // both channels.
        let n = (fs * 2.0) as usize;
        for i in 0..n {
            let t = i as f64 / fs;
            let s = 0.1 * (2.0 * std::f64::consts::PI * 1000.0 * t).sin();
            let weighted: Vec<f64> = filters.iter_mut().map(|f| f.process(s)).collect();
            meter.push_frame(&weighted);
        }
        let lufs = meter.finalize();
        assert!(lufs.is_finite(), "LUFS must be finite, got {lufs}");
        assert!(
            (-21.0..=-19.0).contains(&lufs),
            "dual-mono stereo -20 dBFS sine should read ≈ -20.0 LUFS, got {lufs}"
        );
    }

    /// Dual-mono stereo measures exactly +3.01 dB louder than the same
    /// signal in a single channel (BS.1770 sums channel energies).
    #[test]
    fn loudness_meter_mono_vs_stereo_sum_relationship() {
        let fs = 48000.0;
        let n = (fs * 2.0) as usize;
        let sample = |i: usize| {
            let t = i as f64 / fs;
            0.1 * (2.0 * std::f64::consts::PI * 1000.0 * t).sin()
        };

        let run = |channels: usize| {
            let mut meter = LoudnessMeter::new(channels, fs);
            let mut filters: Vec<KWeight> = (0..channels).map(|_| KWeight::new(fs)).collect();
            for i in 0..n {
                let s = sample(i);
                let weighted: Vec<f64> = filters.iter_mut().map(|f| f.process(s)).collect();
                meter.push_frame(&weighted);
            }
            meter.finalize()
        };

        let mono_lufs = run(1);
        let stereo_lufs = run(2);
        assert!(
            (stereo_lufs - mono_lufs - 3.01).abs() < 0.3,
            "dual-mono stereo should be +3.01 dB louder than mono: mono={mono_lufs}, stereo={stereo_lufs}"
        );
    }

    /// Silence should hit the absolute gate (-70 LUFS).
    #[test]
    fn loudness_meter_silence_hits_absolute_gate() {
        let fs = 48000.0;
        let mut meter = LoudnessMeter::new(2, fs);
        let n = (fs * 2.0) as usize;
        for _ in 0..n {
            meter.push_frame(&[0.0, 0.0]);
        }
        let lufs = meter.finalize();
        assert!((lufs - ABSOLUTE_GATE_LUFS).abs() < 0.01);
    }

    /// The biquad must not blow up on extreme input.
    #[test]
    fn kweight_handles_full_scale_without_nan() {
        let mut kw = KWeight::new(48000.0);
        for i in 0..48000 {
            let s = if i % 2 == 0 { 1.0 } else { -1.0 };
            let y = kw.process(s);
            assert!(y.is_finite(), "K-weight output not finite at {i}");
        }
    }

    /// Sidecar DB round-trip.
    #[test]
    fn sidecar_roundtrip() {
        let path = std::env::temp_dir().join(format!(
            "phonon-rg-sidecar-test-{}.sqlite",
            std::process::id()
        ));
        {
            let conn = open_sidecar(&path).unwrap();
            let r = ReplayGainResult {
                track_gain: -6.5,
                track_peak: 0.95,
                lufs: -11.5,
            };
            store_sidecar(&conn, "/foo/bar.flac", &r).unwrap();
            let read = read_sidecar(&conn, "/foo/bar.flac").unwrap().unwrap();
            assert_eq!(read, r);

            // Upsert overwrites.
            let r2 = ReplayGainResult {
                track_gain: -3.0,
                track_peak: 0.8,
                lufs: -15.0,
            };
            store_sidecar(&conn, "/foo/bar.flac", &r2).unwrap();
            let read2 = read_sidecar(&conn, "/foo/bar.flac").unwrap().unwrap();
            assert_eq!(read2, r2);

            // Missing path → None.
            assert!(read_sidecar(&conn, "/missing").unwrap().is_none());
        }
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    /// ReplayGain gain = -18 - lufs.
    #[test]
    fn gain_conversion() {
        let r = ReplayGainResult {
            track_gain: 0.0,
            track_peak: 0.0,
            lufs: -20.0,
        };
        let gain = REFERENCE_LUFS - r.lufs; // -18 - (-20) = 2
        assert!((gain - (-18.0 - -20.0)).abs() < 0.001);
        assert!((gain - 2.0).abs() < 0.001);
    }
}
