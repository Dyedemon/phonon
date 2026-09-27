//! Main `MediaLibrary` entry point: opens the SQLite database, applies the
//! schema, exposes scan/query/favorite/rating/play-count operations.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use pinyin::ToPinyin;
use rusqlite::Connection;

use crate::error::{MediaLibraryError, Result};
use crate::models::{
    AlbumInfo, ArtistInfo, FolderInfo, GenreInfo, PlayHistoryEntry, SortOrder, ThumbnailInfo,
    TrackFilter, TrackInfo, TrackSortField,
};
use crate::scanner;
use crate::schema;
use crate::search::{self, SearchMode};
use crate::thumbnails;

/// Returns the backend sort_key for a Chinese/ASCII artist or album name.
///
/// Rule set (single source of truth; ALL artist/album list + grouping UI uses
/// these same values):
///   1. Each Chinese character → full pinyin syllable, lowercase, no-tone,
///      concatenated with zero separator.
///   2. ASCII/numbers/punctuation/spaces → `.to_ascii_lowercase()` in place
///      (no reformatting, preserves spaces for mixed names like "JJ Lin").
///   3. Never empty: if the name is empty or pinyin conversion produces zero
///      chars, return `"zzz-" + original name` so it sorts to the very end of
///      the list with other unhandled items (never loses grouping).
pub fn compute_sort_key(name: &str) -> String {
    if name.is_empty() {
        return "zzz-empty".to_string();
    }
    let mut out = String::with_capacity(name.len() * 2);
    let mut produced_output = false;
    for ch in name.chars() {
        if ch.is_ascii() {
            // fast path: to_ascii_lowercase in-place
            out.push(ch.to_ascii_lowercase());
            produced_output = true;
            continue;
        }
        // Hanzi / other non-ASCII: try pinyin
        if let Some(py) = ch.to_pinyin() {
            out.push_str(py.plain());
            produced_output = true;
        } else {
            // Emoji, CJK sym, JP kanji: fall back to original char (keeps
            // ordering stable if not pinyin-convertible)
            out.push(ch);
            produced_output = true;
        }
    }
    if !produced_output {
        return format!("zzz-{name}");
    }
    out
}

/// Build a local-ish `YYYYMMDD-HHMMSS` timestamp for the corrupt-DB backup
/// filename (spec §5.10 #4 / Plan A Task A6).
///
/// `phonon-media` deliberately does not depend on `chrono`, so this uses
/// `std::time::SystemTime` + Howard Hinnant's civil-from-days algorithm.
/// Returns UTC (not local-tz) — the goal is a unique, lexicographically
/// sortable suffix on the backup file; exact wall-clock correctness is
/// not load-bearing.
fn format_local_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs: i64 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let hour = sod / 3600;
    let min = (sod % 3600) / 60;
    let sec = sod % 60;
    // Civil-from-days (Hinnant). `days` is days-since-1970-01-01.
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        year, m, d, hour, min, sec
    )
}

/// Options controlling an incremental scan.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Number of metadata-parsing worker threads (default 10).
    pub parallelism: usize,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self { parallelism: 10 }
    }
}

/// Per-file progress notification delivered during a scan.
#[derive(Debug, Clone)]
pub struct ScanProgress {
    pub total: usize,
    pub processed: usize,
    pub current_path: String,
    pub phase: String,
}

/// Aggregate result of a scan run.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ScanStats {
    pub total_files: u64,
    pub tracks_inserted: u64,
    pub tracks_updated: u64,
    pub tracks_soft_deleted: u64,
    pub cue_sheets_indexed: u64,
    pub cue_sheets_soft_deleted: u64,
}

/// Aggregate result of a ReplayGain batch scan.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ReplayGainScanStats {
    pub total_tracks: u64,
    pub scanned: u64,
    pub skipped: u64,
    pub failed: u64,
}

/// Per-track progress notification delivered during a ReplayGain scan.
#[derive(Debug, Clone)]
pub struct ReplayGainScanProgress {
    pub total: usize,
    pub processed: usize,
    pub current_path: String,
    pub phase: String,
}

/// The media library. Owns the SQLite connection behind an `Arc<Mutex<...>>`
/// since `rusqlite::Connection` is not `Sync`. The `Arc` wrapper lets the
/// background `spawn_sort_key_backfill` thread share the same connection
/// (with short-lived locks per batch) instead of opening a second one.
pub struct MediaLibrary {
    conn: Arc<Mutex<Connection>>,
    db_path: PathBuf,
}

impl MediaLibrary {
    /// Open or create a media library at the given path. The parent
    /// directory must exist.
    ///
    /// §5.10 Resilience #4 (Plan A Task A6): before reading, run
    /// `PRAGMA integrity_check`. If the result isn't `"ok"`, copy the broken
    /// file to `<name>.corrupt-YYYYMMDD-HHMMSS`, delete the live file, open a
    /// fresh empty DB in its place, and emit `AppEvent::LibraryCorrupted`
    /// via `phonon_core::EVENT_TX` so the frontend shows the red banner.
    /// The fresh DB then goes through normal migrations like a brand-new
    /// install.
    pub fn open(db_path: impl AsRef<Path>) -> Result<Self> {
        let db_path = db_path.as_ref().to_path_buf();
        if let Some(parent) = db_path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut conn = Connection::open(&db_path)?;

        // ── §5.10 Resilience #4: auto-detect corrupt DB before reading ──
        let integrity_check: String = conn
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap_or_else(|_| "corrupt".to_string());
        let is_corrupt = !integrity_check.trim().eq_ignore_ascii_case("ok");
        if is_corrupt {
            let backup = db_path.with_file_name(format!(
                "{}.corrupt-{}",
                db_path
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or("library.db"),
                format_local_timestamp()
            ));
            let backup_result = std::fs::copy(&db_path, &backup);
            drop(conn);
            if let Err(e) = backup_result {
                // Never delete the only copy of the library when the backup
                // failed (disk full, permissions, lock). Surface the error so
                // the user can move the file manually and recover.
                log::error!(
                    "[MediaLibrary::open] corrupt DB detected, but backup copy to {:?} FAILED: {}. \
                     Keeping the original file in place.",
                    backup,
                    e
                );
                return Err(MediaLibraryError::CorruptDatabase(format!(
                    "database is corrupt and backup to '{}' failed: {}",
                    backup.to_string_lossy(),
                    e
                )));
            }
            log::error!(
                "[MediaLibrary::open] corrupt DB detected! \
                 BACKUP COPIED to {:?}. Creating fresh empty DB...",
                backup
            );
            let _ = std::fs::remove_file(&db_path);
            conn = Connection::open(&db_path)?;
            // Emit LibraryCorrupted so the frontend shows the red banner
            // (spec §5.10 #4). Best-effort: silently no-ops when the channel
            // isn't initialized (unit tests / library-only contexts).
            if let Some(tx) = phonon_core::EVENT_TX.get() {
                let _ = tx.send(phonon_core::AppEvent::LibraryCorrupted {
                    backup_path: backup.to_string_lossy().to_string(),
                });
            }
        }

        // WAL mode for better concurrent read performance.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        // Larger cache to speed up scans & searches.
        conn.pragma_update(None, "cache_size", "-65536")?; // 64MB

        schema::apply_migrations(&conn)?;

        let conn = Arc::new(Mutex::new(conn));
        // Spawn the legacy-NULL sort_key backfill thread (Plan A Task A2).
        // Non-blocking: returns <50ms regardless of DB size. The thread holds
        // short locks per 500-row batch so concurrent reads aren't blocked.
        spawn_sort_key_backfill(conn.clone());

        Ok(Self { conn, db_path })
    }

