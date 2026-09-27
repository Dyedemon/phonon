//! DSD (Direct Stream Digital) output via DoP (DSD over PCM)
//!
//! Converts PCM float samples to 1-bit DSD using a second-order sigma-delta
//! modulator, then packs the DSD bitstream into DoP-formatted 24-bit PCM frames.
//!
//! DSD64:  2.8224 MHz → packed into 176.4 kHz / 24-bit PCM  (16 DSD bits per sample)
//! DSD128: 5.6448 MHz → packed into 352.8 kHz / 24-bit PCM  (16 DSD bits per sample)
//! DSD256: 11.2896 MHz → packed into 705.6 kHz / 24-bit PCM (16 DSD bits per sample)

/// DSD output mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DsdMode {
    /// No DSD conversion (normal PCM output)
    Off,
    /// DSD64 via DoP (carrier: 176.4 kHz, 16x oversampling)
    DopDsd64,
    /// DSD128 via DoP (carrier: 352.8 kHz, 16x oversampling)
    DopDsd128,
    /// DSD256 via DoP (carrier: 705.6 kHz, 16x oversampling)
    DopDsd256,
}

impl DsdMode {
    /// Get the DoP carrier sample rate for this mode, or None if DSD is off.
    pub fn carrier_rate(self) -> Option<u32> {
        match self {
            DsdMode::Off => None,
            DsdMode::DopDsd64 => Some(176400),
            DsdMode::DopDsd128 => Some(352800),
            DsdMode::DopDsd256 => Some(705600),
        }
    }

    /// Number of DSD bits per DoP PCM sample (always 16).
    pub fn bits_per_sample(self) -> usize {
        16
    }
}

/// Second-order 1-bit sigma-delta modulator.
///
/// Converts a multi-bit PCM input to a 1-bit DSD stream.
/// Uses a classic double-integration topology.
pub struct DeltaSigmaModulator {
    integrator1: f64,
    integrator2: f64,
}

impl DeltaSigmaModulator {
    pub fn new() -> Self {
        Self {
            integrator1: 0.0,
            integrator2: 0.0,
        }
    }

    /// Process one PCM sample and produce `n` DSD bits.
    /// The input is zero-order held (repeated) across the oversampling interval.
    pub fn process(&mut self, input: f64, bits: &mut [u8], n: usize) {
        for (_i, bit) in bits.iter_mut().enumerate().take(n) {
            // Quantize: 1-bit output (±1)
            let output = if self.integrator2 >= 0.0 { 1.0 } else { -1.0 };
            *bit = if output > 0.0 { 1 } else { 0 };

            // Update integrators (error feedback)
            self.integrator1 += input - output;
            self.integrator2 += self.integrator1 - output;

            // Clip integrators to prevent runaway
            self.integrator1 = self.integrator1.clamp(-10.0, 10.0);
            self.integrator2 = self.integrator2.clamp(-10.0, 10.0);
        }
    }

    /// Reset the modulator state.
    pub fn reset(&mut self) {
        self.integrator1 = 0.0;
        self.integrator2 = 0.0;
    }
}

impl Default for DeltaSigmaModulator {
    fn default() -> Self {
        Self::new()
    }
}

/// DoP marker byte for DSD data (non-silence).
const DOP_MARKER: u8 = 0x05;
/// DoP marker byte for DSD silence.
const DOP_MARKER_SILENCE: u8 = 0xFA;

/// DSD silence byte: the standard alternating 0x69 pattern a DAC lowpasses
/// to analogue silence. Used to fill device-buffer gaps in native DSD
/// transport (a run of 0x00 bytes would be a full-scale DC step instead).
pub const DSD_SILENCE_BYTE_F32: f32 = 0x69 as f32;

/// DoP silence word (0xFA marker + 0x69 pattern) as a sign-extended
/// exact-integer f32, for filling ring-buffer underrun gaps on the DoP
/// transport (a zero word has marker 0x00 and can desync a strict DAC).
pub const DOP_SILENCE_WORD_F32: f32 = {
    let w = ((DOP_MARKER_SILENCE as u32) << 16) | 0x6969;
    // Sign-extend the 24-bit word, then reinterpret through i32 — done
    // arithmetically so the const stays legal.
    let v = if w & 0x800000 != 0 { w | 0xFF000000 } else { w };
    (v as i32) as f32
};

