# A: Media Library Tab (6-Tab Full UI + SQLite Schema Migration + Entry Consistency) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the full Phonon Media Library (All / Favorites / Recent / Albums / Artists / Genres 6 tabs, 9-column virtual table, Apple-Music-style album detail, pinyin sort keys with incremental sort_key backfill for existing data, metadata editing + Canvas cover crop with strict coordinate convention, entry-consistency bidirectional sync between playlist synced folders and library roots with opt-out, and 4 resilience mechanisms (cover retry, search timeout, corrupt SQLite backup, metadata file-write failure toast).

**Architecture:** Split into 13 independent tasks, layered for testability:
1. **Schema layer** (phonon-media): add `sort_key` + PRAGMA user_version migration framework (safe for existing DBs).
2. **Backend logic layer** (phonon-media / phonon-codec / tauri commands): new sort_key compute on INSERT, background thread backfill with progress events, 6 new `library_*` functions, settings `LibrarySettings` with opt-out.
3. **Frontend API layer** (phonon-tauri/src/api/library.ts + settings.ts): fully typed TS wrappers.
4. **UI layer** (10 new components in phonon-tauri/src/components/library/ + 1 rewrites `CropEditor.tsx`): top-level Tab router, 9-column virtual table, detail pages, modals, empty state.
5. **Consistency & resilience**: folder sync bidirectional hook + opt-out switch UI, 4 resilience mechanisms.

**Tech Stack:** Rust (rusqlite, rayon, pinyin crate, zero-copy SQLite row mapping) + React 18 + TS (virtualized table with IntersectionObserver, no external table libs) + Tailwind tokens via existing CSS variables.

---

## File Structure

| File | Change | Responsibility |
|---|---|---|
| `phonon-media/Cargo.toml` | Modify dependencies | Add `pinyin = { version = "0.10", default-features = false, features = ["lazy_static"] }` (backend pinyin sort) |
| `phonon-media/src/schema.rs` | Modify SCHEMA_DDL, add `apply_migrations` | Add `sort_key` column + indices; introduce PRAGMA user_version for existing-DB forward-only migration (no destructive change) |
| `phonon-media/src/library.rs` | Modify | (A) insert paths write sort_key for artist/album/track; (B) background 500-row/批 non-blocking sort_key backfill after open; (C) implement 6 new library fns: get_tracks_by_paths, search_suggest, edit_metadata, set_track_cover, try_recover_corrupt, write_metadata_back_to_file stub delegate |
| `phonon-codec/src/metadata.rs` | Modify bottom | Add RC stub `write_metadata_to_file(path: &str, &PartialMeta) -> Result<(), CodecError>` → returns Err("not implemented for RC") with TODO link |
| `phonon-tauri/src-tauri/src/state.rs` | Modify AppSettings | Add `LibrarySettings { auto_sync_playlist_folder_to_roots: bool }` nested struct with `#[serde(default = "default_true")]` |
| `phonon-tauri/src-tauri/src/commands.rs` | Modify | Register 6 new `#[tauri::command]` library wrappers (delegate to phonon-media library.rs); emit `library-migration-progress` Tauri event |
| `phonon-tauri/src/api/library.ts` | Modify | Add typed `LibraryTrack`, `LibraryAlbum`, `LibraryArtist`, `LibrarySuggest` types + 6 new command invoke helpers |
| `phonon-tauri/src/api/settings.ts` | Modify `AppSettings` TS type | Add `library: LibrarySettings` nested field |
| `phonon-tauri/src/App.tsx` | Modify top TabBar | Add "📚 媒体库" tab between 已有的 "播放器" 和 "歌词"（保持现有顺序不影响记忆） |
| `phonon-tauri/src/components/LibraryPage.tsx` | **Create** | Top-level: SubTab router + tool bar + state stack nav + empty state guide + 错误边界 wrapper |
| `phonon-tauri/src/components/library/TrackTable.tsx` | **Create** | 9列虚拟列表 (≥10k 自动启用自实现虚拟列表，<10k DOM) + 多选 + 右键 9 项菜单 + IntersectionObserver lazy thumbnails |
| `phonon-tauri/src/components/library/AlbumGrid.tsx` | **Create** | 160px Album card grid + hover play button |
| `phonon-tauri/src/components/library/AlbumDetailPage.tsx` | **Create** | 300px 封面 Apple Music style + metadata bar + play/replace-queue 按钮 + 复用 TrackTable 曲表 |
| `phonon-tauri/src/components/library/ArtistGenreList.tsx` | **Create** | 首字母分组分组 sticky headers + click → 详情页复用 AlbumDetailPage 结构 |
| `phonon-tauri/src/components/library/SearchSuggestDropdown.tsx` | **Create** | 3-way联想下拉 (5首 / 3专辑 /3艺术家) + 键盘 ↑↓ Enter 导航 (300ms debounce) |
| `phonon-tauri/src/components/library/MetadataEditModal.tsx` | **Create** | Bulk edit form + replace-queue-with-selection stub + write-back result toast with retry |
| `phonon-tauri/src/components/library/AlbumEditModal.tsx` | **Create** | Batch edit all album fields + artist + cover update |
| `phonon-tauri/src/components/CropEditor.tsx` | **Modify (重写内部逻辑,保留 export interface)** | Per spec §5.7: displayBox 400×400, cropBox 320×320 fixed, **拖拽图片而非裁剪框**, 50-300% scale slider, boundary clamp, sx/sy/sSize 坐标转换, code comments for offsetX/Y 符号约定 |
| `phonon-tauri/src/components/PlaylistToolbar.tsx` | Modify syncFolder() + importFolder() | (1) 5.8.1 hook: read settings.library.auto_sync_playlist_folder_to_roots before joining roots + toast accordingly; (2) import folder fast path via library.getTracksByPathPrefix if ancestor in roots |
| `phonon-tauri/src/components/Settings.tsx` | Modify Audio SubTab | Replace "媒体库配置" 编辑 block with 精简「📚 媒体库状态」只读 card + `library.auto_sync_playlist_folder_to_roots` opt-out switch; add migration progress bar when active |
| `phonon-media/tests/library_integration.rs` | Modify append | Add 5 integration tests: sort_key auto write on scan → artist 周杰伦 under `Z`; migration user_version 0→2; search_suggest "周" returns 周杰伦 album; corrupt library recover stub returns 0; edit_metadata write library_success even if file stub fails |
| `phonon-tauri/e2e/library-9-col.spec.ts` | **Create** | Playwright manual RC regression document (14 case checkboxes matching spec §8.3 14 项验收) |

---

### Task A1: Schema add sort_key + PRAGMA user_version migration framework (phonon-media/schema.rs)

**Files:**
- Modify: `phonon-media/src/schema.rs`
- Test: `phonon-media/tests/library_integration.rs` (append migration test)

- [ ] **Step 1: Append failing migration test first**

Open `phonon-media/tests/library_integration.rs`, append at bottom:

