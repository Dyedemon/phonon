//! Source provider traits and types.

use phonon_codec::*;
use serde::{Deserialize, Serialize};

/// Result of a search operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// File path or URI.
    pub uri: String,
    /// Display name.
    pub name: String,
    /// Audio metadata.
    pub metadata: TrackMetadata,
    /// Whether this is part of a CUE sheet.
    pub cue_track_index: Option<u32>,
}

/// The core audio source trait.
/// All sources (local files, network streams, plugins) implement this.
pub trait IAudioSource: Send + Sync {
    /// Search for audio files in the given directory/URI.
    fn search(&self, query: &str) -> Result<Vec<SearchResult>, SourceError>;

    /// Get metadata for a specific URI.
    fn get_metadata(&self, uri: &str) -> Result<TrackMetadata, SourceError>;

    /// Open a decoder for the given URI.
    fn open_decoder(&self, uri: &str) -> Result<Box<dyn AudioSourceReader>, SourceError>;

    /// Get the source name.
    fn name(&self) -> &str;

    /// Get the source type.
    fn source_type(&self) -> SourceType;
}

/// Reader that provides decoded PCM frames from a source.
pub trait AudioSourceReader: Send {
    /// Read the next decoded PCM frame.
    fn next_frame(&mut self) -> Result<Option<AudioFrame>, SourceError>;

    /// Seek to a specific position in seconds.
    fn seek(&mut self, time_secs: f64) -> Result<(), SourceError>;

    /// Get the total duration in seconds.
    fn duration(&self) -> Option<f64>;

    /// Get the sample rate.
    fn sample_rate(&self) -> u32;

    /// Get the channel count.
    fn channels(&self) -> u16;
}

/// Type of audio source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceType {
    LocalFile,
    NetworkStream,
    Plugin,
}

/// Errors for source operations.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Decode error: {0}")]
    Decode(String),
    #[error("Network error: {0}")]
    Network(String),
    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),
}
