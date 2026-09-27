//! SQLite schema definition and migration.
//!
//! The schema is idempotent — running it on an already-initialized database
//! is a no-op. All tables use `IF NOT EXISTS`. Forward-only migrations are
//! tracked in the `schema_version` key of the `settings` table.

use rusqlite::Connection;

use crate::error::Result;

/// Current schema version. Bump when changing DDL.
pub const SCHEMA_VERSION: u32 = 1;

/// Apply the full schema to a fresh or existing database.
///
/// This is safe to call repeatedly. After applying the DDL, the
/// `schema_version` setting is updated to `SCHEMA_VERSION`.
///
/// NOTE: Prefer [`apply_migrations`] for new code; this helper is kept for
/// backwards-compat with callers that don't need the v0→v2 PRAGMA user_version
/// migration path (e.g. tests that build a fresh in-memory DB).
pub fn apply_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_DDL)?;
    set_setting(conn, "schema_version", &SCHEMA_VERSION.to_string())?;
    Ok(())
}

/// Return the schema version recorded in the database, or 0 if unset.
pub fn schema_version(conn: &Connection) -> Result<u32> {
    let row: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .ok();
    Ok(row.and_then(|s| s.parse().ok()).unwrap_or(0))
}

/// Query `PRAGMA user_version` (0 for new unversioned DBs).
pub fn user_version(conn: &Connection) -> Result<u32> {
    let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    Ok(v.max(0) as u32)
}

/// Set `PRAGMA user_version` (called after successful migration).
pub fn set_user_version(conn: &Connection, v: u32) -> Result<()> {
    conn.execute_batch(&format!("PRAGMA user_version = {v}"))?;
    Ok(())
}

/// Forward-only migration pipeline (v0 → v2). Safe on fresh databases AND on
/// existing user DBs.
///
/// VERSIONING UNIFICATION (CRITICAL — do not delete this paragraph):
/// Existing pre-release builds of phonon-media tracked schema migrations via
/// the `settings(key, value)` table (`key = 'schema_version', value = '1'` as
/// of SCHEMA_VERSION=1). The project-memory spec mandates `PRAGMA user_version`
/// for the v0→v2 sort_key migration so we have TWO version numbers on legacy
/// DBs for ONE release cycle. This function resolves the ambiguity at open
/// time:
///   (a) If `PRAGMA user_version == 0` BUT `settings.schema_version >= 1`,
///       treat the DB as already at schema 1 and **immediately bump
///       PRAGMA = 1** before running any migration. (Legacy users who ran the
///       old code last week but haven't run the new code yet.)
///   (b) If both are 0 → brand new DB, SCHEMA_DDL includes sort_key anyway so
///       migrations become no-ops after the DDL runs.
///   (c) After all migrations succeed, both PRAGMA user_version AND
///       settings.schema_version are written to the same target value (2 in
///       RC1). Future releases can delete the settings-table version path
///       entirely.
///
/// Migration steps:
/// - v0 → v1: ALTER TABLE artists/albums ADD sort_key TEXT + idx_*_sort_key
///   indices (instant on 1M rows, SQLite ALTER TABLE is O(1) metadata-only;
///   index build O(N) but non-blocking for read queries).
/// - v1 → v2: no-op; reserved so future versions can add columns without
///   refactoring.
///
/// On fresh DB (user_version = 0 or DB empty), we run SCHEMA_DDL FIRST which
/// already includes sort_key + idx, so v0 → v1 ALTERs are IF-new-column-missing
/// guarded via PRAGMA table_info.
pub fn apply_migrations(conn: &Connection) -> Result<()> {
    // 0) FIRST: reconcile dual versioning (see big comment above). Prefer
    // settings-table value when PRAGMA is still 0 — covers legacy DBs created
    // before this release.
    {
        let pragma = user_version(conn)?;
        let legacy = schema_version(conn).unwrap_or(0);
        if pragma == 0 && legacy >= 1 {
            log::info!(
                "[migration] legacy DB: PRAGMA user_version=0 but settings.schema_version={}. Setting PRAGMA={legacy} to unify.",
                legacy
            );
            set_user_version(conn, legacy)?;
        }
    }

    // Always ensure the full fresh-DB schema is present first (IF NOT EXISTS
    // everywhere).
    conn.execute_batch(SCHEMA_DDL)?;

    let mut current = user_version(conn)?;
    // ──────────────────────────────────────────────────────────
    // v0 → v1: add sort_key (safe no-op if column already exists in fresh DB)
    // ──────────────────────────────────────────────────────────
    if current < 1 {
        // artists.sort_key
        let col_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('artists') WHERE name = 'sort_key'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !col_exists {
            conn.execute_batch("ALTER TABLE artists ADD COLUMN sort_key TEXT;")?;
            conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_artists_sort_key ON artists(sort_key);",
            )?;
        }
        // albums.sort_key
        let col_exists2: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('albums') WHERE name = 'sort_key'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !col_exists2 {
            conn.execute_batch("ALTER TABLE albums ADD COLUMN sort_key TEXT;")?;
            conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_albums_sort_key ON albums(sort_key);",
            )?;
        }
        set_user_version(conn, 1)?;
        current = 1;
    }
    // ──────────────────────────────────────────────────────────
    // v1 → v2: reserved for future RC2 additions (no-op today).
    // ──────────────────────────────────────────────────────────
    if current < 2 {
        set_user_version(conn, 2)?;
    }
    // Keep legacy settings-table version in sync with PRAGMA (for one cycle,
    // then delete).
    set_setting(conn, "schema_version", &user_version(conn)?.to_string())?;
    Ok(())
}