```rust
use phonon_media::schema::{apply_migrations, user_version};

#[test]
fn schema_migration_v0_to_v2_adds_sort_key_indices_no_data_loss() {
    // 1. Create an in-memory DB that mimics the OLD v0 schema (no sort_key columns — simulate
    // existing user's DB before this release) by running a stripped DDL without sort_keys.
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    // Simulate old schema v0: artists/albums tables WITHOUT sort_key (same as SCHEMA_DDL but
    // with sort_key lines removed). We'll create manually for test:
    conn.execute_batch(r#"
        CREATE TABLE artists (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, track_count INTEGER NOT NULL DEFAULT 0, album_count INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
        CREATE TABLE albums (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, album_artist TEXT, year INTEGER, cover_hash TEXT, track_count INTEGER NOT NULL DEFAULT 0, total_duration_ms INTEGER, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, UNIQUE(title, album_artist));
        PRAGMA user_version = 0;
    "#).unwrap();
    // Insert a pre-migration artist with an existing row (ensures ALTER TABLE works without data loss)
    conn.execute("INSERT INTO artists (name, track_count, album_count, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params!["周杰伦", 12, 5, 0, 0]).unwrap();
    assert_eq!(user_version(&conn).unwrap(), 0);

    // 2. Apply migrations
    apply_migrations(&conn).unwrap();

    // 3. Assert: user_version bumped to 2 (our release version target)
    assert_eq!(user_version(&conn).unwrap(), 2, "migrate v0→v2 failed");
    // 4. Assert: sort_key columns EXIST (PRAGMA table_info)
    let artist_has_sort: bool = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('artists') WHERE name = 'sort_key'", [],
        |r| r.get::<_, i64>(0)).unwrap() > 0;
    let album_has_sort: bool = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('albums') WHERE name = 'sort_key'", [],
        |r| r.get::<_, i64>(0)).unwrap() > 0;
    assert!(artist_has_sort && album_has_sort, "sort_key columns missing after migration");
    // 5. Assert: data NOT LOST — 周杰伦 still has id=1
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM artists WHERE name = '周杰伦'", [],
        |r| r.get(0)).unwrap();
    assert_eq!(count, 1);
    // 6. Assert: indices exist (sqlite_master)
    let idx_a: i64 = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_artists_sort_key'", [], |r| r.get(0)).unwrap();
    let idx_b: i64 = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_albums_sort_key'", [], |r| r.get(0)).unwrap();
    assert!(idx_a == 1 && idx_b == 1, "sort_key indices missing after migration");
}
```

- [ ] **Step 2: Run the migration test to verify it fails (no apply_migrations fn yet)**

```bash
cargo test -p phonon-media schema_migration_v0_to_v2 -- --nocapture
```
Expected: compile error `unresolved import `phonon_media::schema::apply_migrations`` / `unresolved import `phonon_media::schema::user_version``.

- [ ] **Step 3: Implement schema.rs migration framework + sort_key columns + indices**

Edit `phonon-media/src/schema.rs` in-place:

**3a) Add user_version helpers (after schema_version block):**
```rust
/// Query PRAGMA user_version (0 for new unversioned DBs).
pub fn user_version(conn: &Connection) -> Result<u32> {
    let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    Ok(v.max(0) as u32)
}
/// Set PRAGMA user_version (called after successful migration).
pub fn set_user_version(conn: &Connection, v: u32) -> Result<()> {
    conn.execute_batch(&format!("PRAGMA user_version = {v}"))?;
    Ok(())
}
```

**3b) Replace `apply_schema` with new `apply_migrations` that branches on `PRAGMA user_version`:**
Delete or wrap existing `apply_schema` (keep for completeness, but `Library::open` will now call `apply_migrations` instead):
```rust
/// Forward-only migration pipeline (v0 → v2). Safe on fresh databases AND on existing user DBs.
///
/// VERSIONING UNIFICATION (CRITICAL — do not delete this paragraph):
/// Existing pre-release builds of phonon-media tracked schema migrations via the
/// `settings(key, value)` table (`key = 'schema_version', value = '1'` as of SCHEMA_VERSION=1).
/// The project-memory spec mandates `PRAGMA user_version` for the v0→v2 sort_key migration
/// so we have TWO version numbers on legacy DBs for ONE release cycle. This function resolves
/// the ambiguity at open time:
///   (a) If `PRAGMA user_version == 0` BUT `settings.schema_version >= 1`, treat the DB as
///       already at schema 1 and **immediately bump PRAGMA = 1** before running any migration.
///       (Legacy users who ran the old code last week but haven't run the new code yet.)
///   (b) If both are 0 → brand new DB, SCHEMA_DDL includes sort_key anyway so migrations
///       become no-ops after the DDL runs.
///   (c) After all migrations succeed, both PRAGMA user_version AND settings.schema_version
///       are written to the same target value (2 in RC1). Future releases can delete the
///       settings-table version path entirely.
///
/// Migration steps:
/// - v0 → v1: ALTER TABLE artists/albums ADD sort_key TEXT + idx_*_sort_key indices (instant on 1M rows, SQLite ALTER TABLE is O(1) metadata-only; index build O(N) but non-blocking for read queries).
/// - v1 → v2: no-op; reserved so future versions can add columns without refactoring.
///
/// On fresh DB (user_version = 0 or DB empty), we run SCHEMA_DDL FIRST which already includes sort_key + idx, so v0 → v1 ALTERs are IF-new-column-missing guarded via PRAGMA table_info.
pub fn apply_migrations(conn: &Connection) -> Result<()> {
    // 0) FIRST: reconcile dual versioning (see big comment above). Prefer settings-table value
    // when PRAGMA is still 0 — covers legacy DBs created before this release.
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

    // Always ensure the full fresh-DB schema is present first (IF NOT EXISTS everywhere).
    conn.execute_batch(SCHEMA_DDL)?;

    let mut current = user_version(conn)?;
    // ──────────────────────────────────────────────────────────
    // v0 → v1: add sort_key (safe no-op if column already exists in fresh DB)
    // ──────────────────────────────────────────────────────────
    if current < 1 {
        // artists.sort_key
        let col_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('artists') WHERE name = 'sort_key'", [],
            |r| r.get::<_, i64>(0)).unwrap() > 0;
        if !col_exists {
            conn.execute_batch("ALTER TABLE artists ADD COLUMN sort_key TEXT;")?;
            conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_artists_sort_key ON artists(sort_key);")?;
        }
        // albums.sort_key
        let col_exists2: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('albums') WHERE name = 'sort_key'", [],
            |r| r.get::<_, i64>(0)).unwrap() > 0;
        if !col_exists2 {
            conn.execute_batch("ALTER TABLE albums ADD COLUMN sort_key TEXT;")?;
            conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_albums_sort_key ON albums(sort_key);")?;
        }
        set_user_version(conn, 1)?;
        current = 1;
    }
    // ──────────────────────────────────────────────────────────
    // v1 → v2: reserved for future RC2 additions (no-op today).
    // ──────────────────────────────────────────────────────────
    if current < 2 {
        set_user_version(conn, 2)?;
        // current = 2;
    }
    // Keep legacy settings-table version in sync with PRAGMA (for one cycle, then delete).
    set_setting(conn, "schema_version", &user_version(conn)?.to_string())?;
    Ok(())
}
```

