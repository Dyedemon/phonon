/**
 * Media Library API — frontend wrapper for the `library_*` Tauri commands.
 *
 * All argument names are passed in snake_case to match the Rust command
 * signatures exactly (per spec §12 "严格 snake_case 传参").
 *
 * Compliance: this module only ever talks to the local SQLite library via
 * Tauri invoke — no third-party metadata/cover APIs are called.
 */

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

// ---------------------------------------------------------------------------
// Types — mirror `phonon-media/src/models.rs` and `library.rs`.
// ---------------------------------------------------------------------------

/** A single track row (mirrors `phonon_media::TrackInfo`). */
export interface TrackInfo {
  id: number;
  file_path: string;
  file_size: number;
  title: string | null;
  artist: string | null;
  album: string | null;
  album_artist: string | null;
  genre: string | null;
  composer: string | null;
  year: number | null;
  track_number: number | null;
  disc_number: number | null;
  duration_ms: number | null;
  sample_rate: number | null;
  channels: number | null;
  bit_depth: number | null;
  bitrate: number | null;
  format: string | null;
  /** SHA1 hash of the embedded cover bytes, if any. */
  cover_hash: string | null;
  replaygain_track_gain: number | null;
  replaygain_album_gain: number | null;
  /** Some when this is a CUE virtual track. */
  cue_sheet_id: number | null;
  cue_track_index: number | null;
  is_deleted: boolean;
  is_favorite: boolean;
  /** 0..=5 (0 = unrated). */
  rating: number;
  play_count: number;
  last_played_at: number | null;
  added_at: number;
  updated_at: number;
}

/** A play history entry (mirrors `phonon_media::PlayHistoryEntry`). */
export interface PlayHistoryEntry {
  /** History row id (stable React key). */
  history_id: number;
  /** Unix timestamp (seconds) when the track was played. */
  played_at: number;
  /** Full track row at query time. */
  track: TrackInfo;
}

export interface AlbumInfo {
  id: number;
  title: string;
  album_artist: string | null;
  year: number | null;
  cover_hash: string | null;
  track_count: number;
  total_duration_ms: number | null;
}

export interface ArtistInfo {
  id: number;
  name: string;
  track_count: number;
  album_count: number;
}

export interface GenreInfo {
  id: number;
  name: string;
  track_count: number;
}

export interface FolderInfo {
  id: number;
  path: string;
  parent_id: number | null;
  track_count: number;
}

/** Cover thumbnail cache entry. `webp` is the raw image bytes. */
export interface ThumbnailInfo {
  hash: string;
  /** 256 or 512. */
  size: number;
  /** WEBP image bytes (transferred as a number array by Tauri). */
  webp: number[];
}

/** Search hit returned by `library_search`. */
export interface SearchResult {
  track: TrackInfo;
  /** BM25 score (higher = more relevant). */
  score: number;
}

export type TrackSortField =
  | 'title'
  | 'artist'
  | 'album'
  | 'year'
  | 'track_number'
  | 'duration'
  | 'play_count'
  | 'last_played'
  | 'added_at'
  | 'rating';

export type SortOrder = 'asc' | 'desc';

/** Filter/pagination parameters for `library_get_tracks`. */
export interface TrackFilter {
  album_id?: number | null;
  artist_id?: number | null;
  genre_id?: number | null;
  folder_id?: number | null;
  favorites_only?: boolean;
  min_rating?: number | null;
  include_deleted?: boolean;
  sort?: TrackSortField | null;
  order?: SortOrder | null;
  limit?: number | null;
  offset?: number | null;
}

/** Aggregate result of a scan run (mirrors `phonon_media::ScanStats`). */
export interface ScanStats {
  total_files: number;
  tracks_inserted: number;
  tracks_updated: number;
  tracks_soft_deleted: number;
  cue_sheets_indexed: number;
  cue_sheets_soft_deleted: number;
}

/** Aggregate result of a ReplayGain batch scan (mirrors `ReplayGainScanStats`). */
export interface ReplayGainScanStats {
  total_tracks: number;
  scanned: number;
  skipped: number;
  failed: number;
}

/** Per-track ReplayGain scan progress (mirrors `ReplayGainScanProgress`). */
export interface ReplayGainScanProgress {
  total: number;
  processed: number;
  current_path: string;
  phase: string;
}

