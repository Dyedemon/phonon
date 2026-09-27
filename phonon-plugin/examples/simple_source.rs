//! Example WASM plugin: Simple Source Provider.
//!
//! This plugin implements the SourceProviderPlugin trait
//! and provides a hardcoded list of audio files.
//!
//! Build with:
//!   cargo build --target wasm32-wasi --release
//!   (requires: rustup target add wasm32-wasi)
//!
//! The resulting .wasm file can be loaded by phonon-plugin.

use std::collections::HashMap;

/// Track entry in the plugin's catalog.
struct TrackEntry {
    path: String,
    title: String,
    artist: String,
    album: String,
}

/// Plugin state.
static mut CATALOG: Vec<TrackEntry> = Vec::new();
static mut INITIALIZED: bool = false;

/// Initialize the plugin catalog (called once).
#[no_mangle]
pub extern "C" fn plugin_init() {
    unsafe {
        if INITIALIZED {
            return;
        }
        CATALOG = vec![
            TrackEntry {
                path: "/music/album1/track01.flac".into(),
                title: "First Track".into(),
                artist: "Test Artist".into(),
                album: "Test Album".into(),
            },
            TrackEntry {
                path: "/music/album1/track02.flac".into(),
                title: "Second Track".into(),
                artist: "Test Artist".into(),
                album: "Test Album".into(),
            },
        ];
        INITIALIZED = true;
    }
}

/// Search for audio files in the given directory.
/// Returns a JSON-encoded list of results.
#[no_mangle]
pub unsafe extern "C" fn plugin_search(query_ptr: *const u8, query_len: usize) -> *mut u8 {
    let query = unsafe {
        std::str::from_utf8(std::slice::from_raw_parts(query_ptr, query_len)).unwrap_or("")
    };

    let query_lower = query.to_lowercase();
    let results: Vec<HashMap<String, String>> = unsafe {
        let catalog = &*std::ptr::addr_of!(CATALOG);
        catalog
            .iter()
            .filter(|t| {
                t.path.to_lowercase().contains(&query_lower)
                    || t.title.to_lowercase().contains(&query_lower)
            })
            .map(|t| {
                let mut map = HashMap::new();
                map.insert("uri".into(), t.path.clone());
                map.insert("name".into(), t.title.clone());
                map.insert("artist".into(), t.artist.clone());
                map.insert("album".into(), t.album.clone());
                map
            })
            .collect()
    };

    let json = serde_json::to_string(&results).unwrap_or_else(|_| "[]".into());
    let bytes = json.into_bytes();
    let ptr = bytes.as_ptr() as *mut u8;
    std::mem::forget(bytes);
    ptr
}

/// Get the plugin name.
#[no_mangle]
pub extern "C" fn plugin_name() -> *const u8 {
    c"simple-source-plugin".as_ptr() as *const u8
}

/// Get the plugin version.
#[no_mangle]
pub extern "C" fn plugin_version() -> *const u8 {
    c"0.1.0".as_ptr() as *const u8
}

/// Free memory allocated by the plugin.
#[no_mangle]
pub unsafe extern "C" fn plugin_free(ptr: *mut u8, len: usize) {
    unsafe {
        let _ = Vec::from_raw_parts(ptr, len, len);
    }
}

// Required for wasm32-wasi
fn main() {}
