//! Incremental scanner.
//!
//! Walks configured library roots, compares each file's
//! `(mtime, ctime, file_size)` triplet against the DB, and only re-parses
//! metadata for files whose triplet changed. CUE sheets are parsed
//! separately to generate virtual tracks.
//!
//! Metadata parsing is parallelized with `rayon` (default 10 workers).
//! DB writes are batched in a single transaction after the parallel scan
//! to minimize SQLite lock contention.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use rayon::prelude::*;
use rusqlite::Connection;

use phonon_codec::{self as codec};

use crate::cue_indexer;
use crate::error::{MediaLibraryError, Result};
use crate::library::{ScanOptions, ScanProgress, ScanStats};
use crate::schema;

/// File extensions recognized as audio (lowercase, no leading dot).
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "flac", "alac", "m4a", "mp4", "wav", "wave", "mp3", "aac", "dsf", "dff", "ogg", "opus",
];

/// File extensions that are NOT audio and should be skipped during scans
/// (mirrors the engine-level filter in the project memory).
pub const NON_AUDIO_EXTENSIONS: &[&str] = &[
    "lrc", "txt", "jpg", "jpeg", "png", "gif", "bmp", "pdf", "zip", "rar", "7z", "cue",
];

/// Returns true if the path has an audio extension.
pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Returns true if the path is a CUE sheet.
pub fn is_cue_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("cue"))
        .unwrap_or(false)
}

/// File triplet used for incremental scan decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileTriplet {
    pub mtime: i64,
    pub ctime: i64,
    pub size: u64,
}

impl FileTriplet {
    /// Read the triplet from a file path.
    pub fn from_path(path: &Path) -> std::io::Result<Self> {
        let meta = std::fs::metadata(path)?;
        let mtime = meta
            .modified()
            .ok()
            .and_then(system_time_to_unix)
            .unwrap_or(0);
        let ctime = meta
            .created()
            .ok()
            .and_then(system_time_to_unix)
            .unwrap_or(mtime);
        Ok(Self {
            mtime,
            ctime,
            size: meta.len(),
        })
    }
}