/** Per-file progress notification delivered during a scan. */
export interface ScanProgress {
  total: number;
  processed: number;
  current_path: string;
  phase: string;
}

/** Editable metadata fields for `library_edit_metadata`. Omitted = unchanged. */
export interface MetadataEdit {
  title?: string | null;
  artist?: string | null;
  album?: string | null;
  album_artist?: string | null;
  genre?: string | null;
  composer?: string | null;
  year?: number | null;
  track_number?: number | null;
  disc_number?: number | null;
}

// ---------------------------------------------------------------------------
// Plan-A types — mirror the new `phonon_media` Library* structs added in A4
// (see `phonon-media/src/models.rs`). These are slimmer than the legacy
// `TrackInfo` / `AlbumInfo` / `ArtistInfo` rows: they only carry the fields
// required by the new 9-col UI, AlbumGrid and omnibox suggest.
// ---------------------------------------------------------------------------

/**
 * Slim track row returned by the Plan-A commands
 * `library_get_tracks_by_paths` / `library_search_suggest` /
 * `library_edit_metadata_partial`. Mirrors `phonon_media::LibraryTrack`.
 */
export interface LibraryTrack {
  id: number;
  file_path: string;
  title: string | null;
  artist: string | null;
  album: string | null;
  genre: string | null;
  year: number | null;
  track_number: number | null;
  duration_ms: number | null;
  format: string | null;
  /** SHA1 hash of the embedded cover bytes, if any. */
  cover_hash: string | null;
  is_favorite: boolean;
  /** 0..=5 (0 = unrated). */
  rating: number;
  play_count: number;
  last_played_at: number | null;
  added_at: number;
}

/**
 * Slim album row returned by `library_search_suggest`. Mirrors
 * `phonon_media::LibraryAlbum`. `sort_key` is the pinyin-derived first-letter
 * key used for the A-Z sticky grouping in `ArtistGenreList`.
 */
export interface LibraryAlbum {
  id: number;
  title: string;
  album_artist: string | null;
  year: number | null;
  cover_hash: string | null;
  track_count: number;
  sort_key: string | null;
}

/**
 * Slim artist row returned by `library_search_suggest`. Mirrors
 * `phonon_media::LibraryArtist`.
 */
export interface LibraryArtist {
  id: number;
  name: string;
  track_count: number;
  album_count: number;
  sort_key: string | null;
}

/**
 * Omnisearch suggest payload returned by `library_search_suggest`.
 * Mirrors `phonon_media::LibrarySuggest` — top 5 tracks + 3 albums + 3 artists.
 */
export interface LibrarySuggest {
  tracks: LibraryTrack[];
  albums: LibraryAlbum[];
  artists: LibraryArtist[];
}

/**
 * Partial metadata patch for `library_edit_metadata_partial`. Mirrors
 * `phonon_codec::PartialMeta` — every field is `Option<...>`, omitted fields
 * are left untouched. The library row is updated before the file-write
 * attempt, so a file-write failure NEVER rolls back the library change.
 */
export interface PartialMeta {
  title?: string | null;
  artist?: string | null;
  album?: string | null;
  album_artist?: string | null;
  genre?: string | null;
  composer?: string | null;
  year?: number | null;
  track_number?: number | null;
  disc_number?: number | null;
  /** 0..=5 (0 = unrated). */
  rating?: number | null;
  /** Raw cover bytes (JPEG/PNG). When set, also set `mime`. */
  cover_bytes?: number[] | null;
}

// ---------------------------------------------------------------------------
// Commands — one function per `library_*` Tauri command.
// ---------------------------------------------------------------------------

/** Configure the library roots. Replaces any previously configured roots. */
export function setRoots(roots: string[]): Promise<void> {
  return invoke('library_set_roots', { roots });
}

/** Get the currently configured library roots. */
export function getRoots(): Promise<string[]> {
  return invoke('library_get_roots');
}

/**
 * Trigger an incremental scan over the configured roots.
 *
 * Emits `library-scan-progress` events as the scan proceeds. The returned
 * promise resolves when the scan completes.
 */
export function scan(): Promise<ScanStats> {
  return invoke('library_scan');
}

/** Full-text search the library (FTS5 BM25). Returns TOP `limit` (default 200). */
export function search(query: string, limit?: number): Promise<SearchResult[]> {
  return invoke('library_search', { query, limit: limit ?? null });
}

