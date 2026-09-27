//! Event push helpers.
//!
//! Provides functions to push real-time data to the frontend
//! via Tauri's event system.

use tauri::Emitter;

/// Push a playback progress update to the frontend.
pub fn push_progress(
    app_handle: &tauri::AppHandle,
    position_secs: f64,
    duration_secs: Option<f64>,
) {
    let _ = app_handle.emit(
        "playback-progress",
        serde_json::json!({
            "position_secs": position_secs,
            "duration_secs": duration_secs,
        }),
    );
}

/// Push spectrum data to the frontend.
pub fn push_spectrum(app_handle: &tauri::AppHandle, values: Vec<f32>) {
    let _ = app_handle.emit("spectrum-data", serde_json::json!({ "values": values }));
}

/// Push a plugin status update to the frontend.
pub fn push_plugin_status(app_handle: &tauri::AppHandle, id: &str, status: &str, message: &str) {
    let _ = app_handle.emit(
        "plugin-status",
        serde_json::json!({
            "id": id,
            "status": status,
            "message": message,
        }),
    );
}
