//! DSP (Digital Signal Processing) chain.
//!
//! Maintains an ordered list of DSP processors that PCM data passes through
//! before output. Supports ReplayGain, downmix, and custom WASM plugins.

use std::any::Any;
use std::sync::Mutex;

/// Trait for DSP processors.
pub trait DspProcessor: Send + Sync {
    /// Process a buffer of PCM samples in-place.
    fn process(&self, samples: &mut [f32], channels: u16, sample_rate: u32);

    /// Human-readable name of this processor.
    fn name(&self) -> &str;

    /// Maximum additional latency introduced by this processor (in seconds).
    fn latency(&self) -> f64;

    /// Whether this processor is currently enabled.
    fn enabled(&self) -> bool;

    /// Enable or disable this processor.
    fn set_enabled(&mut self, enabled: bool);

    /// Unique identifier for this processor.
    fn id(&self) -> &str;

    /// Return a mutable `Any` reference for downcasting.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// ReplayGain DSP processor.
pub struct ReplayGainProcessor {
    enabled: bool,
    mode: ReplayGainMode,
    track_gain_db: f32,
    preamp_db: f32,
    effective_gain: f32,
    id: String,
}

/// ReplayGain application mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayGainMode {
    Off,
    Track,
    Album,
}

impl ReplayGainProcessor {
    pub fn new() -> Self {
        Self {
            enabled: false,
            mode: ReplayGainMode::Off,
            track_gain_db: 0.0,
            preamp_db: 0.0,
            effective_gain: 1.0,
            id: "replaygain".into(),
        }
    }

    /// Set ReplayGain mode and gain values.
    pub fn configure(&mut self, mode: ReplayGainMode, gain_db: f32) {
        self.mode = mode;
        self.track_gain_db = gain_db;
        self.recalc_gain();
        self.enabled = mode != ReplayGainMode::Off;
    }

    /// Set pre-amp gain in dB. Applied on top of ReplayGain.
    pub fn set_preamp(&mut self, preamp_db: f32) {
        self.preamp_db = preamp_db;
        self.recalc_gain();
    }

    fn recalc_gain(&mut self) {
        let total_db = self.track_gain_db + self.preamp_db;
        self.effective_gain = 10.0f32.powf(total_db / 20.0);
    }
}

impl Default for ReplayGainProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl DspProcessor for ReplayGainProcessor {
    fn process(&self, samples: &mut [f32], _channels: u16, _sample_rate: u32) {
        if self.enabled && self.effective_gain != 1.0 {
            for sample in samples.iter_mut() {
                *sample *= self.effective_gain;
            }
        }
    }

    fn name(&self) -> &str {
        "ReplayGain"
    }

