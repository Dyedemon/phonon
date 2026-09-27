//! Error types for the media library.

use std::io;

/// Errors returned by media library operations.
#[derive(Debug, thiserror::Error)]
pub enum MediaLibraryError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Codec error: {0}")]
    Codec(String),
    #[error("Image decode/encode error: {0}")]
    Image(#[from] image::ImageError),
    #[error("Lock poisoned: {0}")]
    Lock(String),
    #[error("Invalid path: {0}")]
    InvalidPath(String),
    #[error("Schema migration failed: {0}")]
    Migration(String),
    /// The database failed its integrity check. Carries the recovery
    /// detail (e.g. why the pre-delete backup copy could not be made).
    #[error("Corrupt database: {0}")]
    CorruptDatabase(String),
    #[error("Track not found: id={0}")]
    TrackNotFound(i64),
    #[error("Invalid rating {0} (must be 0..=5)")]
    InvalidRating(u8),
    #[error("Operation cancelled")]
    Cancelled,
    /// Compliance violation: the user tried to bind a cover file that is
    /// not a local JPEG/PNG, or edit metadata via a disallowed source.
    /// The message is user-facing and localized by the frontend.
    #[error("Compliance violation: {0}")]
    Compliance(String),
}

impl From<phonon_codec::decoder::CodecError> for MediaLibraryError {
    fn from(err: phonon_codec::decoder::CodecError) -> Self {
        MediaLibraryError::Codec(err.to_string())
    }
}

impl From<phonon_codec::cue::CueError> for MediaLibraryError {
    fn from(err: phonon_codec::cue::CueError) -> Self {
        MediaLibraryError::Codec(err.to_string())
    }
}

/// Convenience result alias.
pub type Result<T> = std::result::Result<T, MediaLibraryError>;
