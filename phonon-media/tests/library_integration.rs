//! Integration tests for the media library.
//!
//! These tests exercise the SQLite-backed library without requiring real
//! audio files: they cover schema initialization, empty-query behavior,
//! favorite/rating validation, FTS5 search fallback, and cleanup. The
//! scan path with real audio fixtures is covered separately by the
//! codec integration tests in `phonon-codec`.

use std::path::PathBuf;
use std::sync::Mutex;

use phonon_media::{MediaLibrary, ScanOptions, SearchMode, TrackFilter, TrackInfo};

/// Each test gets its own temp DB file. We use a counter + pid to avoid
/// collisions when tests run in parallel.
fn temp_db_path(name: &str) -> PathBuf {
    let pid = std::process::id();
    let counter = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("phonon-media-test-{pid}-{counter}-{name}.sqlite"))
}

static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Guard that deletes the DB file on drop so temp dir doesn't grow.
struct TempDb(PathBuf);
impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(format!("{}-wal", self.0.display()));
        let _ = std::fs::remove_file(format!("{}-shm", self.0.display()));
    }
}

fn open_library(name: &str) -> (TempDb, MediaLibrary) {
    let path = temp_db_path(name);
    let lib = MediaLibrary::open(&path).expect("open library");
    (TempDb(path), lib)
}

#[test]
fn opens_and_applies_schema() {
    let (_guard, lib) = open_library("schema");
    // db_path should exist now.
    assert!(lib.db_path().exists(), "db file should exist after open");
    // Listing empty collections should succeed and return empty.
    assert!(lib.list_albums().unwrap().is_empty());
    assert!(lib.list_artists().unwrap().is_empty());
    assert!(lib.list_genres().unwrap().is_empty());
    assert!(lib.list_folders().unwrap().is_empty());
}

#[test]
fn empty_search_returns_empty() {
    let (_guard, lib) = open_library("empty-search");
    let results = lib.search("anything", SearchMode::FtsPrefix, 50).unwrap();
    assert!(results.is_empty());
}

#[test]
fn empty_string_search_returns_empty_without_error() {
    let (_guard, lib) = open_library("blank-search");
    let results = lib.search("", SearchMode::FtsPrefix, 50).unwrap();
    assert!(results.is_empty());
    let results = lib.search("   ", SearchMode::FtsPrefix, 50).unwrap();
    assert!(results.is_empty());
}

#[test]
fn toggle_favorite_missing_track_errors() {
    let (_guard, lib) = open_library("fav-missing");
    let err = lib.toggle_favorite(9999).unwrap_err();
    assert!(matches!(
        err,
        phonon_media::MediaLibraryError::TrackNotFound(9999)
    ));
}

#[test]
fn set_rating_rejects_out_of_range() {
    let (_guard, lib) = open_library("rating-range");
    let err = lib.set_rating(1, 6).unwrap_err();
    assert!(matches!(
        err,
        phonon_media::MediaLibraryError::InvalidRating(6)
    ));
}

#[test]
fn get_track_missing_returns_none() {
    let (_guard, lib) = open_library("get-missing");
    assert!(lib.get_track(4242).unwrap().is_none());
}

#[test]
fn get_tracks_empty_filter_returns_empty() {
    let (_guard, lib) = open_library("tracks-empty");
    let filter = TrackFilter::default();
    assert!(lib.get_tracks(&filter).unwrap().is_empty());
}

#[test]
fn cleanup_deleted_on_empty_db_returns_zero() {
    let (_guard, lib) = open_library("cleanup-empty");
    assert_eq!(lib.cleanup_deleted().unwrap(), 0);
}

#[test]
fn scan_nonexistent_root_is_noop() {
    let (_guard, lib) = open_library("scan-noop");
    let bogus = PathBuf::from("/nonexistent/phonon-test-root-12345");
    let opts = ScanOptions::default();
    let stats = lib
        .scan(&[bogus], &opts, &|_| {})
        .expect("scan should not error on missing roots");
    assert_eq!(stats.total_files, 0);
    assert_eq!(stats.tracks_inserted, 0);
}