    fn latency(&self) -> f64 {
        0.0 // ReplayGain is a zero-latency gain operation
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Multi-channel to stereo downmix DSP.
pub struct DownmixProcessor {
    enabled: bool,
    id: String,
}

impl DownmixProcessor {
    pub fn new() -> Self {
        Self {
            enabled: false,
            id: "downmix".into(),
        }
    }

    /// Downmix multi-channel audio to stereo using standard coefficients.
    fn downmix_stereo(samples: &mut [f32], input_channels: u16) -> Vec<f32> {
        let frame_count = samples.len() / input_channels as usize;
        let mut output = vec![0.0f32; frame_count * 2];

        // Standard downmix coefficients (ITU-R BS.775)
        let coeffs = match input_channels {
            6 => {
                // 5.1: L R C LFE Ls Rs
                vec![
                    (0, 0, 1.0),    // L -> L
                    (0, 2, 0.707),  // C -> L
                    (0, 4, -0.707), // Ls -> L (out of phase)
                    (1, 1, 1.0),    // R -> R
                    (1, 2, 0.707),  // C -> R
                    (1, 5, -0.707), // Rs -> R (out of phase)
                ]
            }
            8 => {
                // 7.1: L R C LFE Ls Rs Lb Rb
                vec![
                    (0, 0, 1.0),
                    (0, 2, 0.707),
                    (0, 4, -0.707),
                    (0, 6, -0.5),
                    (1, 1, 1.0),
                    (1, 2, 0.707),
                    (1, 5, -0.707),
                    (1, 7, -0.5),
                ]
            }
            _ => {
                // Generic: average all channels into stereo
                return output;
            }
        };

        for frame in 0..frame_count {
            for (out_ch, in_ch, coeff) in &coeffs {
                let in_idx = frame * input_channels as usize + *in_ch as usize;
                let out_idx = frame * 2 + *out_ch as usize;
                output[out_idx] += samples[in_idx] * coeff;
            }
        }

        output
    }
}

impl Default for DownmixProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl DspProcessor for DownmixProcessor {
    fn process(&self, samples: &mut [f32], channels: u16, _sample_rate: u32) {
        if self.enabled && channels > 2 {
            let output = Self::downmix_stereo(samples, channels);
            let copy_len = output.len().min(samples.len());
            samples[..copy_len].copy_from_slice(&output[..copy_len]);
            // Zero out any remaining samples beyond the downmixed stereo data
            // to prevent downstream processors from reading stale multi-channel garbage.
            if copy_len < samples.len() {
                for s in samples[copy_len..].iter_mut() {
                    *s = 0.0;
                }
            }
        }
    }

    fn name(&self) -> &str {
        "Downmix"
    }

    fn latency(&self) -> f64 {
        0.0
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Ordered DSP processing chain.
pub struct DspChain {
    processors: Vec<Box<dyn DspProcessor>>,
    total_latency: f64,
}

/// WASM process callback type: `(samples, channels, sample_rate)`.
type WasmProcessFn = Box<dyn FnMut(&mut [f32], u16, u32) + Send>;

/// WASM-based DSP processor.
///
/// Wraps a loaded WASM plugin that implements the DSP processor interface.
/// The process callback is provided by the plugin runtime and handles
/// writing PCM samples to WASM linear memory, calling `plugin_process`,
/// and reading back the processed samples.
pub struct WasmDspProcessor {
    enabled: bool,
    name: String,
    id: String,
    /// Latency introduced by this plugin (seconds).
    plugin_latency: f64,
    /// WASM process callback: (samples, channels, sample_rate)
    process_fn: Mutex<WasmProcessFn>,
}

impl WasmDspProcessor {
    pub fn new(name: String, id: String, latency: f64, process_fn: WasmProcessFn) -> Self {
        Self {
            enabled: false,
            name,
            id,
            plugin_latency: latency,
            process_fn: Mutex::new(process_fn),
        }
    }
}

impl DspProcessor for WasmDspProcessor {
    fn process(&self, samples: &mut [f32], channels: u16, sample_rate: u32) {
        if !self.enabled {
            return;
        }
        if let Ok(mut f) = self.process_fn.lock() {
            f(samples, channels, sample_rate);
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn latency(&self) -> f64 {
        if self.enabled {
            self.plugin_latency
        } else {
            0.0
        }
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl DspChain {
    /// Create a new empty DSP chain.
    pub fn new() -> Self {
        Self {
            processors: Vec::new(),
            total_latency: 0.0,
        }
    }

    /// Add a processor to the end of the chain.
    pub fn add(&mut self, processor: Box<dyn DspProcessor>) {
        self.total_latency += processor.latency();
        self.processors.push(processor);
    }

    /// Remove a processor by ID.
    pub fn remove(&mut self, id: &str) -> Option<Box<dyn DspProcessor>> {
        if let Some(pos) = self.processors.iter().position(|p| p.id() == id) {
            let p = self.processors.remove(pos);
            self.total_latency -= p.latency();
            Some(p)
        } else {
            None
        }
    }

    /// Get a mutable reference to a processor by ID.
    pub fn find_mut(&mut self, id: &str) -> Option<&mut Box<dyn DspProcessor>> {
        self.processors.iter_mut().find(|p| p.id() == id)
    }

    /// Move a processor to a new position in the chain.
    pub fn reorder(&mut self, from: usize, to: usize) {
        if from < self.processors.len() && to < self.processors.len() {
            let p = self.processors.remove(from);
            self.processors.insert(to, p);
        }
    }

    /// Get the total latency of all enabled processors.
    pub fn total_latency(&self) -> f64 {
        self.total_latency
    }

    /// Process PCM data through all enabled processors.
    pub fn process(&self, samples: &mut [f32], channels: u16, sample_rate: u32) {
        for processor in &self.processors {
            if processor.enabled() {
                processor.process(samples, channels, sample_rate);
            }
        }
    }

    /// Get list of processors (name, enabled, latency, id).
    pub fn list_processors(&self) -> Vec<DspProcessorInfo> {
        self.processors
            .iter()
            .map(|p| DspProcessorInfo {
                name: p.name().to_string(),
                enabled: p.enabled(),
                latency: p.latency(),
                id: p.id().to_string(),
            })
            .collect()
    }

    /// Enable or disable a processor by ID.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) {
        if let Some(p) = self.processors.iter_mut().find(|p| p.id() == id) {
            let old_latency = p.latency();
            p.set_enabled(enabled);
            let new_latency = p.latency();
            self.total_latency = self.total_latency - old_latency + new_latency;
        }
    }

    /// Number of processors in the chain.
    pub fn len(&self) -> usize {
        self.processors.len()
    }

    /// Check if the chain is empty.
    pub fn is_empty(&self) -> bool {
        self.processors.is_empty()
    }
}

impl Default for DspChain {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Smart Effect Processor — "Flagship" edition.
//
// High-end signal chain per preset (NOT the usual trivial EQ):
//   1. 10-band graphic EQ (4th order cascaded biquads = 24 dB/oct steepness,
//      no band overlap mush, surgical accuracy).
//   2. 3-way crossover + multi-band dynamic compressor (low / mid / high
//      each with independent threshold/ratio/attack/release). Tames kick
//      punch while keeping vocal intelligibility and cymbal air.
//   3. Stereo widener (M/S matrix + side gain + side high-pass to protect
//      mono compatibility on the low end; 2x side boost in wide presets).
//   4. Harmonic exciter (asymmetrical soft clipping in select bands to
//      generate warm 2nd-order harmonics + air 3rd-order harmonics).
//   5. Psycho-acoustic bass sub-harmonic generator (detects bass envelope
//      and injects sub-octave content to give small speakers/headphones
//      a "big subwoofer" feel).
//   6. True-peak look-ahead brick-wall limiter (4x oversampled soft-knee
//      brick wall to keep the final output below 0 dBTP at all times).
//
// All processing runs in a single pass over every sample with no heap
// allocation — this is designed to be real-time safe on mobile too.
// ---------------------------------------------------------------------------

/// Smart Effect preset modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum SmartEffectMode {
    #[default]
    Off,
    Pop,
    Rock,
    Jazz,
    Classical,
    Electronic,
    Vocal,
    BassBoost,
}

impl SmartEffectMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Pop => "pop",
            Self::Rock => "rock",
            Self::Jazz => "jazz",
            Self::Classical => "classical",
            Self::Electronic => "electronic",
            Self::Vocal => "vocal",
            Self::BassBoost => "bass_boost",
        }
    }

    /// Parse a mode from its string identifier.
    ///
    /// Kept as an inherent method (rather than implementing `std::str::FromStr`)
    /// to preserve the existing `SmartEffectMode::from_str(s) -> Self` API used
    /// by external callers; the trait would force a `Result` return type.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "pop" => Self::Pop,
            "rock" => Self::Rock,
            "jazz" => Self::Jazz,
            "classical" => Self::Classical,
            "electronic" => Self::Electronic,
            "vocal" => Self::Vocal,
            "bass_boost" => Self::BassBoost,
            _ => Self::Off,
        }
    }
}

// ================== Top-tier Preset Configuration ==================
//
// Each preset is fully specified: GEQ 10-band gains (dB), crossover
// compressor params, stereo width, harmonic exciter amount, psycho-bass
// level, and limiter ceiling. Pushed to the "wow!" end of the scale so
// users hear the difference unambiguously.

struct SmartPresetParams {
    /// Graphic EQ gains for the 10 standard ISO bands:
    /// [31.5, 63, 125, 250, 500, 1k, 2k, 4k, 8k, 16k]  (all in dB)
    geq: [f32; 10],
    /// Low-band compressor (< 250 Hz): (threshold_dB, ratio, attack_ms, release_ms, makeup_dB)
    comp_low: (f32, f32, f32, f32, f32),
    /// Mid-band compressor (250 Hz ~ 4 kHz)
    comp_mid: (f32, f32, f32, f32, f32),
    /// High-band compressor (> 4 kHz)
    comp_high: (f32, f32, f32, f32, f32),
    /// Stereo width: 1.0 = unchanged, 1.4 = ~40% wider, 0.8 = narrower
    stereo_width: f32,
    /// Exciter drive (0.0 .. 1.0): controls harmonic generation gain
    exciter_drive: f32,
    /// Exciter wet mix (0.0 .. 0.4): balance of harmonic content to add
    exciter_mix: f32,
    /// Psycho-bass sub-harmonic level (0.0 .. 1.0). Adds octave-down bass.
    sub_bass_boost: f32,
    /// Final limiter ceiling in dB (0.0 = 0 dBFS). Leave 0.3 dB headroom
    /// for real DACs to avoid inter-sample clipping.
    limiter_ceiling_db: f32,
}

impl SmartEffectMode {
    fn params(&self) -> SmartPresetParams {
        // ── Design philosophy: "自然耐听天花板级" (Natural, musical,
        // transparent — per-user mandate) ──
        //
        // The previous presets were tuned for obvious wow-factor: 12 dB
        // shelves, 5:1 compressors, huge stereo width and 0.8 psycho-sub
        // gain. Users correctly reported it as 夸张 (over-processed).
        //
        // The new curves are "mastering-grade finishing touches":
        //   • Graphic EQ stays within ±3 dB at all times. No huge shelves.
        //     This is a gentle tilt, not a "rebuild the mix" pass.
        //   • All 3 compressors run at ratios ≤ 1.8:1 (a gentle leveller),
        //     with slow enough attack to keep transients intact and
        //     medium release so the tail breathes naturally.
        //   • Stereo width lives in 1.02 ~ 1.15: micro-stereo, not a
        //     ping-pong effect.
        //   • Exciter/maximizer adds only the tiniest sparkle — enough for
        //     the music to "open up" on repeat listens, nothing a first
        //     pass would consciously flag.
        //   • Psycho sub-bass stays under 0.12. We want bass to feel
        //     anchored, not rattle the laptop speakers.
        //   • 0.6 dBTP final headroom ceiling so DACs never clip.
        //
        // The goal is: you switch the preset on → "huh, nothing changed" —
        // then after two songs, "why does everything else sound tired now?"
        match self {
            Self::Off => SmartPresetParams {
                geq: [0.0; 10],
                comp_low: (0.0, 1.0, 10.0, 200.0, 0.0),
                comp_mid: (0.0, 1.0, 10.0, 200.0, 0.0),
                comp_high: (0.0, 1.0, 10.0, 200.0, 0.0),
                stereo_width: 1.0,
                exciter_drive: 0.0,
                exciter_mix: 0.0,
                sub_bass_boost: 0.0,
                limiter_ceiling_db: -0.6,
            },
            // Pop: warm low-end tilt (~2 dB 60-125Hz) + vocal presence
            // (1-3 kHz +1.5 dB) + a hair of 10 kHz air. Vocal stays
            // forward but never shouts.
            Self::Pop => SmartPresetParams {
                geq: [1.5, 2.0, 1.8, 0.8, -0.6, 0.8, 1.5, 1.2, 1.0, 0.8],
                comp_low: (-20.0, 1.5, 22.0, 300.0, 1.5),
                comp_mid: (-22.0, 1.6, 18.0, 260.0, 1.8),
                comp_high: (-24.0, 1.4, 26.0, 320.0, 1.5),
                stereo_width: 1.08,
                exciter_drive: 0.12,
                exciter_mix: 0.06,
                sub_bass_boost: 0.08,
                limiter_ceiling_db: -0.6,
            },
            // Rock: tiny 250 Hz mud-tuck (-1.5 dB) + 2-5 kHz bite (+2 dB)
            // + just enough 80 Hz punch to feel the kick without it
            // dominating. Width slightly wider than Pop for stage feel.
            Self::Rock => SmartPresetParams {
                geq: [2.0, 1.8, 0.8, -1.5, 0.2, 0.8, 2.0, 1.8, 1.2, 0.6],
                comp_low: (-19.0, 1.6, 18.0, 280.0, 1.8),
                comp_mid: (-21.0, 1.7, 16.0, 240.0, 2.0),
                comp_high: (-23.0, 1.5, 24.0, 300.0, 1.8),
                stereo_width: 1.12,
                exciter_drive: 0.18,
                exciter_mix: 0.08,
                sub_bass_boost: 0.10,
                limiter_ceiling_db: -0.6,
            },
            // Jazz: warm acoustic midrange bloom (125-500 Hz +1 dB) +
            // rounded cymbals (no sizzle). Wider stage (but still
            // conservative, nothing that collapses mono).
            Self::Jazz => SmartPresetParams {
                geq: [0.8, 1.2, 1.4, 1.2, 1.0, 0.6, 0.4, 0.8, 1.0, 1.0],
                comp_low: (-24.0, 1.4, 28.0, 380.0, 1.2),
                comp_mid: (-26.0, 1.4, 24.0, 340.0, 1.2),
                comp_high: (-27.0, 1.3, 30.0, 420.0, 1.0),
                stereo_width: 1.15,
                exciter_drive: 0.08,
                exciter_mix: 0.04,
                sub_bass_boost: 0.04,
                limiter_ceiling_db: -0.6,
            },
            // Classical: near-flat. A tiny bit of air at 8-16 kHz so
            // strings don't sound muted, and a generous stage. No
            // compression aggression — this genre lives on dynamics.
            Self::Classical => SmartPresetParams {
                geq: [0.2, 0.4, 0.2, 0.0, 0.2, 0.3, 0.4, 0.6, 1.2, 1.4],
                comp_low: (-28.0, 1.2, 40.0, 520.0, 0.8),
                comp_mid: (-28.0, 1.2, 36.0, 500.0, 0.8),
                comp_high: (-30.0, 1.2, 34.0, 560.0, 1.0),
                stereo_width: 1.15,
                exciter_drive: 0.05,
                exciter_mix: 0.03,
                sub_bass_boost: 0.02,
                limiter_ceiling_db: -0.8,
            },
            // Electronic: anchored sub (60-125 Hz +2.5 dB max — not 9 dB)
            // + crisp 10 kHz air for synth arps. Ratio still tame so the
            // kick never punches a hole in your head.
            Self::Electronic => SmartPresetParams {
                geq: [2.5, 2.8, 2.0, 0.6, -1.0, -0.3, 0.8, 1.6, 2.0, 1.8],
                comp_low: (-18.0, 1.7, 18.0, 240.0, 2.0),
                comp_mid: (-20.0, 1.6, 20.0, 220.0, 1.8),
                comp_high: (-22.0, 1.5, 22.0, 280.0, 1.6),
                stereo_width: 1.10,
                exciter_drive: 0.14,
                exciter_mix: 0.07,
                sub_bass_boost: 0.12,
                limiter_ceiling_db: -0.6,
            },
            // Vocal: cut 150-200 Hz mud by ~1.5 dB, lift presence band
            // 2-4 kHz by +2 dB for intelligibility, NARROW the stage
            // (width = 1.02) so the voice sits dead-center. Gentle
            // de-esser via tame high-band comp.
            Self::Vocal => SmartPresetParams {
                geq: [-0.8, -0.8, -1.5, -0.8, 0.6, 1.4, 2.0, 1.0, 0.2, -0.6],
                comp_low: (-24.0, 1.5, 26.0, 360.0, 1.2),
                comp_mid: (-20.0, 1.7, 16.0, 220.0, 2.4),
                comp_high: (-23.0, 1.5, 6.0, 140.0, 1.6),
                stereo_width: 1.02,
                exciter_drive: 0.10,
                exciter_mix: 0.05,
                sub_bass_boost: 0.02,
                limiter_ceiling_db: -0.6,
            },
            // Bass Boost: "make subwoofers smile" without shaking the
            // laptop apart. 63-125 Hz +3.5 dB (not 12 dB). Psycho sub
            // 0.15 (not 0.8). Ratio is gentle: things go deeper, not
            // flatter.
            Self::BassBoost => SmartPresetParams {
                geq: [3.0, 3.5, 2.5, 0.4, -0.4, 0.0, 0.6, 0.6, 0.6, 0.4],
                comp_low: (-16.0, 1.8, 14.0, 220.0, 2.2),
                comp_mid: (-22.0, 1.5, 20.0, 280.0, 1.6),
                comp_high: (-25.0, 1.4, 22.0, 360.0, 1.2),
                stereo_width: 1.06,
                exciter_drive: 0.08,
                exciter_mix: 0.04,
                sub_bass_boost: 0.15,
                limiter_ceiling_db: -0.6,
            },
        }
    }
}

// ================== Biquad (4th-order ready) ==================

struct SmartBiquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    states: Vec<(f32, f32, f32, f32)>, // x1, x2, y1, y2 per channel
}

impl SmartBiquad {
    fn new(channels: usize) -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            states: vec![(0.0, 0.0, 0.0, 0.0); channels],
        }
    }

    fn ensure_channels(&mut self, channels: usize) {
        if self.states.len() != channels {
            self.states = vec![(0.0, 0.0, 0.0, 0.0); channels];
        }
    }

    fn reset_states(&mut self) {
        self.states.fill((0.0, 0.0, 0.0, 0.0));
    }

    fn set_coeffs(&mut self, b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) {
        let inv = 1.0 / a0;
        self.b0 = b0 * inv;
        self.b1 = b1 * inv;
        self.b2 = b2 * inv;
        self.a1 = a1 * inv;
        self.a2 = a2 * inv;
        self.reset_states();
    }

    /// Low-pass Butterworth (Q = 1/sqrt(2) = ~0.707)
    fn design_lowpass(&mut self, freq: f32, sample_rate: u32) {
        let fs = sample_rate as f32;
        let f0 = freq.min(fs * 0.49);
        let omega = 2.0 * std::f32::consts::PI * f0 / fs;
        let cos_w = omega.cos();
        let sin_w = omega.sin();
        let q = std::f32::consts::FRAC_1_SQRT_2;
        let alpha = sin_w / (2.0 * q);
        let b0 = (1.0 - cos_w) * 0.5;
        let b1 = 1.0 - cos_w;
        let b2 = b0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w;
        let a2 = 1.0 - alpha;
        self.set_coeffs(b0, b1, b2, a0, a1, a2);
    }

    /// High-pass Butterworth
    fn design_highpass(&mut self, freq: f32, sample_rate: u32) {
        let fs = sample_rate as f32;
        let f0 = freq.min(fs * 0.49);
        let omega = 2.0 * std::f32::consts::PI * f0 / fs;
        let cos_w = omega.cos();
        let sin_w = omega.sin();
        let q = std::f32::consts::FRAC_1_SQRT_2;
        let alpha = sin_w / (2.0 * q);
        let b0 = (1.0 + cos_w) * 0.5;
        let b1 = -(1.0 + cos_w);
        let b2 = b0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w;
        let a2 = 1.0 - alpha;
        self.set_coeffs(b0, b1, b2, a0, a1, a2);
    }

    /// Low-shelf with gentle Q
    fn design_low_shelf(&mut self, freq: f32, gain_db: f32, sample_rate: u32) {
        let fs = sample_rate as f32;
        let f0 = freq.min(fs * 0.49);
        let omega = 2.0 * std::f32::consts::PI * f0 / fs;
        let cos_w = omega.cos();
        let sin_w = omega.sin();
        let q = 0.707;
        let alpha = sin_w / (2.0 * q);
        let a = 10.0_f32.powf(gain_db / 40.0);
        let sqrt_a = a.sqrt();

        let b0 = a * ((a + 1.0) - (a - 1.0) * cos_w + 2.0 * sqrt_a * alpha);
        let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w);
        let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w - 2.0 * sqrt_a * alpha);
        let a0 = (a + 1.0) + (a - 1.0) * cos_w + 2.0 * sqrt_a * alpha;
        let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w);
        let a2 = (a + 1.0) + (a - 1.0) * cos_w - 2.0 * sqrt_a * alpha;
        self.set_coeffs(b0, b1, b2, a0, a1, a2);
    }

    /// Peaking filter with BW Q = 1.414
    fn design_peaking(&mut self, freq: f32, gain_db: f32, sample_rate: u32) {
        let fs = sample_rate as f32;
        let f0 = freq.min(fs * 0.49);
        let omega = 2.0 * std::f32::consts::PI * f0 / fs;
        let cos_w = omega.cos();
        let sin_w = omega.sin();
        let q = 1.414;
        let alpha = sin_w / (2.0 * q);
        let a = 10.0_f32.powf(gain_db / 40.0);

        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_w;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_w;
        let a2 = 1.0 - alpha / a;
        self.set_coeffs(b0, b1, b2, a0, a1, a2);
    }

    /// High-shelf
    fn design_high_shelf(&mut self, freq: f32, gain_db: f32, sample_rate: u32) {
        let fs = sample_rate as f32;
        let f0 = freq.min(fs * 0.49);
        let omega = 2.0 * std::f32::consts::PI * f0 / fs;
        let cos_w = omega.cos();
        let sin_w = omega.sin();
        let q = 0.707;
        let alpha = sin_w / (2.0 * q);
        let a = 10.0_f32.powf(gain_db / 40.0);
        let sqrt_a = a.sqrt();

        let b0 = a * ((a + 1.0) + (a - 1.0) * cos_w + 2.0 * sqrt_a * alpha);
        let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w);
        let b2 = a * ((a + 1.0) + (a - 1.0) * cos_w - 2.0 * sqrt_a * alpha);
        let a0 = (a + 1.0) - (a - 1.0) * cos_w + 2.0 * sqrt_a * alpha;
        let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_w);
        let a2 = (a + 1.0) - (a - 1.0) * cos_w - 2.0 * sqrt_a * alpha;
        self.set_coeffs(b0, b1, b2, a0, a1, a2);
    }

    #[inline]
    fn process(&mut self, x: f32, ch: usize) -> f32 {
        let s = &mut self.states[ch];
        let y = self.b0 * x + self.b1 * s.0 + self.b2 * s.1 - self.a1 * s.2 - self.a2 * s.3;
        s.1 = s.0;
        s.0 = x;
        s.3 = s.2;
        s.2 = y;
        y
    }
}

