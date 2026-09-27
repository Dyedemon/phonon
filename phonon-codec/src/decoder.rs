//! Audio decoder - unified interface for all supported formats.
//!
//! Based on Symphonia for FLAC, ALAC, WAV, MP3, AAC formats.
//! Outputs 32-bit float PCM.

use std::fs::File;
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

use crate::types::*;

/// Result type for codec operations.
pub type CodecResult<T> = Result<T, CodecError>;

/// Errors that can occur during decoding.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Decode error: {0}")]
    Decode(String),

    #[error("Metadata error: {0}")]
    Metadata(String),

    #[error("End of stream")]
    EndOfStream,

    /// Operation is not yet implemented in this release (RC1). The message
    /// is suitable for showing to the user (e.g. as a toast/banner).
    #[error("Not implemented: {0}")]
    NotImplemented(String),
}

/// Audio decoder that provides decoded PCM frames and metadata.
pub struct AudioDecoder {
    format: Box<dyn symphonia::core::formats::FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track: TrackMetadata,
    track_id: u32,
    /// Current sample rate from the decoded track.
    sample_rate: SampleRate,
    /// Current channel count.
    channels: ChannelCount,
    /// Total duration in seconds, if known from metadata.
    duration: Option<f64>,
    /// First embedded picture captured at open time (single-probe access).
    album_art: Option<crate::types::AlbumArt>,
}

impl AudioDecoder {
    /// Open an audio file and create a decoder.
    pub fn open<P: AsRef<Path>>(path: P) -> CodecResult<Self> {
        let path = path.as_ref();
        let src = File::open(path)?;

        let mss = MediaSourceStream::new(Box::new(src), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        let format_opts = FormatOptions::default();
        let metadata_opts = MetadataOptions::default();
        let decoder_opts = DecoderOptions::default();

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &metadata_opts)
            .map_err(|e| CodecError::Decode(format!("Failed to probe format: {}", e)))?;

        let mut format = probed.format;
        let track = format
            .default_track()
            .ok_or_else(|| CodecError::Decode("No supported tracks found".into()))?;

        let dec = symphonia::default::get_codecs()
            .make(&track.codec_params, &decoder_opts)
            .map_err(|e| CodecError::Decode(format!("Failed to create decoder: {}", e)))?;

        let track_id = track.id;

        let cp = track.codec_params.clone();
        let sample_rate = cp.sample_rate.unwrap_or(44100);
        let channels = cp.channels.map(|c| c.count() as ChannelCount).unwrap_or(2);

        let duration = cp.n_frames.map(|n| n as f64 / sample_rate as f64);

        let bit_depth = cp.bits_per_coded_sample.map(|b| b as u16);
        let bitrate = duration.and_then(|dur| {
            if dur > 0.0 {
                let file_size = std::fs::metadata(path).ok()?.len();
                Some(((file_size as f64 * 8.0) / dur / 1000.0) as u32)
            } else {
                None
            }
        });

        let track_meta = TrackMetadata {
            sample_rate: Some(sample_rate),
            channels: Some(channels),
            channel_layout: Some(ChannelLayout::from_count(channels)),
            bit_depth,
            bitrate,
            format: Some(AudioFormat::from_extension(
                path.extension().and_then(|e| e.to_str()).unwrap_or(""),
            )),
            duration,
            ..Default::default()
        };

        let tags = Self::read_tags(format.as_mut());
        let track_meta = Self::enrich_metadata(track_meta, &tags);

        // Capture the first embedded picture while the format reader is at
        // the initial revision — callers that need cover art reuse this via
        // `album_art()` instead of re-probing the whole file.
        let album_art = {
            let metadata = format.metadata();
            let mut art = None;
            if let Some(current) = metadata.current() {
                for visual in current.visuals() {
                    if visual.data.is_empty() {
                        continue;
                    }
                    let mime_type = if visual.media_type.is_empty() {
                        "image/jpeg".to_string() // common default for APIC
                    } else {
                        visual.media_type.clone()
                    };
                    art = Some(crate::types::AlbumArt {
                        data: visual.data.to_vec(),
                        mime_type,
                    });
                    break;
                }
            }
            art
        };

        Ok(Self {
            format,
            decoder: dec,
            track: track_meta,
            track_id,
            sample_rate,
            channels,
            duration,
            album_art,
        })
    }

