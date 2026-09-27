//! DSD (Direct Stream Digital) file support.
//!
//! Parses DSF and DFF files and produces byte-interleaved DSD streams
//! (8 bits per byte, MSB first, one byte per channel per frame) suitable
//! for native-DSD or DoP output. DSF stores audio in per-channel blocks
//! (block_size bytes of channel 0, then channel 1, …) — that layout is
//! de-interleaved here so downstream consumers never see it.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// DSD format type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DsdFormat {
    Dsf,
    Dff,
}

/// DSD file metadata.
#[derive(Debug, Clone)]
pub struct DsdMetadata {
    pub format: DsdFormat,
    pub sample_rate: u32, // DSD bit rate (e.g., 2822400 for DSD64)
    pub channels: u16,
    pub duration: f64, // seconds
    /// Per-channel DSD bits (sampleCount / framesTotal).
    pub total_samples: u64,
    /// 1 for standard DSD packing; 8 for DSD-wide (one bit per byte, LSB).
    pub bits_per_sample: u16,
    pub title: Option<String>,
    pub artist: Option<String>,
}

/// DSD reader that provides byte-interleaved DSD data.
pub struct DsdReader {
    metadata: DsdMetadata,
    reader: BufReader<File>,
    data_start: u64,
    current_position: u64,
    /// Per-channel source bytes still unread (accounts for block padding).
    bytes_remaining_per_ch: u64,
    /// Per-channel source bytes in the audio stream total.
    total_per_ch_bytes: u64,
    /// DSF block size in bytes per channel; 0 for DFF (already interleaved).
    block_size: usize,
    /// De-interleaved bytes decoded from the current block, not yet consumed.
    pending: Vec<u8>,
    pending_pos: usize,
}

impl DsdReader {
    /// Open a DSD file (DSF or DFF).
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, DsdError> {
        let path = path.as_ref();
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);

        let format = match path.extension().and_then(|e| e.to_str()) {
            Some("dsf") | Some("DSF") => DsdFormat::Dsf,
            Some("dff") | Some("DFF") => DsdFormat::Dff,
            _ => return Err(DsdError::UnsupportedFormat),
        };

        let (metadata, data_start, block_size) = match format {
            DsdFormat::Dsf => parse_dsf(&mut reader)?,
            DsdFormat::Dff => parse_dff(&mut reader)?,
        };

        // Per-channel source bytes: 8 samples (bits) per byte for standard
        // packing; one byte per sample for DSD-wide.
        let total_per_ch_bytes = if metadata.bits_per_sample == 8 {
            metadata.total_samples
        } else {
            metadata.total_samples.div_ceil(8)
        };