    /// Path of the underlying SQLite file.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Run an incremental scan over the given roots. `progress` is invoked
    /// for each file processed (best-effort, exceptions ignored).
    pub fn scan(
        &self,
        roots: &[PathBuf],
        options: &ScanOptions,
        progress: &(dyn Fn(ScanProgress) + Send + Sync),
    ) -> Result<ScanStats> {
        let conn = self.conn.lock().map_err(poison)?;
        scanner::scan_incremental(&conn, roots, options, progress)
    }

    /// Search the library. See [`search::search`] for details.
    pub fn search(
        &self,
        query: &str,
        mode: SearchMode,
        limit: i64,
    ) -> Result<Vec<crate::models::SearchResult>> {
        let conn = self.conn.lock().map_err(poison)?;
        search::search(&conn, query, mode, limit)
    }

    /// List tracks matching a filter.
    pub fn get_tracks(&self, filter: &TrackFilter) -> Result<Vec<TrackInfo>> {
        let conn = self.conn.lock().map_err(poison)?;
        get_tracks_inner(&conn, filter)
    }

    /// Fetch a single track by id.
    pub fn get_track(&self, id: i64) -> Result<Option<TrackInfo>> {
        let conn = self.conn.lock().map_err(poison)?;
        get_track_inner(&conn, id)
    }

    /// Fetch a single non-deleted track by its file path.
    ///
    /// Used by the player/playlist to resolve a queue item (which only
    /// carries a path) to a full `TrackInfo` row for favorite/rating UI.
    /// Returns `None` when the path is not in the library (e.g. the file
    /// was added to the queue directly without scanning).
    pub fn get_track_by_path(&self, path: &str) -> Result<Option<TrackInfo>> {
        let conn = self.conn.lock().map_err(poison)?;
        let mut stmt = conn.prepare(
            "SELECT id, file_path, file_size, title, artist, album, album_artist,
                    genre, composer, year, track_number, disc_number,
                    duration_ms, sample_rate, channels, bit_depth, bitrate,
                    format, cover_hash, replaygain_track_gain, replaygain_album_gain,
                    cue_sheet_id, cue_track_index,
                    is_deleted, is_favorite, rating, play_count, last_played_at,
                    added_at, updated_at
             FROM tracks WHERE file_path = ?1 AND is_deleted = 0",
        )?;
        let row = stmt.query_row(rusqlite::params![path], track_from_row).ok();
        Ok(row)
    }