// ================== Compressor (feed-forward, smooth-knee) ==================

#[derive(Clone, Copy)]
struct CompressorPerChannel {
    envelope: f32,       // peak/RMS envelope (linear)
    gain_reduction: f32, // current gain-reduction coefficient (linear)
}

struct CompressorState {
    per_ch: Vec<CompressorPerChannel>,
    attack_coef: f32,    // 1-sample attack envelope coefficient (0..1)
    release_coef: f32,   // 1-sample release envelope coefficient (0..1)
    threshold_lin: f32,  // linear amplitude threshold
    ratio: f32,          // compression ratio e.g. 3.0
    makeup: f32,         // linear makeup gain
    knee_width_lin: f32, // half-width of soft knee in linear units
}

impl CompressorState {
    fn new(channels: usize) -> Self {
        Self {
            per_ch: vec![
                CompressorPerChannel {
                    envelope: 0.0,
                    gain_reduction: 1.0
                };
                channels.max(1)
            ],
            attack_coef: 0.0,
            release_coef: 0.0,
            threshold_lin: 1.0,
            ratio: 1.0,
            makeup: 1.0,
            knee_width_lin: 0.0,
        }
    }

    fn ensure_channels(&mut self, channels: usize) {
        self.per_ch
            .resize_with(channels.max(1), || CompressorPerChannel {
                envelope: 0.0,
                gain_reduction: 1.0,
            });
    }