        Ok(Self {
            metadata,
            reader,
            data_start,
            current_position: data_start,
            bytes_remaining_per_ch: total_per_ch_bytes,
            total_per_ch_bytes,
            block_size,
            pending: Vec::new(),
            pending_pos: 0,
        })
    }

    /// Get file metadata.
    pub fn metadata(&self) -> &DsdMetadata {
        &self.metadata
    }

    /// Read byte-interleaved DSD data: one byte (8 bits, MSB first) per
    /// channel per frame. Returns the number of bytes written (always a
    /// multiple of the channel count, except possibly at clean EOF where
    /// it is 0).
    pub fn read_interleaved(&mut self, out: &mut [u8]) -> Result<usize, DsdError> {
        let ch = self.metadata.channels as usize;
        if ch == 0 {
            return Ok(0);
        }
        let mut written = 0;
        while written < out.len() {
            if self.pending_pos < self.pending.len() {
                let n = (self.pending.len() - self.pending_pos).min(out.len() - written);
                out[written..written + n]
                    .copy_from_slice(&self.pending[self.pending_pos..self.pending_pos + n]);
                self.pending_pos += n;
                written += n;
                continue;
            }
            if self.bytes_remaining_per_ch == 0 {
                break;
            }
            self.refill()?;
            if self.pending.is_empty() {
                break; // EOF / decode failure
            }
        }
        // Only whole frames leave this function.
        let whole = written / ch * ch;
        if whole < written {
            // Push the partial-frame tail back for the next call.
            self.pending = out[whole..written].to_vec();
            self.pending_pos = 0;
        }
        Ok(whole)
    }

    /// Decode the next DSF block (or the next raw span for DFF) into the
    /// byte-interleaved `pending` buffer.
    fn refill(&mut self) -> Result<(), DsdError> {
        let ch = self.metadata.channels as usize;
        if self.block_size > 0 {
            // DSF: one block = block_size bytes per channel, channel-major.
            let valid = (self.block_size as u64).min(self.bytes_remaining_per_ch) as usize;
            let mut block = vec![0u8; self.block_size * ch];
            let mut read = 0;
            while read < block.len() {
                match self.reader.read(&mut block[read..]) {
                    Ok(0) => break,
                    Ok(n) => read += n,
                    Err(e) => return Err(e.into()),
                }
            }
            self.current_position += read as u64;
            self.bytes_remaining_per_ch = self.bytes_remaining_per_ch.saturating_sub(valid as u64);
            if read == 0 {
                self.pending.clear();
                return Ok(());
            }
            // valid must be counted only if the full block read succeeded;
            // if EOF truncated the block mid-way, scale valid down.
            let per_ch_read = read / ch;
            let valid = valid.min(per_ch_read);
            let mut inter = Vec::with_capacity(valid * ch);
            for i in 0..valid {
                for c in 0..ch {
                    inter.push(block[c * self.block_size + i]);
                }
            }
            self.pending = if self.metadata.bits_per_sample == 8 {
                // DSD-wide: each source byte carries one sample bit (LSB);
                // gather 8 of them into one packed byte, MSB first.
                inter
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|g| g.iter().fold(0u8, |acc, &b| (acc << 1) | (b & 1)))
                    .collect()
            } else {
                inter
            };
        } else {
            // DFF: frames are already byte-interleaved in the file.
            let frames = (self.bytes_remaining_per_ch.min(1 << 16)) as usize;
            let want = frames * ch;
            let mut buf = vec![0u8; want];
            let mut read = 0;
            while read < want {
                match self.reader.read(&mut buf[read..]) {
                    Ok(0) => break,
                    Ok(n) => read += n,
                    Err(e) => return Err(e.into()),
                }
            }
            self.current_position += read as u64;
            let whole_frames = read / ch;
            self.bytes_remaining_per_ch = self
                .bytes_remaining_per_ch
                .saturating_sub(whole_frames as u64);
            buf.truncate(whole_frames * ch);
            self.pending = buf;
        }
        self.pending_pos = 0;
        Ok(())
    }

    /// Seek to a specific frame index (one frame = one DSD byte per channel).
    pub fn seek_frames(&mut self, frames: u64) -> Result<(), DsdError> {
        let ch = self.metadata.channels as usize;
        let frames = frames.min(self.total_per_ch_bytes);
        let file_off = if self.block_size > 0 {
            let block = self.block_size as u64;
            let block_idx = frames / block;
            self.data_start + block_idx * (block * ch as u64)
        } else {
            self.data_start + frames * ch as u64
        };
        self.reader.seek(SeekFrom::Start(file_off))?;
        self.current_position = file_off;
        let consumed = if self.block_size > 0 {
            (frames / self.block_size as u64) * self.block_size as u64
        } else {
            frames
        };
        self.bytes_remaining_per_ch = self.total_per_ch_bytes.saturating_sub(consumed);
        self.pending.clear();
        self.pending_pos = 0;
        // Discard the intra-block head so the next read starts at `frames`.
        if self.block_size > 0 {
            let in_block = (frames % self.block_size as u64) as usize;
            if in_block > 0 {
                let mut scratch = vec![0u8; in_block * ch];
                self.read_interleaved(&mut scratch)?;
            }
        }
        Ok(())
    }

    /// Seek to a byte offset within the audio stream (legacy API; the offset
    /// is interpreted as a frame index — one DSD byte per channel).
    pub fn seek(&mut self, offset: u64) -> Result<(), DsdError> {
        self.seek_frames(offset)
    }
}