    /// Open a reader (e.g., network stream, in-memory buffer) and create a decoder.
    pub fn open_reader(reader: Box<dyn MediaSource>, hint_name: &str) -> CodecResult<Self> {
        let mss = MediaSourceStream::new(reader, Default::default());

        let mut hint = Hint::new();
        // Try to infer format from the hint name
        if let Some(ext) = std::path::Path::new(hint_name)
            .extension()
            .and_then(|e| e.to_str())
        {
            hint.with_extension(ext);
        }

        let lower = hint_name.to_lowercase();
        if lower.contains("mp3") {
            hint.with_extension("mp3");
        } else if lower.contains("flac") {
            hint.with_extension("flac");
        } else if lower.contains("wav") {
            hint.with_extension("wav");
        } else if lower.contains("aac") {
            hint.with_extension("aac");
        } else if lower.contains("ogg") || lower.contains("vorbis") {
            hint.with_extension("ogg");
        }

        let format_opts = FormatOptions::default();
        let metadata_opts = MetadataOptions::default();
        let decoder_opts = DecoderOptions::default();

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &metadata_opts)
            .map_err(|e| CodecError::Decode(format!("Failed to probe format: {}", e)))?;

        let format = probed.format;
        let track = format
            .default_track()
            .ok_or_else(|| CodecError::Decode("No supported tracks found".into()))?;

        let dec = symphonia::default::get_codecs()
            .make(&track.codec_params, &decoder_opts)
            .map_err(|e| CodecError::Decode(format!("Failed to create decoder: {}", e)))?;

        let track_id = track.id;

        let cp = track.codec_params.clone();
        let sample_rate = cp.sample_rate.unwrap_or(44100);
        let channels = cp.channels.map(|c| c.count() as ChannelCount).unwrap_or(2);

        let track_meta = TrackMetadata {
            sample_rate: Some(sample_rate),
            channels: Some(channels),
            channel_layout: Some(ChannelLayout::from_count(channels)),
            format: Some(AudioFormat::Unknown),
            duration: None,
            ..Default::default()
        };