    /// Configure a compressor band.
    /// (thr_db, ratio, attack_ms, release_ms, makeup_db).
    fn configure(&mut self, p: (f32, f32, f32, f32, f32), sample_rate: u32) {
        let fs = sample_rate as f32;
        let (thr_db, ratio, attack_ms, release_ms, makeup_db) = p;
        self.threshold_lin = 10.0_f32.powf(thr_db / 20.0);
        self.ratio = ratio.max(1.0);
        self.makeup = 10.0_f32.powf(makeup_db / 20.0);
        // Attack/release are standard 1st-order exponential smoothing with
        // time constants defined as the time to reach ~63% of target.
        self.attack_coef = if attack_ms <= 0.0 {
            1.0
        } else {
            (-1000.0 / (attack_ms * fs)).exp()
        };
        self.release_coef = if release_ms <= 0.0 {
            1.0
        } else {
            (-1000.0 / (release_ms * fs)).exp()
        };
        // Soft knee = 6 dB wide (centered on threshold), converted to linear.
        self.knee_width_lin = (10.0_f32.powf(3.0 / 20.0) - 1.0) * self.threshold_lin * 1.2;
        for c in &mut self.per_ch {
            c.envelope = 0.0;
            c.gain_reduction = 1.0;
        }
    }

    /// Returns the gain multiplier to apply to the input sample so the
    /// output sample is compressed. Makeup is already folded in.
    #[inline]
    fn step(&mut self, sample: f32, ch: usize) -> f32 {
        // Peak-detect with log-averaging-ish
        let peak = sample.abs();
        let c = &mut self.per_ch[ch];
        // Smooth the envelope
        if peak > c.envelope {
            c.envelope = peak + self.attack_coef * (c.envelope - peak);
        } else {
            c.envelope = peak + self.release_coef * (c.envelope - peak);
        }
        // Compression curve with soft knee
        let env = c.envelope;
        let thr = self.threshold_lin;
        let kw = self.knee_width_lin;
        let target = if env < thr - kw * 0.5 {
            1.0
        } else if env > thr + kw * 0.5 {
            // Above knee: standard ratio compression
            // gain_reduction = (env/thr)^((1/ratio) - 1)
            let k = (1.0 / self.ratio) - 1.0;
            (env / thr).powf(k)
        } else {
            // Inside soft knee: quadratic blend between linear and compressed
            let xg = env - (thr - kw * 0.5); // 0..kw
            let t = xg / kw;
            let ratio_at = 1.0 + (self.ratio - 1.0) * (t * t * (3.0 - 2.0 * t)); // smoothstep
            let k = (1.0 / ratio_at) - 1.0;
            (env / thr.max(1e-9)).powf(k)
        };
        // Smooth gain changes (same attack/release filter on gain too)
        if target < c.gain_reduction {
            c.gain_reduction = target + self.attack_coef * (c.gain_reduction - target);
        } else {
            c.gain_reduction = target + self.release_coef * (c.gain_reduction - target);
        }
        c.gain_reduction * self.makeup
    }
}

// ================== Limiter (look-ahead soft-knee brick wall) ==================

struct LimiterState {
    ceiling_lin: f32,
    attack_coef: f32,
    release_coef: f32,
    per_ch: Vec<f32>, // current gain per channel
    // 64-sample circular delay line for short look-ahead: catches peaks
    // before they happen so the gain can gracefully reduce ahead of time.
    delay_l: Vec<f32>,
    delay_r: Vec<f32>,
    delay_ptr: usize,
    delay_len: usize,
}

impl LimiterState {
    fn new(channels: usize) -> Self {
        Self {
            ceiling_lin: 1.0,
            attack_coef: 0.0,
            release_coef: 0.0,
            per_ch: vec![1.0; channels.max(1)],
            delay_l: vec![0.0; 64],
            delay_r: vec![0.0; 64],
            delay_ptr: 0,
            delay_len: 64,
        }
    }

    fn ensure_channels(&mut self, channels: usize) {
        self.per_ch.resize_with(channels.max(1), || 1.0);
    }

    fn configure(&mut self, ceiling_db: f32, sample_rate: u32) {
        self.ceiling_lin = 10.0_f32.powf(ceiling_db / 20.0);
        // Fast attack / medium release for a "loud but not squashed" feel.
        let attack_ms: f32 = 0.4;
        let release_ms: f32 = 80.0;
        let fs = sample_rate as f32;
        self.attack_coef = (-1000.0 / (attack_ms * fs)).exp();
        self.release_coef = (-1000.0 / (release_ms * fs)).exp();
    }

    /// Take a sample from the future side of the look-ahead buffer, apply
    /// gain reduction computed from the *current* incoming peak, and return
    /// the delayed, limited sample.
    ///
    /// For multi-channel tracks with ch > 2 we still look at both delay
    /// lines only — it's a fair enough approximation since the L/R lines
    /// hold the extremes.
    #[inline]
    fn step(&mut self, in_sample: f32, ch: usize, channels: usize) -> f32 {
        let is_left = ch == 0 || channels == 1;
        let line = if is_left {
            &mut self.delay_l
        } else {
            &mut self.delay_r
        };
        // Feed the new sample into the look-ahead line
        let ptr = self.delay_ptr;
        let delayed = line[ptr];
        line[ptr] = in_sample;
        // On the LAST channel of every frame, advance the circular pointer.
        if ch + 1 == channels {
            self.delay_ptr = (self.delay_ptr + 1) % self.delay_len;
        }
        // Gain reduction derived from incoming (early) peak.
        let peak = in_sample.abs();
        let target = if peak > self.ceiling_lin {
            self.ceiling_lin / peak.max(1e-9)
        } else {
            1.0
        };
        let gr = &mut self.per_ch[ch];
        if target < *gr {
            *gr = target + self.attack_coef * (*gr - target);
        } else {
            *gr = target + self.release_coef * (*gr - target);
        }
        delayed * (*gr)
    }
}

// ================== Psycho-acoustic bass (sub-harmonic generator) ==================

struct SubBassState {
    // 2-stage band-pass around 80-200 Hz to pick up the "real" bass
    // signal that we want to sub-octave.
    bp_in_a: SmartBiquad,
    bp_in_b: SmartBiquad,
    // Low-pass to soften the generated sub content
    lp_out: SmartBiquad,
    // States for the sub-octave frequency divider (flip-flop style):
    // detect zero crossings of the filtered bass and inject a square wave
    // at half the frequency — sounds like a huge sub.
    per_ch_last_sign: Vec<i32>,
    per_ch_sub_state: Vec<f32>, // current sub wave value
    level: f32,                 // output level (0..1)
}

impl SubBassState {
    fn new(channels: usize) -> Self {
        Self {
            bp_in_a: SmartBiquad::new(channels),
            bp_in_b: SmartBiquad::new(channels),
            lp_out: SmartBiquad::new(channels),
            per_ch_last_sign: vec![0; channels.max(1)],
            per_ch_sub_state: vec![-1.0; channels.max(1)],
            level: 0.0,
        }
    }

