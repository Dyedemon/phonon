//! Core types for the Phonon audio framework.

use serde::{Deserialize, Serialize};

/// Audio sample format - always 32-bit float PCM internally.
pub type Sample = f32;

/// Number of audio channels.
pub type ChannelCount = u16;

/// Sample rate in Hz.
pub type SampleRate = u32;

/// Duration in seconds.
pub type DurationSecs = f64;

/// Audio format identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioFormat {
    Flac,
    Alac,
    Wav,
    Mp3,
    Aac,
    Dsf,
    Dff,
    Unknown,
}

impl AudioFormat {
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "flac" => AudioFormat::Flac,
            "alac" | "m4a" => AudioFormat::Alac,
            "wav" | "wave" => AudioFormat::Wav,
            "mp3" => AudioFormat::Mp3,
            "aac" => AudioFormat::Aac,
            "dsf" => AudioFormat::Dsf,
            "dff" => AudioFormat::Dff,
            _ => AudioFormat::Unknown,
        }
    }

    pub fn from_mime(mime: &str) -> Self {
        match mime {
            "audio/flac" | "audio/x-flac" => AudioFormat::Flac,
            "audio/mp4" | "audio/x-m4a" => AudioFormat::Alac,
            "audio/wav" | "audio/x-wav" | "audio/wave" => AudioFormat::Wav,
            "audio/mpeg" | "audio/mp3" => AudioFormat::Mp3,
            "audio/aac" => AudioFormat::Aac,
            "audio/x-dsf" => AudioFormat::Dsf,
            "audio/x-dff" => AudioFormat::Dff,
            _ => AudioFormat::Unknown,
        }
    }
}

/// Channel layout mapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelLayout {
    Mono,
    Stereo,
    Layout2_1,
    Layout3_0,
    Layout3_1,
    Layout4_0,
    Layout4_1,
    Layout5_0,
    Layout5_1,
    Layout6_1,
    Layout7_1,
    Custom(Vec<String>),
}

impl ChannelLayout {
    pub fn from_count(count: u16) -> Self {
        match count {
            1 => ChannelLayout::Mono,
            2 => ChannelLayout::Stereo,
            3 => ChannelLayout::Layout2_1,
            4 => ChannelLayout::Layout3_1,
            5 => ChannelLayout::Layout4_1,
            6 => ChannelLayout::Layout5_1,
            7 => ChannelLayout::Layout6_1,
            8 => ChannelLayout::Layout7_1,
            _ => ChannelLayout::Custom(vec!["Unknown".into(); count as usize]),
        }
    }

    pub fn channel_count(&self) -> u16 {
        match self {
            ChannelLayout::Mono => 1,
            ChannelLayout::Stereo => 2,
            ChannelLayout::Layout2_1 => 3,
            ChannelLayout::Layout3_0 => 3,
            ChannelLayout::Layout3_1 => 4,
            ChannelLayout::Layout4_0 => 4,
            ChannelLayout::Layout4_1 => 5,
            ChannelLayout::Layout5_0 => 5,
            ChannelLayout::Layout5_1 => 6,
            ChannelLayout::Layout6_1 => 7,
            ChannelLayout::Layout7_1 => 8,
            ChannelLayout::Custom(ref names) => names.len() as u16,
        }
    }

    pub fn channel_names(&self) -> Vec<String> {
        match self {
            ChannelLayout::Mono => vec!["FrontCenter".into()],
            ChannelLayout::Stereo => vec!["FrontLeft".into(), "FrontRight".into()],
            ChannelLayout::Layout5_1 => vec![
                "FrontLeft".into(),
                "FrontRight".into(),
                "FrontCenter".into(),
                "LFE".into(),
                "BackLeft".into(),
                "BackRight".into(),
            ],
            ChannelLayout::Layout7_1 => vec![
                "FrontLeft".into(),
                "FrontRight".into(),
                "FrontCenter".into(),
                "LFE".into(),
                "BackLeft".into(),
                "BackRight".into(),
                "SideLeft".into(),
                "SideRight".into(),
            ],
            ChannelLayout::Custom(ref names) => names.clone(),
            _ => (0..self.channel_count())
                .map(|i| format!("Channel{}", i + 1))
                .collect(),
        }
    }
}

/// ReplayGain metadata.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReplayGain {
    pub track_gain: Option<f32>,
    pub track_peak: Option<f32>,
    pub album_gain: Option<f32>,
    pub album_peak: Option<f32>,
}

/// Album art data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumArt {
    pub data: Vec<u8>,
    pub mime_type: String,
}

/// A single word with its start timestamp for karaoke-style highlighting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LyricWord {
    /// Start time of this word in milliseconds.
    pub time_ms: u64,
    /// The word text.
    pub text: String,
}

/// A single line of synced (LRC) lyrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LyricLine {
    /// Timestamp in milliseconds.
    pub time_ms: u64,
    /// The lyric text.
    pub text: String,
    /// Optional word-level timestamps for karaoke highlighting.
    /// When empty, the frontend can auto-distribute timing evenly.
    pub words: Vec<LyricWord>,
}

/// Lyrics data extracted from audio metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LyricData {
    /// No lyrics found.
    None,
    /// Unsynced lyrics (plain text, may contain newlines).
    Unsynced(String),
    /// Synced lyrics (LRC format with timestamps).
    Synced(Vec<LyricLine>),
}

impl LyricData {
    pub fn is_none(&self) -> bool {
        matches!(self, LyricData::None)
    }
}

/// Comprehensive audio track metadata.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TrackMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub track_number: Option<u32>,
    pub total_tracks: Option<u32>,
    pub disc_number: Option<u32>,
    pub total_discs: Option<u32>,
    pub composer: Option<String>,
    pub comment: Option<String>,
    pub duration: Option<DurationSecs>,
    pub sample_rate: Option<SampleRate>,
    pub bit_depth: Option<u16>,
    pub channels: Option<ChannelCount>,
    pub channel_layout: Option<ChannelLayout>,
    pub bitrate: Option<u32>,
    pub format: Option<AudioFormat>,
    pub album_art: Option<AlbumArt>,
    pub replay_gain: Option<ReplayGain>,
    pub lyrics: Option<LyricData>,
}

/// Decoded audio frame.
#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub samples: Vec<Sample>,
    pub sample_rate: SampleRate,
    pub channels: ChannelCount,
    pub channel_layout: ChannelLayout,
    /// Timestamp in seconds within the track.
    pub timestamp: DurationSecs,
}

/// CUE sheet track entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CueTrack {
    pub index: u32,
    pub title: Option<String>,
    pub performer: Option<String>,
    /// Start time in seconds.
    pub start_time: DurationSecs,
    /// Duration in seconds (calculated from next track or file end).
    pub duration: Option<DurationSecs>,
}

/// CUE sheet metadata.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CueSheet {
    pub title: Option<String>,
    pub performer: Option<String>,
    pub date: Option<String>,
    pub genre: Option<String>,
    pub file: Option<String>,
    pub tracks: Vec<CueTrack>,
}

/// PCM buffer for audio processing.
#[derive(Debug, Clone)]
pub struct PcmBuffer {
    pub samples: Vec<Sample>,
    pub sample_rate: SampleRate,
    pub channels: ChannelCount,
    pub channel_layout: ChannelLayout,
    pub frame_count: usize,
}
