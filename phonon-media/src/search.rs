//! FTS5 full-text search.
//!
//! Performs BM25-ranked search over `tracks_fts` and falls back to LIKE
//! wildcard matching when FTS5 returns no hits (e.g. for short queries
//! that the tokenizer discards).

use rusqlite::Connection;

use crate::error::Result;
use crate::models::{LibraryAlbum, LibraryArtist, LibrarySuggest, LibraryTrack, SearchResult, TrackInfo};

/// Search mode controls how the query string is interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchMode {
    /// FTS5 raw query syntax. Use for power-user input.
    FtsRaw,
    /// FTS5 prefix query — appends `*` to each token. Good for live search.
    #[default]
    FtsPrefix,
    /// FTS5 OR query — tokens match in any order. Good for fuzzy multi-word.
    FtsOr,
}

/// Run a search. Returns at most `limit` results ordered by BM25 score.
///
/// Falls back to LIKE wildcard search on (title, artist, album, genre) when
/// FTS5 returns zero hits, so very short or non-tokenizable queries still
/// produce results.
pub fn search(
    conn: &Connection,
    query: &str,
    mode: SearchMode,
    limit: i64,
) -> Result<Vec<SearchResult>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let fts_query = build_fts_query(query, mode);
    // FTS5 rejects some user inputs outright (bare operators like NOT/OR,
    // malformed phrases). Fall back to LIKE instead of failing the search.
    let mut results = match fts_search(conn, &fts_query, limit) {
        Ok(r) => r,
        Err(e) => {
            log::debug!("[search] FTS query {fts_query:?} failed ({e}); falling back to LIKE");
            Vec::new()
        }
    };
    if results.is_empty() {
        results = like_search(conn, query, limit)?;
    }
    Ok(results)
}

/// Quote a token as an FTS5 string literal so reserved words (AND/OR/NOT)
/// and special characters are treated as text, with embedded quotes doubled.
fn quote_fts_token(t: &str) -> String {
    format!("\"{}\"", t.replace('"', "\"\""))
}

fn build_fts_query(query: &str, mode: SearchMode) -> String {
    // Strip FTS5 special characters to avoid syntax errors.
    let cleaned: String = query
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect();
    let tokens: Vec<&str> = cleaned
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() {
        // Fall back to a quoted search so FTS won't error out.
        return format!("\"{}\"", query.replace('"', " "));
    }
    match mode {
        SearchMode::FtsRaw => cleaned.trim().to_string(),
        SearchMode::FtsPrefix => tokens
            .iter()
            .map(|t| format!("{}*", quote_fts_token(t)))
            .collect::<Vec<_>>()
            .join(" "),
        SearchMode::FtsOr => tokens
            .iter()
            .map(|t| quote_fts_token(t))
            .collect::<Vec<_>>()
            .join(" OR "),
    }
}

fn fts_search(conn: &Connection, fts_query: &str, limit: i64) -> Result<Vec<SearchResult>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.file_path, t.file_size,
                t.title, t.artist, t.album, t.album_artist,
                t.genre, t.composer,
                t.year, t.track_number, t.disc_number,
                t.duration_ms, t.sample_rate, t.channels, t.bit_depth, t.bitrate,
                t.format, t.cover_hash,
                t.replaygain_track_gain, t.replaygain_album_gain,
                t.cue_sheet_id, t.cue_track_index,
                t.is_deleted, t.is_favorite, t.rating,
                t.play_count, t.last_played_at, t.added_at, t.updated_at,
                bm25(tracks_fts) AS score
         FROM tracks_fts
         JOIN tracks t ON t.id = tracks_fts.rowid
         WHERE tracks_fts MATCH ?1
           AND t.is_deleted = 0
         ORDER BY score ASC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![fts_query, limit], row_to_search_result)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