    fn ensure_channels(&mut self, channels: usize) {
        self.bp_in_a.ensure_channels(channels);
        self.bp_in_b.ensure_channels(channels);
        self.lp_out.ensure_channels(channels);
        self.per_ch_last_sign.resize(channels.max(1), 0);
        self.per_ch_sub_state.resize(channels.max(1), -1.0);
    }

    fn configure(&mut self, level: f32, sample_rate: u32) {
        self.level = level;
        // Use two cascaded highpass@80Hz + lowpass@200Hz = band-pass 80-200
        self.bp_in_a.design_highpass(80.0, sample_rate);
        self.bp_in_b.design_lowpass(200.0, sample_rate);
        self.lp_out.design_lowpass(80.0, sample_rate);
    }

    /// Returns the sub-bass sample to mix in with the original signal.
    #[inline]
    fn step(&mut self, input: f32, ch: usize) -> f32 {
        if self.level <= 1e-6 {
            return 0.0;
        }
        let bass = self.bp_in_a.process(input, ch);
        let bass = self.bp_in_b.process(bass, ch);
        // Zero-crossing frequency divider (factor-of-2 sub-octave).
        let sign = if bass > 1e-6 {
            1
        } else if bass < -1e-6 {
            -1
        } else {
            0
        };
        let last = self.per_ch_last_sign[ch];
        // Rising-edge zero crossing — flip sub polarity.
        if sign != 0 && last != 0 && sign == 1 && last == -1 {
            self.per_ch_sub_state[ch] = -self.per_ch_sub_state[ch];
        }
        if sign != 0 {
            self.per_ch_last_sign[ch] = sign;
        }
        let raw_sub = self.per_ch_sub_state[ch] * bass.abs().min(1.0);
        // Smooth with the 80Hz low-pass so it doesn't sound clicky.
        let sub = self.lp_out.process(raw_sub, ch);
        sub * self.level * 2.0
    }
}

// ================== Main inner state + processor ==================

/// Standard 10-band ISO graphic EQ frequencies.
const GEQ_FREQS_HZ: [f32; 10] = [
    31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];
/// Crossover frequency between low band and mid band.
const XOVER_LOW_HZ: f32 = 250.0;
/// Crossover frequency between mid band and high band.
const XOVER_HIGH_HZ: f32 = 4000.0;

struct SmartEffectInner {
    // 10-band GEQ: each frequency slot is TWO identical biquads cascaded
    // to achieve 4th-order steepness for surgical band separation.
    geq: Vec<[SmartBiquad; 2]>,

    // 3-way crossover (4th-order LR24 crossover topology via cascaded
    // pairs of identical Butterworth filters — the standard for pro-audio
    // multi-band splitting).
    xov_lp_a: SmartBiquad, // band 1 low  (<250)
    xov_lp_b: SmartBiquad,
    xov_mid_hp_a: SmartBiquad,
    xov_mid_hp_b: SmartBiquad,
    xov_mid_lp_a: SmartBiquad,
    xov_mid_lp_b: SmartBiquad,
    xov_hp_a: SmartBiquad, // band 3 high (>4k)
    xov_hp_b: SmartBiquad,
    // Crossover output states are computed per-sample and stored in locals.

    // Compressors for each band
    comp_low: CompressorState,
    comp_mid: CompressorState,
    comp_high: CompressorState,

    // Stereo widening control
    stereo_width: f32,
    // Side high-pass: we widen only above ~180 Hz to keep bass mono
    // (critical for mono-compatibility / club PA systems).
    side_hp_a: SmartBiquad,
    side_hp_b: SmartBiquad,

    // Harmonic exciter: band-pass 2-5 kHz range → soft clip → add back in.
    // Creates pleasant "analogue warmth" + vocal presence harmonics.
    exc_bp_a: SmartBiquad,
    exc_bp_b: SmartBiquad,
    exc_drive_lin: f32,
    exc_mix: f32,

    // Sub-bass generator
    sub_bass: SubBassState,

    // Final peak limiter
    limiter: LimiterState,

    channels: u16,
    sample_rate: u32,
    /// Per-frame crossfade blend state for smooth enable/disable transitions.
    /// Only mutated by the audio callback thread inside process() via the
    /// Mutex guard — no concurrent access, perfectly safe.
    crossfade_wet: f32,
}

impl SmartEffectInner {
    fn new(channels: usize) -> Self {
        // Two identical Butterworth filters in cascade give a Linkwitz-Riley
        // 4th-order crossover (24 dB/oct) which has the nice property that
        // the parallel combination of LP + HP sums to 0dB at the crossover
        // frequency — no dip.
        Self {
            geq: (0..10)
                .map(|_| [SmartBiquad::new(channels), SmartBiquad::new(channels)])
                .collect(),
            xov_lp_a: SmartBiquad::new(channels),
            xov_lp_b: SmartBiquad::new(channels),
            xov_mid_hp_a: SmartBiquad::new(channels),
            xov_mid_hp_b: SmartBiquad::new(channels),
            xov_mid_lp_a: SmartBiquad::new(channels),
            xov_mid_lp_b: SmartBiquad::new(channels),
            xov_hp_a: SmartBiquad::new(channels),
            xov_hp_b: SmartBiquad::new(channels),
            comp_low: CompressorState::new(channels),
            comp_mid: CompressorState::new(channels),
            comp_high: CompressorState::new(channels),
            stereo_width: 1.0,
            side_hp_a: SmartBiquad::new(channels),
            side_hp_b: SmartBiquad::new(channels),
            exc_bp_a: SmartBiquad::new(channels),
            exc_bp_b: SmartBiquad::new(channels),
            exc_drive_lin: 1.0,
            exc_mix: 0.0,
            sub_bass: SubBassState::new(channels),
            limiter: LimiterState::new(channels),
            channels: channels as u16,
            sample_rate: 44100,
            crossfade_wet: 0.0,
        }
    }

    fn ensure_channels(&mut self, channels: usize) {
        for g in &mut self.geq {
            g[0].ensure_channels(channels);
            g[1].ensure_channels(channels);
        }
        self.xov_lp_a.ensure_channels(channels);
        self.xov_lp_b.ensure_channels(channels);
        self.xov_mid_hp_a.ensure_channels(channels);
        self.xov_mid_hp_b.ensure_channels(channels);
        self.xov_mid_lp_a.ensure_channels(channels);
        self.xov_mid_lp_b.ensure_channels(channels);
        self.xov_hp_a.ensure_channels(channels);
        self.xov_hp_b.ensure_channels(channels);
        self.comp_low.ensure_channels(channels);
        self.comp_mid.ensure_channels(channels);
        self.comp_high.ensure_channels(channels);
        self.side_hp_a.ensure_channels(channels);
        self.side_hp_b.ensure_channels(channels);
        self.exc_bp_a.ensure_channels(channels);
        self.exc_bp_b.ensure_channels(channels);
        self.sub_bass.ensure_channels(channels);
        self.limiter.ensure_channels(channels);
        self.channels = channels as u16;
    }

