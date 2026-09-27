//! Data models exposed to the frontend and used internally.

use serde::{Deserialize, Serialize};

/// A single track row, surfaced to the UI.
///
/// `cue_sheet_id` is `Some` when this row is a virtual track derived from a
/// CUE sheet; the underlying audio file path is stored in `file_path` and
/// `cue_track_index` is the 1-based CUE TRACK number.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackInfo {
    pub id: i64,
    pub file_path: String,
    pub file_size: u64,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub composer: Option<String>,
    pub year: Option<i32>,
    pub track_number: Option<i32>,
    pub disc_number: Option<i32>,
    pub duration_ms: Option<i64>,
    pub sample_rate: Option<i32>,
    pub channels: Option<i32>,
    pub bit_depth: Option<i32>,
    pub bitrate: Option<i32>,
    pub format: Option<String>,
    /// SHA1 hash of the embedded cover bytes, if any.
    pub cover_hash: Option<String>,
    /// ReplayGain tags (track peak in dB, may be empty).
    pub replaygain_track_gain: Option<f64>,
    pub replaygain_album_gain: Option<f64>,
    /// CUE linkage: Some when this is a virtual track.
    pub cue_sheet_id: Option<i64>,
    pub cue_track_index: Option<i32>,
    /// Soft-delete flag. UI typically hides `is_deleted=true` rows.
    pub is_deleted: bool,
    pub is_favorite: bool,
    /// 0..=5 (0 = unrated).
    pub rating: u8,
    pub play_count: i64,
    pub last_played_at: Option<i64>,
    pub added_at: i64,
    pub updated_at: i64,
}

/// Album aggregate row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumInfo {
    pub id: i64,
    pub title: String,
    pub album_artist: Option<String>,
    pub year: Option<i32>,
    pub cover_hash: Option<String>,
    pub track_count: i64,
    pub total_duration_ms: Option<i64>,
}

/// Artist aggregate row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtistInfo {
    pub id: i64,
    pub name: String,
    pub track_count: i64,
    pub album_count: i64,
}

/// Genre aggregate row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenreInfo {
    pub id: i64,
    pub name: String,
    pub track_count: i64,
}

/// Folder row (folder tree node).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderInfo {
    pub id: i64,
    pub path: String,
    pub parent_id: Option<i64>,
    pub track_count: i64,
}

/// Cover thumbnail cache entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThumbnailInfo {
    pub hash: String,
    /// 256 or 512.
    pub size: u32,
    /// WEBP image bytes (typically returned to UI as a data URL or blob).
    pub webp: Vec<u8>,
}

/// Search hit returned by `library_search`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub track: TrackInfo,
    /// BM25 score from FTS5 (lower = more relevant; SQLite returns negative
    /// scores so we negate them — higher is better).
    pub score: f64,
}

/// Sort field for track listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackSortField {
    Title,
    Artist,
    Album,
    Year,
    TrackNumber,
    Duration,
    PlayCount,
    LastPlayed,
    #[default]
    AddedAt,
    Rating,
}

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    Asc,
    #[default]
    Desc,
}

/// Filter/pagination parameters for `library_get_tracks`.
///
/// All fields are optional; omitted fields do not constrain the query.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrackFilter {
    /// Limit to tracks in this album (by id).
    pub album_id: Option<i64>,
    /// Limit to tracks by this artist (by id).
    pub artist_id: Option<i64>,
    /// Limit to tracks in this genre (by id).
    pub genre_id: Option<i64>,
    /// Limit to tracks in this folder (by id, recursive).
    pub folder_id: Option<i64>,
    /// Limit to favorites only.
    pub favorites_only: bool,
    /// Only include tracks that have been played at least once (play_count > 0).
    pub played_only: bool,
    /// Minimum rating (1..=5). None = no rating filter.
    pub min_rating: Option<u8>,
    /// Include soft-deleted rows.
    pub include_deleted: bool,
    /// Sort field.
    pub sort: Option<TrackSortField>,
    /// Sort direction.
    pub order: Option<SortOrder>,
    /// Maximum number of rows to return (None = unlimited).
    pub limit: Option<i64>,
    /// Offset for pagination.
    pub offset: Option<i64>,
}

/// A single entry in the play history list.
///
/// Returned by `library_list_play_history`. Joins `play_history` with
/// `tracks` so the UI gets everything it needs in one call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayHistoryEntry {
    /// The history row id (for stable React keys).
    pub history_id: i64,
    /// Unix timestamp (seconds) when the track was played.
    pub played_at: i64,
    /// The full track row at query time. Note: the track may have been
    /// edited or soft-deleted after this play event; the UI should handle
    /// `is_deleted = true` gracefully.
    pub track: TrackInfo,
}

// ─────────────────────────────────────────────────────────────────
// Plan A Task A4: Library-view models (TS-matching subset).
// ─────────────────────────────────────────────────────────────────

/// Library track row (subset of `TrackInfo` matching the frontend's
/// `LibraryTrack` TypeScript interface). Used by `library_get_tracks_by_paths`
/// and `library_search_suggest` so the wire payload stays small.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryTrack {
    pub id: i64,
    pub file_path: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub year: Option<i32>,
    pub track_number: Option<i32>,
    pub duration_ms: Option<i64>,
    pub format: Option<String>,
    pub cover_hash: Option<String>,
    pub is_favorite: bool,
    pub rating: u8,
    pub play_count: i32,
    pub last_played_at: Option<i64>,
    pub added_at: i64,
}

/// Library album row (subset matching the frontend `LibraryAlbum` interface).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryAlbum {
    pub id: i64,
    pub title: String,
    pub album_artist: Option<String>,
    pub year: Option<i32>,
    pub cover_hash: Option<String>,
    pub track_count: i32,
    pub sort_key: Option<String>,
}

/// Library artist row (subset matching the frontend `LibraryArtist` interface).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryArtist {
    pub id: i64,
    pub name: String,
    pub track_count: i32,
    pub album_count: i32,
    pub sort_key: Option<String>,
}

/// Suggest payload returned by `library_search_suggest` for the omnibox
/// dropdown: top tracks/albums/artists matching the query.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibrarySuggest {
    pub tracks: Vec<LibraryTrack>,
    pub albums: Vec<LibraryAlbum>,
    pub artists: Vec<LibraryArtist>,
}