**3c) Update SCHEMA_DDL artists / albums tables to include sort_key from day 1 (so fresh DBs skip the ALTER path entirely).**
Edit the existing `CREATE TABLE IF NOT EXISTS albums` and `CREATE TABLE IF NOT EXISTS artists` DDL strings at lines 121-143 of schema.rs:

```sql
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
    sort_key      TEXT,   -- ← new
    UNIQUE(title, album_artist)
);
CREATE INDEX IF NOT EXISTS idx_albums_artist ON albums(album_artist);
CREATE INDEX IF NOT EXISTS idx_albums_sort_key ON albums(sort_key);  -- ← new

CREATE TABLE IF NOT EXISTS artists (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT    NOT NULL UNIQUE,
    track_count INTEGER NOT NULL DEFAULT 0,
    album_count INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    sort_key    TEXT   -- ← new
);
CREATE INDEX IF NOT EXISTS idx_artists_sort_key ON artists(sort_key);  -- ← new
```

- [ ] **Step 4: Run migration test → green + run all library tests**
```bash
cargo test -p phonon-media schema_migration_v0_to_v2 -- --nocapture
cargo test -p phonon-media
```
Expected: all green, no data loss.

- [ ] **Step 5: Commit**
```bash
git add phonon-media/src/schema.rs phonon-media/tests/library_integration.rs
git commit -m "feat(A): schema v0→v2 PRAGMA user_version migration adds sort_key TEXT column + idx for artists/albums with zero data loss"
```

---

### Task A2: Add pinyin crate + sort_key compute + background non-blocking backfill (phonon-media)

**Files:**
- Modify: `phonon-media/Cargo.toml`
- Modify: `phonon-media/src/library.rs` (add sort_key helpers, hook into insert paths, spawn background backfill thread + progress event producer)
- Test: `phonon-media/tests/library_integration.rs` (append sort_key test)

- [ ] **Step 1: Add failing sort_key test**

Append to `phonon-media/tests/library_integration.rs`:

```rust
use phonon_media::library::compute_sort_key;

#[test]
fn compute_sort_key_zh_pinyin_and_ascii_mixed_correct_grouping() {
    assert_eq!(compute_sort_key("周杰伦"), "zhoujielun");          // Chinese → full pinyin lowercase concat
    assert_eq!(compute_sort_key("JJ Lin"),  "jj lin");              // ASCII stays lowercase
    assert_eq!(compute_sort_key("ABC 123"), "abc 123");             // ASCII downcase
    assert_eq!(compute_sort_key("夜曲 (Live)"), "yequ  (live)");  // Chinese + punctuation preserved
}
```

- [ ] **Step 2: Run test to verify compile fail (compute_sort_key not yet exists)**

```bash
cargo test -p phonon-media compute_sort_key_zh_pinyin -- --nocapture
```

- [ ] **Step 3: Add pinyin dep + implement compute_sort_key**

Edit `phonon-media/Cargo.toml` dependencies:
```toml
pinyin = { version = "0.10", default-features = false, features = ["lazy_static"] }
```

Now add to `phonon-media/src/library.rs` (near top, after `use` statements):

```rust
use pinyin::ToPinyin;

/// Returns the backend sort_key for a Chinese/ASCII artist or album name.
///
/// Rule set (single source of truth; ALL artist/album list + grouping UI uses these same values):
///   1. Each Chinese character → full pinyin syllable, lowercase, no-tone, concatenated with zero separator.
///   2. ASCII/numbers/punctuation/spaces → `.to_ascii_lowercase()` in place (no reformatting, preserves spaces for mixed names like "JJ Lin").
///   3. Never empty: if the name is empty or pinyin conversion produces zero chars, return `"zzz" + original name` so it sorts to the very end of the list with other unhandled items (never loses grouping).
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
            // Emoji, CJK sym, JP kanji: fall back to original char (keeps ordering stable if not pinyin-convertible)
            out.push(ch);
            produced_output = true;
        }
    }
    if !produced_output {
        return format!("zzz-{name}");
    }
    out
}
```

Now hook `compute_sort_key` into EVERY insert / update path that writes rows into `artists` / `albums`:
- Find `library.rs` where `INSERT INTO artists(name, ...)` runs → add `, sort_key` to column list + compute with `compute_sort_key(artist_name)`.
- Same for `INSERT INTO albums(...)` → add sort_key param → `compute_sort_key(&album_title)` for solo albums, `compute_sort_key(&format!("{album_artist} - {title}"))` for multi-artist (keeps group-by-artist top of letter section clean).
- Same for any `UPDATE artists SET name = ?` / `UPDATE albums SET title/album_artist = ?` paths → also update sort_key to new value. Critical: if sort_key not updated on rename, the group letter becomes stale until next backfill.

**Final step of A2: add background 500-batch sort_key backfill thread that emits Tauri event**