/// Parse DSF file header. Returns (metadata, data_start, block_size) with the
/// reader positioned at the first data byte.
fn parse_dsf<R: Read + Seek>(reader: &mut R) -> Result<(DsdMetadata, u64, usize), DsdError> {
    let mut header = [0u8; 4];

    // Chunk ID "DSD "
    reader.read_exact(&mut header)?;
    if &header != b"DSD " {
        return Err(DsdError::InvalidHeader("Not a DSF file".into()));
    }

    // Chunk size (skip)
    reader.read_exact(&mut [0u8; 8])?;

    // Total file size (skip)
    reader.read_exact(&mut [0u8; 8])?;

    // ID3v2 offset (skip)
    reader.read_exact(&mut [0u8; 8])?;

    // "fmt " chunk
    reader.read_exact(&mut header)?;
    if &header != b"fmt " {
        return Err(DsdError::InvalidHeader("Missing fmt chunk".into()));
    }

    // fmt chunk size
    let mut size_buf = [0u8; 8];
    reader.read_exact(&mut size_buf)?;

    // Format version (unused, must be 1)
    let mut _version = [0u8; 4];
    reader.read_exact(&mut _version)?;

    // Format ID (0 = DSD raw)
    let mut format_id = [0u8; 4];
    reader.read_exact(&mut format_id)?;
    if u32::from_le_bytes(format_id) != 0 {
        return Err(DsdError::InvalidHeader("DSF format ID != 0 (DST?)".into()));
    }

    // Channel type (mono/stereo/multich layout tag — channel count is what matters)
    let mut _channel_type = [0u8; 4];
    reader.read_exact(&mut _channel_type)?;

    // Channel number
    let mut channels = [0u8; 4];
    reader.read_exact(&mut channels)?;
    let channels = u32::from_le_bytes(channels) as u16;
    if channels == 0 {
        return Err(DsdError::InvalidHeader("Zero channel count".into()));
    }

    // Sample rate (DSD bit rate)
    let mut sample_rate = [0u8; 4];
    reader.read_exact(&mut sample_rate)?;
    let sample_rate = u32::from_le_bytes(sample_rate);
    if sample_rate == 0 {
        return Err(DsdError::InvalidHeader("Zero sample rate".into()));
    }

    // Bits per sample (1 or 8)
    let mut bits = [0u8; 4];
    reader.read_exact(&mut bits)?;
    let bits_per_sample = u32::from_le_bytes(bits) as u16;

    // Sample count (per channel, in bits)
    let mut sample_count = [0u8; 8];
    reader.read_exact(&mut sample_count)?;
    let total_samples = u64::from_le_bytes(sample_count);

    let duration = total_samples as f64 / sample_rate as f64;

    // Block size per channel (must be a multiple of 4096 per spec)
    let mut block_size = [0u8; 4];
    reader.read_exact(&mut block_size)?;
    let block_size = u32::from_le_bytes(block_size) as usize;
    if block_size == 0 {
        return Err(DsdError::InvalidHeader("Zero block size".into()));
    }

    // Reserved (4 bytes)
    let mut _reserved = [0u8; 4];
    reader.read_exact(&mut _reserved)?;

    // Skip to data chunk
    loop {
        let mut chunk_header = [0u8; 4];
        if reader.read_exact(&mut chunk_header).is_err() {
            break;
        }

        let mut chunk_size = [0u8; 8];
        reader.read_exact(&mut chunk_size)?;
        let chunk_size = u64::from_le_bytes(chunk_size);

        if &chunk_header == b"data" {
            let meta = DsdMetadata {
                format: DsdFormat::Dsf,
                sample_rate,
                channels,
                duration,
                total_samples,
                bits_per_sample,
                title: None,
                artist: None,
            };
            let data_start = reader.stream_position()?;
            return Ok((meta, data_start, block_size));
        } else {
            // Skip unknown chunk
            let mut skip = vec![0u8; chunk_size as usize];
            reader.read_exact(&mut skip)?;
        }
    }

    Err(DsdError::InvalidHeader("No data chunk found".into()))
}