fn system_time_to_unix(t: SystemTime) -> Option<i64> {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

/// In-memory representation of a freshly parsed track, ready for DB insert.
pub(crate) struct ParsedTrack {
    pub file_path: String,
    pub file_size: u64,
    pub triplet: FileTriplet,
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
    pub cover_hash: Option<String>,
    pub replaygain_track_gain: Option<f64>,
    pub replaygain_track_peak: Option<f64>,
    pub replaygain_album_gain: Option<f64>,
    pub replaygain_album_peak: Option<f64>,
}

/// Files and parsed metadata collected by [`collect_changes`], ready for
/// [`commit_changes`] to write.
pub(crate) struct ScanCollection {
    pub audio_files: Vec<PathBuf>,
    pub cue_files: Vec<PathBuf>,
    pub parsed: Vec<ParsedTrack>,
    pub total: usize,
}

/// Phase 1 of the scan — walk roots and parse metadata. Touches NO database
/// state: the caller passes a snapshot of existing file triplets (taken
/// under a short lock) so the potentially minutes-long parallel parse runs
/// without holding the library mutex.
pub(crate) fn collect_changes(
    roots: &[PathBuf],
    existing: &std::collections::HashMap<String, FileTriplet>,
    options: &ScanOptions,
    progress: &(dyn Fn(ScanProgress) + Send + Sync),
) -> Result<ScanCollection> {
    // Collect candidate files.
    let mut audio_files: Vec<PathBuf> = Vec::new();
    let mut cue_files: Vec<PathBuf> = Vec::new();
    for root in roots {
        if !root.exists() {
            log::warn!("[media] scan root does not exist: {}", root.display());
            continue;
        }
        for entry in walkdir::WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if is_audio_file(path) {
                audio_files.push(path.to_path_buf());
            } else if is_cue_file(path) {
                cue_files.push(path.to_path_buf());
            }
        }
    }

    let total = audio_files.len() + cue_files.len();
    let processed = Arc::new(AtomicUsize::new(0));

    progress(ScanProgress {
        total,
        processed: 0,
        current_path: String::new(),
        phase: "scanning".into(),
    });

    // Parallel parse via rayon.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(options.parallelism.max(1))
        .build()
        .map_err(|e| MediaLibraryError::Codec(format!("rayon pool: {e}")))?;
    let parsed: Vec<ParsedTrack> = pool.install(|| {
        audio_files
            .par_iter()
            .filter_map(|path| {
                let n = processed.fetch_add(1, Ordering::Relaxed) + 1;
                let path_str = path.to_string_lossy().to_string();
                progress(ScanProgress {
                    total,
                    processed: n,
                    current_path: path_str.clone(),
                    phase: "scanning".into(),
                });

                // Incremental skip: triplet unchanged → skip.
                let triplet = match FileTriplet::from_path(path) {
                    Ok(t) => t,
                    Err(e) => {
                        log::warn!("[media] stat failed {}: {}", path.display(), e);
                        return None;
                    }
                };
                if let Some(prev) = existing.get(&path_str) {
                    if *prev == triplet {
                        return None;
                    }
                }

                // Parse metadata + cover hash from a SINGLE decoder open —
                // previously this probed the file twice (extract_metadata +
                // extract_album_art), doubling first-scan I/O.
                match codec::AudioDecoder::open(&path_str) {
                    Ok(decoder) => {
                        let md = decoder.metadata().clone();
                        let cover_hash = decoder.album_art().and_then(|art| sha1_hex(&art.data));
                        Some(build_parsed_track(&path_str, triplet, md, cover_hash))
                    }
                    Err(e) => {
                        log::warn!("[media] metadata failed {}: {}", path.display(), e);
                        // Still insert a minimal row so the file appears in the
                        // library even if its tags are unreadable.
                        Some(ParsedTrack {
                            file_path: path_str,
                            file_size: triplet.size,
                            triplet,
                            title: path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .map(|s| s.to_string()),
                            artist: None,
                            album: None,
                            album_artist: None,
                            genre: None,
                            composer: None,
                            year: None,
                            track_number: None,
                            disc_number: None,
                            duration_ms: None,
                            sample_rate: None,
                            channels: None,
                            bit_depth: None,
                            bitrate: None,
                            format: path
                                .extension()
                                .and_then(|e| e.to_str())
                                .map(|s| s.to_uppercase()),
                            cover_hash: None,
                            replaygain_track_gain: None,
                            replaygain_track_peak: None,
                            replaygain_album_gain: None,
                            replaygain_album_peak: None,
                        })
                    }
                }
            })
            .collect()
    });

    Ok(ScanCollection {
        audio_files,
        cue_files,
        parsed,
        total,
    })
}

/// Phase 2 of the scan — write everything `collect_changes` found. The
/// caller must hold the connection lock; work here is short (CUE parsing
/// plus one batched transaction), so other library operations only block
/// for the commit, not for the filesystem walk.
pub(crate) fn commit_changes(
    conn: &Connection,
    coll: &ScanCollection,
    progress: &(dyn Fn(ScanProgress) + Send + Sync),
) -> Result<ScanStats> {
    // CUE sheets — parse sequentially (typically very few).
    let cue_count = coll.cue_files.len();
    for cue_path in &coll.cue_files {
        let _ = progress(ScanProgress {
            total: coll.total,
            processed: 0,
            current_path: cue_path.to_string_lossy().to_string(),
            phase: "indexing-cue".into(),
        });
        if let Err(e) = cue_indexer::index_cue_file(conn, cue_path) {
            log::warn!("[media] CUE index failed {}: {}", cue_path.display(), e);
        }
    }

    // Batch insert / update in a single transaction.
    let mut stats = ScanStats::default();
    let tx = conn.unchecked_transaction()?;
    {
        let now = schema::now_ts();
        for t in &coll.parsed {
            let changed = upsert_track(&tx, t, now)?;
            if changed {
                stats.tracks_updated += 1;
            } else {
                stats.tracks_inserted += 1;
            }
        }
        // Soft-delete rows whose file is gone from disk.
        let still_present: std::collections::HashSet<String> = coll
            .audio_files
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        stats.tracks_soft_deleted += soft_delete_missing(&tx, &still_present, now)?;
        // Also drop CUE virtual tracks whose sheet no longer exists.
        let cue_present: std::collections::HashSet<String> = coll
            .cue_files
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        stats.cue_sheets_soft_deleted +=
            cue_indexer::soft_delete_missing_cue_sheets(&tx, &cue_present, now)?;
    }
    tx.commit()?;

    // Re-aggregate album/artist/genre/folder counts.
    crate::library::rebuild_aggregates(conn)?;

    stats.total_files = coll.total as u64;
    stats.cue_sheets_indexed = cue_count as u64;
    log::info!(
        "[media] scan complete: {} files, {} inserted, {} updated, {} soft-deleted",
        stats.total_files,
        stats.tracks_inserted,
        stats.tracks_updated,
        stats.tracks_soft_deleted
    );
    Ok(stats)
}