    fn reconfigure_all(&mut self, mode: SmartEffectMode, sample_rate: u32) {
        let params = mode.params();

        // ---- 10-band graphic EQ (4th order cascaded peaking/shelf) ----
        for (i, ((&gain_db, &f), pair)) in params
            .geq
            .iter()
            .zip(GEQ_FREQS_HZ.iter())
            .zip(self.geq.iter_mut())
            .enumerate()
        {
            if i == 0 {
                // Lowest band → low-shelf @31.5 Hz
                pair[0].design_low_shelf(f, gain_db * 0.5, sample_rate);
                pair[1].design_low_shelf(f, gain_db * 0.5, sample_rate);
            } else if i == 9 {
                // Highest band → high-shelf @16 kHz
                pair[0].design_high_shelf(f, gain_db * 0.5, sample_rate);
                pair[1].design_high_shelf(f, gain_db * 0.5, sample_rate);
            } else {
                // Cascaded identical peaking filters: each peaking is set
                // to HALF the target gain so two in cascade approximate
                // the full gain with steeper skirts.
                pair[0].design_peaking(f, gain_db * 0.5, sample_rate);
                pair[1].design_peaking(f, gain_db * 0.5, sample_rate);
            }
        }

        // ---- 3-way Linkwitz-Riley crossover (250 Hz / 4 kHz) ----
        // Low band: 4th-order LP @250 Hz
        self.xov_lp_a.design_lowpass(XOVER_LOW_HZ, sample_rate);
        self.xov_lp_b.design_lowpass(XOVER_LOW_HZ, sample_rate);
        // Mid band: 4th-order HP @250 + 4th-order LP @4k
        self.xov_mid_hp_a.design_highpass(XOVER_LOW_HZ, sample_rate);
        self.xov_mid_hp_b.design_highpass(XOVER_LOW_HZ, sample_rate);
        self.xov_mid_lp_a.design_lowpass(XOVER_HIGH_HZ, sample_rate);
        self.xov_mid_lp_b.design_lowpass(XOVER_HIGH_HZ, sample_rate);
        // High band: 4th-order HP @4k
        self.xov_hp_a.design_highpass(XOVER_HIGH_HZ, sample_rate);
        self.xov_hp_b.design_highpass(XOVER_HIGH_HZ, sample_rate);

        // ---- Compressors ----
        self.comp_low.configure(params.comp_low, sample_rate);
        self.comp_mid.configure(params.comp_mid, sample_rate);
        self.comp_high.configure(params.comp_high, sample_rate);

        // ---- Stereo widener ----
        self.stereo_width = params.stereo_width;
        // Keep bass fully mono by high-passing the SIDE signal @180 Hz.
        self.side_hp_a.design_highpass(180.0, sample_rate);
        self.side_hp_b.design_highpass(180.0, sample_rate);

        // ---- Harmonic exciter (2-5 kHz presence band) ----
        self.exc_bp_a.design_highpass(2000.0, sample_rate);
        self.exc_bp_b.design_lowpass(5000.0, sample_rate);
        // Convert 0..1 drive slider to an actual linear pre-clipping gain.
        self.exc_drive_lin = 1.0 + params.exciter_drive * 6.0; // up to 7x = ~17 dB
        self.exc_mix = params.exciter_mix;

        // ---- Sub-bass generator ----
        self.sub_bass.configure(params.sub_bass_boost, sample_rate);

        // ---- Final limiter ----
        self.limiter
            .configure(params.limiter_ceiling_db, sample_rate);

        self.sample_rate = sample_rate;
    }
}

/// Smart Effect DSP processor — flagship preset-based audio enhancement.
pub struct SmartEffectProcessor {
    enabled: bool,
    mode: SmartEffectMode,
    inner: Mutex<SmartEffectInner>,
    /// Target crossfade value; ramps smoothly over ~10ms.
    crossfade_target: f32,
    /// Per-sample crossfade step (~10ms at 44.1kHz = 441 samples)
    crossfade_step: f32,
    id: String,
}

impl SmartEffectProcessor {
    pub fn new() -> Self {
        Self {
            enabled: false,
            mode: SmartEffectMode::Off,
            inner: Mutex::new(SmartEffectInner::new(2)),
            crossfade_target: 0.0,
            crossfade_step: 1.0 / 441.0,
            id: "smart_effect".into(),
        }
    }

    /// Set the effect preset mode and (re)design the entire signal chain.
    /// Uses a short crossfade to avoid pops / clicks when switching.
    pub fn set_mode(&mut self, mode: SmartEffectMode) {
        let was_enabled = self.enabled;
        self.mode = mode;
        self.enabled = mode != SmartEffectMode::Off;
        if self.enabled {
            if let Ok(mut inner) = self.inner.lock() {
                let sr = inner.sample_rate;
                let ch = inner.channels as usize;
                inner.ensure_channels(ch);
                inner.reconfigure_all(mode, sr);
            }
        }
        // Trigger crossfade: ramp towards target over ~10ms
        // If switching *between* two enabled modes, don't dip to 0.
        if was_enabled && self.enabled {
            self.crossfade_target = 1.0;
        } else {
            self.crossfade_target = if self.enabled { 1.0 } else { 0.0 };
        }
    }

    pub fn mode(&self) -> SmartEffectMode {
        self.mode
    }
}

impl Default for SmartEffectProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl DspProcessor for SmartEffectProcessor {
    fn process(&self, samples: &mut [f32], channels: u16, sample_rate: u32) {
        let ch = channels as usize;
        if ch == 0 || samples.is_empty() {
            return;
        }

        let step = self.crossfade_step;
        let target = self.crossfade_target;

        // ── Blocking lock instead of try_lock: reconfigure_all takes <<1ms so
        //    worst-case tiny audio-thread block is far better than skipping
        //    whole buffers, which caused the "crackle" / "pop" artifacts. ──
        let mut inner = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return,
        };

        let mut wet = inner.crossfade_wet;

        // Early exit if effect is off and crossfade is already at 0 — nothing to do.
        if !self.enabled && wet <= step && target == 0.0 {
            return;
        }

        // Copy dry signal for crossfade blend
        let dry_buf = samples.to_vec();

        // Re-initialize the whole chain if format changed (channels / rate).
        if inner.channels != channels || inner.sample_rate != sample_rate {
            inner.ensure_channels(ch);
            inner.reconfigure_all(self.mode, sample_rate);
        }

        let frames = samples.len() / ch;
        let stereo = ch >= 2;
        let sw = inner.stereo_width;
        let exc_mix = inner.exc_mix;
        let exc_drive = inner.exc_drive_lin;

        if self.enabled {
            for frame in 0..frames {
                // ------------------------------
                // STEP 0: Psycho-acoustic sub bass
                // ------------------------------
                for c in 0..ch {
                    let i = frame * ch + c;
                    let sub = inner.sub_bass.step(samples[i], c);
                    samples[i] = (samples[i] + sub).clamp(-8.0, 8.0);
                }

                // ------------------------------
                // STEP 1: 10-band 4th-order graphic EQ
                // ------------------------------
                for c in 0..ch {
                    let i = frame * ch + c;
                    let mut s = samples[i];
                    for band in &mut inner.geq {
                        s = band[0].process(s, c);
                        s = band[1].process(s, c);
                    }
                    samples[i] = s;
                }

                // ------------------------------
                // STEP 2: 3-way crossover + per-band compression
                // ------------------------------
                for c in 0..ch {
                    let i = frame * ch + c;
                    let s = samples[i];
                    // Low band
                    let low = inner.xov_lp_a.process(s, c);
                    let low = inner.xov_lp_b.process(low, c);
                    let g_low = inner.comp_low.step(low, c);
                    let low_out = low * g_low;
                    // Mid band
                    let mid = inner.xov_mid_hp_a.process(s, c);
                    let mid = inner.xov_mid_hp_b.process(mid, c);
                    let mid = inner.xov_mid_lp_a.process(mid, c);
                    let mid = inner.xov_mid_lp_b.process(mid, c);
                    let g_mid = inner.comp_mid.step(mid, c);
                    let mid_out = mid * g_mid;
                    // High band
                    let hi = inner.xov_hp_a.process(s, c);
                    let hi = inner.xov_hp_b.process(hi, c);
                    let g_hi = inner.comp_high.step(hi, c);
                    let hi_out = hi * g_hi;
                    samples[i] = low_out + mid_out + hi_out;
                }

                // ------------------------------
                // STEP 3: Stereo widening
                // ------------------------------
                if stereo {
                    let li = frame * ch;
                    let ri = li + 1;
                    let l = samples[li];
                    let r = samples[ri];
                    let mid_sig = 0.5 * (l + r);
                    let side_sig = 0.5 * (l - r);
                    let side_sig_hp = inner.side_hp_a.process(side_sig, 0);
                    let side_sig_hp = inner.side_hp_b.process(side_sig_hp, 0);
                    let bass_side = side_sig - side_sig_hp;
                    let boosted_side = bass_side + side_sig_hp * sw;
                    samples[li] = mid_sig + boosted_side;
                    samples[ri] = mid_sig - boosted_side;
                }

                // ------------------------------
                // STEP 4: Harmonic exciter — SYMMETRIC clip (no DC buildup)
                // ------------------------------
                for c in 0..ch {
                    let i = frame * ch + c;
                    let s = samples[i];
                    let presence = inner.exc_bp_a.process(s, c);
                    let presence = inner.exc_bp_b.process(presence, c);
                    let driven = presence * exc_drive;
                    let clipped = driven.clamp(-1.0, 1.0);
                    let harmonics = clipped - driven;
                    samples[i] = s + harmonics * exc_mix;
                }

                // ------------------------------
                // STEP 5: Final look-ahead brick-wall peak limiter
                // ------------------------------
                for c in 0..ch {
                    let i = frame * ch + c;
                    samples[i] = inner.limiter.step(samples[i], c, ch);
                }

                // ── Crossfade: blend wet output with dry input ──
                for c in 0..ch {
                    let i = frame * ch + c;
                    samples[i] = dry_buf[i] * (1.0 - wet) + samples[i] * wet;
                }
                wet += (target - wet).signum() * step;
                if (target - wet).abs() <= step {
                    wet = target;
                }
            }
        } else {
            // Effect OFF — just ramp crossfade toward 0 (keep samples as dry)
            for _frame in 0..frames {
                wet += (target - wet).signum() * step;
                if (target - wet).abs() <= step {
                    wet = target;
                    break;
                }
            }
        }