/// Pack interleaved DSD bytes into DoP 24-bit words stored as
/// exact-integer f32 samples.
///
/// `dsd` holds one byte (8 bits, MSB first) per channel per frame, exactly
/// what `phonon_codec::DsdReader::read_interleaved` produces. Two
/// consecutive bytes per channel form one DoP word (16 DSD bits), so the
/// output rate is the DSD bit rate / 16 per channel — the DoP carrier rate.
/// `marker_parity` carries the 0x05/0xFA alternation across calls and MUST
/// persist for the whole track so a DAC sees an unbroken marker sequence.
///
/// Returns the number of f32 words appended to `out` (interleaved per
/// frame: L word, R word, …).
pub fn dsd_bytes_to_dop(
    dsd: &[u8],
    channels: usize,
    out: &mut Vec<f32>,
    marker_parity: &mut bool,
) -> usize {
    let ch = channels.max(1);
    let frames = dsd.len() / ch;
    let word_frames = frames / 2; // 2 DSD bytes per channel per DoP word
    out.reserve(word_frames * ch);
    for wf in 0..word_frames {
        let marker: u32 = if *marker_parity {
            DOP_MARKER_SILENCE as u32
        } else {
            DOP_MARKER as u32
        };
        *marker_parity = !*marker_parity;
        for c in 0..ch {
            let b0 = dsd[(wf * 2) * ch + c] as u32;
            let b1 = dsd[(wf * 2 + 1) * ch + c] as u32;
            let dop_value = (marker << 16) | (b0 << 8) | b1;
            // Sign-extend the 24-bit word; the PCM24 transport shifts it
            // left-aligned into the 32-bit container bit-exact.
            let v24 = if dop_value & 0x800000 != 0 {
                dop_value | 0xFF000000
            } else {
                dop_value
            };
            out.push(v24 as i32 as f32);
        }
    }
    word_frames * ch
}

/// Pack a DSD bitstream into DoP-formatted PCM frames.
///
/// Each DoP frame is a 24-bit PCM sample where:
/// - Bits 23-16: marker byte (0x05 or 0xFA)
/// - Bits 15-0: 16 DSD bits
///
/// The output `pcm` buffer receives 32-bit float samples in the range [0.0, 1.0],
/// representing the 24-bit DoP values scaled so that the host can cast to 24-bit int.
/// `pcm` must have length >= `dsd_bits.len() / 16`.
pub fn dsd_to_dop(dsd_bits: &[u8], pcm: &mut [f32]) {
    // Max 24-bit value for normalization
    const MAX_24BIT: f32 = (1 << 23) as f32;

    for (out_idx, chunk) in dsd_bits.chunks(16).enumerate() {
        if out_idx >= pcm.len() {
            break;
        }

        let mut dsd_word: u16 = 0;
        for (i, &bit) in chunk.iter().enumerate() {
            if bit != 0 {
                dsd_word |= 1 << (15 - i);
            }
        }

        // DoP standard: the marker byte alternates 0x05/0xFA on every sample
        // (it encodes a per-sample parity the DAC checks) — it must NOT
        // depend on the data content.
        let marker: u32 = if out_idx % 2 == 0 {
            DOP_MARKER as u32
        } else {
            DOP_MARKER_SILENCE as u32
        };

        // 24-bit DoP word: marker << 16 | dsd_word
        let dop_value = (marker << 16) | (dsd_word as u32);

        // Normalize to float (0.0 to 1.0 range for 24-bit unsigned)
        pcm[out_idx] = dop_value as f32 / MAX_24BIT;
    }
}