/// Build a `ParsedTrack` from codec metadata.
fn build_parsed_track(
    path: &str,
    triplet: FileTriplet,
    md: codec::TrackMetadata,
    cover_hash: Option<String>,
) -> ParsedTrack {
    ParsedTrack {
        file_path: path.to_string(),
        file_size: triplet.size,
        triplet,
        title: md.title,
        artist: md.artist,
        album: md.album,
        album_artist: md.album_artist,
        genre: md.genre,
        composer: md.composer,
        year: md.year.map(|y| y as i32),
        track_number: md.track_number.map(|n| n as i32),
        disc_number: md.disc_number.map(|n| n as i32),
        duration_ms: md.duration.map(|d| (d * 1000.0) as i64),
        sample_rate: md.sample_rate.map(|r| r as i32),
        channels: md.channels.map(|c| c as i32),
        bit_depth: md.bit_depth.map(|b| b as i32),
        bitrate: md.bitrate.map(|b| b as i32),
        format: md.format.map(|f| format!("{:?}", f)),
        cover_hash,
        replaygain_track_gain: md
            .replay_gain
            .as_ref()
            .and_then(|r| r.track_gain)
            .map(|v| v as f64),
        replaygain_track_peak: md
            .replay_gain
            .as_ref()
            .and_then(|r| r.track_peak)
            .map(|v| v as f64),
        replaygain_album_gain: md
            .replay_gain
            .as_ref()
            .and_then(|r| r.album_gain)
            .map(|v| v as f64),
        replaygain_album_peak: md
            .replay_gain
            .as_ref()
            .and_then(|r| r.album_peak)
            .map(|v| v as f64),
    }
}

/// Insert or update a track row. Returns `true` if updated, `false` if inserted.
fn upsert_track(conn: &Connection, t: &ParsedTrack, now: i64) -> Result<bool> {
    // Try UPDATE first keyed on file_path; if 0 rows affected, INSERT.
    let updated = conn.execute(
        "UPDATE tracks SET
            file_size = ?2,
            mtime = ?3, ctime = ?4,
            title = ?5, artist = ?6, album = ?7, album_artist = ?8,
            genre = ?9, composer = ?10,
            year = ?11, track_number = ?12, disc_number = ?13,
            duration_ms = ?14, sample_rate = ?15, channels = ?16,
            bit_depth = ?17, bitrate = ?18, format = ?19,
            cover_hash = ?20,
            replaygain_track_gain = ?21, replaygain_track_peak = ?22,
            replaygain_album_gain = ?23, replaygain_album_peak = ?24,
            is_deleted = 0,
            updated_at = ?25
         WHERE file_path = ?1",
        rusqlite::params![
            t.file_path,
            t.file_size as i64,
            t.triplet.mtime,
            t.triplet.ctime,
            t.title,
            t.artist,
            t.album,
            t.album_artist,
            t.genre,
            t.composer,
            t.year,
            t.track_number,
            t.disc_number,
            t.duration_ms,
            t.sample_rate,
            t.channels,
            t.bit_depth,
            t.bitrate,
            t.format,
            t.cover_hash,
            t.replaygain_track_gain,
            t.replaygain_track_peak,
            t.replaygain_album_gain,
            t.replaygain_album_peak,
            now,
        ],
    )?;
    if updated > 0 {
        return Ok(true);
    }
    conn.execute(
        "INSERT INTO tracks (
            file_path, file_size, mtime, ctime,
            title, artist, album, album_artist, genre, composer,
            year, track_number, disc_number,
            duration_ms, sample_rate, channels, bit_depth, bitrate, format,
            cover_hash,
            replaygain_track_gain, replaygain_track_peak,
            replaygain_album_gain, replaygain_album_peak,
            is_deleted, is_favorite, rating, play_count,
            added_at, updated_at
        ) VALUES (
            ?1, ?2, ?3, ?4,
            ?5, ?6, ?7, ?8, ?9, ?10,
            ?11, ?12, ?13,
            ?14, ?15, ?16, ?17, ?18, ?19,
            ?20,
            ?21, ?22, ?23, ?24,
            0, 0, 0, 0,
            ?25, ?25
        )",
        rusqlite::params![
            t.file_path,
            t.file_size as i64,
            t.triplet.mtime,
            t.triplet.ctime,
            t.title,
            t.artist,
            t.album,
            t.album_artist,
            t.genre,
            t.composer,
            t.year,
            t.track_number,
            t.disc_number,
            t.duration_ms,
            t.sample_rate,
            t.channels,
            t.bit_depth,
            t.bitrate,
            t.format,
            t.cover_hash,
            t.replaygain_track_gain,
            t.replaygain_track_peak,
            t.replaygain_album_gain,
            t.replaygain_album_peak,
            now,
        ],
    )?;
    Ok(false)
}