fn like_search(conn: &Connection, query: &str, limit: i64) -> Result<Vec<SearchResult>> {
    // Escape LIKE metacharacters so user input like "100%" matches literally.
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let like = format!("%{escaped}%");
    let mut stmt = conn.prepare(
        "SELECT t.id, t.file_path, t.file_size,
                t.title, t.artist, t.album, t.album_artist,
                t.genre, t.composer,
                t.year, t.track_number, t.disc_number,
                t.duration_ms, t.sample_rate, t.channels, t.bit_depth, t.bitrate,
                t.format, t.cover_hash,
                t.replaygain_track_gain, t.replaygain_album_gain,
                t.cue_sheet_id, t.cue_track_index,
                t.is_deleted, t.is_favorite, t.rating,
                t.play_count, t.last_played_at, t.added_at, t.updated_at
         FROM tracks t
         WHERE t.is_deleted = 0
           AND (t.title LIKE ?1 ESCAPE '\'
                OR t.artist LIKE ?1 ESCAPE '\'
                OR t.album LIKE ?1 ESCAPE '\'
                OR t.genre LIKE ?1 ESCAPE '\')
         ORDER BY t.added_at DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![like, limit], |r| {
        Ok(SearchResult {
            track: row_to_track(r)?,
            // LIKE has no score; assign a constant so the contract holds.
            score: 0.0,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

fn row_to_search_result(r: &rusqlite::Row<'_>) -> rusqlite::Result<SearchResult> {
    // Read the score BEFORE row_to_track consumes the row, since column
    // indices are positional and the score sits at index 30 in the FTS path.
    //
    // bm25() returns a score where more-negative = more relevant.
    // We negate so that higher score = better match.
    let raw: f64 = r.get(30)?;
    let track = row_to_track(r)?;
    Ok(SearchResult { track, score: -raw })
}

/// Shared row decoder used by both FTS and LIKE paths.
/// Column order must match the SELECT statements above.
fn row_to_track(r: &rusqlite::Row<'_>) -> rusqlite::Result<TrackInfo> {
    Ok(TrackInfo {
        id: r.get(0)?,
        file_path: r.get(1)?,
        file_size: r.get::<_, i64>(2)? as u64,
        title: r.get(3)?,
        artist: r.get(4)?,
        album: r.get(5)?,
        album_artist: r.get(6)?,
        genre: r.get(7)?,
        composer: r.get(8)?,
        year: r.get(9)?,
        track_number: r.get(10)?,
        disc_number: r.get(11)?,
        duration_ms: r.get(12)?,
        sample_rate: r.get(13)?,
        channels: r.get(14)?,
        bit_depth: r.get(15)?,
        bitrate: r.get(16)?,
        format: r.get(17)?,
        cover_hash: r.get(18)?,
        replaygain_track_gain: r.get(19)?,
        replaygain_album_gain: r.get(20)?,
        cue_sheet_id: r.get(21)?,
        cue_track_index: r.get(22)?,
        is_deleted: r.get::<_, i64>(23)? != 0,
        is_favorite: r.get::<_, i64>(24)? != 0,
        rating: r.get::<_, i64>(25)? as u8,
        play_count: r.get(26)?,
        last_played_at: r.get(27)?,
        added_at: r.get(28)?,
        updated_at: r.get(29)?,
    })
}

// ─────────────────────────────────────────────────────────────────
// Plan A Task A4: Omnibox `search_suggest` helper.
// ─────────────────────────────────────────────────────────────────

/// Omnisearch suggest: top 5 tracks + 3 albums + 3 artists matching `query`.
///
/// Uses the same FTS5 prefix search as `search::search` for tracks (so
/// Chinese pinyin + ASCII prefix both work via `unicode61 remove_diacritics`
/// tokenizer). Albums/artists don't have their own FTS5 tables, so we fall
/// back to LIKE prefix matching on `title` / `name` — these tables are small
/// (bounded by library size) so a linear scan is fine.
pub fn search_suggest(conn: &Connection, query: &str) -> Result<LibrarySuggest> {
    if query.trim().is_empty() {
        return Ok(LibrarySuggest::default());
    }
    // Tracks: reuse the FTS5 path, clipped to 5.
    let track_hits = search(conn, query, SearchMode::FtsPrefix, 5)?;
    let mut tracks: Vec<LibraryTrack> = Vec::with_capacity(track_hits.len());
    for hit in track_hits {
        let t = hit.track;
        tracks.push(LibraryTrack {
            id: t.id,
            file_path: t.file_path,
            title: t.title,
            artist: t.artist,
            album: t.album,
            genre: t.genre,
            year: t.year,
            track_number: t.track_number,
            duration_ms: t.duration_ms,
            format: t.format,
            cover_hash: t.cover_hash,
            is_favorite: t.is_favorite,
            rating: t.rating,
            play_count: t.play_count as i32,
            last_played_at: t.last_played_at,
            added_at: t.added_at,
        });
    }

    // Albums: LIKE prefix match on title OR album_artist, clipped to 3.
    let like = format!("%{query}%");
    let mut stmt = conn.prepare(
        "SELECT id, title, album_artist, year, cover_hash, track_count, sort_key
         FROM albums
         WHERE title LIKE ?1 OR album_artist LIKE ?1
         ORDER BY album_artist NULLS LAST, title
         LIMIT 3",
    )?;
    let album_rows = stmt.query_map(rusqlite::params![like], |r| {
        Ok(LibraryAlbum {
            id: r.get(0)?,
            title: r.get(1)?,
            album_artist: r.get(2)?,
            year: r.get(3)?,
            cover_hash: r.get(4)?,
            track_count: r.get::<_, i64>(5)? as i32,
            sort_key: r.get(6)?,
        })
    })?;
    let mut albums = Vec::new();
    for r in album_rows {
        albums.push(r?);
    }
    drop(stmt);

    // Artists: LIKE prefix match on name, clipped to 3.
    let mut stmt = conn.prepare(
        "SELECT id, name, track_count, album_count, sort_key
         FROM artists
         WHERE name LIKE ?1
         ORDER BY sort_key NULLS LAST, name
         LIMIT 3",
    )?;
    let artist_rows = stmt.query_map(rusqlite::params![like], |r| {
        Ok(LibraryArtist {
            id: r.get(0)?,
            name: r.get(1)?,
            track_count: r.get::<_, i64>(2)? as i32,
            album_count: r.get::<_, i64>(3)? as i32,
            sort_key: r.get(4)?,
        })
    })?;
    let mut artists = Vec::new();
    for r in artist_rows {
        artists.push(r?);
    }

    Ok(LibrarySuggest {
        tracks,
        albums,
        artists,
    })
}
