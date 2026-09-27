//! Cover thumbnail cache.
//!
//! Generates 256×256 and 512×512 WEBP thumbnails from embedded cover bytes,
//! keyed by SHA1 hash. LRU eviction is implemented by trimming the
//! `thumbnails_cache` table to a configurable maximum on each insert, using
//! `last_accessed_at` as the recency indicator.

use rusqlite::Connection;

use crate::error::Result;
use crate::schema;

/// Default LRU limit (number of distinct hashes).
pub const DEFAULT_LRU_LIMIT: usize = 10_000;

/// Ensure thumbnails for the given cover bytes exist in the cache.
///
/// Returns the SHA1 hash of the cover bytes. If the bytes are empty, returns
/// `None` and writes nothing.
pub fn ensure_thumbnail(conn: &Connection, cover_bytes: &[u8]) -> Result<Option<String>> {
    if cover_bytes.is_empty() {
        return Ok(None);
    }
    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    h.update(cover_bytes);
    let hash = hex(&h.finalize());

    // Generate both sizes if missing.
    for size in [256u32, 512u32] {
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM thumbnails_cache WHERE hash = ?1 AND size = ?2",
                rusqlite::params![hash, size as i64],
                |_| Ok(true),
            )
            .ok()
            .unwrap_or(false);
        if !exists {
            let webp = encode_thumbnail(cover_bytes, size)?;
            let now = schema::now_ts();
            conn.execute(
                "INSERT OR REPLACE INTO thumbnails_cache
                    (hash, size, webp, created_at, last_accessed_at)
                 VALUES (?1, ?2, ?3, ?4, ?4)",
                rusqlite::params![hash, size as i64, webp, now],
            )?;
        }
    }

    // LRU trim: keep only the N most-recently-accessed hashes (across both sizes).
    lru_trim(conn, DEFAULT_LRU_LIMIT)?;

    Ok(Some(hash))
}

/// Retrieve a thumbnail by hash and size, refreshing `last_accessed_at`.
pub fn get_thumbnail(conn: &Connection, hash: &str, size: u32) -> Result<Option<Vec<u8>>> {
    let now = schema::now_ts();
    let webp: Option<Vec<u8>> = conn
        .query_row(
            "SELECT webp FROM thumbnails_cache WHERE hash = ?1 AND size = ?2",
            rusqlite::params![hash, size as i64],
            |r| r.get::<_, Vec<u8>>(0),
        )
        .ok();
    if webp.is_some() {
        conn.execute(
            "UPDATE thumbnails_cache SET last_accessed_at = ?3
             WHERE hash = ?1 AND size = ?2",
            rusqlite::params![hash, size as i64, now],
        )?;
    }
    Ok(webp)
}

/// Remove thumbnails for hashes that have not been accessed recently,
/// keeping at most `limit` distinct hashes.
fn lru_trim(conn: &Connection, limit: usize) -> Result<()> {
    // Delete rows belonging to hashes that fall outside the top-`limit`
    // by last_accessed_at. We compute the cutoff rowid set in a subquery.
    conn.execute(
        "DELETE FROM thumbnails_cache
         WHERE hash IN (
             SELECT hash FROM (
                 SELECT hash, MAX(last_accessed_at) AS la
                 FROM thumbnails_cache
                 GROUP BY hash
                 ORDER BY la DESC
                 LIMIT -1 OFFSET ?1
             )
         )",
        rusqlite::params![limit as i64],
    )?;
    Ok(())
}

/// Decode → resize → WEBP-encode the cover bytes.
fn encode_thumbnail(cover_bytes: &[u8], target: u32) -> Result<Vec<u8>> {
    let img = image::load_from_memory(cover_bytes)?;
    let resized = img.resize_to_fill(target, target, image::imageops::FilterType::Lanczos3);
    let mut cursor = std::io::Cursor::new(Vec::with_capacity(8 * 1024));
    resized.write_to(&mut cursor, image::ImageFormat::WebP)?;
    Ok(cursor.into_inner())
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}