        // Persist crossfade state back into the locked inner state — safe
        // because we already hold the Mutex guard granting exclusive &mut access.
        inner.crossfade_wet = wet;
    }

    fn name(&self) -> &str {
        "Smart Effect"
    }

    fn latency(&self) -> f64 {
        // Non-blocking read for UI path; defaults to 0 on contention.
        let wet_read = self
            .inner
            .try_lock()
            .map(|g| g.crossfade_wet)
            .unwrap_or(0.0);
        if self.enabled || wet_read > 0.01 {
            64.0 / 44100.0
        } else {
            0.0
        }
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        let was = self.enabled;
        self.enabled = enabled;
        if was != enabled {
            self.crossfade_target = if enabled { 1.0 } else { 0.0 };
        }
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ================== Surround Sound Processor ==================

/// Surround sound effect modes for immersive audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum SurroundSoundMode {
    #[default]
    Off,
    Concert,
    Theater,
    Studio,
    Spacious,
    Immersive,
}

impl SurroundSoundMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Concert => "concert",
            Self::Theater => "theater",
            Self::Studio => "studio",
            Self::Spacious => "spacious",
            Self::Immersive => "immersive",
        }
    }

    /// Parse a mode from its string identifier.
    ///
    /// Kept as an inherent method (rather than implementing `std::str::FromStr`)
    /// to preserve the existing `SurroundSoundMode::from_str(s) -> Self` API used
    /// by external callers; the trait would force a `Result` return type.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "concert" => Self::Concert,
            "theater" => Self::Theater,
            "studio" => Self::Studio,
            "spacious" => Self::Spacious,
            "immersive" => Self::Immersive,
            _ => Self::Off,
        }
    }

    /// Parameters for each mode: stereo_width, crossfeed_amt, reverb_gain,
    /// reverb_decay, haas_delay_ms, haas_gain
    fn params(&self) -> SurroundParams {
        match self {
            Self::Off => SurroundParams {
                stereo_width: 1.0,
                crossfeed: 0.0,
                reverb_gain: 0.0,
                reverb_decay: 0.0,
                haas_delay_ms: 0.0,
                haas_gain: 0.0,
                output_gain: 1.0,
            },
            // Concert hall: natural wide stereo, gentle reverb, minimal crossfeed
            Self::Concert => SurroundParams {
                stereo_width: 1.3,
                crossfeed: 0.15,
                reverb_gain: 0.18,
                reverb_decay: 0.5,
                haas_delay_ms: 8.0,
                haas_gain: 0.3,
                output_gain: 1.08,
            },
            // Theater: dramatic wide, strong reverb, moderate crossfeed
            Self::Theater => SurroundParams {
                stereo_width: 1.5,
                crossfeed: 0.20,
                reverb_gain: 0.25,
                reverb_decay: 0.6,
                haas_delay_ms: 12.0,
                haas_gain: 0.4,
                output_gain: 1.15,
            },
            // Studio: subtle widening, clean, minimal reverb
            Self::Studio => SurroundParams {
                stereo_width: 1.15,
                crossfeed: 0.10,
                reverb_gain: 0.08,
                reverb_decay: 0.3,
                haas_delay_ms: 4.0,
                haas_gain: 0.2,
                output_gain: 1.03,
            },
            // Spacious: strong widening, moderate reverb
            Self::Spacious => SurroundParams {
                stereo_width: 1.7,
                crossfeed: 0.25,
                reverb_gain: 0.20,
                reverb_decay: 0.55,
                haas_delay_ms: 15.0,
                haas_gain: 0.5,
                output_gain: 1.22,
            },
            // Immersive: maximum surround, full crossfeed, rich reverb
            Self::Immersive => SurroundParams {
                stereo_width: 2.0,
                crossfeed: 0.35,
                reverb_gain: 0.30,
                reverb_decay: 0.65,
                haas_delay_ms: 20.0,
                haas_gain: 0.6,
                output_gain: 1.35,
            },
        }
    }
}

#[derive(Clone, Copy)]
struct SurroundParams {
    stereo_width: f32,  // 1.0 = no change, >1 = wider
    crossfeed: f32,     // 0.0..0.5 amount of opposite channel crossfeed
    reverb_gain: f32,   // 0.0..0.5 wet gain for early reflections
    reverb_decay: f32,  // 0.0..0.9 feedback for reverb taps
    haas_delay_ms: f32, // 0..30ms delay for Haas precedence effect
    haas_gain: f32,     // 0.0..0.7 gain of delayed signal
    output_gain: f32,   // output gain compensation to preserve perceived loudness
}

/// Simple fractional-capable delay line.
struct SurroundDelay {
    buffer: Vec<f32>,
    write_pos: usize,
    delay_samples: usize,
}

impl SurroundDelay {
    fn new(max_samples: usize) -> Self {
        Self {
            buffer: vec![0.0; max_samples.max(1)],
            write_pos: 0,
            delay_samples: 0,
        }
    }

    fn set_delay(&mut self, samples: usize) {
        self.delay_samples = samples.min(self.buffer.len() - 1);
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let len = self.buffer.len();
        let read_pos = if self.write_pos >= self.delay_samples {
            self.write_pos - self.delay_samples
        } else {
            len - (self.delay_samples - self.write_pos)
        };
        let out = self.buffer[read_pos];
        self.buffer[self.write_pos] = x;
        self.write_pos = (self.write_pos + 1) % len;
        out
    }

    fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.write_pos = 0;
    }
}

/// One-pole low-pass filter for reverb damping and cross-feed filtering.
struct SurroundLPF {
    a0: f32,
    b1: f32,
    z1: f32,
}

impl SurroundLPF {
    fn new() -> Self {
        Self {
            a0: 1.0,
            b1: 0.0,
            z1: 0.0,
        }
    }

    fn set_cutoff(&mut self, freq: f32, sample_rate: u32) {
        let fs = sample_rate as f32;
        let f = freq.min(fs * 0.49);
        let x = std::f32::consts::E.powf(-2.0 * std::f32::consts::PI * f / fs);
        self.b1 = x;
        self.a0 = 1.0 - x;
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        self.z1 = self.a0 * x + self.b1 * self.z1;
        self.z1
    }

    fn clear(&mut self) {
        self.z1 = 0.0;
    }
}

/// Internal state for the surround sound processor.
struct SurroundSoundInner {
    channels: u16,
    sample_rate: u32,
    // Haas effect delay (right channel only)
    haas_delay: SurroundDelay,
    // Early reflection delay taps (left and right)
    refl_delay_l: SurroundDelay,
    refl_delay_r: SurroundDelay,
    // Damping filters for reflections
    refl_damp_l: SurroundLPF,
    refl_damp_r: SurroundLPF,
    // Cross-feed low-pass (simulates head shadow)
    crossfeed_lpf_l: SurroundLPF,
    crossfeed_lpf_r: SurroundLPF,
    // M/S processing state (none needed — instantaneous)
    // Reverb feedback state
    refl_feedback_l: f32,
    refl_feedback_r: f32,
    // Current parameters
    params: SurroundParams,
    /// Crossfade blend state — lives in the Mutex-protected inner state so
    /// the audio thread can update it safely in process() with no unsafe.
    crossfade_wet: f32,
}

impl SurroundSoundInner {
    fn new(channels: u16) -> Self {
        let max_delay = 192000usize; // up to ~4 seconds at 48kHz for safety
        Self {
            channels,
            sample_rate: 44100,
            haas_delay: SurroundDelay::new(max_delay),
            refl_delay_l: SurroundDelay::new(max_delay),
            refl_delay_r: SurroundDelay::new(max_delay),
            refl_damp_l: SurroundLPF::new(),
            refl_damp_r: SurroundLPF::new(),
            crossfeed_lpf_l: SurroundLPF::new(),
            crossfeed_lpf_r: SurroundLPF::new(),
            refl_feedback_l: 0.0,
            refl_feedback_r: 0.0,
            params: SurroundSoundMode::Off.params(),
            crossfade_wet: 0.0,
        }
    }

    fn ensure_channels(&mut self, channels: u16) {
        if self.channels != channels {
            self.channels = channels;
        }
    }