/** List tracks with optional filter/pagination. */
export function getTracks(filter?: TrackFilter): Promise<TrackInfo[]> {
  return invoke('library_get_tracks', { filter: filter ?? null });
}

/** Get a single track by id. */
export function getTrack(id: number): Promise<TrackInfo | null> {
  return invoke('library_get_track', { id });
}

/**
 * Resolve a file path to a track row.
 *
 * Returns `null` when the path is not in the library. Used by the
 * playlist/player to attach favorite/rating UI to queue items.
 */
export function getTrackByPath(path: string): Promise<TrackInfo | null> {
  return invoke('library_get_track_by_path', { path });
}

/**
 * List recent play history entries (newest first).
 *
 * `limit` caps the number of rows (default 200, max 5000). Each entry
 * joins the full track row for one-round-trip rendering.
 */
export function listPlayHistory(limit?: number): Promise<PlayHistoryEntry[]> {
  return invoke('library_list_play_history', { limit: limit ?? null });
}

/** Toggle favorite flag. Returns the new state. */
export function toggleFavorite(id: number): Promise<boolean> {
  return invoke('library_toggle_favorite', { id });
}

/** Set rating (0..=5). */
export function setRating(id: number, rating: number): Promise<void> {
  return invoke('library_set_rating', { id, rating });
}

/** Record a play event (bumps play_count + appends to play_history). */
export function recordPlay(id: number): Promise<void> {
  return invoke('library_record_play', { id });
}

/**
 * Manually edit a track's metadata. Only the supplied fields are updated.
 *
 * Compliance: only manual text input is accepted — no third-party metadata API.
 */
export function editMetadata(id: number, edit: MetadataEdit): Promise<TrackInfo> {
  return invoke('library_edit_metadata', { id, ...edit });
}

/**
 * Bind a local cover file (JPEG/PNG) to a track.
 *
 * Compliance: only local file paths supplied by the user are accepted.
 */
export function setTrackCover(id: number, cover_path: string): Promise<TrackInfo> {
  return invoke('library_set_track_cover', { id, cover_path });
}

/** Physically remove soft-deleted rows. Returns the number of rows deleted. */
export function cleanupDeleted(): Promise<number> {
  return invoke('library_cleanup_deleted');
}

/** List all albums. */
export function listAlbums(): Promise<AlbumInfo[]> {
  return invoke('library_list_albums');
}

/** List all artists. */
export function listArtists(): Promise<ArtistInfo[]> {
  return invoke('library_list_artists');
}

/** List all genres. */
export function listGenres(): Promise<GenreInfo[]> {
  return invoke('library_list_genres');
}

/**
 * Retrieve a cover thumbnail by hash and size (256 or 512).
 * Returns the WEBP bytes wrapped in `ThumbnailInfo`.
 */
export function getThumbnail(hash: string, size: number): Promise<ThumbnailInfo | null> {
  return invoke('library_get_thumbnail', { hash, size });
}

// ---------------------------------------------------------------------------
// Helpers — convenience wrappers on top of the raw commands.
// ---------------------------------------------------------------------------

/**
 * Subscribe to `library-scan-progress` events emitted during `scan()`.
 *
 * Returns an unlisten function — callers MUST call it in `useEffect` cleanup
 * to avoid duplicate listeners under React StrictMode (per spec §12).
 */
export function onScanProgress(
  cb: (p: ScanProgress) => void,
): Promise<UnlistenFn> {
  return listen<ScanProgress>('library-scan-progress', (e) => cb(e.payload));
}

/** In-memory cache of `hash:size` → object URL, to avoid repeated invokes. */
const thumbnailUrlCache = new Map<string, string>();

/**
 * Fetch a cover thumbnail and return a blob URL suitable for `<img src>`.
 *
 * Uses 256px by default; pass `512` for the larger size. Results are cached
 * per `hash:size` for the lifetime of the page. Returns `null` when the track
 * has no cover (missing hash or thumbnail not yet generated).
 *
 * The caller is responsible for revoking the URL via `URL.revokeObjectURL`
 * if it needs to be freed early; otherwise URLs are released on page unload.
 */
