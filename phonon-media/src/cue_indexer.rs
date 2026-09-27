//! CUE virtual track indexer.
//!
//! Parses `.cue` files, generates one virtual track row per `TRACK` entry,
//! and links them to the parent `cue_sheets` row. When the underlying audio
//! file disappears, all virtual tracks are soft-deleted automatically.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use phonon_codec::cue::parse_cue;

use crate::error::{MediaLibraryError, Result};
use crate::schema;

/// Index a single CUE file: insert/refresh the `cue_sheets` row and its
/// `tracks` virtual rows.
pub(crate) fn index_cue_file(conn: &Connection, cue_path: &Path) -> Result<()> {
    let content = std::fs::read_to_string(cue_path).map_err(MediaLibraryError::Io)?;
    let sheet = parse_cue(&content)?;

    let cue_path_str = cue_path.to_string_lossy().to_string();

    // Resolve the referenced audio file path (relative to CUE dir if needed).
    let audio_path = resolve_audio_path(cue_path, sheet.file.as_deref());
    let audio_path_str = audio_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    // Get total duration of the underlying audio file from DB if available.
    let total_duration_ms: Option<i64> = conn
        .query_row(
            "SELECT duration_ms FROM tracks WHERE file_path = ?1 AND cue_sheet_id IS NULL",
            rusqlite::params![audio_path_str],
            |r| r.get(0),
        )
        .ok();

    let now = schema::now_ts();
    let track_count = sheet.tracks.len() as i64;

    // Upsert cue_sheets row.
    conn.execute(
        "INSERT INTO cue_sheets (file_path, audio_file_path, title, performer,
            total_duration_ms, track_count, is_deleted, added_at, updated_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?7)
        ON CONFLICT(file_path) DO UPDATE SET
            audio_file_path = excluded.audio_file_path,
            title = excluded.title,
            performer = excluded.performer,
            total_duration_ms = excluded.total_duration_ms,
            track_count = excluded.track_count,
            is_deleted = 0,
            updated_at = excluded.updated_at",
        rusqlite::params![
            cue_path_str,
            audio_path_str,
            sheet.title,
            sheet.performer,
            total_duration_ms,
            track_count,
            now,
        ],
    )?;

    let cue_sheet_id: i64 = conn.query_row(
        "SELECT id FROM cue_sheets WHERE file_path = ?1",
        rusqlite::params![cue_path_str],
        |r| r.get(0),
    )?;

    // Upsert virtual tracks keyed by the stable virtual_path. Content
    // columns are refreshed, but user data (is_favorite / rating /
    // play_count / added_at) is preserved — the previous DELETE + re-INSERT
    // wiped it on every rescan (and the delete cascaded play_history).
    // `unchecked_transaction` is used because the caller (`scan_incremental`)
    // invokes this function before opening its own main transaction, so there
    // is no nesting. Using `transaction` would require `&mut Connection`,
    // which would force a wider mutability signature through the call chain.
    let tx = conn.unchecked_transaction()?;
    let mut current_paths: Vec<String> = Vec::with_capacity(sheet.tracks.len());
    for t in sheet.tracks.iter() {
        let duration_ms = t.duration.map(|d| (d * 1000.0) as i64).or(total_duration_ms);
        let title = t
            .title
            .clone()
            .or_else(|| Some(format!("Track {}", t.index)));
        let performer = t.performer.clone().or_else(|| sheet.performer.clone());

        // Use a synthetic file_path so the unique constraint still holds.
        // Format: <audio_path>#<cue_sheet_id>#<track_index>
        let virtual_path = format!("{}#{}#{}", audio_path_str, cue_sheet_id, t.index);

        tx.execute(
            "INSERT INTO tracks (
                file_path, file_size, mtime, ctime,
                title, artist, album, album_artist, genre, composer,
                year, track_number, disc_number,
                duration_ms, sample_rate, channels, bit_depth, bitrate, format,
                cover_hash,
                replaygain_track_gain, replaygain_track_peak,
                replaygain_album_gain, replaygain_album_peak,
                is_deleted, is_favorite, rating, play_count,
                cue_sheet_id, cue_track_index,
                added_at, updated_at
            ) VALUES (
                ?1, 0, 0, 0,
                ?2, ?3, ?4, NULL, ?5, NULL,
                NULL, ?6, NULL,
                ?7, NULL, NULL, NULL, NULL, NULL,
                NULL,
                NULL, NULL, NULL, NULL,
                0, 0, 0, 0,
                ?8, ?9,
                ?10, ?10
            )
            ON CONFLICT(file_path) DO UPDATE SET
                title           = excluded.title,
                artist          = excluded.artist,
                album           = excluded.album,
                genre           = excluded.genre,
                track_number    = excluded.track_number,
                duration_ms     = excluded.duration_ms,
                cue_sheet_id    = excluded.cue_sheet_id,
                cue_track_index = excluded.cue_track_index,
                is_deleted      = 0,
                updated_at      = excluded.updated_at",
            rusqlite::params![
                virtual_path,
                title,
                performer,
                sheet.title,
                sheet.genre,
                t.index as i32,
                duration_ms,
                cue_sheet_id,
                t.index as i32,
                now,
            ],
        )?;
        current_paths.push(virtual_path);
    }

    // Tracks removed from the sheet keep their rows (and user data) but are
    // soft-deleted, mirroring how removed physical files are handled.
    let mut stmt = tx.prepare(
        "SELECT id, file_path FROM tracks
         WHERE cue_sheet_id = ?1 AND is_deleted = 0",
    )?;
    let stale_ids: Vec<i64> = stmt
        .query_map(rusqlite::params![cue_sheet_id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?
        .filter_map(|r| r.ok())
        .filter(|(_, p)| !current_paths.contains(p))
        .map(|(id, _)| id)
        .collect();
    drop(stmt);
    for id in stale_ids {
        tx.execute(
            "UPDATE tracks SET is_deleted = 1, updated_at = ?2 WHERE id = ?1",
            rusqlite::params![id, now],
        )?;
    }
    tx.commit()?;

    Ok(())
}

/// Resolve the audio file referenced by a CUE sheet, relative to the CUE
/// file's parent directory if the path is not absolute.
fn resolve_audio_path(cue_path: &Path, file: Option<&str>) -> Option<PathBuf> {
    let file = file?;
    let p = Path::new(file);
    if p.is_absolute() {
        return Some(p.to_path_buf());
    }
    let parent = cue_path.parent()?;
    Some(parent.join(p))
}

/// Soft-delete CUE sheets whose `file_path` is not in `present`.
/// Cascades to virtual tracks via the FK ON DELETE CASCADE.
pub(crate) fn soft_delete_missing_cue_sheets(
    conn: &Connection,
    present: &std::collections::HashSet<String>,
    now: i64,
) -> Result<u64> {
    let mut stmt = conn.prepare("SELECT id, file_path FROM cue_sheets WHERE is_deleted = 0")?;
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
            "UPDATE cue_sheets SET is_deleted = 1, updated_at = ?2 WHERE id = ?1",
            rusqlite::params![id, now],
        )? as u64;
        // Cascade soft-delete to virtual tracks.
        conn.execute(
            "UPDATE tracks SET is_deleted = 1, updated_at = ?2 WHERE cue_sheet_id = ?1",
            rusqlite::params![id, now],
        )?;
    }
    Ok(count)
}