/// Convert a buffer of PCM float samples to DoP-encoded output.
///
/// `pcm_input` is the original PCM float buffer (interleaved channels).
/// `pcm_output` receives the DoP-encoded output. Must have length >=
/// `pcm_input.len() * samples_per_channel / channels`.
/// `channels` is the number of audio channels.
/// `dsd_mode` specifies the DSD mode.
///
/// The sigma-delta modulator operates on each channel independently.
/// Returns the number of DoP samples written to `pcm_output`.
pub fn convert_pcm_to_dop(
    pcm_input: &[f32],
    pcm_output: &mut [f32],
    channels: usize,
    dsd_mode: DsdMode,
    modulators: &mut [DeltaSigmaModulator],
) -> usize {
    if channels == 0 || dsd_mode == DsdMode::Off {
        return 0;
    }

    let oversample = dsd_mode.bits_per_sample(); // 16
    let frame_count = pcm_input.len() / channels;
    let mut out_idx = 0;

    // Temporary buffers for DSD bits per channel
    let mut dsd_buf = vec![0u8; oversample * frame_count * channels];

    // Process each channel independently through the sigma-delta modulator
    for ch in 0..channels {
        let modulator = &mut modulators[ch];
        let ch_offset = ch * oversample * frame_count;

        for frame in 0..frame_count {
            let input = pcm_input[frame * channels + ch] as f64;
            let offset = ch_offset + frame * oversample;
            let bits = &mut dsd_buf[offset..offset + oversample];
            modulator.process(input, bits, oversample);
        }
    }

    // Interleave DSD bits into DoP frames
    for frame in 0..frame_count {
        for bit_pos in 0..oversample {
            for ch in 0..channels {
                let ch_offset = ch * oversample * frame_count;
                let _dsd_bit = dsd_buf[ch_offset + frame * oversample + bit_pos];

                // Build DoP word: 8 DSD bits packed per channel
                let _byte_idx = bit_pos / 8;
                let _bit_idx = bit_pos % 8;

                // We need to accumulate 16 bits per channel for DoP
                // For simplicity, pack 16 DSD bits into one DoP sample
                if bit_pos % 16 == 0 {
                    let mut dsd_word: u16 = 0;
                    for b in 0..16.min(oversample - bit_pos) {
                        let ch_off = ch_offset + frame * oversample + bit_pos + b;
                        if ch_off < dsd_buf.len() && dsd_buf[ch_off] != 0 {
                            dsd_word |= 1 << (15 - b);
                        }
                    }
                    let marker: u32 = if out_idx % 2 == 0 {
                        DOP_MARKER as u32
                    } else {
                        DOP_MARKER_SILENCE as u32
                    };
                    let dop_value = (marker << 16) | (dsd_word as u32);
                    if out_idx < pcm_output.len() {
                        pcm_output[out_idx] = dop_value as f32 / ((1 << 23) as f32);
                        out_idx += 1;
                    }
                }
            }
        }
    }

    out_idx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sigma_delta_zero_input() {
        let mut mod1 = DeltaSigmaModulator::new();
        let mut bits = [0u8; 16];
        mod1.process(0.0, &mut bits, 16);
        // For zero input, the output should alternate between 0 and 1
        let sum: u32 = bits.iter().map(|&b| b as u32).sum();
        assert!(
            sum > 0 && sum < 16,
            "Zero input should produce alternating bits, got {}",
            sum
        );
    }

    #[test]
    fn test_sigma_delta_positive_input() {
        let mut mod1 = DeltaSigmaModulator::new();
        let mut bits = [0u8; 16];
        mod1.process(0.5, &mut bits, 16);
        let ones: u32 = bits.iter().map(|&b| b as u32).sum();
        assert!(
            ones > 8,
            "Positive input should produce more ones than zeros, got {}",
            ones
        );
    }

    #[test]
    fn test_sigma_delta_negative_input() {
        let mut mod1 = DeltaSigmaModulator::new();
        let mut bits = [0u8; 16];
        mod1.process(-0.5, &mut bits, 16);
        let ones: u32 = bits.iter().map(|&b| b as u32).sum();
        assert!(
            ones < 8,
            "Negative input should produce more zeros than ones, got {}",
            ones
        );
    }

    #[test]
    fn test_dop_encoding() {
        let dsd_bits = [1u8, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0];
        let mut pcm = [0.0f32; 1];
        dsd_to_dop(&dsd_bits, &mut pcm);

        let dop_value = (pcm[0] * ((1 << 23) as f32)) as u32;
        let marker = (dop_value >> 16) as u8;
        let dsd_word = (dop_value & 0xFFFF) as u16;

        // First sample carries the 0x05 marker.
        assert_eq!(marker, DOP_MARKER);
        assert_eq!(dsd_word, 0xAAAA);
    }

    #[test]
    fn test_dop_marker_alternates() {
        // The marker byte must alternate 0x05 / 0xFA per sample regardless of
        // the DSD data content (the old data-dependent marker was invalid).
        let dsd = [0u8; 48]; // 3 samples of silence
        let mut pcm = [0.0f32; 3];
        dsd_to_dop(&dsd, &mut pcm);

        for (i, p) in pcm.iter().enumerate() {
            let dop_value = (*p * ((1 << 23) as f32)) as u32;
            let expected = if i % 2 == 0 {
                DOP_MARKER
            } else {
                DOP_MARKER_SILENCE
            };
            assert_eq!((dop_value >> 16) as u8, expected, "sample {i}");
        }
    }

    #[test]
    fn test_dop_silence_encoding() {
        // Second sample (odd index) carries the 0xFA marker.
        let dsd_bits = [0u8; 32];
        let mut pcm = [0.0f32; 2];
        dsd_to_dop(&dsd_bits, &mut pcm);

        let dop_value = (pcm[1] * ((1 << 23) as f32)) as u32;
        let marker = (dop_value >> 16) as u8;

        assert_eq!(marker, DOP_MARKER_SILENCE);
    }

    #[test]
    fn test_dsd_bytes_to_dop() {
        // Two stereo frames of real DSD bytes → one DoP word per channel.
        let dsd = [0xAA, 0xBB, 0xCC, 0xDD];
        let mut out = Vec::new();
        let mut parity = false;

        let n = dsd_bytes_to_dop(&dsd, 2, &mut out, &mut parity);
        assert_eq!(n, 2);
        assert!(parity, "marker parity flips per word frame");
        let w0 = (out[0] as i32) as u32 & 0xFFFFFF;
        let w1 = (out[1] as i32) as u32 & 0xFFFFFF;
        assert_eq!(w0, (0x05 << 16) | 0xAACC);
        assert_eq!(w1, (0x05 << 16) | 0xBBDD);

        // A second call continues the marker sequence (0xFA next).
        let n = dsd_bytes_to_dop(&dsd, 2, &mut out, &mut parity);
        assert_eq!(n, 2);
        let w2 = (out[2] as i32) as u32 & 0xFFFFFF;
        assert_eq!((w2 >> 16) as u8, DOP_MARKER_SILENCE);
        assert_eq!((w2 & 0xFFFF) as u16, 0xAACC);
    }
}