/// Load (file_path, triplet) rows for incremental skip.
///
/// Only live rows (`is_deleted = 0`) are returned: a file that comes back
/// after being soft-deleted (drive remount, transient offline root) must
/// re-run `upsert_track` even with an unchanged triplet, because that is
/// what clears `is_deleted` and restores it to the UI.
pub(crate) fn load_existing_triplets(
    conn: &Connection,
) -> Result<std::collections::HashMap<String, FileTriplet>> {
    let mut stmt = conn.prepare(
        "SELECT file_path, mtime, ctime, file_size FROM tracks
         WHERE cue_sheet_id IS NULL AND is_deleted = 0",
    )?;
    let rows = stmt.query_map([], |r| {
        let path: String = r.get(0)?;
        let mtime: i64 = r.get(1)?;
        let ctime: i64 = r.get(2)?;
        let size: i64 = r.get(3)?;
        Ok((
            path,
            FileTriplet {
                mtime,
                ctime,
                size: size as u64,
            },
        ))
    })?;
    let mut map = std::collections::HashMap::new();
    for row in rows {
        let (path, t) = row?;
        map.insert(path, t);
    }
    Ok(map)
}

/// Mark tracks whose file_path is not in `present` as `is_deleted = 1`.
/// Returns the number of rows newly marked.
fn soft_delete_missing(
    conn: &Connection,
    present: &std::collections::HashSet<String>,
    now: i64,
) -> Result<u64> {
    let mut stmt = conn.prepare(
        "SELECT id, file_path FROM tracks
         WHERE is_deleted = 0 AND cue_sheet_id IS NULL",
    )?;
    let to_mark: Vec<i64> = stmt
        .query_map([], |r| {
            let id: i64 = r.get(0)?;
            let path: String = r.get(1)?;
            Ok((id, path))
        })?
        .filter_map(|r| r.ok())
        .filter(|(_, p)| !present.contains(p))
        .map(|(id, _)| id)
        .collect();
    drop(stmt);

    let mut count = 0u64;
    for id in &to_mark {
        count += conn.execute(
            "UPDATE tracks SET is_deleted = 1, updated_at = ?2 WHERE id = ?1",
            rusqlite::params![id, now],
        )? as u64;
    }
    Ok(count)
}

/// Lowercase hex encode a byte slice (no alloc for small inputs).
fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

/// Compute the SHA1 hash of `data` and return it as a lowercase hex string.
fn sha1_hex(data: &[u8]) -> Option<String> {
    use sha1::{Digest, Sha1};
    if data.is_empty() {
        return None;
    }
    let mut h = Sha1::new();
    h.update(data);
    Some(hex(&h.finalize()))
}