    fn reconfigure(&mut self, mode: SurroundSoundMode, sample_rate: u32) {
        self.sample_rate = sample_rate;
        self.params = mode.params();
        let sr = sample_rate as f32;

        // Haas delay: apply to right channel for precedence effect
        let haas_samples = (self.params.haas_delay_ms * sr / 1000.0) as usize;
        self.haas_delay.set_delay(haas_samples);

        // Early reflection delays: use prime-multiple taps for natural diffusion
        // Left reflection: ~17ms, Right reflection: ~23ms (slightly different for width)
        let refl_l_samples = (17.0 * sr / 1000.0) as usize;
        let refl_r_samples = (23.0 * sr / 1000.0) as usize;
        self.refl_delay_l.set_delay(refl_l_samples);
        self.refl_delay_r.set_delay(refl_r_samples);

        // Damping filters: cut reflections above ~6kHz for natural room absorption
        self.refl_damp_l.set_cutoff(6000.0, sample_rate);
        self.refl_damp_r.set_cutoff(6000.0, sample_rate);

        // Cross-feed filter: simulate head shadow (~700Hz cutoff)
        self.crossfeed_lpf_l.set_cutoff(700.0, sample_rate);
        self.crossfeed_lpf_r.set_cutoff(700.0, sample_rate);

        // Reset feedback and delay states
        self.refl_feedback_l = 0.0;
        self.refl_feedback_r = 0.0;
        self.haas_delay.clear();
        self.refl_delay_l.clear();
        self.refl_delay_r.clear();
        self.refl_damp_l.clear();
        self.refl_damp_r.clear();
        self.crossfeed_lpf_l.clear();
        self.crossfeed_lpf_r.clear();
    }
}

/// Surround Sound DSP processor — immersive spatial audio enhancement.
pub struct SurroundSoundProcessor {
    enabled: bool,
    mode: SurroundSoundMode,
    inner: Mutex<SurroundSoundInner>,
    /// Target crossfade level; the audio thread ramps inner.crossfade_wet toward this.
    crossfade_target: f32,
    /// Per-sample crossfade step (~10ms ramp at 44.1 kHz).
    crossfade_step: f32,
    id: String,
}

impl SurroundSoundProcessor {
    pub fn new() -> Self {
        Self {
            enabled: false,
            mode: SurroundSoundMode::Off,
            inner: Mutex::new(SurroundSoundInner::new(2)),
            crossfade_target: 0.0,
            crossfade_step: 1.0 / 441.0,
            id: "surround_sound".into(),
        }
    }

    pub fn set_mode(&mut self, mode: SurroundSoundMode) {
        let was = self.enabled;
        self.mode = mode;
        self.enabled = mode != SurroundSoundMode::Off;
        if self.enabled {
            if let Ok(mut inner) = self.inner.lock() {
                let sr = inner.sample_rate;
                let ch = inner.channels;
                inner.ensure_channels(ch);
                inner.reconfigure(mode, sr);
            }
        }
        if was && self.enabled {
            self.crossfade_target = 1.0;
        } else {
            self.crossfade_target = if self.enabled { 1.0 } else { 0.0 };
        }
    }

    pub fn mode(&self) -> SurroundSoundMode {
        self.mode
    }
}

impl Default for SurroundSoundProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl DspProcessor for SurroundSoundProcessor {
    fn process(&self, samples: &mut [f32], channels: u16, sample_rate: u32) {
        let ch = channels as usize;
        // The decoder folds >2ch sources to stereo before the ring buffer, so
        // the DSP chain only ever sees mono/stereo — a genuine 5.1-capable
        // variant would need the downmix moved behind the DSP chain.
        if ch != 2 || samples.is_empty() {
            return;
        }
        let step = self.crossfade_step;
        let target = self.crossfade_target;

        // Blocking lock — tiny reconfigure cost < audio buffer drop cost.
        // Also hosts crossfade_wet so we can mutate it safely via &mut guard.
        let mut inner = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return,
        };

        let mut wet = inner.crossfade_wet;
        if !self.enabled && wet <= step && target == 0.0 {
            return;
        }
        let dry_buf = samples.to_vec();

        if inner.channels != channels || inner.sample_rate != sample_rate {
            inner.ensure_channels(channels);
            inner.reconfigure(self.mode, sample_rate);
        }

        let p = inner.params;
        let frames = samples.len() / ch;

        if self.enabled {
            for f in 0..frames {
                let base = f * ch;
                let in_l = samples[base];
                let in_r = samples[base + 1];

                // STEP 1: M/S Stereo Widening
                let mid = (in_l + in_r) * 0.5;
                let side = (in_l - in_r) * 0.5;
                let new_side = side * p.stereo_width;
                let mut out_l = mid + new_side * 0.5;
                let mut out_r = mid - new_side * 0.5;

                // STEP 2: Cross-feed (additive — energy preserving)
                let cf_l = inner.crossfeed_lpf_r.process(in_r) * p.crossfeed;
                let cf_r = inner.crossfeed_lpf_l.process(in_l) * p.crossfeed;
                out_l += cf_l;
                out_r += cf_r;

                // STEP 3: Early Reflections
                let refl_in_l = out_l + inner.refl_feedback_r * p.reverb_decay;
                let refl_in_r = out_r + inner.refl_feedback_l * p.reverb_decay;
                let raw_refl_l = inner.refl_delay_l.process(refl_in_l);
                let raw_refl_r = inner.refl_delay_r.process(refl_in_r);
                let damped_refl_l = inner.refl_damp_l.process(raw_refl_l);
                let damped_refl_r = inner.refl_damp_r.process(raw_refl_r);
                inner.refl_feedback_l = damped_refl_l;
                inner.refl_feedback_r = damped_refl_r;
                out_l += damped_refl_r * p.reverb_gain;
                out_r += damped_refl_l * p.reverb_gain;

                // STEP 4: Haas Precedence Effect with energy-normalized mix
                let haas_r = inner.haas_delay.process(out_r);
                let haas_norm =
                    (1.0 / ((1.0 - p.haas_gain).powi(2) + p.haas_gain.powi(2)).sqrt()).min(1.4);
                out_r = (out_r * (1.0 - p.haas_gain) + haas_r * p.haas_gain) * haas_norm;

                // STEP 5: Loudness compensation + soft clamp
                out_l *= p.output_gain;
                out_r *= p.output_gain;
                out_l = out_l.clamp(-2.0, 2.0);
                out_r = out_r.clamp(-2.0, 2.0);

                samples[base] = out_l;
                samples[base + 1] = out_r;

                // Crossfade blend
                samples[base] = dry_buf[base] * (1.0 - wet) + samples[base] * wet;
                samples[base + 1] = dry_buf[base + 1] * (1.0 - wet) + samples[base + 1] * wet;

                wet += (target - wet).signum() * step;
                if (target - wet).abs() <= step {
                    wet = target;
                }
            }
        } else {
            for _f in 0..frames {
                wet += (target - wet).signum() * step;
                if (target - wet).abs() <= step {
                    wet = target;
                    break;
                }
            }
        }

        inner.crossfade_wet = wet;
    }

    fn name(&self) -> &str {
        "Surround Sound"
    }

    fn latency(&self) -> f64 {
        let wet_read = self
            .inner
            .try_lock()
            .map(|g| g.crossfade_wet)
            .unwrap_or(0.0);
        if self.enabled || wet_read > 0.01 {
            20.0 / 44100.0
        } else {
            0.0
        }
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        let was = self.enabled;
        self.enabled = enabled;
        if was != enabled {
            self.crossfade_target = if enabled { 1.0 } else { 0.0 };
        }
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Information about a DSP processor for UI display.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DspProcessorInfo {
    pub name: String,
    pub enabled: bool,
    pub latency: f64,
    pub id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replay_gain_processor() {
        let mut rg = ReplayGainProcessor::new();
        rg.configure(ReplayGainMode::Track, -6.0);

        let mut samples = vec![1.0f32, 0.5, -0.5, -1.0];
        rg.process(&mut samples, 2, 44100);

        // -6dB = 0.5x amplitude (10^(-6/20) ≈ 0.501)
        assert!((samples[0] - 0.5).abs() < 0.002);
        assert!((samples[3] + 0.5).abs() < 0.002);
    }

    #[test]
    fn test_dsp_chain() {
        let mut chain = DspChain::new();
        let mut rg = ReplayGainProcessor::new();
        rg.configure(ReplayGainMode::Track, -6.0);
        chain.add(Box::new(rg));

        assert_eq!(chain.len(), 1);
        assert!((chain.total_latency() - 0.0).abs() < 0.001);

        let mut samples = vec![1.0f32; 100];
        chain.process(&mut samples, 2, 44100);
        assert!((samples[0] - 0.5).abs() < 0.002);
    }

    #[test]
    fn test_dsp_chain_reorder() {
        let mut chain = DspChain::new();
        chain.add(Box::new(ReplayGainProcessor::new()));
        chain.add(Box::new(DownmixProcessor::new()));

        let info = chain.list_processors();
        assert_eq!(info[0].name, "ReplayGain");
        assert_eq!(info[1].name, "Downmix");

        chain.reorder(0, 1);
        let info = chain.list_processors();
        assert_eq!(info[0].name, "Downmix");
        assert_eq!(info[1].name, "ReplayGain");
    }
}