/// Get a setting value by key.
pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    let v = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            rusqlite::params![key],
            |r| r.get::<_, String>(0),
        )
        .ok();
    Ok(v)
}

/// Upsert a setting value.
pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        rusqlite::params![key, value, now_ts()],
    )?;
    Ok(())
}

/// All schema DDL, idempotent.
const SCHEMA_DDL: &str = r#"
-- ── Core tracks table ──────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS tracks (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path               TEXT    NOT NULL UNIQUE,
    file_size               INTEGER NOT NULL,
    mtime                   INTEGER NOT NULL,
    ctime                   INTEGER NOT NULL,
    title                   TEXT,
    artist                  TEXT,
    album                   TEXT,
    album_artist            TEXT,
    genre                   TEXT,
    composer                TEXT,
    year                    INTEGER,
    track_number            INTEGER,
    disc_number             INTEGER,
    duration_ms             INTEGER,
    sample_rate             INTEGER,
    channels                INTEGER,
    bit_depth               INTEGER,
    bitrate                 INTEGER,
    format                  TEXT,
    cover_hash              TEXT,
    replaygain_track_gain   REAL,
    replaygain_track_peak   REAL,
    replaygain_album_gain   REAL,
    replaygain_album_peak   REAL,
    -- CUE linkage: when not NULL, this row is a virtual track.
    cue_sheet_id            INTEGER REFERENCES cue_sheets(id) ON DELETE CASCADE,
    cue_track_index         INTEGER,
    is_deleted              INTEGER NOT NULL DEFAULT 0,
    is_favorite             INTEGER NOT NULL DEFAULT 0,
    rating                  INTEGER NOT NULL DEFAULT 0 CHECK (rating BETWEEN 0 AND 5),
    play_count              INTEGER NOT NULL DEFAULT 0,
    last_played_at          INTEGER,
    added_at                INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tracks_artist       ON tracks(artist)     WHERE is_deleted = 0;
CREATE INDEX IF NOT EXISTS idx_tracks_album        ON tracks(album)      WHERE is_deleted = 0;
CREATE INDEX IF NOT EXISTS idx_tracks_genre        ON tracks(genre)      WHERE is_deleted = 0;
CREATE INDEX IF NOT EXISTS idx_tracks_favorite     ON tracks(is_favorite) WHERE is_deleted = 0;
CREATE INDEX IF NOT EXISTS idx_tracks_rating       ON tracks(rating)     WHERE is_deleted = 0;
CREATE INDEX IF NOT EXISTS idx_tracks_cue_sheet    ON tracks(cue_sheet_id);
CREATE INDEX IF NOT EXISTS idx_tracks_path         ON tracks(file_path);

-- ── CUE sheets ─────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS cue_sheets (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path       TEXT    NOT NULL UNIQUE,
    audio_file_path TEXT    NOT NULL,
    title           TEXT,
    performer       TEXT,
    total_duration_ms INTEGER,
    track_count     INTEGER NOT NULL DEFAULT 0,
    is_deleted      INTEGER NOT NULL DEFAULT 0,
    added_at        INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

-- ── Albums (denormalized aggregate) ────────────────────────────────
CREATE TABLE IF NOT EXISTS albums (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    title         TEXT    NOT NULL,
    album_artist  TEXT,
    year          INTEGER,
    cover_hash    TEXT,
    track_count   INTEGER NOT NULL DEFAULT 0,
    total_duration_ms INTEGER,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    sort_key      TEXT,
    UNIQUE(title, album_artist)
);
CREATE INDEX IF NOT EXISTS idx_albums_artist ON albums(album_artist);

-- Note: idx_albums_sort_key is created by the v0→v1 migration step below.
-- We intentionally do NOT create it here in SCHEMA_DDL because SCHEMA_DDL
-- runs first inside apply_migrations(), and if albums was created by an
-- OLD schema (pre-v2) without sort_key, the CREATE INDEX would fail with
-- "no such column: sort_key". The v0→v1 step adds the column + index atomically.

-- ── Artists ────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS artists (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT    NOT NULL UNIQUE,
    track_count INTEGER NOT NULL DEFAULT 0,
    album_count INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    sort_key    TEXT
);
-- Note: idx_artists_sort_key lives in the v0→v1 migration step (see comment on
-- idx_albums_sort_key above — same rationale: legacy DBs without sort_key column
-- would fail SCHEMA_DDL CREATE INDEX).

-- ── Genres ─────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS genres (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT    NOT NULL UNIQUE,
    track_count INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

-- ── Folders (folder tree) ──────────────────────────────────────────
CREATE TABLE IF NOT EXISTS folders (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    path        TEXT    NOT NULL UNIQUE,
    parent_id   INTEGER REFERENCES folders(id) ON DELETE CASCADE,
    track_count INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_folders_parent ON folders(parent_id);

-- ── Play history ───────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS play_history (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id    INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    played_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_play_history_track ON play_history(track_id);
CREATE INDEX IF NOT EXISTS idx_play_history_time  ON play_history(played_at DESC);

-- ── Cover thumbnail cache (LRU via last_accessed_at) ───────────────
CREATE TABLE IF NOT EXISTS thumbnails_cache (
    hash              TEXT    NOT NULL,
    size              INTEGER NOT NULL CHECK (size IN (256, 512)),
    webp              BLOB    NOT NULL,
    created_at        INTEGER NOT NULL,
    last_accessed_at  INTEGER NOT NULL,
    PRIMARY KEY (hash, size)
);
-- LRU trim in thumbnails.rs evicts the oldest last_accessed_at rows on
-- every insert — without this index each insert scans the whole cache.
CREATE INDEX IF NOT EXISTS idx_thumbnails_last_accessed
    ON thumbnails_cache(last_accessed_at);

-- ── Library settings (key/value) ───────────────────────────────────
CREATE TABLE IF NOT EXISTS settings (
    key         TEXT PRIMARY KEY,
    value       TEXT NOT NULL,
    updated_at  INTEGER NOT NULL
);

-- ── FTS5 full-text index over tracks ───────────────────────────────
-- content=tracks so FTS mirrors the tracks table; external-content tables
-- require triggers to stay in sync.
CREATE VIRTUAL TABLE IF NOT EXISTS tracks_fts USING fts5(
    title, artist, album, genre,
    content='tracks',
    content_rowid='id',
    tokenize='unicode61 remove_diacritics 2'
);

-- Triggers: keep FTS in sync with tracks INSERT/UPDATE/DELETE.
CREATE TRIGGER IF NOT EXISTS tracks_ai_fts AFTER INSERT ON tracks BEGIN
    INSERT INTO tracks_fts(rowid, title, artist, album, genre)
    VALUES (new.id, new.title, new.artist, new.album, new.genre);
END;
CREATE TRIGGER IF NOT EXISTS tracks_ad_fts AFTER DELETE ON tracks BEGIN
    INSERT INTO tracks_fts(tracks_fts, rowid, title, artist, album, genre)
    VALUES ('delete', old.id, old.title, old.artist, old.album, old.genre);
END;
CREATE TRIGGER IF NOT EXISTS tracks_au_fts AFTER UPDATE ON tracks BEGIN
    INSERT INTO tracks_fts(tracks_fts, rowid, title, artist, album, genre)
    VALUES ('delete', old.id, old.title, old.artist, old.album, old.genre);
    INSERT INTO tracks_fts(rowid, title, artist, album, genre)
    VALUES (new.id, new.title, new.artist, new.album, new.genre);
END;
"#;

/// Current Unix timestamp in seconds (UTC).
pub(crate) fn now_ts() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