/// Parse DFF (DSDIFF) file header. Returns (metadata, data_start, 0) with the
/// reader positioned at the first data byte of the DSD chunk.
fn parse_dff<R: Read + Seek>(reader: &mut R) -> Result<(DsdMetadata, u64, usize), DsdError> {
    let mut header = [0u8; 4];

    // "FRM8" container
    reader.read_exact(&mut header)?;
    if &header != b"FRM8" {
        return Err(DsdError::InvalidHeader("Not a DFF file".into()));
    }

    // FRM8 size
    let mut size_buf = [0u8; 8];
    reader.read_exact(&mut size_buf)?;

    // Form type "DSD "
    reader.read_exact(&mut header)?;
    if &header != b"DSD " {
        return Err(DsdError::InvalidHeader("Missing DSD form type".into()));
    }

    let mut sample_rate: u32 = 0;
    let mut channels: u16 = 0;
    let mut total_samples: u64 = 0;
    let mut dsd_data_start: Option<u64> = None;
    let mut dsd_data_size: u64 = 0;

    // Top-level chunks inside FRM8
    loop {
        let mut chunk_header = [0u8; 4];
        if reader.read_exact(&mut chunk_header).is_err() {
            break;
        }
        let mut size_buf = [0u8; 8];
        if reader.read_exact(&mut size_buf).is_err() {
            break;
        }
        let chunk_size = u64::from_be_bytes(size_buf);

        match &chunk_header {
            b"FVER" | b"COMT" | b"DIIN" | b"ID3 " | b"DSTI" => {
                skip_aligned(reader, chunk_size)?;
            }
            b"FS  " => {
                // Standalone sampling-frequency property (4 bytes BE)
                let mut buf = [0u8; 4];
                reader.read_exact(&mut buf)?;
                sample_rate = u32::from_be_bytes(buf);
                skip_aligned(reader, chunk_size.saturating_sub(4))?;
            }
            b"PROP" => {
                let mut prop_id = [0u8; 4];
                reader.read_exact(&mut prop_id)?;
                let mut consumed = 4u64;
                if &prop_id == b"SND " {
                    // numChannels(2) channelType(4) compressionType(4)
                    // samplingFreq(4) framesTotal(8)
                    let mut ch_buf = [0u8; 2];
                    reader.read_exact(&mut ch_buf)?;
                    channels = u16::from_be_bytes(ch_buf);
                    let mut ct_buf = [0u8; 4];
                    reader.read_exact(&mut ct_buf)?; // channelType (unused)
                    reader.read_exact(&mut ct_buf)?;
                    let compression = ct_buf;
                    let mut sr_buf = [0u8; 4];
                    reader.read_exact(&mut sr_buf)?;
                    sample_rate = u32::from_be_bytes(sr_buf);
                    let mut fs_buf = [0u8; 8];
                    reader.read_exact(&mut fs_buf)?;
                    total_samples = u64::from_be_bytes(fs_buf);
                    consumed += 2 + 4 + 4 + 4 + 8;
                    if &compression == b"DST " {
                        return Err(DsdError::UnsupportedFormat);
                    }
                }
                skip_aligned(reader, chunk_size.saturating_sub(consumed))?;
            }
            b"DST " => {
                return Err(DsdError::UnsupportedFormat);
            }
            b"DSD " => {
                dsd_data_start = Some(reader.stream_position()?);
                dsd_data_size = chunk_size;
                // Do not skip the audio — seek back to its start before
                // returning so the caller can stream it directly.
                skip_aligned(reader, chunk_size)?;
            }
            _ => {
                skip_aligned(reader, chunk_size)?;
            }
        }
    }

    let channels = if channels == 0 { 2 } else { channels };
    if sample_rate == 0 {
        sample_rate = 2822400; // DSD64 fallback
    }
    if total_samples == 0 && dsd_data_start.is_some() {
        total_samples = dsd_data_size.saturating_mul(8) / channels.max(1) as u64;
    }

    let data_start =
        dsd_data_start.ok_or_else(|| DsdError::InvalidHeader("No DSD audio chunk found".into()))?;
    reader.seek(SeekFrom::Start(data_start))?;

    let duration = total_samples as f64 / sample_rate as f64;
    Ok((
        DsdMetadata {
            format: DsdFormat::Dff,
            sample_rate,
            channels,
            duration,
            total_samples,
            bits_per_sample: 1,
            title: None,
            artist: None,
        },
        data_start,
        0,
    ))
}

