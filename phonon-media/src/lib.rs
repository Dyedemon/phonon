//! Phonon Media - Media library management with SQLite FTS5.
//!
//! Provides incremental scanning, CUE virtual track indexing, cover thumbnail
//! caching, FTS5 full-text search, and favorite/rating/play-count tracking.
//!
//! # Legal compliance
//! All metadata is sourced exclusively from:
//! 1. Local audio file tags (parsed via `phonon-codec`)
//! 2. User manual input via `edit_metadata`
//! 3. Local JPEG/PNG files supplied by the user as cover art
//!
//! No third-party metadata/cover APIs are invoked. No bulk download or
//! collection of metadata/lyrics/cover from any external platform.

pub mod cue_indexer;
pub mod error;
pub mod library;
pub mod models;
pub mod replaygain;
pub mod scanner;
pub mod schema;
pub mod search;
pub mod thumbnails;

pub use error::{MediaLibraryError, Result};
pub use library::{
    compute_sort_key, spawn_sort_key_backfill, MediaLibrary, ReplayGainScanProgress,
    ReplayGainScanStats, ScanOptions, ScanProgress, ScanStats,
};
pub use models::{
    AlbumInfo, ArtistInfo, FolderInfo, GenreInfo, LibraryAlbum, LibraryArtist, LibrarySuggest,
    LibraryTrack, PlayHistoryEntry, SearchResult, SortOrder, ThumbnailInfo, TrackFilter, TrackInfo,
    TrackSortField,
};
pub use replaygain::{
    calculate_replaygain, open_sidecar, read_sidecar, store_sidecar, ReplayGainResult,
};
pub use scanner::AUDIO_EXTENSIONS;
pub use search::SearchMode;