    /// List recent play history entries, newest first.
    ///
    /// Joins `play_history` with `tracks` so the UI gets title/artist/etc.
    /// in one call. `limit` caps the number of rows (default 200) to avoid
    /// unbounded result sets.
    pub fn list_play_history(&self, limit: Option<i64>) -> Result<Vec<PlayHistoryEntry>> {
        let conn = self.conn.lock().map_err(poison)?;
        let cap = limit.unwrap_or(200).clamp(1, 5000);
        let mut stmt = conn.prepare(
            "SELECT ph.id, ph.played_at,
                    t.id, t.file_path, t.file_size, t.title, t.artist, t.album,
                    t.album_artist, t.genre, t.composer, t.year, t.track_number,
                    t.disc_number, t.duration_ms, t.sample_rate, t.channels,
                    t.bit_depth, t.bitrate, t.format, t.cover_hash,
                    t.replaygain_track_gain, t.replaygain_album_gain,
                    t.cue_sheet_id, t.cue_track_index,
                    t.is_deleted, t.is_favorite, t.rating, t.play_count,
                    t.last_played_at, t.added_at, t.updated_at
             FROM play_history ph
             JOIN tracks t ON t.id = ph.track_id
             ORDER BY ph.played_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![cap], |row| {
            // track_from_row reads 30 columns starting at offset 2 (after
            // history_id and played_at). We map them manually here to keep
            // the column order in sync with the SELECT above.
            let track = track_from_row_offset(row, 2)?;
            Ok(PlayHistoryEntry {
                history_id: row.get(0)?,
                played_at: row.get(1)?,
                track,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Toggle favorite flag. Returns the new state.
    pub fn toggle_favorite(&self, id: i64) -> Result<bool> {
        let conn = self.conn.lock().map_err(poison)?;
        let now = schema::now_ts();
        let affected = conn.execute(
            "UPDATE tracks SET is_favorite = 1 - is_favorite, updated_at = ?2
             WHERE id = ?1",
            rusqlite::params![id, now],
        )?;
        if affected == 0 {
            return Err(MediaLibraryError::TrackNotFound(id));
        }
        let fav: bool = conn.query_row(
            "SELECT is_favorite FROM tracks WHERE id = ?1",
            rusqlite::params![id],
            |r| r.get::<_, i64>(0).map(|v| v != 0),
        )?;
        Ok(fav)
    }

    /// Set rating (0..=5).
    pub fn set_rating(&self, id: i64, rating: u8) -> Result<()> {
        if rating > 5 {
            return Err(MediaLibraryError::InvalidRating(rating));
        }
        let conn = self.conn.lock().map_err(poison)?;
        let now = schema::now_ts();
        let affected = conn.execute(
            "UPDATE tracks SET rating = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![id, rating as i64, now],
        )?;
        if affected == 0 {
            return Err(MediaLibraryError::TrackNotFound(id));
        }
        Ok(())
    }

    /// Record a play event: bump play_count + insert into play_history.
    pub fn record_play(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().map_err(poison)?;
        let now = schema::now_ts();
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE tracks SET play_count = play_count + 1, last_played_at = ?2, updated_at = ?2
             WHERE id = ?1",
            rusqlite::params![id, now],
        )?;
        tx.execute(
            "INSERT INTO play_history (track_id, played_at) VALUES (?1, ?2)",
            rusqlite::params![id, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Manually edit a track's metadata. Compliant: only accepts manual
    /// text input or a local cover hash supplied by the user. No third-party
    /// API calls are made.
    #[allow(clippy::too_many_arguments)]
    pub fn edit_metadata(
        &self,
        id: i64,
        title: Option<String>,
        artist: Option<String>,
        album: Option<String>,
        album_artist: Option<String>,
        genre: Option<String>,
        composer: Option<String>,
        year: Option<i32>,
        track_number: Option<i32>,
        disc_number: Option<i32>,
    ) -> Result<TrackInfo> {
        let conn = self.conn.lock().map_err(poison)?;
        let now = schema::now_ts();
        let affected = conn.execute(
            "UPDATE tracks SET
                title           = COALESCE(?2, title),
                artist          = COALESCE(?3, artist),
                album           = COALESCE(?4, album),
                album_artist    = COALESCE(?5, album_artist),
                genre           = COALESCE(?6, genre),
                composer        = COALESCE(?7, composer),
                year            = COALESCE(?8, year),
                track_number    = COALESCE(?9, track_number),
                disc_number     = COALESCE(?10, disc_number),
                updated_at      = ?11
             WHERE id = ?1",
            rusqlite::params![
                id,
                title,
                artist,
                album,
                album_artist,
                genre,
                composer,
                year,
                track_number,
                disc_number,
                now,
            ],
        )?;
        if affected == 0 {
            return Err(MediaLibraryError::TrackNotFound(id));
        }
        // Re-aggregate since album/artist names may have changed.
        rebuild_aggregates(&conn)?;
        get_track_inner(&conn, id)?.ok_or(MediaLibraryError::TrackNotFound(id))
    }

    /// Physically remove soft-deleted rows. Returns the number of rows
    /// deleted. Also vacuums the FTS table.
    pub fn cleanup_deleted(&self) -> Result<u64> {
        let conn = self.conn.lock().map_err(poison)?;
        let n = conn.execute(
            "DELETE FROM tracks WHERE is_deleted = 1 AND cue_sheet_id IS NULL",
            [],
        )? as u64;
        // Cascade-delete virtual tracks of soft-deleted CUE sheets.
        conn.execute(
            "DELETE FROM tracks WHERE cue_sheet_id IN (
                SELECT id FROM cue_sheets WHERE is_deleted = 1
             )",
            [],
        )?;
        conn.execute("DELETE FROM cue_sheets WHERE is_deleted = 1", [])?;
        conn.execute("INSERT INTO tracks_fts(tracks_fts) VALUES('rebuild')", [])?;
        rebuild_aggregates(&conn)?;
        Ok(n)
    }

    /// List all albums.
    pub fn list_albums(&self) -> Result<Vec<AlbumInfo>> {
        let conn = self.conn.lock().map_err(poison)?;
        let mut stmt = conn.prepare(
            "SELECT id, title, album_artist, year, cover_hash, track_count, total_duration_ms
             FROM albums ORDER BY album_artist NULLS LAST, title",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(AlbumInfo {
                id: r.get(0)?,
                title: r.get(1)?,
                album_artist: r.get(2)?,
                year: r.get(3)?,
                cover_hash: r.get(4)?,
                track_count: r.get(5)?,
                total_duration_ms: r.get(6)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// List all artists.
    pub fn list_artists(&self) -> Result<Vec<ArtistInfo>> {
        let conn = self.conn.lock().map_err(poison)?;
        let mut stmt =
            conn.prepare("SELECT id, name, track_count, album_count FROM artists ORDER BY name")?;
        let rows = stmt.query_map([], |r| {
            Ok(ArtistInfo {
                id: r.get(0)?,
                name: r.get(1)?,
                track_count: r.get(2)?,
                album_count: r.get(3)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// List all genres.
    pub fn list_genres(&self) -> Result<Vec<GenreInfo>> {
        let conn = self.conn.lock().map_err(poison)?;
        let mut stmt = conn.prepare("SELECT id, name, track_count FROM genres ORDER BY name")?;
        let rows = stmt.query_map([], |r| {
            Ok(GenreInfo {
                id: r.get(0)?,
                name: r.get(1)?,
                track_count: r.get(2)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// List all folders.
    pub fn list_folders(&self) -> Result<Vec<FolderInfo>> {
        let conn = self.conn.lock().map_err(poison)?;
        let mut stmt =
            conn.prepare("SELECT id, path, parent_id, track_count FROM folders ORDER BY path")?;
        let rows = stmt.query_map([], |r| {
            Ok(FolderInfo {
                id: r.get(0)?,
                path: r.get(1)?,
                parent_id: r.get(2)?,
                track_count: r.get(3)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Get a cover thumbnail by hash and size (256 or 512).
    pub fn get_thumbnail(&self, hash: &str, size: u32) -> Result<Option<ThumbnailInfo>> {
        let conn = self.conn.lock().map_err(poison)?;
        let webp = thumbnails::get_thumbnail(&conn, hash, size)?;
        Ok(webp.map(|bytes| ThumbnailInfo {
            hash: hash.to_string(),
            size,
            webp: bytes,
        }))
    }

    /// Bind a local JPEG/PNG file as the cover for a track.
    ///
    /// # Compliance
    /// Only local JPEG/PNG files supplied by the user are accepted. This
    /// method validates:
    /// 1. The file exists on the local filesystem (no URLs / network paths).
    /// 2. The extension is `.jpg`, `.jpeg`, or `.png`.
    /// 3. The file size is ≤ 20 MB (reject absurdly large inputs).
    /// 4. The magic bytes match JPEG (FF D8 FF) or PNG (89 50 4E 47).
    ///
    /// No third-party cover/metadata API is ever called. The bytes are
    /// read locally and hashed for the thumbnail cache.
    pub fn set_track_cover_from_file(&self, track_id: i64, cover_path: &Path) -> Result<TrackInfo> {
        use crate::error::MediaLibraryError;

        // 1. Reject non-local paths (URLs, UNC prefixes that aren't files).
        if cover_path.as_os_str().is_empty() {
            return Err(MediaLibraryError::Compliance(
                "cover path is empty".to_string(),
            ));
        }
        // Block http(s):// — these must never be fetched silently.
        let path_str = cover_path.to_string_lossy().to_lowercase();
        if path_str.starts_with("http://")
            || path_str.starts_with("https://")
            || path_str.starts_with("ftp://")
        {
            return Err(MediaLibraryError::Compliance(
                "remote URLs are not allowed; supply a local JPEG/PNG file".to_string(),
            ));
        }

        // 2. Extension whitelist.
        let ext = cover_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_lowercase())
            .unwrap_or_default();
        let is_jpeg = ext == "jpg" || ext == "jpeg";
        let is_png = ext == "png";
        if !is_jpeg && !is_png {
            return Err(MediaLibraryError::Compliance(format!(
                "unsupported cover extension '.{}': only .jpg / .jpeg / .png are allowed",
                if ext.is_empty() { "(none)" } else { &ext }
            )));
        }

        // 3. File must exist; size cap at 20 MB.
        const MAX_COVER_BYTES: u64 = 20 * 1024 * 1024;
        let meta = std::fs::metadata(cover_path)
            .map_err(|e| MediaLibraryError::Compliance(format!("cannot read cover file: {}", e)))?;
        if !meta.is_file() {
            return Err(MediaLibraryError::Compliance(
                "cover path is not a regular file".to_string(),
            ));
        }
        if meta.len() > MAX_COVER_BYTES {
            return Err(MediaLibraryError::Compliance(format!(
                "cover file too large: {} bytes (max {} bytes)",
                meta.len(),
                MAX_COVER_BYTES
            )));
        }

        // 4. Read bytes + magic-byte validation.
        let bytes = std::fs::read(cover_path)?;
        if bytes.len() < 4 {
            return Err(MediaLibraryError::Compliance(
                "cover file too small to be a valid image".to_string(),
            ));
        }
        let magic_ok = if is_jpeg {
            // JPEG SOI: FF D8 FF
            bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF
        } else {
            // PNG signature: 89 50 4E 47 0D 0A 1A 0A (first 4 suffice)
            bytes[0] == 0x89 && bytes[1] == 0x50 && bytes[2] == 0x4E && bytes[3] == 0x47
        };
        if !magic_ok {
            return Err(MediaLibraryError::Compliance(format!(
                "cover file extension is .{} but the content is not a valid {} image",
                ext,
                if is_jpeg { "JPEG" } else { "PNG" }
            )));
        }

        let conn = self.conn.lock().map_err(poison)?;
        let hash = thumbnails::ensure_thumbnail(&conn, &bytes)?.unwrap_or_default();
        let now = schema::now_ts();
        let affected = conn.execute(
            "UPDATE tracks SET cover_hash = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![track_id, hash, now],
        )?;
        if affected == 0 {
            return Err(MediaLibraryError::TrackNotFound(track_id));
        }
        get_track_inner(&conn, track_id)?.ok_or(MediaLibraryError::TrackNotFound(track_id))
    }

    /// Batch-scan ReplayGain for all non-deleted, non-CUE-virtual tracks.
    ///
    /// Mode A (compliant): results are stored in a sidecar SQLite database
    /// (`sidecar_path`) AND mirrored back into the `tracks` table's
    /// `replaygain_track_gain` / `replaygain_track_peak` columns. The original
    /// audio files are never modified.
    ///
    /// `progress` is invoked per track. `force` rescan skips the sidecar cache.
    pub fn batch_scan_replaygain(
        &self,
        sidecar_path: &Path,
        force: bool,
        progress: &(dyn Fn(ReplayGainScanProgress) + Send + Sync),
    ) -> Result<ReplayGainScanStats> {
        let conn = self.conn.lock().map_err(poison)?;

        // Collect candidate file paths (non-deleted, non-CUE-virtual).
        let candidates: Vec<(i64, String)> = {
            let mut stmt = conn.prepare(
                "SELECT id, file_path FROM tracks
                 WHERE is_deleted = 0 AND cue_sheet_id IS NULL
                 ORDER BY file_path",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            out
        };

        let total = candidates.len();
        progress(ReplayGainScanProgress {
            total,
            processed: 0,
            current_path: String::new(),
            phase: "replaygain-start".into(),
        });

        let sidecar = crate::replaygain::open_sidecar(sidecar_path)?;
        let mut stats = ReplayGainScanStats {
            total_tracks: total as u64,
            ..Default::default()
        };
        let now = schema::now_ts();

        for (i, (track_id, file_path)) in candidates.iter().enumerate() {
            progress(ReplayGainScanProgress {
                total,
                processed: i + 1,
                current_path: file_path.clone(),
                phase: "replaygain-scan".into(),
            });

            // Skip if cached and not forced.
            if !force {
                if let Ok(Some(cached)) = crate::replaygain::read_sidecar(&sidecar, file_path) {
                    mirror_replaygain_to_track(&conn, *track_id, &cached, now)?;
                    stats.skipped += 1;
                    continue;
                }
            }

            match crate::replaygain::calculate_replaygain(std::path::Path::new(file_path)) {
                Ok(result) => {
                    let _ = crate::replaygain::store_sidecar(&sidecar, file_path, &result);
                    mirror_replaygain_to_track(&conn, *track_id, &result, now)?;
                    stats.scanned += 1;
                }
                Err(e) => {
                    log::warn!("[media] ReplayGain scan failed for {}: {}", file_path, e);
                    stats.failed += 1;
                }
            }
        }

        progress(ReplayGainScanProgress {
            total,
            processed: total,
            current_path: String::new(),
            phase: "replaygain-done".into(),
        });

        log::info!(
            "[media] ReplayGain scan: {} total, {} scanned, {} skipped, {} failed",
            stats.total_tracks,
            stats.scanned,
            stats.skipped,
            stats.failed
        );
        Ok(stats)
    }

    // ── Plan A Task A4: 6 new library_* backend functions ───────────────

    /// Resolve a batch of file paths to library track rows. Paths are
    /// preserved in input order; duplicates are kept. Max 500 paths per
    /// call (caller should chunk larger sets).
    pub fn get_tracks_by_paths(&self, paths: &[String]) -> Result<Vec<crate::models::LibraryTrack>> {
        if paths.len() > 500 {
            return Err(MediaLibraryError::InvalidPath(format!(
                "paths batch limit 500 (got {}); use multiple calls for larger sets",
                paths.len()
            )));
        }
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().map_err(poison)?;
        // Build a placeholder list (?, ?, ...) and a row index map so we
        // can return rows in the SAME ORDER as the input paths.
        let placeholders: Vec<&str> = paths.iter().map(|_| "?").collect();
        let sql = format!(
            "SELECT id, file_path, title, artist, album, genre, year, track_number,
                    duration_ms, format, cover_hash, is_favorite, rating, play_count,
                    last_played_at, added_at
             FROM tracks
             WHERE is_deleted = 0 AND file_path IN ({})",
            placeholders.join(", ")
        );
        let params: Vec<&dyn rusqlite::ToSql> = paths
            .iter()
            .map(|p| p as &dyn rusqlite::ToSql)
            .collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params.as_slice(), library_track_from_row)?;
        let mut by_path: std::collections::HashMap<String, crate::models::LibraryTrack> =
            std::collections::HashMap::new();
        for r in rows {
            let t = r?;
            by_path.insert(t.file_path.clone(), t);
        }
        // Preserve input order; missing paths are silently dropped (caller
        // can detect by length mismatch).
        let out: Vec<crate::models::LibraryTrack> = paths
            .iter()
            .filter_map(|p| by_path.get(p).cloned())
            .collect();
        Ok(out)
    }

    /// Omnisearch suggest: top 5 tracks + 3 albums + 3 artists matching the
    /// query (FTS5 prefix on tracks, LIKE on albums/artists).
    pub fn search_suggest(&self, query: &str) -> Result<crate::models::LibrarySuggest> {
        let conn = self.conn.lock().map_err(poison)?;
        search::search_suggest(&conn, query)
    }

    /// Apply a partial metadata patch to a track. Updates only the `Some`
    /// fields; rebuilds aggregates (which fills sort_keys); rebuilds FTS via
    /// the existing trigger; attempts file write-back (failures are logged
    /// only — the library write is already committed at that point and is
    /// NEVER rolled back because of a file-write failure).
    ///
    /// NOTE: This is the `PartialMeta`-accepting variant of the existing
    /// [`Self::edit_metadata`] method. Renamed to avoid an impl-level name
    /// collision with that existing method.
    pub fn edit_metadata_partial(
        &self,
        track_id: i64,
        partial: &phonon_codec::PartialMeta,
    ) -> Result<crate::models::LibraryTrack> {
        let conn = self.conn.lock().map_err(poison)?;
        let now = schema::now_ts();
        let affected = conn.execute(
            "UPDATE tracks SET
                title           = COALESCE(?2, title),
                artist          = COALESCE(?3, artist),
                album           = COALESCE(?4, album),
                album_artist    = COALESCE(?5, album_artist),
                genre           = COALESCE(?6, genre),
                composer        = COALESCE(?7, composer),
                year            = COALESCE(?8, year),
                track_number    = COALESCE(?9, track_number),
                disc_number    = COALESCE(?10, disc_number),
                updated_at      = ?11
             WHERE id = ?1",
            rusqlite::params![
                track_id,
                partial.title,
                partial.artist,
                partial.album,
                partial.album_artist,
                partial.genre,
                partial.composer,
                partial.year,
                partial.track_number,
                partial.disc_number,
                now,
            ],
        )?;
        if affected == 0 {
            return Err(MediaLibraryError::TrackNotFound(track_id));
        }
        // Re-aggregate since album/artist names may have changed — this also
        // refreshes sort_keys via fill_null_sort_keys in rebuild_aggregates.
        rebuild_aggregates(&conn)?;

        // Best-effort file write-back. RC1 stub always returns NotImplemented.
        // We log the message but NEVER propagate the error — the library row
        // is already updated and the user can retry file-write later.
        let file_path: Option<String> = conn
            .query_row(
                "SELECT file_path FROM tracks WHERE id = ?1",
                rusqlite::params![track_id],
                |r| r.get(0),
            )
            .ok();
        if let Some(path) = file_path {
            if let Err(e) = phonon_codec::write_metadata_to_file(&path, partial) {
                log::warn!(
                    "[media] write_metadata_to_file failed for {path}: {e}. Library row was updated; file write-back deferred to RC2."
                );
            }
        }

        let mut stmt = conn.prepare(
            "SELECT id, file_path, title, artist, album, genre, year, track_number,
                    duration_ms, format, cover_hash, is_favorite, rating, play_count,
                    last_played_at, added_at
             FROM tracks WHERE id = ?1 AND is_deleted = 0",
        )?;
        let row = stmt.query_row(rusqlite::params![track_id], library_track_from_row)?;
        Ok(row)
    }

    /// Bind raw image bytes as the cover for a track. Hashes via
    /// `thumbnails::ensure_thumbnail`, writes `tracks.cover_hash`, returns
    /// the SHA1 hash. Caller is responsible for content-type validation —
    /// this method accepts any non-empty byte slice.
    ///
    /// NOTE: This is the byte-payload variant of the existing
    /// [`Self::set_track_cover_from_file`] method (which takes a Path).
    /// Renamed to avoid an impl-level name collision.
    pub fn set_track_cover_bytes(
        &self,
        track_id: i64,
        cover_bytes: Vec<u8>,
        _mime: &str,
    ) -> Result<String> {
        if cover_bytes.is_empty() {
            return Err(MediaLibraryError::Compliance(
                "cover bytes are empty".to_string(),
            ));
        }
        let conn = self.conn.lock().map_err(poison)?;
        let hash = thumbnails::ensure_thumbnail(&conn, &cover_bytes)?
            .ok_or_else(|| MediaLibraryError::Compliance("thumbnail encoding failed".to_string()))?;
        let now = schema::now_ts();
        let affected = conn.execute(
            "UPDATE tracks SET cover_hash = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![track_id, &hash, now],
        )?;
        if affected == 0 {
            return Err(MediaLibraryError::TrackNotFound(track_id));
        }
        Ok(hash)
    }

    /// Retry writing a track's library metadata back into the actual audio
    /// file. Used by the "稍后重试写回" toast action after an earlier
    /// `edit_metadata_partial` call failed at the file-write step.
    ///
    /// Builds a `PartialMeta` from the current track row and delegates to
    /// `phonon_codec::write_metadata_to_file`. Returns the codec error
    /// directly (NOT a `MediaLibraryError`) so the frontend can distinguish
    /// "library error" from "codec/file-write error".
    pub fn write_metadata_back_to_file(
        &self,
        track_id: i64,
    ) -> std::result::Result<(), phonon_codec::decoder::CodecError> {
        let conn = self.conn.lock().map_err(|_| {
            phonon_codec::decoder::CodecError::Decode(
                "media library mutex poisoned".to_string(),
            )
        })?;
        let row = conn
            .query_row(
                "SELECT file_path, title, artist, album, album_artist, genre, composer,
                        year, track_number, disc_number, rating
                 FROM tracks WHERE id = ?1",
                rusqlite::params![track_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<String>>(5)?,
                        r.get::<_, Option<String>>(6)?,
                        r.get::<_, Option<i32>>(7)?,
                        r.get::<_, Option<i32>>(8)?,
                        r.get::<_, Option<i32>>(9)?,
                        r.get::<_, i64>(10)? as u8,
                    ))
                },
            )
            .map_err(|e| phonon_codec::decoder::CodecError::Decode(format!(
                "failed to load track row: {e}"
            )))?;
        let (
            file_path,
            title,
            artist,
            album,
            album_artist,
            genre,
            composer,
            year,
            track_number,
            disc_number,
            rating,
        ) = row;
        let partial = phonon_codec::PartialMeta {
            title,
            artist,
            album,
            album_artist,
            genre,
            composer,
            year,
            track_number,
            disc_number,
            rating: Some(rating),
            cover_bytes: None,
        };
        phonon_codec::write_metadata_to_file(&file_path, &partial)
    }
}

/// Row decoder for the `LibraryTrack` subset. Column order must match the
/// SELECT statements in `get_tracks_by_paths` / `edit_metadata_partial`.
fn library_track_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<crate::models::LibraryTrack> {
    Ok(crate::models::LibraryTrack {
        id: r.get(0)?,
        file_path: r.get(1)?,
        title: r.get(2)?,
        artist: r.get(3)?,
        album: r.get(4)?,
        genre: r.get(5)?,
        year: r.get(6)?,
        track_number: r.get(7)?,
        duration_ms: r.get(8)?,
        format: r.get(9)?,
        cover_hash: r.get(10)?,
        is_favorite: r.get::<_, i64>(11)? != 0,
        rating: r.get::<_, i64>(12)? as u8,
        play_count: r.get::<_, i64>(13)? as i32,
        last_played_at: r.get(14)?,
        added_at: r.get(15)?,
    })
}

/// Try to recover tracks from a corrupt-library backup file (Plan A §5.10 #4).
///
/// RC1 stub: opens the backup, runs `PRAGMA integrity_check`; if the backup
/// is readable, row-by-row copy into the live DB; return the count of
/// recovered rows. Today (RC1) we just log a TODO and return `Ok(0)` — the
/// full row-by-row recovery is tracked for RC2.
pub fn try_recover_corrupt(backup_path: &str) -> Result<u32> {
    log::warn!(
        "[media] try_recover_corrupt: backup={backup_path} — RC1 stub returns 0; full recovery planned for RC2"
    );
    // Open the backup so we can at least report integrity_check status in
    // the log (helps the user decide whether to manually extract rows).
    match Connection::open(backup_path) {
        Ok(conn) => {
            let integrity: String = conn
                .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                .unwrap_or_else(|_| "error".to_string());
            log::info!(
                "[media] try_recover_corrupt: backup integrity_check = {integrity}"
            );
        }
        Err(e) => {
            log::error!(
                "[media] try_recover_corrupt: cannot open backup {backup_path}: {e}"
            );
        }
    }
    Ok(0)
}

/// Mirror a computed ReplayGain result into the `tracks` table.
fn mirror_replaygain_to_track(
    conn: &Connection,
    track_id: i64,
    r: &crate::replaygain::ReplayGainResult,
    now: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE tracks SET
            replaygain_track_gain = ?2,
            replaygain_track_peak = ?3,
            updated_at = ?4
         WHERE id = ?1",
        rusqlite::params![track_id, r.track_gain, r.track_peak, now],
    )?;
    Ok(())
}

/// Rebuild denormalized aggregate tables (albums, artists, genres, folders).
/// Called after every scan and metadata edit to keep counts consistent.
///
/// Uses `INSERT ... ON CONFLICT DO UPDATE` so existing primary-key ids stay
/// stable across scans (UI can safely cache ids).
pub(crate) fn rebuild_aggregates(conn: &Connection) -> Result<()> {
    let now = schema::now_ts();

    // Albums: keyed by (title, album_artist).
    //
    // SQLite's UNIQUE constraint treats NULLs as distinct, so the previous
    // INSERT ... ON CONFLICT(title, album_artist) never fired for albums
    // without an album_artist tag and inserted a duplicate row on every
    // rebuild. Split into NULL-safe steps instead:
    //   1. one-time dedup of rows duplicated by past scans (GROUP BY treats
    //      NULLs as equal, unlike UNIQUE), keeping the oldest row so ids stay
    //      stable;
    //   2. UPDATE existing rows via a NULL-safe `IS` join;
    //   3. INSERT only groups that don't exist yet.
    conn.execute(
        "DELETE FROM albums WHERE rowid NOT IN (
            SELECT MIN(rowid) FROM albums GROUP BY title, album_artist
        )",
        [],
    )?;
    conn.execute(
        "UPDATE albums SET
            year              = agg.year,
            cover_hash        = agg.cover_hash,
            track_count       = agg.track_count,
            total_duration_ms = agg.total_duration_ms,
            updated_at        = ?1
         FROM (
            SELECT t.album AS title, t.album_artist,
                   MIN(t.year) AS year, MIN(t.cover_hash) AS cover_hash,
                   COUNT(*) AS track_count, SUM(t.duration_ms) AS total_duration_ms
            FROM tracks t
            WHERE t.is_deleted = 0 AND t.album IS NOT NULL
            GROUP BY t.album, t.album_artist
         ) AS agg
         WHERE albums.title = agg.title
           AND albums.album_artist IS agg.album_artist",
        rusqlite::params![now],
    )?;
    conn.execute(
        "INSERT INTO albums (title, album_artist, year, cover_hash,
                             track_count, total_duration_ms, created_at, updated_at)
         SELECT g.title, g.album_artist, g.year, g.cover_hash,
                g.track_count, g.total_duration_ms, ?1, ?1
         FROM (
            SELECT t.album AS title, t.album_artist,
                   MIN(t.year) AS year, MIN(t.cover_hash) AS cover_hash,
                   COUNT(*) AS track_count, SUM(t.duration_ms) AS total_duration_ms
            FROM tracks t
            WHERE t.is_deleted = 0 AND t.album IS NOT NULL
            GROUP BY t.album, t.album_artist
         ) AS g
         WHERE NOT EXISTS (
            SELECT 1 FROM albums a
            WHERE a.title = g.title AND a.album_artist IS g.album_artist
         )",
        rusqlite::params![now],
    )?;
    // Drop albums with no live tracks.
    conn.execute(
        "DELETE FROM albums WHERE NOT EXISTS (
            SELECT 1 FROM tracks t
            WHERE t.is_deleted = 0
              AND t.album = albums.title
              AND COALESCE(t.album_artist,'') = COALESCE(albums.album_artist,'')
        )",
        [],
    )?;

    // Artists: keyed by name UNIQUE.
    conn.execute(
        "INSERT INTO artists (name, track_count, album_count, created_at, updated_at)
         SELECT t.artist, COUNT(*), COUNT(DISTINCT t.album), ?1, ?1
         FROM tracks t
         WHERE t.is_deleted = 0 AND t.artist IS NOT NULL
         GROUP BY t.artist
         ON CONFLICT(name) DO UPDATE SET
            track_count = excluded.track_count,
            album_count = excluded.album_count,
            updated_at  = excluded.updated_at",
        rusqlite::params![now],
    )?;
    conn.execute(
        "DELETE FROM artists WHERE NOT EXISTS (
            SELECT 1 FROM tracks t
            WHERE t.is_deleted = 0 AND t.artist = artists.name
        )",
        [],
    )?;

    // Genres: keyed by name UNIQUE.
    conn.execute(
        "INSERT INTO genres (name, track_count, created_at, updated_at)
         SELECT t.genre, COUNT(*), ?1, ?1
         FROM tracks t
         WHERE t.is_deleted = 0 AND t.genre IS NOT NULL
         GROUP BY t.genre
         ON CONFLICT(name) DO UPDATE SET
            track_count = excluded.track_count,
            updated_at  = excluded.updated_at",
        rusqlite::params![now],
    )?;
    conn.execute(
        "DELETE FROM genres WHERE NOT EXISTS (
            SELECT 1 FROM tracks t
            WHERE t.is_deleted = 0 AND t.genre = genres.name
        )",
        [],
    )?;

    rebuild_folders(conn, now)?;

    // Fill sort_key for any NULL rows in artists/albums (new rows created by
    // the INSERT...ON CONFLICT paths above when a fresh artist/album name
    // appears). This is the synchronous hot-path complement to the
    // background `spawn_sort_key_backfill` (which handles legacy NULL rows
    // from before the v2 migration). Together they satisfy the plan A2 hook
    // requirement: every INSERT/UPDATE path that touches artists/albums
    // ends up here, so NULL sort_keys are filled inline.
    fill_null_sort_keys(conn)?;

    Ok(())
}

/// Fill NULL `sort_key` rows in `artists` and `albums` by computing
/// `compute_sort_key` in Rust. Single-pass (no batching needed; the
/// `artists` and `albums` tables are bounded by the user's library size —
/// typically <10k rows; even 1M rows finish in well under a second because
/// the heavy lifting is the pinyin conversion of distinct names).
fn fill_null_sort_keys(conn: &Connection) -> Result<()> {
    // Artists: SELECT id, name WHERE sort_key IS NULL → compute → UPDATE.
    {
        let mut stmt = conn.prepare(
            "SELECT id, name FROM artists WHERE sort_key IS NULL",
        )?;
        let rows: Vec<(i64, String)> = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
            .filter_map(|x| x.ok())
            .collect();
        drop(stmt);
        let mut upd = conn.prepare("UPDATE artists SET sort_key = ?1 WHERE id = ?2")?;
        for (id, name) in &rows {
            upd.execute(rusqlite::params![compute_sort_key(name), id])?;
        }
    }
    // Albums: composite key is `album_artist || ' - ' || title` so that
    // grouping by artist first keeps all of an artist's albums together
    // under the same first letter.
    {
        let mut stmt = conn.prepare(
            "SELECT id, COALESCE(album_artist,'') || ' - ' || title FROM albums WHERE sort_key IS NULL",
        )?;
        let rows: Vec<(i64, String)> = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
            .filter_map(|x| x.ok())
            .collect();
        drop(stmt);
        let mut upd = conn.prepare("UPDATE albums SET sort_key = ?1 WHERE id = ?2")?;
        for (id, key_src) in &rows {
            upd.execute(rusqlite::params![compute_sort_key(key_src), id])?;
        }
    }
    Ok(())
}

/// Spawns a background thread that fills `sort_key` for any legacy NULL rows
/// (created before the v2 migration). 500 rows per batch. Emits
/// `AppEvent::LibraryMigrationProgress` via `phonon_core::EVENT_TX` every
/// 5% so the Settings UI card + EmptyStateGuide progress bar can animate.
///
/// Returns immediately (non-blocking; `MediaLibrary::open()` returns in
/// <50ms regardless of DB size). The thread holds a short lock per batch
/// only — long enough to SELECT + UPDATE 500 rows — then drops it between
/// batches so concurrent reads (search/scan) aren't blocked.
pub fn spawn_sort_key_backfill(conn: Arc<Mutex<Connection>>) {
    std::thread::Builder::new()
        .name("phonon-media-sort-key-backfill".to_string())
        .spawn(move || {
            // Count rows to fill first.
            let total: i64 = {
                let Ok(lock) = conn.lock() else { return };
                lock.query_row(
                    "SELECT COUNT(*) FROM (
                        SELECT id FROM artists WHERE sort_key IS NULL
                        UNION ALL
                        SELECT id FROM albums WHERE sort_key IS NULL
                    )",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap_or(0)
            };
            if total == 0 {
                log::info!("[sort_key backfill] nothing to do (0 rows with sort_key IS NULL)");
                emit_progress("done", 0, 0, 100);
                return;
            }
            log::info!(
                "[sort_key backfill] starting: {total} rows missing sort_key, 500/batch"
            );

            let mut processed: i64 = 0;
            let mut last_pct: i32 = -1;

            // ── Process artists first, 500/batch ──
            loop {
                let batch: Vec<(i64, String)> = {
                    let Ok(lock) = conn.lock() else { break };
                    let mut stmt = match lock.prepare(
                        "SELECT id, name FROM artists WHERE sort_key IS NULL LIMIT 500",
                    ) {
                        Ok(s) => s,
                        Err(_) => break,
                    };
                    stmt.query_map([], |r| {
                        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                    })
                    .ok()
                    .map(|rows| rows.filter_map(|x| x.ok()).collect())
                    .unwrap_or_default()
                };
                if batch.is_empty() {
                    break;
                }
                let updates: Vec<(String, i64)> = batch
                    .iter()
                    .map(|(id, name)| (compute_sort_key(name), *id))
                    .collect();
                let n = {
                    let Ok(lock) = conn.lock() else { break };
                    let mut upd = match lock.prepare(
                        "UPDATE artists SET sort_key = ?1 WHERE id = ?2 AND sort_key IS NULL",
                    ) {
                        Ok(s) => s,
                        Err(_) => break,
                    };
                    let mut done = 0i64;
                    for (sk, id) in &updates {
                        done += upd.execute(rusqlite::params![sk, id]).unwrap_or(0) as i64;
                    }
                    done
                };
                processed += n;
                let pct = ((100 * processed) / total) as i32;
                if pct != last_pct && pct % 5 == 0 {
                    last_pct = pct;
                    log::info!("[sort_key backfill] {pct}% ({processed}/{total})");
                    emit_progress("sort_key", processed, total as u64, pct as u8);
                }
                if (n as usize) < 500 {
                    break;
                }
            }

            // ── Then albums (same 500/batch loop; composite key) ──
            loop {
                let batch: Vec<(i64, String)> = {
                    let Ok(lock) = conn.lock() else { break };
                    let mut stmt = match lock.prepare(
                        "SELECT id, COALESCE(album_artist,'') || ' - ' || title
                         FROM albums WHERE sort_key IS NULL LIMIT 500",
                    ) {
                        Ok(s) => s,
                        Err(_) => break,
                    };
                    stmt.query_map([], |r| {
                        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                    })
                    .ok()
                    .map(|rows| rows.filter_map(|x| x.ok()).collect())
                    .unwrap_or_default()
                };
                if batch.is_empty() {
                    break;
                }
                let updates: Vec<(String, i64)> = batch
                    .iter()
                    .map(|(id, key_src)| (compute_sort_key(key_src), *id))
                    .collect();
                let n = {
                    let Ok(lock) = conn.lock() else { break };
                    let mut upd = match lock.prepare(
                        "UPDATE albums SET sort_key = ?1 WHERE id = ?2 AND sort_key IS NULL",
                    ) {
                        Ok(s) => s,
                        Err(_) => break,
                    };
                    let mut done = 0i64;
                    for (sk, id) in &updates {
                        done += upd.execute(rusqlite::params![sk, id]).unwrap_or(0) as i64;
                    }
                    done
                };
                processed += n;
                let pct = ((100 * processed) / total) as i32;
                if pct != last_pct && pct % 5 == 0 {
                    last_pct = pct;
                    log::info!("[sort_key backfill] {pct}% ({processed}/{total})");
                    emit_progress("sort_key", processed, total as u64, pct as u8);
                }
                if (n as usize) < 500 {
                    break;
                }
            }

            log::info!("[sort_key backfill] done: processed {processed} rows");
            emit_progress("done", processed, total as u64, 100);
        })
        .expect("failed to spawn sort_key backfill thread");
}

/// Best-effort emit of `AppEvent::LibraryMigrationProgress` over the global
/// `phonon_core::EVENT_TX`. Silently no-ops when the channel isn't
/// initialized (e.g. unit tests / library-only contexts).
fn emit_progress(phase: &str, processed: i64, total: u64, pct: u8) {
    if let Some(tx) = phonon_core::EVENT_TX.get() {
        let _ = tx.send(phonon_core::AppEvent::LibraryMigrationProgress {
            phase: phase.to_string(),
            processed,
            total,
            pct,
        });
    }
}

/// Rebuild the folder tree from track file paths.
///
/// Extracts the parent directory of each non-deleted, non-CUE-virtual
/// track, inserts each directory and all its ancestors into `folders`,
/// then updates `track_count` per directory. Orphaned folders (no tracks,
/// no children) are removed.
fn rebuild_folders(conn: &Connection, now: i64) -> Result<()> {
    use std::collections::BTreeMap;
    use std::path::Path;

    conn.execute("UPDATE folders SET track_count = 0", [])?;

    // Collect track counts per immediate parent directory.
    let mut stmt = conn.prepare(
        "SELECT file_path FROM tracks
         WHERE is_deleted = 0 AND cue_sheet_id IS NULL",
    )?;
    let mut dir_counts: BTreeMap<String, i64> = BTreeMap::new();
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    for row in rows {
        let path = row?;
        if let Some(parent) = Path::new(&path).parent() {
            let dir = parent.to_string_lossy().to_string();
            if !dir.is_empty() {
                *dir_counts.entry(dir).or_insert(0) += 1;
            }
        }
    }
    drop(stmt);

    // Insert each directory and its ancestors (root-first) so parent_id
    // can be resolved. BTreeMap iterates sorted, so ancestor paths
    // (shorter) are visited before descendant paths (longer).
    let mut dir_to_id: BTreeMap<String, i64> = BTreeMap::new();
    for dir in dir_counts.keys() {
        let mut ancestors: Vec<String> = Vec::new();
        let mut current = Path::new(dir);
        loop {
            ancestors.push(current.to_string_lossy().to_string());
            match current.parent() {
                Some(p) if !p.as_os_str().is_empty() => current = p,
                _ => break,
            }
        }
        ancestors.reverse();

        let mut parent_id: Option<i64> = None;
        for ancestor in &ancestors {
            if let Some(&id) = dir_to_id.get(ancestor) {
                parent_id = Some(id);
                continue;
            }
            conn.execute(
                "INSERT INTO folders (path, parent_id, track_count, created_at, updated_at)
                 VALUES (?1, ?2, 0, ?3, ?3)
                 ON CONFLICT(path) DO UPDATE SET
                    parent_id = excluded.parent_id,
                    updated_at = excluded.updated_at",
                rusqlite::params![ancestor, parent_id, now],
            )?;
            let id = conn.query_row(
                "SELECT id FROM folders WHERE path = ?1",
                rusqlite::params![ancestor],
                |r| r.get::<_, i64>(0),
            )?;
            dir_to_id.insert(ancestor.clone(), id);
            parent_id = Some(id);
        }
    }

    // Update track_count for directories that contain tracks.
    for (dir, count) in &dir_counts {
        conn.execute(
            "UPDATE folders SET track_count = ?2, updated_at = ?3 WHERE path = ?1",
            rusqlite::params![dir, count, now],
        )?;
    }

    // Remove orphaned folders: no tracks and no child folders.
    conn.execute(
        "DELETE FROM folders WHERE track_count = 0 AND NOT EXISTS (
            SELECT 1 FROM folders f2 WHERE f2.parent_id = folders.id
        )",
        [],
    )?;

    Ok(())
}

fn get_track_inner(conn: &Connection, id: i64) -> Result<Option<TrackInfo>> {
    let mut stmt = conn.prepare(
        "SELECT id, file_path, file_size, title, artist, album, album_artist,
                genre, composer, year, track_number, disc_number,
                duration_ms, sample_rate, channels, bit_depth, bitrate,
                format, cover_hash, replaygain_track_gain, replaygain_album_gain,
                cue_sheet_id, cue_track_index,
                is_deleted, is_favorite, rating, play_count, last_played_at,
                added_at, updated_at
         FROM tracks WHERE id = ?1",
    )?;
    let row = stmt.query_row(rusqlite::params![id], track_from_row).ok();
    Ok(row)
}

fn get_tracks_inner(conn: &Connection, filter: &TrackFilter) -> Result<Vec<TrackInfo>> {
    let mut sql = String::from(
        "SELECT id, file_path, file_size, title, artist, album, album_artist,
                genre, composer, year, track_number, disc_number,
                duration_ms, sample_rate, channels, bit_depth, bitrate,
                format, cover_hash, replaygain_track_gain, replaygain_album_gain,
                cue_sheet_id, cue_track_index,
                is_deleted, is_favorite, rating, play_count, last_played_at,
                added_at, updated_at
         FROM tracks WHERE 1=1",
    );
    if !filter.include_deleted {
        sql.push_str(" AND is_deleted = 0");
    }
    if filter.favorites_only {
        sql.push_str(" AND is_favorite = 1");
    }
    if filter.played_only {
        sql.push_str(" AND play_count > 0");
    }
    if let Some(r) = filter.min_rating {
        sql.push_str(&format!(" AND rating >= {}", r as i64));
    }
    if let Some(aid) = filter.album_id {
        sql.push_str(&format!(
            " AND album = (SELECT title FROM albums WHERE id = {})",
            aid
        ));
    }
    if let Some(aid) = filter.artist_id {
        sql.push_str(&format!(
            " AND artist = (SELECT name FROM artists WHERE id = {})",
            aid
        ));
    }
    if let Some(gid) = filter.genre_id {
        sql.push_str(&format!(
            " AND genre = (SELECT name FROM genres WHERE id = {})",
            gid
        ));
    }
    if let Some(fid) = filter.folder_id {
        // Folder filter (folder tree). The folders table doesn't store file
        // paths per track, so we filter by file_path prefix using the
        // folder's path. The pattern anchors on the path separator so sibling
        // directories sharing a string prefix ("C:\Music" vs "C:\Music
        // Backup") don't leak in, and LIKE metacharacters in the path are
        // escaped.
        if let Ok(folder_path) = conn.query_row(
            "SELECT path FROM folders WHERE id = ?1",
            rusqlite::params![fid],
            |r| r.get::<_, String>(0),
        ) {
            let esc = folder_path
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            sql.push_str(&format!(
                " AND file_path LIKE {} || '\\%' ESCAPE '\\'",
                quote_sql_string(&esc)
            ));
        }
    }
    // ORDER BY
    let sort = filter.sort.unwrap_or_default();
    let order = filter.order.unwrap_or_default();
    let col = match sort {
        TrackSortField::Title => "title",
        TrackSortField::Artist => "artist",
        TrackSortField::Album => "album",
        TrackSortField::Year => "year",
        TrackSortField::TrackNumber => "disc_number, track_number",
        TrackSortField::Duration => "duration_ms",
        TrackSortField::PlayCount => "play_count",
        TrackSortField::LastPlayed => "last_played_at",
        TrackSortField::Rating => "rating",
        TrackSortField::AddedAt => "added_at",
    };
    let dir = match order {
        SortOrder::Asc => "ASC",
        SortOrder::Desc => "DESC",
    };
    sql.push_str(&format!(" ORDER BY {col} {dir}"));
    if col != "added_at" {
        sql.push_str(", added_at DESC");
    }
    if let Some(l) = filter.limit {
        sql.push_str(&format!(" LIMIT {}", l));
    }
    if let Some(o) = filter.offset {
        // SQLite only accepts OFFSET after a LIMIT — emit a sentinel
        // LIMIT -1 (no limit) when the caller paged by offset alone.
        if filter.limit.is_none() {
            sql.push_str(" LIMIT -1");
        }
        sql.push_str(&format!(" OFFSET {}", o));
    }

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], track_from_row)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

fn track_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<TrackInfo> {
    track_from_row_offset(r, 0)
}

/// Same as `track_from_row` but reads the 30 track columns starting at
/// `offset` instead of 0. Used by `list_play_history` where the SELECT
/// prefix is `history_id, played_at, ...track cols...`.
fn track_from_row_offset(r: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<TrackInfo> {
    let o = offset;
    Ok(TrackInfo {
        id: r.get(o)?,
        file_path: r.get(o + 1)?,
        file_size: r.get::<_, i64>(o + 2)? as u64,
        title: r.get(o + 3)?,
        artist: r.get(o + 4)?,
        album: r.get(o + 5)?,
        album_artist: r.get(o + 6)?,
        genre: r.get(o + 7)?,
        composer: r.get(o + 8)?,
        year: r.get(o + 9)?,
        track_number: r.get(o + 10)?,
        disc_number: r.get(o + 11)?,
        duration_ms: r.get(o + 12)?,
        sample_rate: r.get(o + 13)?,
        channels: r.get(o + 14)?,
        bit_depth: r.get(o + 15)?,
        bitrate: r.get(o + 16)?,
        format: r.get(o + 17)?,
        cover_hash: r.get(o + 18)?,
        replaygain_track_gain: r.get(o + 19)?,
        replaygain_album_gain: r.get(o + 20)?,
        cue_sheet_id: r.get(o + 21)?,
        cue_track_index: r.get(o + 22)?,
        is_deleted: r.get::<_, i64>(o + 23)? != 0,
        is_favorite: r.get::<_, i64>(o + 24)? != 0,
        rating: r.get::<_, i64>(o + 25)? as u8,
        play_count: r.get(o + 26)?,
        last_played_at: r.get(o + 27)?,
        added_at: r.get(o + 28)?,
        updated_at: r.get(o + 29)?,
    })
}

/// Escape a string for inclusion in a SQL single-quoted literal.
/// Doubles any embedded single quote.
fn quote_sql_string(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn poison<T>(_: std::sync::PoisonError<T>) -> MediaLibraryError {
    MediaLibraryError::Lock("media library mutex poisoned".into())
}