/// Skip `n` bytes, honouring the DSDIFF even-alignment padding rule.
fn skip_aligned<R: Read>(reader: &mut R, n: u64) -> Result<(), DsdError> {
    let total = n + (n & 1);
    let mut remaining = total;
    let mut buf = [0u8; 4096];
    while remaining > 0 {
        let want = remaining.min(buf.len() as u64) as usize;
        match reader.read(&mut buf[..want]) {
            Ok(0) => break,
            Ok(n) => remaining -= n as u64,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Errors for DSD operations.
#[derive(Debug, thiserror::Error)]
pub enum DsdError {
    #[error("Unsupported format")]
    UnsupportedFormat,
    #[error("Invalid header: {0}")]
    InvalidHeader(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal 2-channel DSF file in memory:
    /// block_size=4, 6 valid bytes per channel (48 bits), 2 blocks where the
    /// last one is half padding.
    fn make_dsf() -> Vec<u8> {
        let mut v = Vec::new();
        let payload: Vec<u8> = [
            // Block 1: L[0..4], R[0..4]
            10, 11, 12, 13, 20, 21, 22, 23, // Block 2: L[4..6] + 2 pad, R[4..6] + 2 pad
            14, 15, 0, 0, 24, 25, 0, 0,
        ]
        .to_vec();

        // "DSD " chunk header (size, filesize, id3 offset)
        v.extend_from_slice(b"DSD ");
        v.extend_from_slice(&28u64.to_le_bytes());
        v.extend_from_slice(&0u64.to_le_bytes()); // placeholder file size
        v.extend_from_slice(&0u64.to_le_bytes()); // id3v2 offset

        // "fmt " chunk
        v.extend_from_slice(b"fmt ");
        v.extend_from_slice(&52u64.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes()); // version
        v.extend_from_slice(&0u32.to_le_bytes()); // format id (DSD raw)
        v.extend_from_slice(&1u32.to_le_bytes()); // channel type (stereo)
        v.extend_from_slice(&2u32.to_le_bytes()); // channels
        v.extend_from_slice(&2822400u32.to_le_bytes()); // DSD64
        v.extend_from_slice(&1u32.to_le_bytes()); // bits per sample
        v.extend_from_slice(&48u64.to_le_bytes()); // sample count (bits/ch)
        v.extend_from_slice(&4u32.to_le_bytes()); // block size
        v.extend_from_slice(&0u32.to_le_bytes()); // reserved

        // "data" chunk
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        v.extend_from_slice(&payload);
        // Patch the file size field (offset 12).
        let fsz = (v.len() as u64).to_le_bytes();
        v[12..20].copy_from_slice(&fsz);
        v
    }

    fn open_temp(bytes: &[u8], ext: &str) -> DsdReader {
        let mut path = std::env::temp_dir();
        path.push(format!("phonon_dsd_test_{}.{}", std::process::id(), ext));
        std::fs::write(&path, bytes).unwrap();
        DsdReader::open(&path).unwrap()
    }

    #[test]
    fn dsf_block_deinterleave() {
        let mut r = open_temp(&make_dsf(), "dsf");
        assert_eq!(r.metadata().channels, 2);
        assert_eq!(r.metadata().sample_rate, 2822400);
        assert_eq!(r.metadata().duration, 48.0 / 2_822_400.0);

        let mut out = [0u8; 12];
        let n = r.read_interleaved(&mut out).unwrap();
        assert_eq!(n, 12);
        assert_eq!(out, [10, 20, 11, 21, 12, 22, 13, 23, 14, 24, 15, 25]);

        // EOF
        let n = r.read_interleaved(&mut out).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn dsf_seek_frames() {
        let mut r = open_temp(&make_dsf(), "dsf");
        r.seek_frames(3).unwrap();
        let mut out = [0u8; 4];
        let n = r.read_interleaved(&mut out).unwrap();
        assert_eq!(n, 4);
        assert_eq!(out, [13, 23, 14, 24]);
    }

    /// Build a minimal 2-channel DFF file: FRM8 { FVER, PROP(SND), DSD }.
    fn make_dff() -> Vec<u8> {
        let mut v = Vec::new();
        // 16 frames interleaved: L=0x10+i, R=0x50+i
        let mut payload = Vec::new();
        for i in 0..16u8 {
            payload.push(0x10 + i);
            payload.push(0x50 + i);
        }

        let mut body = Vec::new();
        body.extend_from_slice(b"DSD "); // form type

        body.extend_from_slice(b"FVER");
        body.extend_from_slice(&4u64.to_be_bytes());
        body.extend_from_slice(&0x01050000u32.to_be_bytes());

        body.extend_from_slice(b"PROP");
        body.extend_from_slice(&26u64.to_be_bytes());
        body.extend_from_slice(b"SND ");
        body.extend_from_slice(&2u16.to_be_bytes()); // channels
        body.extend_from_slice(&0u32.to_be_bytes()); // channel type
        body.extend_from_slice(b"DSD "); // compression (uncompressed)
        body.extend_from_slice(&2822400u32.to_be_bytes());
        body.extend_from_slice(&128u64.to_be_bytes()); // framesTotal (bits/ch)

        body.extend_from_slice(b"DSD ");
        body.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        body.extend_from_slice(&payload);

        v.extend_from_slice(b"FRM8");
        v.extend_from_slice(&(body.len() as u64).to_be_bytes());
        v.extend_from_slice(&body);
        v
    }

    #[test]
    fn dff_parse_and_read() {
        let mut r = open_temp(&make_dff(), "dff");
        assert_eq!(r.metadata().channels, 2);
        assert_eq!(r.metadata().sample_rate, 2822400);
        assert_eq!(r.metadata().duration, 128.0 / 2_822_400.0);

        let mut out = [0u8; 8];
        let n = r.read_interleaved(&mut out).unwrap();
        assert_eq!(n, 8);
        assert_eq!(out, [0x10, 0x50, 0x11, 0x51, 0x12, 0x52, 0x13, 0x53]);

        r.seek_frames(10).unwrap();
        let n = r.read_interleaved(&mut out).unwrap();
        assert_eq!(n, 8);
        assert_eq!(out, [0x1A, 0x5A, 0x1B, 0x5B, 0x1C, 0x5C, 0x1D, 0x5D]);
    }

    #[test]
    fn dff_dst_rejected() {
        let mut v = make_dff();
        // Swap the compression type marker to "DST ": it sits 10 bytes into
        // the SND property (id 4 + channels 2 + channelType 4).
        let pos = v.windows(4).position(|w| w == b"SND ").unwrap();
        let cpos = pos + 10;
        v[cpos..cpos + 4].copy_from_slice(b"DST ");
        let mut path = std::env::temp_dir();
        path.push(format!("phonon_dsd_test_dst{}.dff", std::process::id()));
        std::fs::write(&path, &v).unwrap();
        assert!(matches!(
            DsdReader::open(&path),
            Err(DsdError::UnsupportedFormat)
        ));
    }
}