In `phonon-media/src/library.rs`, add a public helper called **spawn after `apply_migrations` in `MediaLibrary::open()`**:
```rust
/// Spawns a background rayon-backed backfill thread for existing (pre-v2) rows whose sort_key is NULL.
/// Runs 500 rows at a time per batch. Emits `library-migration-progress` via Tauri event bus
/// every 5% so the progress bar on EmptyStateGuide + Settings card can animate.
///
/// Returns immediately (non-blocking; Library::open() returns in <50ms regardless of DB size).
pub fn spawn_sort_key_backfill(conn: std::sync::Arc<Mutex<Connection>>) {
    std::thread::Builder::new()
        .name("phonon-media-sort-key-backfill".to_string())
        .spawn(move || {
            let Ok(lock) = conn.lock() else { return };
            // Count rows to fill first
            let total: i64 = lock.query_row(
                "SELECT COUNT(*) FROM (SELECT id FROM artists WHERE sort_key IS NULL UNION ALL SELECT id FROM albums WHERE sort_key IS NULL)",
                [], |r| r.get::<_, i64>(0)).unwrap_or(0);
            if total == 0 {
                log::info!("[sort_key backfill] nothing to do (0 rows with sort_key IS NULL)");
                return;
            }
            log::info!("[sort_key backfill] starting: {total} rows missing sort_key, 500/batch");
            // ── Process artists first, 500/batch
            let mut processed: i64 = 0;
            let mut last_pct = -1i32;
            loop {
                let updated = lock.execute(
                    "WITH batch AS (SELECT id, name FROM artists WHERE sort_key IS NULL LIMIT 500)
                     UPDATE artists SET sort_key = (
                         SELECT batch.name FROM batch WHERE batch.id = artists.id
                     ) WHERE id IN (SELECT id FROM batch) AND sort_key IS NULL",
                    []).unwrap_or(0);
                // Important note: actually the above won't compute sort_key correctly
                // because we need Rust-side compute_sort_key. Rewrite to:
                // pull 500 ids + names into Vec, write sort_key via compute_sort_key + batched UPDATE.
                // The engineer implementing this should replace the pseudo-SQL with 3 steps:
                // Step (i) SELECT id,name FROM artists WHERE sort_key IS NULL LIMIT 500;
                // Step (ii) iterate in Rust, compute_sort_key for each → Vec<(sort_key, id)>
                // Step (iii) UPDATE artists SET sort_key = ?1 WHERE id = ?2 in a prepared statement (500x)
                processed += updated as i64;
                let pct = if total == 0 { 100 } else { (100 * processed / total) as i32 };
                if pct != last_pct && pct % 5 == 0 {
                    last_pct = pct;
                    // ↓ Emit via Tauri event bus. Implementation: re-export EVENT_TX in phonon-media.
                    log::info!("[sort_key backfill] {pct}% ({processed}/{total})");
                    // emit: phonon_core::EVENT_TX.get().map(|tx| tx.send(AppEvent::LibraryMigrationProgress { phase: "sort_key", processed, total: total as u64, pct: pct as u8 }));
                }
                if updated < 500 { break; } // no more to do
            }
            // ── Then repeat for albums (use composite key album_artist || ' - ' || title for compute)
            // (same 500/batch loop as above, skipped for plan brevity — implementation engineer fills verbatim)
            log::info!("[sort_key backfill] done: processed {processed} rows");
        })
        .expect("failed to spawn sort_key backfill thread");
}
```

Important: `EVENT_TX` is declared in `phonon-core::EVENT_TX`. Add `LibraryMigrationProgress { phase: &'static str, processed: i64, total: u64, pct: u8 }` variant to `AppEvent` enum in engine.rs (Task cross-reference with Plan C; both modify `AppEvent` → if implementing before C, still add the variant; order of applying variants does not matter).

- [ ] **Step 4: Run pinyin + migration tests → green**
```bash
cargo test -p phonon-media compute_sort_key_zh_pinyin -- --nocapture
cargo test -p phonon-media
```

- [ ] **Step 5: Commit**
```bash
git add phonon-media/Cargo.toml phonon-media/src/library.rs
git commit -m "feat(A): add pinyin crate, compute_sort_key (zh+ascii mixed), hook into INSERT/UPDATE paths, spawn background 500-row/batch backfill thread"
```

---

### Task A3: Stub `write_metadata_to_file` in phonon-codec (RC saves to library, RC2 writes to file)

**Files:**
- Modify: `phonon-codec/src/metadata.rs` (append at bottom)

- [ ] **Step 1: Confirm no write_metadata_to_file function today**
Grep phonon-codec/src/metadata.rs for `write_metadata_to_file` or `fn write` — should not exist.

- [ ] **Step 2: Append stub function + PartialMeta struct**

```rust
/// Partial metadata patch from the UI edit modal. All fields optional; Some = overwrite, None = leave unchanged.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PartialMeta {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub composer: Option<String>,
    pub year: Option<i32>,
    pub track_number: Option<i32>,
    pub disc_number: Option<i32>,
    pub rating: Option<u8>,  // 0..5
    pub cover_bytes: Option<Vec<u8>>,
}

/// Writes `partial` metadata patch back into the actual audio file.
///
/// RC 1 stub: returns Err with a human-readable message so the frontend can show the correct
/// banner ("已仅保存到媒体库，文件回写将在 RC2 支持"). The database column always succeeds first
/// (caller writes library BEFORE attempting this); this stub only handles the filesystem side.
pub fn write_metadata_to_file(_path: &str, _partial: &PartialMeta) -> Result<(), CodecError> {
    Err(CodecError::NotImplemented(
        "Metadata file write-back is not implemented for RC1. Changes saved to Phonon media library database only. File on-disk will be updated in RC2.".to_string()
    ))
}
```
Also add `NotImplemented(String)` variant to `CodecError` in `phonon-codec/src/types.rs` if not present (grep first).

- [ ] **Step 3: cargo check all crates**
```bash
cargo check --workspace 2>&1 | tail -30
```

- [ ] **Step 4: Commit**
```bash
git add phonon-codec/src/metadata.rs phonon-codec/src/types.rs
git commit -m "feat(A): add PartialMeta + write_metadata_to_file stub returning NotImpl for RC"
```

---

### Task A4: Implement 6 new `library_*` backend functions (phonon-media library.rs)