export async function getThumbnailUrl(
  hash: string | null | undefined,
  size: 256 | 512 = 256,
): Promise<string | null> {
  if (!hash) return null;
  const key = `${hash}:${size}`;
  const cached = thumbnailUrlCache.get(key);
  if (cached) return cached;

  const info = await getThumbnail(hash, size);
  if (!info || !info.webp.length) return null;

  const bytes = new Uint8Array(info.webp);
  const blob = new Blob([bytes], { type: 'image/webp' });
  const url = URL.createObjectURL(blob);
  thumbnailUrlCache.set(key, url);
  return url;
}

/** Clear the in-memory thumbnail URL cache (e.g. on library rescan). */
export function clearThumbnailUrlCache(): void {
  for (const url of thumbnailUrlCache.values()) {
    URL.revokeObjectURL(url);
  }
  thumbnailUrlCache.clear();
}

// ---------------------------------------------------------------------------
// Plan-A commands — 6 new wrappers matching the A5/A6 backend registrations.
//
// Naming convention (per `commands.rs` L4849-4853 RENAME NOTE): the new
// `edit_metadata` / `set_track_cover` commands collide at the Tauri
// command-name level with the existing per-field variants, so the new ones
// are registered under the suffixed names `_partial` / `_bytes`. Both
// variants coexist; the frontend picks the suffixed names for the new
// bulk-patch / raw-bytes flow.
// ---------------------------------------------------------------------------

/**
 * Resolve a batch of file paths to library track rows. Used by the import
 * fast path (§5.8.1) to attach full library metadata to queue items whose
 * folder already lives under a configured root.
 *
 * @param paths  max 500 per call (use multiple calls for larger sets).
 * @returns rows in input order; missing paths silently dropped.
 */
export function getTracksByPaths(paths: string[]): Promise<LibraryTrack[]> {
  return invoke('library_get_tracks_by_paths', { paths });
}

/**
 * Omnisearch suggest payload — top 5 tracks + 3 albums + 3 artists matching
 * `query`. Empty/whitespace query returns the default (all-empty) payload.
 */
export function searchSuggest(query: string): Promise<LibrarySuggest> {
  return invoke('library_search_suggest', { query });
}

/**
 * Apply a partial metadata patch (`PartialMeta`) to a track. The library
 * row is always updated BEFORE the file-write attempt, so a file-write
 * failure NEVER rolls back the library change — the returned track row
 * reflects the new library state. The frontend uses this to drive the
 * optimistic UI update + the "稍后重试写回" toast (via
 * `writeMetadataBackToFile`).
 */
export function editMetadataPartial(
  track_id: number,
  partial: PartialMeta,
): Promise<LibraryTrack> {
  return invoke('library_edit_metadata_partial', { track_id, partial });
}

/**
 * Bind raw cover bytes (JPEG/PNG) as the cover for a track. Returns the
 * SHA1 hash of the encoded WEBP thumbnail (used by `getThumbnailUrl`).
 */
export function setTrackCoverBytes(
  track_id: number,
  cover_bytes: number[] | Uint8Array,
  mime: string,
): Promise<string> {
  // Tauri serializes `Vec<u8>` from a JSON array of numbers; accept both
  // number[] and Uint8Array for caller ergonomics.
  const bytes = cover_bytes instanceof Uint8Array
    ? Array.from(cover_bytes)
    : cover_bytes;
  return invoke('library_set_track_cover_bytes', {
    track_id,
    cover_bytes: bytes,
    mime,
  });
}

/**
 * Try to recover tracks from a corrupt-library backup file. RC1 stub:
 * returns 0 (full recovery planned for RC2). The frontend shows a
 * progress banner while this runs.
 */
export function tryRecoverCorrupt(backup_path: string): Promise<number> {
  return invoke('library_try_recover_corrupt', { backup_path });
}

/**
 * Retry writing a track's library metadata back into the actual audio file.
 * Used by the "稍后重试写回" toast action after an earlier
 * `library_edit_metadata_partial` call failed at the file-write step.
 *
 * Returns the codec error string directly so the frontend can distinguish
 * "library error" from "codec/file-write error" — when this rejects, the
 * library row has already been updated successfully and only the file
 * write-back failed.
 */
export function writeMetadataBackToFile(track_id: number): Promise<void> {
  return invoke('library_write_metadata_back_to_file', { track_id });
}