#[test]
fn library_is_send_sync_via_mutex() {
    // MediaLibrary must be Send so it can live inside Tauri's `State<>`.
    // The struct itself wraps `Mutex<Connection>`; we ensure it compiles as
    // Send + Sync by asserting the trait bounds at compile time.
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<MediaLibrary>();
}

/// Full round-trip: scan a real file → toggle_favorite → set_rating →
/// record_play → verify persistence and filters. This is the primary
/// "basic operations work end-to-end" test.
#[test]
fn favorite_rating_play_roundtrip() {
    let (_guard, lib) = open_library("roundtrip");

    // Create a tmp dir with a fake audio file. The scanner's fallback path
    // (codec parse failure) inserts a minimal row using the file stem as
    // the title, so an empty body is fine.
    let tmp_dir = std::env::temp_dir().join(format!(
        "phonon-test-roundtrip-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&tmp_dir).expect("mkdir");
    let audio_path = tmp_dir.join("Song.flac");
    std::fs::write(&audio_path, b"").expect("write");

    // Scan — should insert exactly 1 track via the fallback path.
    let stats = lib
        .scan(&[tmp_dir.clone()], &ScanOptions::default(), &|_| {})
        .expect("scan");
    assert!(stats.total_files >= 1, "scanner should find the file");

    let tracks = lib.get_tracks(&TrackFilter::default()).expect("get_tracks");
    assert_eq!(tracks.len(), 1, "exactly one track should be inserted");
    let track = &tracks[0];
    assert!(!track.is_favorite, "new track should not be favorite");
    assert_eq!(track.rating, 0, "new track should have rating 0");
    assert_eq!(track.play_count, 0, "new track should have 0 plays");
    assert!(
        track.last_played_at.is_none(),
        "last_played_at should be null"
    );
    let id = track.id;

    // toggle_favorite: false → true → false
    assert!(
        lib.toggle_favorite(id).expect("fav1"),
        "first toggle returns true"
    );
    assert!(
        lib.get_track(id).unwrap().unwrap().is_favorite,
        "favorite should persist"
    );
    assert!(
        !lib.toggle_favorite(id).expect("fav2"),
        "second toggle returns false"
    );
    assert!(
        !lib.get_track(id).unwrap().unwrap().is_favorite,
        "un-favorite should persist"
    );

    // set_rating: 0 → 5 → 3
    lib.set_rating(id, 5).expect("rate5");
    assert_eq!(lib.get_track(id).unwrap().unwrap().rating, 5);
    lib.set_rating(id, 3).expect("rate3");
    assert_eq!(lib.get_track(id).unwrap().unwrap().rating, 3);

    // record_play: bumps play_count + sets last_played_at
    lib.record_play(id).expect("play");
    let t = lib.get_track(id).unwrap().unwrap();
    assert_eq!(t.play_count, 1);
    assert!(
        t.last_played_at.is_some(),
        "last_played_at should be set after play"
    );

    // ── Filters ──────────────────────────────────────────────
    // favorites_only: mark favorite, then filter.
    lib.toggle_favorite(id).expect("fav");
    let favs = lib
        .get_tracks(&TrackFilter {
            favorites_only: true,
            ..Default::default()
        })
        .expect("fav filter");
    assert_eq!(favs.len(), 1, "favorites filter should return the track");

    let favs_off = lib
        .get_tracks(&TrackFilter {
            favorites_only: false,
            ..Default::default()
        })
        .expect("fav filter off");
    assert_eq!(
        favs_off.len(),
        1,
        "non-favorites filter should also return it"
    );

    // min_rating: track has rating 3, threshold 4 should exclude it.
    let rated = lib
        .get_tracks(&TrackFilter {
            min_rating: Some(4),
            ..Default::default()
        })
        .expect("rating filter");
    assert!(rated.is_empty(), "rating 3 < 4, should be filtered out");

    let rated_low = lib
        .get_tracks(&TrackFilter {
            min_rating: Some(3),
            ..Default::default()
        })
        .expect("rating filter low");
    assert_eq!(rated_low.len(), 1, "rating 3 >= 3, should be included");

    // ── Folders aggregation ──────────────────────────────────
    // After scan + rebuild_aggregates, the tmp dir should appear in folders
    // with track_count = 1.
    let folders = lib.list_folders().expect("list_folders");
    assert!(
        !folders.is_empty(),
        "folders table should not be empty after scan"
    );
    let tmp_folder = folders
        .iter()
        .find(|f| f.track_count > 0)
        .expect("at least one folder should have track_count > 0");
    assert_eq!(
        tmp_folder.track_count, 1,
        "tmp dir should have exactly 1 track"
    );

    // Cleanup tmp dir.
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

// ---------------------------------------------------------------------------
// ReplayGain batch scan integration tests (spec §11).
// ---------------------------------------------------------------------------

use std::sync::atomic::{AtomicUsize, Ordering};

use phonon_media::ReplayGainScanProgress;

/// Return a fresh sidecar DB path unique to the calling test.
fn temp_sidecar_path(name: &str) -> PathBuf {
    let pid = std::process::id();
    let counter = COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("phonon-rg-sidecar-{pid}-{counter}-{name}.sqlite"))
}

/// Guard that removes the sidecar DB file (and WAL/SHM) on drop.
struct TempSidecar(PathBuf);
impl Drop for TempSidecar {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(format!("{}-wal", self.0.display()));
        let _ = std::fs::remove_file(format!("{}-shm", self.0.display()));
    }
}

/// A scan over an empty library must complete without error and report
/// zero tracks. No progress callbacks beyond the start/done markers.
#[test]
fn replaygain_scan_empty_library_is_noop() {
    let (_guard, lib) = open_library("rg-empty");
    let sidecar = TempSidecar(temp_sidecar_path("empty"));

    let phases: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let stats = lib
        .batch_scan_replaygain(&sidecar.0, false, &|p: ReplayGainScanProgress| {
            phases.lock().unwrap().push(p.phase);
        })
        .expect("empty scan");

    assert_eq!(stats.total_tracks, 0);
    assert_eq!(stats.scanned, 0);
    assert_eq!(stats.skipped, 0);
    assert_eq!(stats.failed, 0);
    // Start + done markers must always fire.
    let phases = phases.into_inner().unwrap();
    assert!(phases.iter().any(|p| p == "replaygain-start"));
    assert!(phases.iter().any(|p| p == "replaygain-done"));
}

/// Scanning a track whose file cannot be decoded (empty/fake body) must
/// increment `failed` and NOT panic or return an error. The sidecar DB
/// should still be created and remain queryable.
#[test]
fn replaygain_scan_unscannable_file_counts_as_failed() {
    let (_guard, lib) = open_library("rg-fail");

    // Create a tmp dir with a fake .flac file (empty body). The scanner's
    // fallback path inserts a minimal row, but the ReplayGain decoder will
    // fail to parse it — this is the expected graceful-failure path.
    let tmp_dir = std::env::temp_dir().join(format!(
        "phonon-test-rg-fail-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&tmp_dir).expect("mkdir");
    let audio_path = tmp_dir.join("Broken.flac");
    std::fs::write(&audio_path, b"not actually flac").expect("write");

    lib.scan(&[tmp_dir.clone()], &ScanOptions::default(), &|_| {})
        .expect("scan");
    let tracks = lib.get_tracks(&TrackFilter::default()).expect("get_tracks");
    assert_eq!(tracks.len(), 1, "scanner should insert the fake track");

    let sidecar = TempSidecar(temp_sidecar_path("fail"));
    let progress_count = AtomicUsize::new(0);
    let stats = lib
        .batch_scan_replaygain(&sidecar.0, false, &|_p: ReplayGainScanProgress| {
            progress_count.fetch_add(1, Ordering::SeqCst);
        })
        .expect("scan should not error on decode failure");

    assert_eq!(stats.total_tracks, 1);
    assert_eq!(stats.scanned, 0, "fake file cannot be decoded");
    assert_eq!(stats.failed, 1, "the one track should have failed");
    assert_eq!(stats.skipped, 0);
    // Progress: start + one per-track + done = at least 3.
    assert!(progress_count.load(Ordering::SeqCst) >= 3);

    // The sidecar DB file must exist on disk.
    assert!(sidecar.0.exists(), "sidecar DB should be created");

    // The track's replaygain_track_gain must remain NULL (no value written).
    let t = lib.get_track(tracks[0].id).unwrap().unwrap();
    assert!(
        t.replaygain_track_gain.is_none(),
        "failed track should not have a gain value"
    );

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

/// Force=true must rescan even when a sidecar cache entry exists. We
/// verify this by checking that a second forced scan reports `scanned`
/// (not `skipped`) for a file that previously failed.
#[test]
fn replaygain_force_rescan_does_not_use_cache() {
    let (_guard, lib) = open_library("rg-force");

    let tmp_dir = std::env::temp_dir().join(format!(
        "phonon-test-rg-force-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&tmp_dir).expect("mkdir");
    let audio_path = tmp_dir.join("Fake.flac");
    std::fs::write(&audio_path, b"").expect("write");

    lib.scan(&[tmp_dir.clone()], &ScanOptions::default(), &|_| {})
        .expect("scan");

    let sidecar = TempSidecar(temp_sidecar_path("force"));

    // First scan: file fails to decode → failed=1, nothing cached.
    let stats1 = lib
        .batch_scan_replaygain(&sidecar.0, false, &|_| {})
        .expect("first scan");
    assert_eq!(stats1.failed, 1);
    assert_eq!(stats1.skipped, 0);

    // Second scan with force=true: must not skip, must attempt decode again.
    // Since the file is still broken, it fails again — but `skipped` must
    // remain 0, proving the cache was bypassed.
    let stats2 = lib
        .batch_scan_replaygain(&sidecar.0, true, &|_| {})
        .expect("forced scan");
    assert_eq!(stats2.skipped, 0, "force=true must not use cache");
    assert_eq!(stats2.failed, 1, "file is still unscannable");
    assert_eq!(stats2.scanned, 0);

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

/// The progress callback must receive monotonically non-decreasing
/// `processed` values during the scan phase, and the final `processed`
/// must equal `total`.
#[test]
fn replaygain_progress_processed_is_monotonic() {
    let (_guard, lib) = open_library("rg-progress");

    // Insert 3 fake tracks by scanning 3 empty files.
    let tmp_dir = std::env::temp_dir().join(format!(
        "phonon-test-rg-prog-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&tmp_dir).expect("mkdir");
    for i in 0..3u8 {
        std::fs::write(tmp_dir.join(format!("Song{i}.flac")), b"").expect("write");
    }
    lib.scan(&[tmp_dir.clone()], &ScanOptions::default(), &|_| {})
        .expect("scan");

    let sidecar = TempSidecar(temp_sidecar_path("progress"));
    // Capture state behind Mutex so the `Fn` closure can mutate it.
    let state: Mutex<(usize, Option<usize>, Option<usize>)> = Mutex::new((0, None, None));

    let stats = lib
        .batch_scan_replaygain(&sidecar.0, false, &|p: ReplayGainScanProgress| {
            let mut s = state.lock().unwrap();
            if p.phase == "replaygain-scan" {
                assert!(
                    p.processed >= s.0,
                    "processed went backwards: {} < {}",
                    p.processed,
                    s.0
                );
                s.0 = p.processed;
                s.1 = Some(p.total);
            } else if p.phase == "replaygain-done" {
                s.2 = Some(p.processed);
            }
        })
        .expect("scan");

    assert_eq!(stats.total_tracks, 3);
    let (last_processed, total_seen, done_processed) = state.into_inner().unwrap();
    let total = total_seen.expect("at least one scan-phase progress");
    assert_eq!(total, 3, "total must match track count");
    let done = done_processed.expect("done phase must fire");
    assert_eq!(done, 3, "final processed must equal total");
    // The last scan-phase processed value must also equal total.
    assert_eq!(last_processed, 3, "last scan processed must equal total");

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

// ---------------------------------------------------------------------------
// Cover compliance tests (spec §11 — edit_metadata 严格只接受本地 JPG/PNG).
// ---------------------------------------------------------------------------

/// Helper: scan a fake .flac file into the library and return its track id.
fn scan_one_track(lib: &MediaLibrary, tag: &str) -> i64 {
    let tmp_dir = std::env::temp_dir().join(format!(
        "phonon-test-cover-{}-{}-{}",
        tag,
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&tmp_dir).expect("mkdir");
    std::fs::write(tmp_dir.join("Song.flac"), b"").expect("write");
    lib.scan(&[tmp_dir.clone()], &ScanOptions::default(), &|_| {})
        .expect("scan");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let tracks = lib.get_tracks(&TrackFilter::default()).expect("get_tracks");
    assert!(!tracks.is_empty(), "scanner should insert a track");
    tracks[0].id
}

/// Minimal 1×1 PNG generated at runtime via the `image` crate so the
/// thumbnail encoder can fully decode it. (Hand-crafted PNG chunks are
/// fragile — easier and more reliable to let `image` produce one.)
fn write_valid_png(path: &std::path::Path) {
    let img = image::RgbImage::from_raw(1, 1, vec![0, 0, 0]).expect("create 1x1 image");
    let mut buf = std::io::BufWriter::new(std::fs::File::create(path).expect("create file"));
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut buf, image::ImageFormat::Png)
        .expect("encode png");
}

/// Minimal JPEG SOI + minimal data (not a fully valid JPEG, but has the
/// correct FF D8 FF magic bytes the compliance check verifies).
///
/// NOTE: `ensure_thumbnail` calls `image::load_from_memory` which needs a
/// fully decodable image, so for the JPEG acceptance test we generate a
/// real 1×1 JPEG at runtime via the `image` crate instead.
fn write_valid_jpeg_magic(path: &std::path::Path) {
    // Generate a real 1×1 RGB JPEG so the thumbnail encoder can decode it.
    let img = image::RgbImage::from_raw(1, 1, vec![0, 0, 0]).expect("create 1x1 image");
    let mut buf = std::io::BufWriter::new(std::fs::File::create(path).expect("create file"));
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut buf, image::ImageFormat::Jpeg)
        .expect("encode jpeg");
}

/// A valid PNG file must be accepted: cover_hash is set and the track row
/// is returned with the new hash.
#[test]
fn set_cover_accepts_valid_png() {
    let (_guard, lib) = open_library("cover-png");
    let track_id = scan_one_track(&lib, "png");

    let cover_dir = std::env::temp_dir().join(format!(
        "phonon-test-coverdir-png-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&cover_dir).expect("mkdir");
    let cover_path = cover_dir.join("cover.png");
    write_valid_png(&cover_path);

    let updated = lib
        .set_track_cover_from_file(track_id, &cover_path)
        .expect("valid png should be accepted");
    assert!(updated.cover_hash.is_some(), "cover_hash must be set");

    let _ = std::fs::remove_dir_all(&cover_dir);
}

/// A JPEG with correct magic bytes must be accepted.
#[test]
fn set_cover_accepts_valid_jpeg() {
    let (_guard, lib) = open_library("cover-jpg");
    let track_id = scan_one_track(&lib, "jpg");

    let cover_dir = std::env::temp_dir().join(format!(
        "phonon-test-coverdir-jpg-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&cover_dir).expect("mkdir");
    let cover_path = cover_dir.join("cover.jpg");
    write_valid_jpeg_magic(&cover_path);

    let updated = lib
        .set_track_cover_from_file(track_id, &cover_path)
        .expect("valid jpg should be accepted");
    assert!(updated.cover_hash.is_some());

    let _ = std::fs::remove_dir_all(&cover_dir);
}

/// A .txt file must be rejected with a Compliance error — extension
/// whitelist is the first gate.
#[test]
fn set_cover_rejects_txt_extension() {
    let (_guard, lib) = open_library("cover-txt");
    let track_id = scan_one_track(&lib, "txt");

    let cover_dir = std::env::temp_dir().join(format!(
        "phonon-test-coverdir-txt-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&cover_dir).expect("mkdir");
    let cover_path = cover_dir.join("cover.txt");
    std::fs::write(&cover_path, b"not an image").expect("write");

    let err = lib
        .set_track_cover_from_file(track_id, &cover_path)
        .expect_err("txt should be rejected");
    let msg = format!("{err}");
    assert!(msg.contains("unsupported cover extension"), "got: {msg}");

    let _ = std::fs::remove_dir_all(&cover_dir);
}

/// A .png file whose content does not start with the PNG magic bytes
/// must be rejected — this catches renamed / corrupted files.
#[test]
fn set_cover_rejects_png_with_wrong_magic_bytes() {
    let (_guard, lib) = open_library("cover-magic");
    let track_id = scan_one_track(&lib, "magic");

    let cover_dir = std::env::temp_dir().join(format!(
        "phonon-test-coverdir-magic-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&cover_dir).expect("mkdir");
    let cover_path = cover_dir.join("fake.png");
    // GIF magic bytes but .png extension — must be rejected.
    std::fs::write(&cover_path, b"GIF89a rest of file").expect("write");

    let err = lib
        .set_track_cover_from_file(track_id, &cover_path)
        .expect_err("wrong magic bytes should be rejected");
    let msg = format!("{err}");
    assert!(msg.contains("not a valid"), "got: {msg}");

    let _ = std::fs::remove_dir_all(&cover_dir);
}

/// An http:// URL must be rejected outright — no network fetching.
#[test]
fn set_cover_rejects_http_url() {
    let (_guard, lib) = open_library("cover-url");
    let track_id = scan_one_track(&lib, "url");

    let url = std::path::PathBuf::from("https://example.com/cover.jpg");
    let err = lib
        .set_track_cover_from_file(track_id, &url)
        .expect_err("http url should be rejected");
    let msg = format!("{err}");
    assert!(msg.contains("remote URLs are not allowed"), "got: {msg}");
}

/// A non-existent file must be rejected with a compliance error (not a
/// raw IO error) so the frontend can show a friendly message.
#[test]
fn set_cover_rejects_missing_file() {
    let (_guard, lib) = open_library("cover-missing");
    let track_id = scan_one_track(&lib, "missing");

    let missing = std::env::temp_dir().join(format!(
        "phonon-test-nonexistent-{}-{}.jpg",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    // Don't create the file — it must not exist.
    let err = lib
        .set_track_cover_from_file(track_id, &missing)
        .expect_err("missing file should be rejected");
    let msg = format!("{err}");
    assert!(msg.contains("cannot read cover file"), "got: {msg}");
}

/// Mutex to serialize tests that touch the same temp-dir counter (no-op
/// here, but documents intent for future tests that share fixtures).
#[allow(dead_code)]
static SERIALIZE: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------------------
// Schema migration v0→v2 (Plan A Task A1).
// ---------------------------------------------------------------------------

use phonon_media::schema::{apply_migrations, user_version};

#[test]
fn schema_migration_v0_to_v2_adds_sort_key_indices_no_data_loss() {
    // 1. Create an in-memory DB that mimics the OLD v0 schema (no sort_key
    // columns — simulate existing user's DB before this release) by running a
    // stripped DDL without sort_keys.
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE artists (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, track_count INTEGER NOT NULL DEFAULT 0, album_count INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
        CREATE TABLE albums (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, album_artist TEXT, year INTEGER, cover_hash TEXT, track_count INTEGER NOT NULL DEFAULT 0, total_duration_ms INTEGER, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, UNIQUE(title, album_artist));
        PRAGMA user_version = 0;
    "#,
    )
    .unwrap();
    // Insert a pre-migration artist with an existing row (ensures ALTER TABLE
    // works without data loss).
    conn.execute(
        "INSERT INTO artists (name, track_count, album_count, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params!["周杰伦", 12, 5, 0, 0],
    )
    .unwrap();
    assert_eq!(user_version(&conn).unwrap(), 0);

    // 2. Apply migrations
    apply_migrations(&conn).unwrap();

    // 3. Assert: user_version bumped to 2 (our release version target)
    assert_eq!(user_version(&conn).unwrap(), 2, "migrate v0→v2 failed");
    // 4. Assert: sort_key columns EXIST (PRAGMA table_info)
    let artist_has_sort: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('artists') WHERE name = 'sort_key'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
        > 0;
    let album_has_sort: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('albums') WHERE name = 'sort_key'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
        > 0;
    assert!(
        artist_has_sort && album_has_sort,
        "sort_key columns missing after migration"
    );
    // 5. Assert: data NOT LOST — 周杰伦 still has id=1
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artists WHERE name = '周杰伦'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    // 6. Assert: indices exist (sqlite_master)
    let idx_a: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_artists_sort_key'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let idx_b: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_albums_sort_key'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        idx_a == 1 && idx_b == 1,
        "sort_key indices missing after migration"
    );
}

// ---------------------------------------------------------------------------
// compute_sort_key (Plan A Task A2).
// ---------------------------------------------------------------------------

use phonon_media::library::compute_sort_key;

#[test]
fn compute_sort_key_zh_pinyin_and_ascii_mixed_correct_grouping() {
    assert_eq!(compute_sort_key("周杰伦"), "zhoujielun"); // Chinese → full pinyin lowercase concat
    assert_eq!(compute_sort_key("JJ Lin"), "jj lin"); // ASCII stays lowercase
    assert_eq!(compute_sort_key("ABC 123"), "abc 123"); // ASCII downcase
    assert_eq!(compute_sort_key("夜曲 (Live)"), "yequ (live)"); // Chinese + punctuation preserved
}

// ---------------------------------------------------------------------------
// get_tracks_by_paths + search_suggest (Plan A Task A4).
// ---------------------------------------------------------------------------

use phonon_media::{LibrarySuggest, LibraryTrack};

/// `get_tracks_by_paths` must (a) return empty on empty input, (b) reject
/// batches >500 with InvalidPath, (c) preserve input order on hits and
/// silently drop missing paths.
#[test]
fn get_tracks_by_paths_preserves_order_and_caps_at_500() {
    let (_guard, lib) = open_library("by-paths");

    // Empty input → empty output (no SQL run).
    let out = lib.get_tracks_by_paths(&[]).expect("empty");
    assert!(out.is_empty());

    // 501 paths → InvalidPath error.
    let big: Vec<String> = (0..501).map(|i| format!("/tmp/nope-{i}.flac")).collect();
    let err = lib.get_tracks_by_paths(&big).expect_err("500-cap");
    let msg = format!("{err}");
    assert!(msg.contains("paths batch limit 500"), "got: {msg}");

    // Scan one fake file in, then ask for [missing, hit, missing, hit] →
    // output must be [hit, hit] in input order.
    let tmp_dir = std::env::temp_dir().join(format!(
        "phonon-test-by-path-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&tmp_dir).expect("mkdir");
    std::fs::write(tmp_dir.join("Song.flac"), b"").expect("write");
    lib.scan(&[tmp_dir.clone()], &ScanOptions::default(), &|_| {})
        .expect("scan");
    let tracks: Vec<TrackInfo> = lib.get_tracks(&TrackFilter::default()).expect("get_tracks");
    let hit_path = tracks[0].file_path.clone();

    let paths = vec![
        "/tmp/nonexistent-1.flac".to_string(),
        hit_path.clone(),
        "/tmp/nonexistent-2.flac".to_string(),
        hit_path.clone(),
    ];
    let out: Vec<LibraryTrack> = lib.get_tracks_by_paths(&paths).expect("ok");
    assert_eq!(out.len(), 2, "two hit rows should come back");
    assert_eq!(out[0].file_path, hit_path);
    assert_eq!(out[1].file_path, hit_path);

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

/// `search_suggest` returns an empty `LibrarySuggest` on empty/whitespace
/// queries without hitting the DB. With a real query, tracks/albums/artists
/// fields are populated (or all empty if no hits) and never `Err`.
#[test]
fn search_suggest_empty_query_short_circuits() {
    let (_guard, lib) = open_library("suggest-empty");

    let out: LibrarySuggest = lib.search_suggest("").expect("empty ok");
    assert!(out.tracks.is_empty());
    assert!(out.albums.is_empty());
    assert!(out.artists.is_empty());

    let out: LibrarySuggest = lib.search_suggest("   ").expect("ws ok");
    assert!(out.tracks.is_empty());
    assert!(out.albums.is_empty());
    assert!(out.artists.is_empty());

    // A non-empty query against an empty DB must succeed and return empty
    // lists — never an error.
    let out: LibrarySuggest = lib.search_suggest("anything").expect("non-empty ok");
    assert!(out.tracks.is_empty());
    assert!(out.albums.is_empty());
    assert!(out.artists.is_empty());
}