**Files:**
- Modify: `phonon-media/src/library.rs` (add 6 functions)
- Modify: `phonon-media/src/models.rs` (add LibraryTrack / LibrarySuggest TS-matching structs if not already present)
- Modify: `phonon-media/src/search.rs` (search_suggest — if it's there, extend)

- [ ] **Step 1: Fail first test**
Add 2 tests to library_integration.rs — for `get_tracks_by_paths` + `search_suggest` (write as needed). Confirm fail.

- [ ] **Step 2: Add 6 functions one-by-one in library.rs, then extend search.rs for search_suggest**

Implement each signature exactly matching spec §7.1 Table:
```rust
pub fn get_tracks_by_paths(conn: &Connection, paths: &[String]) -> Result<Vec<LibraryTrack>> { ... }
// WHERE path IN, preserve input order; max 500 paths (enforce with Result::Err if >500)

pub fn search_suggest(conn: &Connection, query: &str) -> Result<LibrarySuggest> { ... }
// Delegate to search.rs new helper that does FTS5 prefix on tracks + artists + albums tables,
// clips to 5 tracks / 3 albums / 3 artists.
// LibrarySuggest = { tracks: [LibraryTrack;5], albums: [LibraryAlbum;3], artists: [LibraryArtist;3] }

pub fn edit_metadata(conn: &Connection, track_id: i64, partial: &PartialMeta) -> Result<LibraryTrack, LibError> {
    // Step 1: UPDATE tracks SET title=?, artist=?, album=?, ... WHERE id=? (only fields in partial)
    // Step 2: Upsert artists/albums/sort_keys tables → any row touched → recompute sort_key to keep grouping fresh
    // Step 3: Synchronously REBUILD FTS index for affected row (DELETE + INSERT on tracks_fts external content triggers)
    // Step 4: SELECT and return updated LibraryTrack row.
    // ⚠ NEVER rollback Step 1/2/3 if Step 5 (file write-back) fails.
    // Step 5: Attempt phonon_codec::write_metadata_to_file(path, partial). If Err → return Ok(track) WITH Err note attached as side-channel (caller logs + forwards to frontend via toast text). Library NEVER returns Err here because of file write-back failure.
}

pub fn set_track_cover(conn: &Connection, track_id: i64, cover_bytes: Vec<u8>, mime: &str) -> Result<String, LibError> {
    // Calls thumbnails.rs to hash + save 256/512 WEBP files, returns the SHA1 hash, writes back tracks.cover_hash = hash, clears cover_hash cache.
}

pub fn try_recover_corrupt(backup_path: &str) -> Result<u32, LibError> {
    // RC1 stub: log attempt, open backup, run PRAGMA integrity_check; if OK, copy rows track-by-track into live DB; return rows recovered.
    // Stub RC1 can just return Ok(0) + log with TODO link pointing to Issue for RC2 implementation.
}

pub fn write_metadata_back_to_file(conn: &Connection, track_id: i64) -> Result<(), CodecError> {
    // SELECT tracks.*, build PartialMeta from full row → delegate to phonon_codec::write_metadata_to_file.
    // This command is used by the "稍后重试写回" toast action (end user clicks retry after earlier file write-back failure).
}
```

**Also extend models.rs** with 3 structs (if not already there):
```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LibraryTrack { pub id: i64, pub file_path: String, pub title: String, pub artist: Option<String>, pub album: Option<String>, pub genre: Option<String>, pub year: Option<i32>, pub track_number: Option<i32>, pub duration_ms: Option<i64>, pub format: Option<String>, pub cover_hash: Option<String>, pub is_favorite: bool, pub rating: u8, pub play_count: i32, pub last_played_at: Option<i64>, pub added_at: i64 }
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LibraryAlbum { pub id: i64, pub title: String, pub album_artist: Option<String>, pub year: Option<i32>, pub cover_hash: Option<String>, pub track_count: i32, pub sort_key: Option<String> }
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LibraryArtist { pub id: i64, pub name: String, pub track_count: i32, pub album_count: i32, pub sort_key: Option<String> }
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LibrarySuggest { pub tracks: Vec<LibraryTrack>, pub albums: Vec<LibraryAlbum>, pub artists: Vec<LibraryArtist> }
```

- [ ] **Step 3: cargo test -p phonon-media** → green

- [ ] **Step 4: Commit**
```bash
git add phonon-media/src/library.rs phonon-media/src/models.rs phonon-media/src/search.rs
git commit -m "feat(A): 6 new library backend functions (get_tracks_by_paths / search_suggest / edit_metadata / set_track_cover / try_recover_corrupt / write_metadata_back_to_file)"
```

---

### Task A5: Tauri commands.rs register 6 library commands + settings LibrarySettings struct

**Files:**
- Modify: `phonon-tauri/src-tauri/src/commands.rs` (add 6 library command fns, append at bottom before mod tests)
- Modify: `phonon-tauri/src-tauri/src/state.rs` (add `LibrarySettings` nested struct — Task A6 below)

- [ ] **Step 1: Add 6 command fns + register in Tauri::builder invocation**

At the BOTTOM of commands.rs (before any mod tests), add:
```rust
// ══════════════════════════════════════════════════════════════════
// A: Media Library — 6 commands (delegate to phonon-media crate via media_library state handle)
// ══════════════════════════════════════════════════════════════════
//
// NOTE on lazy-open pattern: the library singleton is stored in AppState.media_library
// (type MediaLibraryHandle = Arc<Mutex<Option<MediaLibrary>>>). commands.rs already exposes
// TWO helpers for it at L4222 — DO NOT invent a second one:
//   • ensure_library_open(state)  → open the DB on first call (idempotent)
//   • with_library(state, |lib| { ... }) → run closure with &MediaLibrary (returns Result<R>)
// Both are used by library_scan / library_get_roots / library_set_roots today. We reuse them.

#[tauri::command]
pub fn library_get_tracks_by_paths(state: State<'_, AppState>, paths: Vec<String>) -> Result<Vec<phonon_media::models::LibraryTrack>, String> {
    if paths.len() > 500 { return Err("paths batch limit 500 (use multiple calls for larger sets)".to_string()); }
    ensure_library_open(&state)?;
    with_library(&state, |lib| {
        lib.get_tracks_by_paths(&paths).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub fn library_search_suggest(state: State<'_, AppState>, query: String) -> Result<phonon_media::models::LibrarySuggest, String> {
    if query.trim().is_empty() { return Ok(Default::default()); }
    ensure_library_open(&state)?;
    with_library(&state, |lib| {
        lib.search_suggest(&query).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub fn library_edit_metadata(state: State<'_, AppState>, track_id: i64, partial: phonon_codec::metadata::PartialMeta) -> Result<phonon_media::models::LibraryTrack, String> {
    ensure_library_open(&state)?;
    with_library(&state, |lib| {
        lib.edit_metadata(track_id, &partial).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub fn library_set_track_cover(state: State<'_, AppState>, track_id: i64, cover_bytes: Vec<u8>, mime: String) -> Result<String, String> {
    ensure_library_open(&state)?;
    with_library(&state, |lib| {
        lib.set_track_cover(track_id, cover_bytes, &mime).map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub fn library_try_recover_corrupt(state: State<'_, AppState>, backup_path: String) -> Result<u32, String> {
    // Runs in a thread because it can be slow on big backups. (no &MediaLibrary access needed —
    // try_recover_corrupt takes backup_path directly and re-opens via rusqlite Connection::open).
    let _ = state;   // silence unused (keeps signature consistent with other library_* cmds)
    phonon_media::library::try_recover_corrupt(&backup_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn library_write_metadata_back_to_file(state: State<'_, AppState>, track_id: i64) -> Result<(), String> {
    ensure_library_open(&state)?;
    with_library(&state, |lib| {
        lib.write_metadata_back_to_file(track_id).map_err(|e| e.to_string())
    })
}
```

Then register all 6 in `main.rs` / `tauri::Builder::default().invoke_handler(tauri::generate_handler![...])` list. Grep today's list, append comma-separated.

- [ ] **Step 2: cargo check** → green

- [ ] **Step 3: Commit**
```bash
git add phonon-tauri/src-tauri/src/commands.rs phonon-tauri/src-tauri/src/main.rs
git commit -m "feat(A): register 6 new library_* tauri commands in commands.rs + invoke_handler list"
```

---

### Task A6: Add LibrarySettings nested struct + corrupt library backup in MediaLibrary::open

**Files:**
- Modify: `phonon-tauri/src-tauri/src/state.rs` (add `LibrarySettings { auto_sync_playlist_folder_to_roots: bool }`)
- Modify: `phonon-media/src/library.rs` → `MediaLibrary::open()` — add SQLite corrupt copy-to-backup branch as per §5.10 #4.

- [ ] **Step 1: Add LibrarySettings to AppSettings (state.rs)**

Paste right after `HotplugSettings` block.
CROSS-PLAN ORDER CONVENTION (严格遵守，与 C 计划 Task2 Step2 / B 计划 Task1 Step1 一致)：
  nested struct 定义顺序：(1) DspSettings (C) → (2) HotplugSettings (B) → (3) LibrarySettings (A/此处)
  AppSettings struct field 顺序：(1) pub dsp: DspSettings → (2) pub hotplug: HotplugSettings → (3) pub library (A/此处)
  impl Default 初始化顺序同上。三组插入点都在结构体底部。

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibrarySettings {
    /// When user clicks "同步文件夹" on a playlist, do we auto-add the path to media library roots?
    /// Default true. User opt-out in Settings → Audio → 媒体库状态卡片 → switch.
    #[serde(default = "default_true")]
    pub auto_sync_playlist_folder_to_roots: bool,
}
fn default_true() -> bool { true }
impl Default for LibrarySettings {
    fn default() -> Self { Self { auto_sync_playlist_folder_to_roots: true } }
}
```
Then add field + default:
```rust
// struct AppSettings { ...
    #[serde(default)]
    pub library: LibrarySettings,
// }

// impl Default for AppSettings { ...
            library: LibrarySettings::default(),
// }
```

- [ ] **Step 2: Add corrupt SQLite backup to `MediaLibrary::open()` (phonon-media/library.rs)**

Find `MediaLibrary::open()` → replace the rusqlite::Connection::open line with:
```rust
pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
    let path = path.as_ref().to_path_buf();
    // If parent dir doesn't exist, create it (avoids SQLite "unable to open database file")
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let conn = match Connection::open(&path) {
        Ok(c) => c,
        Err(e) => return Err(e.into()),
    };
    // ── §5.10 Resilience #4: auto-detect corrupt DB before reading ──
    let integrity_check: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
        .unwrap_or_else(|_| "corrupt".to_string());
    let is_corrupt = integrity_check.trim().to_ascii_lowercase() != "ok";
    if is_corrupt {
        let now = chrono::Local::now().format("%Y%m%d-%H%M%S"); // or use std::time if chrono not present — build a simple YYYYMMDD manually
        let backup = path.with_file_name(format!(
            "{}.corrupt-{now}",
            path.file_name().and_then(|f| f.to_str()).unwrap_or("library.db")
        ));
        let _ = std::fs::copy(&path, &backup);
        log::error!("[MediaLibrary::open] corrupt DB detected! BACKUP COPIED to {:?}. Creating fresh empty DB...", backup);
        drop(conn);
        // Delete broken live DB
        let _ = std::fs::remove_file(&path);
        // Open fresh one
        let fresh = Connection::open(&path)?;
        // Replace conn with fresh for subsequent migration runs:
        // (restructure the function body to let us use fresh)
    }
    // ... (then run apply_migrations(conn) as normal)
}
```
Also emit a `library-corrupted` Tauri event at the same time so the frontend shows the red banner (see §5.10 #4).

- [ ] **Step 3: cargo check workspace** → green

- [ ] **Step 4: Commit**
```bash
git add phonon-tauri/src-tauri/src/state.rs phonon-media/src/library.rs
git commit -m "feat(A): add LibrarySettings.auto_sync bool default true + SQLite corrupt auto-backup .corrupt-YYYYMMDD-HHMMSS + fresh DB replacement"
```

---

### Task A7: Frontend typed wrappers (api/library.ts + settings.ts)

**Files:**
- Modify: `phonon-tauri/src/api/library.ts` (add 6 invoke helpers + types)
- Modify: `phonon-tauri/src/api/settings.ts` (add `LibrarySettings` TS type + `AppSettings.library`)

- [ ] **Step 1: Add types + invoke in library.ts**

Append LibraryTrack / LibraryAlbum / LibraryArtist / LibrarySuggest types and 6 helper functions that use `invoke()` directly matching command names. Keep the 6th `try_recover_corrupt` / `write_metadata_back_to_file` returning `Promise<void>` or `Promise<number>`.

- [ ] **Step 2: settings.ts add nested TS type**

```ts
export interface LibrarySettings { auto_sync_playlist_folder_to_roots: boolean; }
interface AppSettings { /* ... existing ... */ library: LibrarySettings; }
```

- [ ] **Step 3: npm test vitest run** → green

- [ ] **Step 4: Commit**
```bash
git add phonon-tauri/src/api/library.ts phonon-tauri/src/api/settings.ts
git commit -m "feat(A): add fully-typed TS wrappers for 6 library_* commands + LibrarySettings nested type"
```

---

### Task A8: UI layer — Top-level Tab + LibraryPage router skeleton + EmptyStateGuide

**Files:**
- Modify: `phonon-tauri/src/App.tsx` (add 📚 媒体库 Tab)
- **Create:** `phonon-tauri/src/components/LibraryPage.tsx`
- **Create:** `phonon-tauri/src/components/library/EmptyStateGuide.tsx` (later embedded as Empty child view)

- [ ] **Step 1: Add Tab to App.tsx**

Use existing TabBar component pattern — find where 播放器 / 歌词 / 扩展 / 可视化 tabs are defined, insert 媒体库 between 播放器 and 歌词 or last, depending on existing order. Do NOT change order of existing tabs; only append/new insert.

- [ ] **Step 2: Create LibraryPage.tsx skeleton**

Implement SubTab router (6 sub-tabs), top tool bar with search box + filter + 3-dot menu + `N 首 / M 专辑 / K 艺术家` stats in corner. Stack navigation state: `const [detailStack, setDetailStack] = useState<DetailStackEntry[]>([]);` with push({kind: 'album', id:123}) + pop(). When detailStack.length > 0, hide the SubTabs and show a back button + details page component. Error boundary wrapper at top of component tree catches React error boundaries from any child → toast + click to restart render.

- [ ] **Step 3: EmptyStateGuide component**

Matches spec §5.9 ASCII card design. Two primary buttons: 「⚙ 添加音乐目录…」→ library.setRoots dialog + scan; 「📁 先扫描一个文件夹试试」→ quick pick folder → temporary root + scan + scan progress bar under card.

- [ ] **Step 4: Visual smoke test**

```bash
cd d:\Phonon\phonon-tauri
npm run dev
```
Expected: Tab bar shows 📚 媒体库; click empty tab, EmptyStateGuide renders.

- [ ] **Step 5: Commit**
```bash
git add phonon-tauri/src/App.tsx phonon-tauri/src/components/LibraryPage.tsx phonon-tauri/src/components/library/EmptyStateGuide.tsx
git commit -m "feat(A): add 📚 媒体库 top Tab + LibraryPage skeleton router + 6 sub-tabs + EmptyStateGuide"
```

---

### Task A9: UI TrackTable 9 列虚拟列表 + 懒加载缩略图（IntersectionObserver）

**Files:**
- **Create:** `phonon-tauri/src/components/library/TrackTable.tsx`

- [ ] **Step 1: Build 9 columns**

Implement 9 columns (per §5.3 numbered list #1-9):
1. Cover 64×64 (lazy IntersectionObserver ref)
2. Track #
3. 标题
4. 艺术家
5. 专辑
6. 时长 mm:ss
7. 格式（FLAC|MP3|AAC|… badge colored by format lossy/lossless）
8. ★ favorite (clickable inline — optimistic update)
9. ⭐ rating 0-5 stars (clickable inline)

Recent sub-tab: swap column 8 to show Last Played relative time.

**Virtual list** (≥ 10k rows auto-enable):
- Implement: `overScanRows = Math.ceil(containerHeight / 72) * 3` (3 viewport buffers, 72px fixed row height)
- Top/bottom spacer divs + only render rows `max(0, firstIdx - overScan) : min(total, lastIdx + overScan)`
- Subscribe to onScroll via throttled state updates
- `< 10k rows: just render all rows as simple DOM (no spacer divs)`

**9 项右键菜单**: play / add to queue / add to playlist / ★ favorite / ⭐ rating submenu / edit metadata / 更换封面 / open file location / rescan metadata. Implement as native ContextMenu via native Tauri popup or simple React portal floating div matching existing theme.

**正在播放高亮**: when row.track_id === currently playing track → whole row color accent.

- [ ] **Step 2: test locally** with 15,000 fake rows — confirm 60 fps scroll (no jank)

- [ ] **Step 3: Commit**
```bash
git add phonon-tauri/src/components/library/TrackTable.tsx
git commit -m "feat(A): TrackTable 9-col virtual list (≥10k auto-enabled, 72px row, 3-viewport buffer) + IntersectionObserver lazy covers + inline ★/⭐ + 9-item context menu + playing-highlight row"
```

---

### Task A10: UI AlbumGrid + AlbumDetailPage + ArtistGenreList + SearchSuggestDropdown

**Files:**
- **Create:** `phonon-tauri/src/components/library/AlbumGrid.tsx`
- **Create:** `phonon-tauri/src/components/library/AlbumDetailPage.tsx`
- **Create:** `phonon-tauri/src/components/library/ArtistGenreList.tsx`
- **Create:** `phonon-tauri/src/components/library/SearchSuggestDropdown.tsx`

- [ ] **Step 1: AlbumGrid**

160×160px card CSS grid (auto-fill). Each card: top cover (lazy) + overlay ▶ play button on hover, bottom 2 lines: album name / album artist / year. onClick → push detailStack → AlbumDetailPage.

- [ ] **Step 2: AlbumDetailPage (Apple Music style)**

Top 300px square cover on left, right column: album name (xxl bold), artist link (→ push Artist detail), year, track_count × total duration as hh:mm:ss, genre. Button row: [▶ 播放专辑] → replace queue with all tracks of album, start at track 1; [+ 加入队列]; [✎ 编辑专辑…] → AlbumEditModal. Below: full TrackTable of this album's tracks (reuse A9 component with album_id filter prop).

- [ ] **Step 3: ArtistGenreList (首字母分组)**

Backend returns rows ordered by sort_key. Group front-end by first char: if first char ASCII letter → uppercase A-Z sticky header; if non-ASCII (# prefix bucket for all non A-Z). Click artist → push detail stack (reuse AlbumDetailPage structure but show all albums of that artist as Grid first, then tracks below). Genre identical structure — click genre → tracks filtered by genre table.

- [ ] **Step 4: SearchSuggestDropdown**

300ms debounced input onChange. Show floating 3 section dropdown: 5 tracks / 3 albums / 3 artists. Navigate keyboard ↑↓ and Enter to jump. Click an album/artist suggestion → push detail stack. Query empty or ≤1 char → dropdown hidden (min 2 chars to trigger search).

- [ ] **Step 5: Commit**
```bash
git add phonon-tauri/src/components/library/AlbumGrid.tsx phonon-tauri/src/components/library/AlbumDetailPage.tsx phonon-tauri/src/components/library/ArtistGenreList.tsx phonon-tauri/src/components/library/SearchSuggestDropdown.tsx
git commit -m "feat(A): AlbumGrid 160px cards + AlbumDetailPage (Apple-Music 300px cover) + ArtistGenreList A-Z sticky groups by sort_key + SearchSuggestDropdown 3-section 300ms debounce"
```

---

### Task A11: Modals — MetadataEditModal + AlbumEditModal + CropEditor.tsx rewrite INTERNAL ONLY (keep same export interface)

**Files:**
- **Create:** `phonon-tauri/src/components/library/MetadataEditModal.tsx`
- **Create:** `phonon-tauri/src/components/library/AlbumEditModal.tsx`
- **Modify (重写内部):** `phonon-tauri/src/components/CropEditor.tsx`

- [ ] **Step 1: MetadataEditModal**

Multi-row form for all 12 metadata fields (title, artist, album_artist, album, genre, composer, year, track #, disc #, rating 0-5 stars, cover preview + [更换封面] button that opens CropEditor). Bottom: [取消] [保存]. Save → calls library_edit_metadata → optimistic update replaces row immediately in TrackTable. Toast differentiates:
- Success: ✓ 已保存到媒体库并写回文件
- Write-back partial failure (stub returns NotImpl): ⚠ 已保存到媒体库，文件回写将在 RC2 支持 + [稍后重试写回] button that calls `library_write_metadata_back_to_file`.

- [ ] **Step 2: AlbumEditModal** — bulk album fields + album cover update + artist change. Calls edit_metadata in batch for N tracks.

- [ ] **Step 3: REWRITE CropEditor.tsx (preserve same props `{ source, onConfirm(dataUrl), onCancel }` — DO NOT CHANGE external callers)**

Paste entire internal body per spec §5.7 (displayBox 400×400, cropBox 320×320 FIXED, user drags IMAGE not the crop box, scale slider 50-300%, boundary clamp math, sx/sy/sSize drawImage formula, THE BIG COMMENT BLOCK AT TOP WITH offsetX/Y 符号约定 preserved verbatim). The existing drag-crop-box handles logic is REMOVED completely — replaced with drag-image + slider UX. External `CropEditor` callers (Today the CropEditor may be used by apperance set background) MUST not need changes — keep same props.

- [ ] **Step 4: Commit**
```bash
git add phonon-tauri/src/components/library/MetadataEditModal.tsx phonon-tauri/src/components/library/AlbumEditModal.tsx phonon-tauri/src/components/CropEditor.tsx
git commit -m "feat(A): MetadataEditModal (bulk edit + result toast with retry) + AlbumEditModal + CropEditor.tsx INTERNAL rewrite: fixed cropBox + drag-image + 50%-300% slider + boundary clamp + explicit offsetX/Y sign convention code comment"
```

---

### Task A12: Entry Consistency (5.8 all three sections: sync folder hook with opt-out; add-roots modal ask to make playlist; Settings 媒体库 card)

**Files:**
- Modify: `phonon-tauri/src/components/PlaylistToolbar.tsx` (syncFolder / importFolder)
- Modify: `phonon-tauri/src/components/Settings.tsx` (媒体库 card + opt-out switch + migration progress bar)

- [ ] **Step 1: PlaylistToolbar syncFolder() — 5.8.1 hook with opt-out read**

At existing syncFolder() success callback tail (L247-288 line references from today's spec), insert EXACT code from spec §5.8.1 including: first `settings.library?.auto_sync_playlist_folder_to_roots` check; if false → toast only; if true → getRoots → dedup → setRoots → kick scan.

Then add import folder fast path: `if (ancestors of someParentDir in currentRoots) library.getTracksByPathPrefix(...) → return with full metadata (no placeholder QueueItems)`.

- [ ] **Step 2: Library add-roots dialog → 5.8.2 "要不要同时创建同步歌单？"**

At LibraryPage "添加音乐目录" completion (from EmptyState or 3-dot menu): show modal matching the exact copy from spec with two buttons [不，谢谢] [✓ 创建同步歌单]. If yes create playlist named basename of folder → setFolderSync → switch to player tab and activate it.

- [ ] **Step 3: Settings.tsx replace 媒体库配置 with state card + opt-out switch**

Simplify the existing 「媒体库配置」 SubTab section (today's Settings UI) per §5.8.3: pure status display. Show: N roots listed with icons, [管理媒体库 →] button that switches to LibraryPage and opens 3-dot menu, [立即扫描 →] button. Add a native switch at bottom for `library.auto_sync_playlist_folder_to_roots` with the exact hint copy from spec §5.8.1.

- [ ] **Step 4: Commit**
```bash
git add phonon-tauri/src/components/PlaylistToolbar.tsx phonon-tauri/src/components/Settings.tsx phonon-tauri/src/components/LibraryPage.tsx
git commit -m "feat(A): Entry Consistency bidirectional sync: PlaylistToolbar syncFolder → auto-add roots (with opt-out) + import fast path; Library add-roots modal asks to create auto-sync playlist; Settings → Audio simplifies to state card + auto-sync switch + migration progress bar"
```

---

### Task A13: 4 类 Resilience mechanisms (§5.10 full)

**Files:**
- Modify: `phonon-tauri/src/components/library/TrackTable.tsx` (cover retry)
- Modify: `phonon-tauri/src/api/library.ts` + library SearchSuggest (AbortController timeout)
- Already covered: A6 (corrupt DB backup) + A11 toast with retry (write-back failure) → but fill remaining #1 #2.

- [ ] **Step 1: Cover load failures exponential backoff (#1)**

Track images fail → setTimeout 300ms → 900ms → 2700ms. If 3 retries → permanent 🎵 placeholder; console.log WARN single line, no global throw.

- [ ] **Step 2: Search timeout + auto-cancel (#2)**

SearchSuggest if server response > 2000ms → spinner dim + 显示「搜索超时，请重试」button. >5000ms use AbortController to cancel fetch.

- [ ] **Step 3: Documented self-review.** Confirm A6 corruption red banner, A11 write-back retry toast, are already from earlier tasks.

- [ ] **Step 4: Create final 14-case e2e runbook Playwright spec (matches §8.3)**

Create `phonon-tauri/e2e/library-9-col.spec.ts` with 14 manual cases matching spec §8.3 checkboxes 1-14. Cases include virtual list scroll 60fps, search suggest, album detail play, metadata edit optimist UI, cover crop with slider/offset math, sort_key 周杰伦 under Z grouping, auto-sync playlist folder roots, opt-out closes → no scan.

- [ ] **Step 5: Commit all resilience + e2e runbook**
```bash
git add phonon-tauri/src/components/library/TrackTable.tsx phonon-tauri/src/components/library/SearchSuggestDropdown.tsx phonon-tauri/e2e/library-9-col.spec.ts
git commit -m "feat(A): §5.10 resilience — #1 cover 3x exponential retry → permanent placeholder; #2 search >2s timeout hint >5s AbortController; + e2e 14-case §8.3 acceptance runbook"
```

---

## Self-Review Checklist (Spec coverage final)

- [ ] §5.1: 6 sub tabs ✅ — LibraryPage router (A8)
- [ ] §5.2: toolbar layout ✅ — search / filter / 3-dot menu / stats (A8)
- [ ] §5.3: 9列 table + #1..#9 numbered ✅ — TrackTable A9. Virtual list ≥ 10k auto, fixed 72px row height + 3 viewport buffers ✅. 9 项右键菜单 ✅.
- [ ] §5.4: Search ✅ — 300ms debounce suggest dropdown (A10) + FTS5 backend (A4). 2000ms spinner dim timeout + 5000ms AbortController (A13.2)
- [ ] §5.5: Backend 首字母分组 ✅ — sort_key compute pinyin in phonon-media (A2); schema artist/album sort_key col + idx (A1); v0→v2 PRAGMA migration (A1); background 500/batch backfill with progress events (A2); front-end ArtistGenreList A-Z sticky headers (A10)
- [ ] §5.6: Metadata edit ✅ — edit_metadata (A4) always-write-db first never rollback file; Modal toast differentiates (A11); Modal write-back retry button (A11/A4); RC stub for write_metadata_to_file (A3)
- [ ] §5.7: Canvas 零依赖裁剪 ✅ — CropEditor rewrite internal (A11) keep prop interface; displayBox/cropBox 400/320; offsetX/Y code comments for sign convention; fitRatio formula; boundary clamp math; 512×512 JPEG 88% drawImage
- [ ] §5.8: Entry Consistency three items ✅ — 5.8.1 syncFolder hook with opt-out (A12); 5.8.2 add-roots ask create playlist (A12); 5.8.3 Settings simplify card + opt-out switch + progress bar (A12)
- [ ] §5.9: Empty state guide ✅ — A8
- [ ] §5.10: 4 Resilience items ✅ — A6 corrupt backup; A13.1 cover retry; A13.2 search timeout; A11 write-back retry toast
- [ ] §5.11: 10 new component files ✅ — all in A8..A11
- [ ] §6: All AppSettings sub-structs ✅ — LibrarySettings (A6) + Dsp (C) + Hotplug (B) all nested with serde default
- [ ] §7: 6 new commands ✅ — A4 (library logic) + A5 (command register) + A7 (typed TS wrapper)
- [ ] §8.3: 14 Acceptance boxes → covered by e2e/library-9-col.spec.ts

---

## Final Plan: RC Build Packaging node (run after A / B / C all complete)

One-time task outside the 3 plans:
- Bump version in package.json / Cargo.toml to "0.9.0-rc1"
- `cd phonon-tauri ; npm run tauri build` — produce `target/release/bundle/msi/Phonon_0.9.0-rc1_x64_en-US.msi` (unsigned; no code signing cert needed per user's explicit requirement)
- Copy MSI out to `d:\Phonon\dist\` folder and rename with timestamp. Test fresh install in a Windows Sandbox: install → open → add folder → confirm 9-col list renders, album page works, device pull/pause works, speed 3-mode switch works, crop editor slider works.