        Ok(Self {
            format,
            decoder: dec,
            track: track_meta,
            track_id,
            sample_rate,
            channels,
            duration: None,
            album_art: None, // network streams: cover art not captured
        })
    }

    /// Read the next decoded PCM frame.
    pub fn next_frame(&mut self) -> CodecResult<Option<AudioFrame>> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                Err(symphonia::core::errors::Error::IoError(ref e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(None)
                }
                Err(symphonia::core::errors::Error::IoError(ref e))
                    if e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    return Ok(None)
                }
                Err(e) => return Err(CodecError::Decode(format!("Packet error: {}", e))),
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            let decoded = self
                .decoder
                .decode(&packet)
                .map_err(|e| CodecError::Decode(format!("Decode error: {}", e)))?;

            let spec = *decoded.spec();
            let _duration = decoded.capacity() as f64 / spec.rate as f64;

            let mut sample_buf = SampleBuffer::<Sample>::new(decoded.capacity() as u64, spec);
            sample_buf.copy_interleaved_ref(decoded);

            let frame = AudioFrame {
                samples: sample_buf.samples().to_vec(),
                sample_rate: spec.rate,
                channels: spec.channels.count() as ChannelCount,
                channel_layout: ChannelLayout::from_count(spec.channels.count() as ChannelCount),
                timestamp: 0.0,
            };

            return Ok(Some(frame));
        }
    }

    /// Get the track metadata.
    pub fn metadata(&self) -> &TrackMetadata {
        &self.track
    }

    /// Get the first embedded picture captured at open time. Zero extra
    /// file/probe cost — captured during `open()` from the initial metadata
    /// revision.
    pub fn album_art(&self) -> Option<crate::types::AlbumArt> {
        self.album_art.clone()
    }

    /// Get sample rate.
    pub fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    /// Get channel count.
    pub fn channels(&self) -> ChannelCount {
        self.channels
    }

    /// Get total duration in seconds, if known from metadata.
    pub fn duration(&self) -> Option<f64> {
        self.duration
    }

    /// Read all tags from the format's metadata.
    fn read_tags(format: &mut dyn symphonia::core::formats::FormatReader) -> Vec<(String, String)> {
        let mut tags = Vec::new();
        let metadata = format.metadata();
        if let Some(current) = metadata.current() {
            for tag in current.tags() {
                tags.push((tag.key.to_string(), tag.value.to_string()));
            }
        }
        tags
    }

    /// Enrich TrackMetadata with parsed tags and ReplayGain.
    fn enrich_metadata(mut meta: TrackMetadata, tags: &[(String, String)]) -> TrackMetadata {
        for (key, value) in tags {
            match key.to_uppercase().as_str() {
                "TITLE" => meta.title = Some(value.clone()),
                "ARTIST" => meta.artist = Some(value.clone()),
                "ALBUM" => meta.album = Some(value.clone()),
                "ALBUMARTIST" | "ALBUM_ARTIST" => meta.album_artist = Some(value.clone()),
                "GENRE" => meta.genre = Some(value.clone()),
                "DATE" | "YEAR" => {
                    meta.year = value.parse::<u32>().ok();
                }
                "TRACKNUMBER" | "TRACK" => meta.track_number = value.parse::<u32>().ok(),
                "TRACKTOTAL" | "TOTALTRACKS" => meta.total_tracks = value.parse::<u32>().ok(),
                "DISCNUMBER" | "DISC" => meta.disc_number = value.parse::<u32>().ok(),
                "DISCTOTAL" | "TOTALDISCS" => meta.total_discs = value.parse::<u32>().ok(),
                "COMPOSER" => meta.composer = Some(value.clone()),
                "COMMENT" => meta.comment = Some(value.clone()),
                _ => {}
            }
        }

        let rg = crate::metadata::extract_replay_gain(tags);
        if rg.track_gain.is_some() || rg.track_peak.is_some() {
            meta.replay_gain = Some(rg);
        }

        meta
    }

    /// Seek to a specific timestamp in seconds using Symphonia's native seek.
    /// After seeking, the next call to `next_frame()` will return frames from the new position.
    pub fn seek(&mut self, time_secs: DurationSecs) -> CodecResult<()> {
        let seek_to = SeekTo::Time {
            time: Time::from(time_secs),
            track_id: Some(self.track_id),
        };

        let _seeked = self
            .format
            .seek(SeekMode::Accurate, seek_to)
            .map_err(|e| CodecError::Decode(format!("Seek error: {}", e)))?;

        // Recreate decoder after seeking (codec state may have changed)
        let track = self
            .format
            .default_track()
            .ok_or_else(|| CodecError::Decode("No supported tracks after seek".into()))?;

        let decoder_opts = DecoderOptions::default();
        self.decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &decoder_opts)
            .map_err(|e| {
                CodecError::Decode(format!("Failed to recreate decoder after seek: {}", e))
            })?;

        log::debug!("Seeked to {:.1}s", time_secs);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_format_from_extension() {
        assert_eq!(AudioFormat::from_extension("flac"), AudioFormat::Flac);
        assert_eq!(AudioFormat::from_extension("mp3"), AudioFormat::Mp3);
        assert_eq!(AudioFormat::from_extension("wav"), AudioFormat::Wav);
        assert_eq!(AudioFormat::from_extension("dsf"), AudioFormat::Dsf);
        assert_eq!(AudioFormat::from_extension("xyz"), AudioFormat::Unknown);
    }

    #[test]
    fn test_channel_layout() {
        assert_eq!(ChannelLayout::Mono.channel_count(), 1);
        assert_eq!(ChannelLayout::Stereo.channel_count(), 2);
        assert_eq!(ChannelLayout::Layout5_1.channel_count(), 6);
        assert_eq!(ChannelLayout::Layout7_1.channel_count(), 8);
    }
}
