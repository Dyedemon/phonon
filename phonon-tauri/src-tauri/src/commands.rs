//! Tauri command handlers.
//!
//! All commands follow the naming convention `verb_noun`.
//! They access AppState via tauri::State<>.

use crate::state::{AppSettings, AppState, DspInfo, ReplayGainMode, VolumeMode};
use phonon_core::dsp::ReplayGainMode as DspReplayGainMode;
use phonon_core::eq::{EqBand, EqMode, FilterType};
use phonon_core::volume::VolumeMode as CoreVolumeMode;
#[cfg(windows)]
use phonon_core::wasapi::WasapiEndpointVolume;
use phonon_core::DsdMode;
use phonon_core::PlaybackState;
use phonon_core::QueueItem;
use phonon_plugin::PluginInfo;
use phonon_plugin::PluginType;
use phonon_source::IAudioSource;
use phonon_source::Playlist;
use serde::Serialize;
use std::collections::HashMap;
use tauri::Emitter;
use tauri::Manager;
use tauri::State;

/// Check if a path has a supported audio file extension.
/// Used to filter non-audio files (.lrc, .cue, .txt, images, etc.)
/// from playlists and session restoration.
fn is_audio_path(path: &str) -> bool {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    const AUDIO_EXTS: &[&str] = &[
        "flac", "alac", "m4a", "wav", "wave", "mp3", "aac", "dsf", "dff",
    ];
    AUDIO_EXTS.contains(&ext.as_str())
}

// ── Playback Control ──────────────────────────────────────────

/// Push current engine state to the frontend immediately, so the UI (song
/// title, play/pause button icon, progress bar) updates without waiting for
/// the 3 s polling loop. Called after every command that changes playback.
fn emit_state_snapshot(app: &tauri::AppHandle, state: &AppState) {
    // Suppress all snapshot emits while the frontend is reloading.
    // Without this, commands that finish during the narrow unload
    // window (e.g. a `stop` invoke that was in-flight when HMR fired)
    // would emit into a dead webview.
    if state
        .frontend_reloading
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return;
    }
    let snapshot = PlaybackStateInfo {
        state: format!("{:?}", state.engine.state()),
        position_secs: state.engine.position(),
        duration_secs: state.engine.duration(),
        buffer_fill: state.engine.buffer_fill(),
        current_track: state.engine.current_track(),
        queue_length: state.engine.queue_len(),
        speed: state.engine.get_speed(),
    };
    let _ = app.emit("playback-state-snapshot", &snapshot);
}

#[tauri::command]
pub fn play(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let s = state.engine.state();
    match s {
        PlaybackState::Playing => Ok(()),
        PlaybackState::Paused => {
            state.engine.resume();
            #[cfg(target_os = "windows")]
            crate::update_taskbar_play_icon(true);
            emit_state_snapshot(&app, &state);
            Ok(())
        }
        PlaybackState::Idle | PlaybackState::Stopped => {
            // Gracefully handle empty queue: no need to surface an error
            // when the user clicks play with nothing in the playlist.
            if state.engine.queue_len() == 0 {
                return Ok(());
            }
            state.engine.start_playback().map_err(|e| e.to_string())?;
            #[cfg(target_os = "windows")]
            crate::update_taskbar_play_icon(true);
            emit_state_snapshot(&app, &state);
            Ok(())
        }
    }
}

#[tauri::command]
pub fn toggle_play_pause(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let s = state.engine.state();
    match s {
        PlaybackState::Playing => {
            state.engine.pause();
            #[cfg(target_os = "windows")]
            crate::update_taskbar_play_icon(false);
            emit_state_snapshot(&app, &state);
            Ok(())
        }
        PlaybackState::Paused => {
            state.engine.resume();
            #[cfg(target_os = "windows")]
            crate::update_taskbar_play_icon(true);
            emit_state_snapshot(&app, &state);
            Ok(())
        }
        PlaybackState::Idle | PlaybackState::Stopped => {
            if state.engine.queue_len() == 0 {
                return Ok(());
            }
            state.engine.start_playback().map_err(|e| e.to_string())?;
            #[cfg(target_os = "windows")]
            crate::update_taskbar_play_icon(true);
            emit_state_snapshot(&app, &state);
            Ok(())
        }
    }
}

#[tauri::command]
pub fn pause(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.engine.pause();
    #[cfg(target_os = "windows")]
    crate::update_taskbar_play_icon(false);
    emit_state_snapshot(&app, &state);
    Ok(())
}

#[tauri::command]
pub fn stop(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.engine.stop();
    #[cfg(target_os = "windows")]
    crate::update_taskbar_play_icon(false);
    emit_state_snapshot(&app, &state);
    Ok(())
}

/// Called by the frontend's `beforeunload` handler just before the
/// webview is torn down (HMR refresh, F5). Does three things:
///
/// 1. Stops the audio engine so no more `PlaybackProgress` / `SpectrumData`
///    events are generated — those events would reach a dead JS context
///    and, if any listener callback inside JS re-invoked Rust, produce
///    "Couldn't find callback id" warnings.
/// 2. Sets `frontend_reloading = true` so the event-forwarding thread in
///    lib.rs discards all pending events instead of `app.emit`-ing them
///    into the void.
/// 3. Returns immediately (sync command) so the callback id is resolved
///    before the JS context disappears.
///
/// The flag is reset by `load_session` on the next mount.
#[tauri::command]
pub fn prepare_for_reload(state: State<'_, AppState>) -> Result<(), String> {
    // Stop engine first — this halts the decode thread which is the
    // primary source of periodic events.
    state.engine.stop();
    #[cfg(target_os = "windows")]
    crate::update_taskbar_play_icon(false);
    // Set the flag so the event-forwarding thread stops emitting.
    state
        .frontend_reloading
        .store(true, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub fn next(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.engine.next();
    // next() auto-starts playback when paused/stopped — flip taskbar icon.
    #[cfg(target_os = "windows")]
    crate::update_taskbar_play_icon(true);
    emit_state_snapshot(&app, &state);
    Ok(())
}

#[tauri::command]
pub fn previous(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.engine.previous();
    // previous() auto-starts playback when paused/stopped — flip taskbar icon.
    #[cfg(target_os = "windows")]
    crate::update_taskbar_play_icon(true);
    emit_state_snapshot(&app, &state);
    Ok(())
}

#[tauri::command]
pub fn seek(state: State<'_, AppState>, position_secs: f64) -> Result<(), String> {
    log::info!(
        "[seek] command received: position_secs={:.1}",
        position_secs
    );
    state.engine.seek(position_secs);
    Ok(())
}

#[tauri::command]
pub fn set_volume(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    level: f32,
) -> Result<(), String> {
    let mut vol = state.volume.lock().unwrap();
    vol.set_volume(level);
    #[allow(unused_variables)]
    let mode = vol.mode();
    drop(vol);
    #[cfg(windows)]
    {
        use phonon_core::volume::VolumeMode;
        if mode == VolumeMode::Hardware {
            ensure_com_initialized();
            // Prefer the controller attached by the polling task (no device
            // enumeration); only construct a one-off controller when none is
            // attached yet — a volume-slider drag previously did a full
            // CoCreateInstance + endpoint enumeration per tick.
            let applied = {
                let vol = state.volume.lock().unwrap();
                vol.apply_hw_volume()
            };
            if !applied {
                let dm = state.device_manager.lock().unwrap();
                let device_name = dm
                    .current_device_name()
                    .or_else(|| dm.default_device_name());
                if let Some(device_name) = device_name {
                    drop(dm);
                    if let Ok(hw) = WasapiEndpointVolume::new(&device_name) {
                        if hw.set_master_volume(level.clamp(0.0, 1.0)).is_ok() {
                            log::debug!("HW volume applied (fallback controller): {:.2}", level);
                        }
                    }
                }
            } else {
                log::debug!("HW volume applied: {:.2}", level);
            }
        }
    }
    // Broadcast volume change to all windows (tray popup, desktop lyrics, main)
    // so their volume sliders stay in sync regardless of which mode is active.
    let _ = app.emit("hw-volume", level.clamp(0.0, 1.0));
    Ok(())
}

#[tauri::command]
pub fn get_volume_mode(state: State<'_, AppState>) -> Result<String, String> {
    let vol = state.volume.lock().unwrap();
    let mode = vol.mode();
    Ok(format!("{:?}", mode))
}

/// Host OS for frontend platform gating (exclusive mode, DSD, hardware
/// volume and file associations are Windows-only features).
#[tauri::command]
pub fn get_platform() -> String {
    std::env::consts::OS.to_string()
}

/// Build identity (semver + git commit the binary was compiled from).
/// Lets the About tab prove which commit an installed app carries —
/// a stale install shows a stale SHA.
#[derive(serde::Serialize)]
pub struct BuildInfo {
    pub version: String,
    pub build_sha: String,
}

#[tauri::command]
pub fn get_build_info(app: tauri::AppHandle) -> BuildInfo {
    BuildInfo {
        version: app.package_info().version.to_string(),
        build_sha: env!("PHONON_BUILD_SHA").to_string(),
    }
}

/// Live audio-pipeline telemetry for the settings diagnostics card.
#[derive(serde::Serialize)]
pub struct AudioDiagnosticsInfo {
    /// "shared" | "exclusive" | "stopped"
    pub mode: String,
    /// Negotiated container label, empty when not exclusive.
    pub format: String,
    pub sample_rate: u32,
    pub channels: u16,
    /// Device buffer in frames (exclusive only; 0 in shared mode).
    pub buffer_frames: u32,
    /// Device-side queued frames (exclusive only).
    pub padding_frames: u32,
    pub ring_capacity_frames: usize,
    pub ring_available_frames: usize,
    pub underruns: u32,
    pub device_name: Option<String>,
    pub output_running: bool,
}

#[tauri::command]
pub fn get_audio_diagnostics(state: State<'_, AppState>) -> Result<AudioDiagnosticsInfo, String> {
    let engine = &state.engine;
    let diag = state.device_manager.lock().unwrap().diagnostics();
    let mode_code = diag.mode.load(std::sync::atomic::Ordering::Relaxed);
    let running = engine.output_running();
    let mode = if !running {
        "stopped".to_string()
    } else if mode_code == 1 {
        "exclusive".to_string()
    } else {
        "shared".to_string()
    };
    Ok(AudioDiagnosticsInfo {
        mode,
        format: if mode_code == 1 {
            diag.format_label().to_string()
        } else {
            "Float 32-bit (混音器)".to_string()
        },
        sample_rate: engine.current_output_rate(),
        channels: engine.channels(),
        buffer_frames: diag
            .buffer_frames
            .load(std::sync::atomic::Ordering::Relaxed),
        padding_frames: diag
            .padding_frames
            .load(std::sync::atomic::Ordering::Relaxed),
        ring_capacity_frames: engine.ring_capacity(),
        ring_available_frames: engine.ring_available(),
        underruns: engine.underruns(),
        device_name: {
            // config.device_name is only set when the user picked a specific
            // device; fall back to the actually-open device for display.
            let n = engine.current_device_name();
            if !n.is_empty() {
                Some(n)
            } else {
                let dm = state.device_manager.lock().unwrap();
                dm.current_device_name()
                    .or_else(|| dm.default_device().ok().map(|d| d.name))
            }
        },
        output_running: running,
    })
}

#[tauri::command]
pub fn get_volume_level(state: State<'_, AppState>) -> Result<f32, String> {
    let vol = state.volume.lock().unwrap();
    Ok(vol.level())
}

#[tauri::command]
pub fn sync_volume_from_device(
    #[allow(unused_variables)] state: State<'_, AppState>,
) -> Result<f32, String> {
    #[cfg(windows)]
    {
        ensure_com_initialized();
        // Prefer the attached controller; fall back to a fresh one only when
        // no controller is attached yet.
        if let Ok(mut vol) = state.volume.lock() {
            if let Some(v) = vol.sync_from_device() {
                return Ok(v);
            }
        }
        let dm = state.device_manager.lock().unwrap();
        let device_name = dm
            .current_device_name()
            .or_else(|| dm.default_device_name());
        if let Some(device_name) = device_name {
            drop(dm);
            if let Ok(hw) = WasapiEndpointVolume::new(&device_name) {
                if let Ok(sys_vol) = hw.get_master_volume() {
                    let clamped = sys_vol.clamp(0.0, 1.0);
                    let mut vol = state.volume.lock().unwrap();
                    vol.set_hw_level(clamped);
                    return Ok(clamped);
                }
            }
        }
        Err("Hardware volume controller not available".to_string())
    }
    #[cfg(not(windows))]
    {
        Err("Hardware volume sync not supported on this platform".to_string())
    }
}

#[derive(Serialize)]
pub struct PlaybackStateInfo {
    pub state: String,
    pub position_secs: f64,
    pub duration_secs: Option<f64>,
    pub buffer_fill: f64,
    pub current_track: Option<String>,
    pub queue_length: usize,
    pub speed: f32,
}

#[tauri::command]
pub fn get_playback_state(state: State<'_, AppState>) -> Result<PlaybackStateInfo, String> {
    Ok(PlaybackStateInfo {
        state: format!("{:?}", state.engine.state()),
        position_secs: state.engine.position(),
        duration_secs: state.engine.duration(),
        buffer_fill: state.engine.buffer_fill(),
        current_track: state.engine.current_track(),
        queue_length: state.engine.queue_len(),
        speed: state.engine.get_speed(),
    })
}

/// Set playback speed. Clamped to 0.25–2.0.
#[tauri::command]
pub fn set_playback_speed(state: State<'_, AppState>, speed: f32) -> Result<f32, String> {
    let clamped = speed.clamp(0.5, 2.0);
    state.engine.set_speed(clamped);
    Ok(clamped)
}

/// Get current playback speed.
#[tauri::command]
pub fn get_playback_speed(state: State<'_, AppState>) -> Result<f32, String> {
    Ok(state.engine.get_speed())
}

// ── Playback Mode ──────────────────────────────────────────────

#[tauri::command]
pub fn set_playback_mode(state: State<'_, AppState>, mode: String) -> Result<(), String> {
    use phonon_core::engine::PlaybackMode;
    let mode = match mode.as_str() {
        "normal" => PlaybackMode::Normal,
        "repeat_one" => PlaybackMode::RepeatOne,
        "repeat_all" => PlaybackMode::RepeatAll,
        _ => return Err(format!("Invalid playback mode: {}", mode)),
    };
    log::info!("Setting playback mode to {:?}", mode);
    state.engine.set_playback_mode(mode);
    Ok(())
}

#[tauri::command]
pub fn get_playback_mode(state: State<'_, AppState>) -> Result<String, String> {
    let mode = state.engine.get_playback_mode();
    let s = match mode {
        phonon_core::engine::PlaybackMode::Normal => "normal",
        phonon_core::engine::PlaybackMode::RepeatOne => "repeat_one",
        phonon_core::engine::PlaybackMode::RepeatAll => "repeat_all",
    };
    Ok(s.to_string())
}

// ── Device Management ─────────────────────────────────────────

#[derive(Serialize)]
pub struct DeviceInfoJson {
    pub name: String,
    pub id: String,
    pub is_default: bool,
    pub sample_rates: Vec<u32>,
    pub max_channels: u16,
    pub supports_exclusive: bool,
}

#[tauri::command]
pub fn list_devices(state: State<'_, AppState>) -> Result<Vec<DeviceInfoJson>, String> {
    let dm = state.device_manager.lock().unwrap();
    let devices = dm.list_devices().map_err(|e| e.to_string())?;
    Ok(devices
        .into_iter()
        .map(|d| DeviceInfoJson {
            name: d.name,
            id: d.id,
            is_default: d.is_default,
            sample_rates: d.supported_sample_rates,
            max_channels: d.max_channels,
            supports_exclusive: d.supports_exclusive,
        })
        .collect())
}

#[tauri::command]
pub fn set_device(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    device_id: String,
) -> Result<(), String> {
    // ════════════════════════════════════════════════════════════
    // SINGLE WRITER for settings.hotplug.last_used_device_id.
    // Step ① runs BEFORE any hardware mutation. If ③/④ later fail
    // the old id is ALREADY persisted atomically on disk.
    //
    // IMPORTANT: File I/O (save_settings_sync) MUST NOT happen while
    // holding the settings MutexGuard — clone first, drop the lock,
    // then write to disk. This follows the same pattern as
    // persist_settings_after and prevents blocking other threads.
    // ════════════════════════════════════════════════════════════
    let persisted_old = {
        // Get old device name first, release device_manager lock before locking settings
        let old_device_id = state
            .device_manager
            .lock()
            .map_err(|e| e.to_string())?
            .current_device_name();
        // Clone settings snapshot BEFORE dropping the lock, so the write
        // happens outside the critical section (no I/O under mutex).
        let settings_snapshot = {
            let mut settings = state.settings.lock().map_err(|e| e.to_string())?;
            match (&old_device_id, &device_id) {
                (Some(old), new) if old != new => {
                    settings.hotplug.last_used_device_id = Some(old.clone());
                    log::info!(
                        "[set_device] pre-switch saved last_used_device_id = Some({:?}) → new = {:?}",
                        old,
                        new
                    );
                    (true, Some(settings.clone()))
                }
                _ => (false, None),
            }
        }; // <-- settings lock DROPPED here (critical: no I/O under lock)

        let (changed, snapshot_opt) = settings_snapshot;
        if let Some(snap) = snapshot_opt {
            // File I/O happens OUTSIDE the settings lock.
            if let Err(err) = save_settings_sync(&snap) {
                log::error!(
                    "[set_device] CRITICAL save_settings_sync BEFORE switch failed: {}",
                    err
                );
            }
        }
        changed
    };

    // ════════════════════════════════════════════════════════════
    // Step ③ Actual hardware switch. Keep the EXISTING engine flow:
    // dm.set_default_device → if playing/paused → spawn thread →
    // pause (if paused) → seek(position) → engine.restart_output_stream().
    // DO NOT REPLACE with stop/start_playback.
    // ════════════════════════════════════════════════════════════
    {
        let mut dm = state.device_manager.lock().map_err(|e| e.to_string())?;
        dm.set_default_device(&device_id)
            .map_err(|e| e.to_string())?;
    }
    let engine = state.engine.clone();
    let current_state = engine.state();
    if current_state == PlaybackState::Playing || current_state == PlaybackState::Paused {
        let position = engine.position();
        let was_paused = current_state == PlaybackState::Paused;
        std::thread::spawn(move || {
            // If was paused, re-set paused flag before restarting output
            // so the output callback outputs silence immediately.
            if was_paused {
                engine.pause();
            }
            // Seek to current position — this signals the decode thread to
            // re-open the track at the correct offset and clears the ring buffer.
            engine.seek(position);
            // Restart the output stream on the new device. The decode thread
            // stays alive and will resume writing to the ring buffer.
            if let Err(e) = engine.restart_output_stream() {
                log::error!("Failed to restart output after device switch: {}", e);
            }
        });
    }

    // Step ④ Emit phonon:audioDeviceSwitch (prefix `phonon:` — v2 P2#9).
    let _ = app.emit(
        "phonon:audioDeviceSwitch",
        serde_json::json!({
            "new_device_id": device_id,
            "last_used_persisted": persisted_old,
        }),
    );
    // Persist the newly selected device for startup restoration.
    if let Err(e) = persist_settings_after(&state, |s| {
        s.hotplug.last_output_device_id = Some(device_id.clone());
    }) {
        log::error!(
            "[set_device] failed to persist last_output_device_id: {}",
            e
        );
    }
    Ok(())
}

#[tauri::command]
pub fn get_current_device(state: State<'_, AppState>) -> Result<Option<DeviceInfoJson>, String> {
    let dm = state.device_manager.lock().unwrap();
    Ok(dm.current_device_info().map(|d| DeviceInfoJson {
        name: d.name,
        id: d.id,
        is_default: d.is_default,
        sample_rates: d.supported_sample_rates,
        max_channels: d.max_channels,
        supports_exclusive: d.supports_exclusive,
    }))
}

/// Synthetic device event injector — for QA / Playwright harness (no physical
/// hardware needed). Safety: only available when `cfg!(debug_assertions)` is
/// true (dev builds). Release builds return Err. Forwards to the module-level
/// `state::hotplug_event_handler` so the strategy pipeline runs exactly as it
/// would for a real hotplug event.
#[tauri::command]
pub fn debug_inject_hotplug_event(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    kind: String,
    device_id: String,
) -> Result<(), String> {
    if !cfg!(debug_assertions) {
        return Err("debug_inject_hotplug_event is only available in dev mode".to_string());
    }
    let event = match kind.as_str() {
        "added" => phonon_core::device::DeviceEvent::DeviceAdded(phonon_core::device::DeviceInfo {
            name: device_id.clone(),
            id: device_id.clone(),
            is_default: false,
            supported_sample_rates: Vec::new(),
            max_channels: 2,
            supports_exclusive: false,
        }),
        "removed" => phonon_core::device::DeviceEvent::DeviceRemoved(device_id),
        "default_changed" => phonon_core::device::DeviceEvent::DefaultDeviceChanged(
            phonon_core::device::DeviceInfo {
                name: device_id.clone(),
                id: device_id,
                is_default: true,
                supported_sample_rates: Vec::new(),
                max_channels: 2,
                supports_exclusive: false,
            },
        ),
        other => return Err(format!("unknown kind: {}", other)),
    };
    crate::state::hotplug_event_handler(
        event,
        state.device_manager.clone(),
        state.engine.clone(),
        state.settings.clone(),
        Some(app),
    );
    Ok(())
}

// ── Queue Management ──────────────────────────────────────────

#[derive(Serialize)]
pub struct AddToQueueResult {
    pub added: usize,
    pub duplicates: usize,
}

#[tauri::command]
pub fn add_to_queue(
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<AddToQueueResult, String> {
    let mut added = 0usize;
    let mut duplicates = 0usize;
    // Track which paths were actually accepted by the engine so we only
    // persist those to the playlist — rejected paths (non-audio, dupes)
    // must NOT leak into the playlist display.
    let mut accepted_paths: Vec<String> = Vec::with_capacity(paths.len());
    // Prepend in reverse order so first file ends up at the top
    for path in paths.iter().rev() {
        match state
            .engine
            .enqueue_front(path)
            .map_err(|e| e.to_string())?
        {
            true => {
                added += 1;
                accepted_paths.push(path.clone());
            }
            false => duplicates += 1,
        }
    }
    // Ensure a Default playlist always exists.
    // Lock order: active_playlist FIRST, then playlists — consistent everywhere.
    let active_name = {
        let mut active = state.active_playlist.lock().map_err(|e| e.to_string())?;
        if active.is_empty() {
            *active = "Default".to_string();
        }
        active.clone()
    };
    {
        let mut playlists = state.playlists.lock().map_err(|e| e.to_string())?;
        if playlists.is_empty() {
            playlists.insert("Default".to_string(), Vec::new());
        }
    }

    // Save to active playlist — ONLY if it's not a system playlist.
    // System playlists (收藏/最近播放) are managed by DB queries and must
    // not be polluted by manually added files.
    if !is_system_playlist(&active_name) {
        let mut playlists = state.playlists.lock().map_err(|e| e.to_string())?;
        if let Some(pl) = playlists.get_mut(&active_name) {
            for p in &accepted_paths {
                if !pl.contains(p) {
                    pl.insert(0, p.clone());
                }
            }
        }
    }
    Ok(AddToQueueResult { added, duplicates })
}

#[tauri::command]
pub fn add_cue_to_queue(
    state: State<'_, AppState>,
    path: String,
) -> Result<AddToQueueResult, String> {
    let (added, duplicates) = state
        .engine
        .enqueue_cue_front(&path)
        .map_err(|e| e.to_string())?;
    // Ensure a Default playlist always exists.
    // Lock order: active_playlist FIRST, then playlists — consistent everywhere
    // (the previous order here was inverted and could deadlock against
    // switch_playlist / remove_from_queue which hold both locks).
    let active_name = {
        let mut active = state.active_playlist.lock().unwrap();
        if active.is_empty() {
            *active = "Default".to_string();
        }
        active.clone()
    };
    {
        let mut playlists = state.playlists.lock().unwrap();
        if playlists.is_empty() {
            playlists.insert("Default".to_string(), Vec::new());
        }
    }

    // Skip playlist persistence for system playlists — they are managed
    // by DB queries and must not be overwritten with queue snapshots.
    if !is_system_playlist(&active_name) {
        let mut playlists = state.playlists.lock().unwrap();
        if let Some(pl) = playlists.get_mut(&active_name) {
            let snapshot = state.engine.queue_snapshot();
            let paths: Vec<String> = snapshot.iter().map(|q| q.path.clone()).collect();
            *pl = paths;
        }
    }
    Ok(AddToQueueResult { added, duplicates })
}

#[tauri::command]
pub fn remove_from_queue(state: State<'_, AppState>, indices: Vec<usize>) -> Result<(), String> {
    state.engine.remove_from_queue(&indices);
    // Only sync to playlist for non-system playlists
    let active = state.active_playlist.lock().unwrap();
    if !active.is_empty() && !is_system_playlist(&active) {
        let mut playlists = state.playlists.lock().unwrap();
        if let Some(pl) = playlists.get_mut(&*active) {
            let snapshot = state.engine.queue_snapshot();
            let paths: Vec<String> = snapshot.iter().map(|q| q.path.clone()).collect();
            *pl = paths;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn reorder_queue(state: State<'_, AppState>, from: usize, to: usize) -> Result<(), String> {
    state.engine.reorder_queue(from, to);
    // Only sync to playlist for non-system playlists
    let active = state.active_playlist.lock().unwrap();
    if !active.is_empty() && !is_system_playlist(&active) {
        let mut playlists = state.playlists.lock().unwrap();
        if let Some(pl) = playlists.get_mut(&*active) {
            let snapshot = state.engine.queue_snapshot();
            let paths: Vec<String> = snapshot.iter().map(|q| q.path.clone()).collect();
            *pl = paths;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn get_queue(state: State<'_, AppState>) -> Result<Vec<QueueItem>, String> {
    Ok(state.engine.queue_snapshot())
}

#[tauri::command]
pub fn clear_queue(state: State<'_, AppState>) -> Result<(), String> {
    state.engine.clear_queue();
    Ok(())
}

#[tauri::command]
pub fn play_index(state: State<'_, AppState>, index: usize) -> Result<(), String> {
    state
        .engine
        .play_from_index(index)
        .map_err(|e| e.to_string())
}

// ── Playlist Management ───────────────────────────────────────

#[derive(Serialize)]
pub struct UserPlaylist {
    pub name: String,
    pub track_count: usize,
    /// If set, this is a system playlist with a stable identifier.
    /// Values: "favorites", "recent" — guaranteed not to change with language.
    #[serde(default)]
    pub system_id: Option<String>,
}

#[tauri::command]
pub fn create_playlist(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let mut playlists = state.playlists.lock().unwrap();
    if playlists.contains_key(&name) {
        return Err(format!("Playlist '{}' already exists", name));
    }
    playlists.insert(name.clone(), Vec::new());
    // New playlists go at the bottom
    state.playlist_order.lock().unwrap().push(name.clone());
    log::info!("Created playlist: {}", name);
    Ok(())
}

#[tauri::command]
pub fn delete_playlist(state: State<'_, AppState>, name: String) -> Result<(), String> {
    if is_system_playlist(&name) {
        return Err(format!("Cannot delete system playlist '{}'", name));
    }
    // Lock order: active_playlist FIRST, then playlists — consistent everywhere.
    {
        let mut active = state.active_playlist.lock().map_err(|e| e.to_string())?;
        if *active == name {
            *active = String::new();
        }
    }
    let mut playlists = state.playlists.lock().map_err(|e| e.to_string())?;
    if !playlists.contains_key(&name) {
        return Err(format!("Playlist '{}' not found", name));
    }
    playlists.remove(&name);
    // Remove from display order
    state
        .playlist_order
        .lock()
        .map_err(|e| e.to_string())?
        .retain(|n| n != &name);
    log::info!("Deleted playlist: {}", name);
    Ok(())
}

#[tauri::command]
pub fn switch_playlist(state: State<'_, AppState>, name: String) -> Result<(), String> {
    // Save current queue to active playlist before switching
    {
        let active = state.active_playlist.lock().unwrap();
        if !active.is_empty() {
            let snapshot = state.engine.queue_snapshot();
            let paths: Vec<String> = snapshot.iter().map(|q| q.path.clone()).collect();
            if let Some(pl) = state.playlists.lock().unwrap().get_mut(&*active) {
                *pl = paths;
            }
        }
    }

    // Clear current queue and load new playlist
    state.engine.clear_queue();

    // Load the target playlist
    let playlists = state.playlists.lock().unwrap();
    let paths = playlists.get(&name).cloned().unwrap_or_default();
    drop(playlists);

    for path in paths.iter().rev() {
        let _ = state.engine.enqueue_front(path);
    }

    // Set queue index past the end so current track finishes
    // and playback stops automatically without auto-playing from new playlist
    state.engine.set_queue_index(paths.len());

    *state.active_playlist.lock().unwrap() = name.clone();
    log::info!("Switched to playlist: {} ({} tracks)", name, paths.len());
    Ok(())
}

/// Switch to a system playlist ("收藏" or "最近播放").
/// Loads tracks from the library DB (not the playlists HashMap).
/// "收藏" → favorites_only, sorted by added_at DESC.
/// "最近播放" → last_played_at DESC, limit 200.
#[tauri::command]
pub fn switch_system_playlist(state: State<'_, AppState>, name: String) -> Result<(), String> {
    // Save current queue to active playlist before switching
    {
        let active = state.active_playlist.lock().unwrap();
        if !active.is_empty() && !is_system_playlist(&active) {
            let snapshot = state.engine.queue_snapshot();
            let paths: Vec<String> = snapshot.iter().map(|q| q.path.clone()).collect();
            if let Some(pl) = state.playlists.lock().unwrap().get_mut(&*active) {
                *pl = paths;
            }
        }
    }

    // Build filter based on system playlist type
    let filter = if name == "收藏" {
        phonon_media::TrackFilter {
            favorites_only: true,
            ..Default::default()
        }
    } else {
        // 最近播放
        phonon_media::TrackFilter {
            sort: Some(phonon_media::TrackSortField::LastPlayed),
            order: Some(phonon_media::SortOrder::Desc),
            limit: Some(200),
            played_only: true,
            ..Default::default()
        }
    };

    // Query library
    let tracks = with_library(&state, |lib| {
        lib.get_tracks(&filter).map_err(|e| e.to_string())
    })?;

    // Clear current queue
    state.engine.clear_queue();

    // Enqueue tracks (reverse order so first track ends up at front)
    for track in tracks.iter().rev() {
        let _ = state.engine.enqueue_front(&track.file_path);
    }

    // Set queue index past the end so current track finishes
    state.engine.set_queue_index(tracks.len());

    *state.active_playlist.lock().unwrap() = name.clone();
    log::info!(
        "Switched to system playlist: {} ({} tracks)",
        name,
        tracks.len()
    );
    Ok(())
}

fn is_system_playlist(name: &str) -> bool {
    name == "收藏" || name == "最近播放"
}

/// Return system playlists (收藏 / 最近播放) with track counts from the library DB.
#[tauri::command]
pub fn list_system_playlists(state: State<'_, AppState>) -> Result<Vec<UserPlaylist>, String> {
    let lib_guard = state.media_library.lock().map_err(|e| e.to_string())?;
    let lib = lib_guard.as_ref();

    let fav_count = lib
        .map(|l| {
            l.get_tracks(&phonon_media::TrackFilter {
                favorites_only: true,
                ..Default::default()
            })
            .map(|t| t.len())
            .unwrap_or(0)
        })
        .unwrap_or(0);

    let recent_count = lib
        .map(|l| {
            l.get_tracks(&phonon_media::TrackFilter {
                sort: Some(phonon_media::TrackSortField::LastPlayed),
                order: Some(phonon_media::SortOrder::Desc),
                limit: Some(200),
                played_only: true,
                ..Default::default()
            })
            .map(|t| t.len())
            .unwrap_or(0)
        })
        .unwrap_or(0);

    Ok(vec![
        UserPlaylist {
            name: "收藏".into(),
            track_count: fav_count,
            system_id: Some("favorites".into()),
        },
        UserPlaylist {
            name: "最近播放".into(),
            track_count: recent_count,
            system_id: Some("recent".into()),
        },
    ])
}

#[tauri::command]
pub fn list_playlists(state: State<'_, AppState>) -> Result<Vec<UserPlaylist>, String> {
    let playlists = state.playlists.lock().unwrap();
    let mut order = state.playlist_order.lock().unwrap();

    // NOTE: System playlists are returned by list_system_playlists, not here.

    let mut result: Vec<UserPlaylist> = Vec::new();

    // User playlists in order
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut stragglers: Vec<String> = Vec::new();
    for name in order.iter() {
        if let Some(tracks) = playlists.get(name) {
            seen.insert(name);
            result.push(UserPlaylist {
                name: name.clone(),
                track_count: tracks.len(),
                system_id: None,
            });
        }
    }
    // Collect playlists not in the order list, then add them
    for (name, tracks) in playlists.iter() {
        if !seen.contains(name.as_str()) {
            stragglers.push(name.clone());
            result.push(UserPlaylist {
                name: name.clone(),
                track_count: tracks.len(),
                system_id: None,
            });
        }
    }
    for name in stragglers {
        order.push(name);
    }
    // Clean up stale entries in order (playlists that were deleted)
    order.retain(|n| playlists.contains_key(n));
    Ok(result)
}

#[tauri::command]
pub fn get_active_playlist(state: State<'_, AppState>) -> Result<String, String> {
    Ok(state.active_playlist.lock().unwrap().clone())
}

#[tauri::command]
pub fn rename_playlist(
    state: State<'_, AppState>,
    old_name: String,
    new_name: String,
) -> Result<(), String> {
    // Lock order: active_playlist FIRST, then playlists — consistent everywhere.
    {
        let mut active = state.active_playlist.lock().map_err(|e| e.to_string())?;
        if *active == old_name {
            *active = new_name.clone();
        }
    }
    let mut playlists = state.playlists.lock().map_err(|e| e.to_string())?;
    if playlists.contains_key(&new_name) {
        return Err(format!("Playlist '{}' already exists", new_name));
    }
    let tracks = playlists
        .remove(&old_name)
        .ok_or_else(|| format!("Playlist '{}' not found", old_name))?;
    playlists.insert(new_name.clone(), tracks);
    // Update display order
    {
        let mut order = state.playlist_order.lock().map_err(|e| e.to_string())?;
        if let Some(pos) = order.iter().position(|n| n == &old_name) {
            order[pos] = new_name.clone();
        }
    }
    Ok(())
}

#[tauri::command]
pub fn reorder_playlists(state: State<'_, AppState>, order: Vec<String>) -> Result<(), String> {
    let playlists = state.playlists.lock().unwrap();
    // Verify all names exist in playlists
    for name in &order {
        if !playlists.contains_key(name) {
            return Err(format!("Playlist '{}' not found", name));
        }
    }
    drop(playlists);
    *state.playlist_order.lock().unwrap() = order;
    log::info!("Playlist order updated");
    Ok(())
}

#[tauri::command]
pub fn save_current_to_playlist(state: State<'_, AppState>) -> Result<(), String> {
    let active = state.active_playlist.lock().unwrap();
    if active.is_empty() {
        return Err("No active playlist".into());
    }
    let snapshot = state.engine.queue_snapshot();
    let paths: Vec<String> = snapshot.iter().map(|q| q.path.clone()).collect();
    let mut playlists = state.playlists.lock().unwrap();
    if let Some(pl) = playlists.get_mut(&*active) {
        *pl = paths;
    }
    Ok(())
}

#[tauri::command]
pub fn save_playlist(
    state: State<'_, AppState>,
    file_path: String,
    format: String,
) -> Result<(), String> {
    let playlist = state.playlist.lock().unwrap();
    // Validate format is supported (Playlist::save detects format from extension)
    match format.to_lowercase().as_str() {
        "m3u" | "m3u8" | "pls" | "xspf" => {}
        _ => return Err(format!("Unsupported format: {}", format)),
    };
    playlist.save(&file_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn load_playlist(
    state: State<'_, AppState>,
    file_path: String,
) -> Result<Vec<QueueItem>, String> {
    let playlist = Playlist::load(&file_path).map_err(|e| e.to_string())?;
    let items: Vec<QueueItem> = playlist
        .entries
        .iter()
        .map(|e| QueueItem {
            path: e.path.clone(),
            title: e.title.clone().unwrap_or_else(|| "Unknown".into()),
            artist: e.artist.clone(),
            duration: e.duration,
        })
        .collect();

    *state.playlist.lock().unwrap() = playlist;
    Ok(items)
}

// ── DSP Management ────────────────────────────────────────────

#[tauri::command]
pub fn list_dsp_processors(state: State<'_, AppState>) -> Result<Vec<DspInfo>, String> {
    let chain = state.dsp_chain.lock().unwrap();
    Ok(chain
        .list_processors()
        .into_iter()
        .map(|p| DspInfo {
            id: p.id,
            name: p.name,
            enabled: p.enabled,
            // `DspProcessorInfo.latency` is in SECONDS; the field name (and
            // the UI rendering it as ms) expects milliseconds.
            latency_ms: p.latency * 1000.0,
        })
        .collect())
}

#[tauri::command]
pub fn enable_dsp(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut chain = state.dsp_chain.lock().unwrap();
    chain.set_enabled(&id, true);
    Ok(())
}

#[tauri::command]
pub fn disable_dsp(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut chain = state.dsp_chain.lock().unwrap();
    chain.set_enabled(&id, false);
    Ok(())
}

#[tauri::command]
pub fn reorder_dsp(state: State<'_, AppState>, from: usize, to: usize) -> Result<(), String> {
    let mut chain = state.dsp_chain.lock().unwrap();
    chain.reorder(from, to);
    Ok(())
}

#[tauri::command]
pub fn get_dsp_latency(state: State<'_, AppState>) -> Result<f64, String> {
    // Live sum instead of the cached `chain.total_latency()`: builtin
    // processors (Smart Effect / Surround Sound) flip their own enabled flag
    // inside `set_mode()` without going through `DspChain::set_enabled`, so
    // the cached sum goes stale and would report 0.0ms forever. `latency()`
    // already returns 0 for disabled processors and uses a non-blocking
    // try_lock on the UI path.
    // The DSP trait reports latency in SECONDS; this UI-facing command
    // returns milliseconds (the frontend renders `{value.toFixed(1)}ms`).
    let chain = state.dsp_chain.lock().unwrap();
    Ok(chain
        .list_processors()
        .iter()
        .map(|p| p.latency)
        .sum::<f64>()
        * 1000.0)
}

/// Get the current smart effect mode.
#[tauri::command]
pub fn get_smart_effect_mode(state: State<'_, AppState>) -> Result<String, String> {
    let mut chain = state.dsp_chain.lock().unwrap();
    if let Some(proc) = chain.find_mut("smart_effect") {
        if let Some(se) = proc
            .as_any_mut()
            .downcast_mut::<phonon_core::dsp::SmartEffectProcessor>()
        {
            return Ok(se.mode().as_str().to_string());
        }
    }
    Ok("off".to_string())
}

/// Set the smart effect mode.
#[tauri::command]
pub fn set_smart_effect_mode(state: State<'_, AppState>, mode: String) -> Result<(), String> {
    let mut chain = state.dsp_chain.lock().unwrap();
    if let Some(proc) = chain.find_mut("smart_effect") {
        if let Some(se) = proc
            .as_any_mut()
            .downcast_mut::<phonon_core::dsp::SmartEffectProcessor>()
        {
            let m = phonon_core::dsp::SmartEffectMode::from_str(&mode);
            se.set_mode(m);
            log::info!("[smart_effect] Mode set to: {}", mode);
            return Ok(());
        }
    }
    Err("Smart Effect processor not found".to_string())
}

/// Get the current surround sound mode.
#[tauri::command]
pub fn get_surround_sound_mode(state: State<'_, AppState>) -> Result<String, String> {
    let mut chain = state.dsp_chain.lock().unwrap();
    if let Some(proc) = chain.find_mut("surround_sound") {
        if let Some(ss) = proc
            .as_any_mut()
            .downcast_mut::<phonon_core::dsp::SurroundSoundProcessor>()
        {
            return Ok(ss.mode().as_str().to_string());
        }
    }
    Ok("off".to_string())
}

/// Set the surround sound mode.
#[tauri::command]
pub fn set_surround_sound_mode(state: State<'_, AppState>, mode: String) -> Result<(), String> {
    let mut chain = state.dsp_chain.lock().unwrap();
    if let Some(proc) = chain.find_mut("surround_sound") {
        if let Some(ss) = proc
            .as_any_mut()
            .downcast_mut::<phonon_core::dsp::SurroundSoundProcessor>()
        {
            let m = phonon_core::dsp::SurroundSoundMode::from_str(&mode);
            ss.set_mode(m);
            log::info!("[surround_sound] Mode set to: {}", mode);
            return Ok(());
        }
    }
    Err("Surround Sound processor not found".to_string())
}

#[tauri::command]
pub fn add_wasm_dsp(
    state: State<'_, AppState>,
    name: String,
    id: String,
    latency: f64,
) -> Result<(), String> {
    // Check for duplicate (without holding dsp lock)
    {
        let chain = state.dsp_chain.lock().unwrap();
        if chain.list_processors().iter().any(|p| p.id == id) {
            return Err(format!("DSP processor '{}' already exists", id));
        }
    }
    // Auto-load the plugin if not already loaded (needed for import flow)
    let plugins = state.plugin_runtime.list_plugins();
    if let Some(info) = plugins.iter().find(|p| p.manifest.name == name) {
        use phonon_plugin::PluginState;
        if info.state != PluginState::Loaded {
            state
                .plugin_runtime
                .load(&info.file_path)
                .map_err(|e| e.to_string())?;
        }
    }
    // Create processor without holding dsp lock (avoids blocking audio callback)
    let process_fn = state
        .plugin_runtime
        .create_processor(&name)
        .map_err(|e| e.to_string())?;
    let processor = phonon_core::dsp::WasmDspProcessor::new(name, id, latency, process_fn);
    // Add to chain (brief dsp lock)
    let mut chain = state.dsp_chain.lock().unwrap();
    chain.add(Box::new(processor));
    Ok(())
}

#[tauri::command]
pub fn remove_wasm_dsp(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut chain = state.dsp_chain.lock().unwrap();
    chain.remove(&id);
    Ok(())
}

#[tauri::command]
pub fn import_plugin(
    state: State<'_, AppState>,
    source_path: String,
) -> Result<Vec<PluginInfo>, String> {
    let plugin_dir = state.plugin_runtime.plugin_dir();
    let src = std::path::Path::new(&source_path);
    let filename = src
        .file_name()
        .ok_or_else(|| "Invalid file path".to_string())?;
    let dest = plugin_dir.join(filename);
    std::fs::copy(&source_path, &dest).map_err(|e| format!("Failed to copy plugin: {}", e))?;
    log::info!("Imported plugin: {}", dest.display());
    state.plugin_runtime.scan().map_err(|e| e.to_string())
}

// ── Plugin Management ─────────────────────────────────────────

#[tauri::command]
pub fn list_plugins(state: State<'_, AppState>) -> Result<Vec<PluginInfo>, String> {
    let plugins = state.plugin_runtime.list_plugins();
    log::info!("list_plugins: returning {} plugins", plugins.len());
    Ok(plugins)
}

#[tauri::command]
pub fn enable_plugin(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.plugin_runtime.load(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn disable_plugin(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.plugin_runtime.unload(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn scan_plugins(state: State<'_, AppState>) -> Result<Vec<PluginInfo>, String> {
    log::info!(
        "scan_plugins: scanning plugin dir: {}",
        state.plugin_runtime.plugin_dir().display()
    );
    let result = state.plugin_runtime.scan().map_err(|e| e.to_string());
    if let Ok(ref plugins) = result {
        log::info!("scan_plugins: found {} plugins", plugins.len());
    }
    result
}

#[tauri::command]
pub fn get_plugin_dir(state: State<'_, AppState>) -> String {
    state
        .plugin_runtime
        .plugin_dir()
        .to_string_lossy()
        .to_string()
}

#[tauri::command]
pub fn open_plugin_dir(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let dir = state.plugin_runtime.plugin_dir();
    // Ensure the directory exists before opening
    let _ = std::fs::create_dir_all(&dir);
    let path_str = dir.to_string_lossy().to_string();
    open_path_in_explorer(&app, &path_str)
}

fn open_path_in_explorer(_app: &tauri::AppHandle, path: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[tauri::command]
pub fn remove_plugin(
    state: State<'_, AppState>,
    file_path: String,
) -> Result<Vec<PluginInfo>, String> {
    // Get plugin info before removal to know its name and type
    let plugin_name = {
        let plugins = state.plugin_runtime.list_plugins();
        plugins
            .iter()
            .find(|p| p.file_path == file_path)
            .map(|p| (p.manifest.name.clone(), p.manifest.plugin_type))
    };

    state
        .plugin_runtime
        .remove(&file_path)
        .map_err(|e| e.to_string())?;

    // Also remove from DSP chain if it's a DSP processor
    if let Some((name, plugin_type)) = plugin_name {
        if plugin_type == PluginType::DspProcessor {
            let dsp_id = format!("wasm_{}", name);
            let mut chain = state.dsp_chain.lock().unwrap();
            chain.remove(&dsp_id);
            log::info!("Removed DSP processor '{}' from chain", dsp_id);
        }
    }

    // Return updated list
    Ok(state.plugin_runtime.list_plugins())
}

#[tauri::command]
pub fn reload_plugin(
    state: State<'_, AppState>,
    file_path: String,
) -> Result<Vec<PluginInfo>, String> {
    state
        .plugin_runtime
        .reload(&file_path)
        .map_err(|e| e.to_string())?;
    Ok(state.plugin_runtime.list_plugins())
}

#[tauri::command]
pub fn get_plugin_config(
    state: State<'_, AppState>,
    name: String,
) -> Result<Option<String>, String> {
    use phonon_core::dsp::{DownmixProcessor, DspProcessor, SmartEffectMode, SurroundSoundMode};
    use phonon_core::eq::EqualizerProcessor;

    // 判定 origin：Builtin 前缀列表，其他 → Wasm 走 plugin_runtime
    let builtin_id: Option<&str> = match name.as_str() {
        "phonon_equalizer" => Some("equalizer"),
        "phonon_smart_effect" => Some("smart_effect"),
        "phonon_surround_sound" => Some("surround_sound"),
        "phonon_downmix" => Some("downmix"),
        "phonon_replaygain" => Some("replaygain"),
        _ => None,
    };

    if let Some(dsp_id) = builtin_id {
        // ── Builtin：直接从 DSP 链/settings 合成 JSON ────────────
        let mut dsp = state.dsp_chain.lock().unwrap();
        match dsp_id {
            "equalizer" => {
                if let Some(proc) = dsp.find_mut("equalizer") {
                    if let Some(eq) = proc.as_any_mut().downcast_mut::<EqualizerProcessor>() {
                        let st = EqState {
                            bands: eq.get_bands(),
                            mode: eq.mode(),
                            presets: eq.get_presets(),
                            geq_band_count: eq.geq_band_count(),
                            preamp_db: eq.preamp(),
                        };
                        return Ok(Some(
                            serde_json::to_string(&st)
                                .map_err(|e| format!("Serialize EqState: {}", e))?,
                        ));
                    }
                }
                return Err("Equalizer processor not found".into());
            }
            "smart_effect" => {
                let mode: String = dsp
                    .find_mut("smart_effect")
                    .and_then(|p| {
                        p.as_any_mut()
                            .downcast_mut::<phonon_core::dsp::SmartEffectProcessor>()
                    })
                    .map(|se| se.mode().as_str().to_string())
                    .unwrap_or_else(|| SmartEffectMode::default().as_str().to_string());
                return Ok(Some(format!("{{\"mode\":\"{}\"}}", mode)));
            }
            "surround_sound" => {
                let mode: String = dsp
                    .find_mut("surround_sound")
                    .and_then(|p| {
                        p.as_any_mut()
                            .downcast_mut::<phonon_core::dsp::SurroundSoundProcessor>()
                    })
                    .map(|ss| ss.mode().as_str().to_string())
                    .unwrap_or_else(|| SurroundSoundMode::default().as_str().to_string());
                return Ok(Some(format!("{{\"mode\":\"{}\"}}", mode)));
            }
            "downmix" => {
                let enabled = dsp
                    .find_mut("downmix")
                    .and_then(|p| p.as_any_mut().downcast_mut::<DownmixProcessor>())
                    .map(|d| d.enabled())
                    .unwrap_or(false);
                return Ok(Some(format!("{{\"enabled\":{}}}", enabled)));
            }
            "replaygain" => {
                let mode = state.settings.lock().unwrap().replay_gain_mode;
                let s = match mode {
                    ReplayGainMode::Off => "Off",
                    ReplayGainMode::Track => "Track",
                    ReplayGainMode::Album => "Album",
                };
                return Ok(Some(format!("{{\"mode\":\"{}\"}}", s)));
            }
            _ => return Ok(None),
        }
    }

    // ── Wasm：沿用原有持久化路径 ──────────────────────────────
    // （此时 PluginOrigin 只能是 WasmExample / WasmExternal；但这里我们不用 origin 区分，
    //   get_plugin_config 对两者行为一致：读 plugins_dir/{name}.json）
    Ok(state.plugin_runtime.get_plugin_config(&name))
}

#[tauri::command]
pub fn set_plugin_config(
    state: State<'_, AppState>,
    name: String,
    config: String,
) -> Result<(), String> {
    use phonon_core::dsp::{
        DownmixProcessor, DspProcessor, ReplayGainMode as DspReplayGainMode, SmartEffectMode,
        SurroundSoundMode,
    };
    use phonon_core::eq::{EqBand, EqMode, EqualizerProcessor, FilterType};

    // Validate JSON（通用前置检查，保持原有语义）
    if !config.trim().is_empty() {
        let _: serde_json::Value =
            serde_json::from_str(&config).map_err(|e| format!("Invalid JSON: {}", e))?;
    }

    let builtin_id: Option<&str> = match name.as_str() {
        "phonon_equalizer" => Some("equalizer"),
        "phonon_smart_effect" => Some("smart_effect"),
        "phonon_surround_sound" => Some("surround_sound"),
        "phonon_downmix" => Some("downmix"),
        "phonon_replaygain" => Some("replaygain"),
        _ => None,
    };

    if let Some(dsp_id) = builtin_id {
        let v: serde_json::Value =
            serde_json::from_str(&config).map_err(|e| format!("Invalid JSON: {}", e))?;

        match dsp_id {
            "equalizer" => {
                let mut dsp = state.dsp_chain.lock().unwrap();
                let proc = dsp
                    .find_mut("equalizer")
                    .ok_or_else(|| "Equalizer processor not found".to_string())?;
                let eq = proc
                    .as_any_mut()
                    .downcast_mut::<EqualizerProcessor>()
                    .ok_or_else(|| "Equalizer downcast failed".to_string())?;
                let sr = state.engine.sample_rate();

                // ── Partial update：哪个字段给了就更新哪个 ──────
                if let Some(mode_val) = v.get("mode").and_then(|x| x.as_str()) {
                    let m = match mode_val {
                        "Graphic" => EqMode::Graphic,
                        "Parametric" => EqMode::Parametric,
                        _ => return Err(format!("Invalid EQ mode: {}", mode_val)),
                    };
                    eq.set_mode(m, sr);
                }
                if let Some(count) = v.get("geq_band_count").and_then(|x| x.as_u64()) {
                    eq.set_geq_bands(count as usize, sr);
                }
                if let Some(db) = v.get("preamp_db").and_then(|x| x.as_f64()) {
                    eq.set_preamp(db as f32);
                }
                if let Some(bands_val) = v.get("bands").and_then(|x| x.as_array()) {
                    for (i, band_val) in bands_val.iter().enumerate() {
                        let frequency = band_val
                            .get("frequency")
                            .and_then(|x| x.as_f64())
                            .ok_or_else(|| format!("bands[{}].frequency missing", i))?
                            as f32;
                        let gain_db = band_val
                            .get("gain_db")
                            .and_then(|x| x.as_f64())
                            .ok_or_else(|| format!("bands[{}].gain_db missing", i))?
                            as f32;
                        let q = band_val
                            .get("q")
                            .and_then(|x| x.as_f64())
                            .ok_or_else(|| format!("bands[{}].q missing", i))?
                            as f32;
                        let ft_str = band_val
                            .get("filterType")
                            .and_then(|x| x.as_str())
                            .or_else(|| band_val.get("filter_type").and_then(|x| x.as_str()))
                            .ok_or_else(|| format!("bands[{}].filterType missing", i))?;
                        let ft = match ft_str {
                            "Peaking" => FilterType::Peaking,
                            "LowShelf" => FilterType::LowShelf,
                            "HighShelf" => FilterType::HighShelf,
                            "LowPass" => FilterType::LowPass,
                            "HighPass" => FilterType::HighPass,
                            _ => {
                                return Err(format!("bands[{}] invalid filterType: {}", i, ft_str))
                            }
                        };
                        eq.set_band(
                            i,
                            EqBand {
                                frequency,
                                gain_db,
                                q,
                                filter_type: ft,
                            },
                            sr,
                        );
                    }
                }
                return Ok(());
            }
            "smart_effect" => {
                let mode = v
                    .get("mode")
                    .and_then(|x| x.as_str())
                    .ok_or_else(|| "Missing 'mode' field".to_string())?;
                let mut chain = state.dsp_chain.lock().unwrap();
                let proc = chain
                    .find_mut("smart_effect")
                    .ok_or_else(|| "Smart Effect processor not found".to_string())?;
                let se = proc
                    .as_any_mut()
                    .downcast_mut::<phonon_core::dsp::SmartEffectProcessor>()
                    .ok_or_else(|| "SmartEffect downcast failed".to_string())?;
                let m = SmartEffectMode::from_str(mode);
                se.set_mode(m);
                log::info!(
                    "[builtin:smart_effect] Mode set via plugin config: {}",
                    mode
                );
                return Ok(());
            }
            "surround_sound" => {
                let mode = v
                    .get("mode")
                    .and_then(|x| x.as_str())
                    .ok_or_else(|| "Missing 'mode' field".to_string())?;
                let mut chain = state.dsp_chain.lock().unwrap();
                let proc = chain
                    .find_mut("surround_sound")
                    .ok_or_else(|| "Surround Sound processor not found".to_string())?;
                let ss = proc
                    .as_any_mut()
                    .downcast_mut::<phonon_core::dsp::SurroundSoundProcessor>()
                    .ok_or_else(|| "SurroundSound downcast failed".to_string())?;
                let m = SurroundSoundMode::from_str(mode);
                ss.set_mode(m);
                log::info!(
                    "[builtin:surround_sound] Mode set via plugin config: {}",
                    mode
                );
                return Ok(());
            }
            "downmix" => {
                let enabled = v
                    .get("enabled")
                    .and_then(|x| x.as_bool())
                    .ok_or_else(|| "Missing 'enabled' bool field".to_string())?;
                let mut chain = state.dsp_chain.lock().unwrap();
                let proc = chain
                    .find_mut("downmix")
                    .ok_or_else(|| "Downmix processor not found".to_string())?;
                let dm = proc
                    .as_any_mut()
                    .downcast_mut::<DownmixProcessor>()
                    .ok_or_else(|| "Downmix downcast failed".to_string())?;
                dm.set_enabled(enabled);
                return Ok(());
            }
            "replaygain" => {
                let mode_str = v
                    .get("mode")
                    .and_then(|x| x.as_str())
                    .ok_or_else(|| "Missing 'mode' field".to_string())?;
                // 先设置到 settings（state 层枚举）
                let rg_mode = match mode_str {
                    "Off" => ReplayGainMode::Off,
                    "Track" => ReplayGainMode::Track,
                    "Album" => ReplayGainMode::Album,
                    _ => return Err(format!("Invalid ReplayGain mode: {}", mode_str)),
                };
                state.settings.lock().unwrap().replay_gain_mode = rg_mode;

                // 再设置到 DSP 链处理器（core 层枚举，和上面保持平行命名）
                let dsp_rg_mode = match rg_mode {
                    ReplayGainMode::Off => DspReplayGainMode::Off,
                    ReplayGainMode::Track => DspReplayGainMode::Track,
                    ReplayGainMode::Album => DspReplayGainMode::Album,
                };

                let gain_db = if dsp_rg_mode == DspReplayGainMode::Off {
                    None
                } else if let Some(current) = state.engine.current_track() {
                    match phonon_codec::metadata::extract_metadata(&current) {
                        Ok(meta) => meta
                            .replay_gain
                            .as_ref()
                            .map(|rg| match dsp_rg_mode {
                                DspReplayGainMode::Track => rg.track_gain,
                                DspReplayGainMode::Album => rg.album_gain,
                                DspReplayGainMode::Off => None,
                            })
                            .flatten(),
                        Err(_) => None,
                    }
                } else {
                    None
                };
                let mut dsp = state.dsp_chain.lock().unwrap();
                if let Some(proc) = dsp.find_mut("replaygain") {
                    if let Some(rg) = proc
                        .as_any_mut()
                        .downcast_mut::<phonon_core::dsp::ReplayGainProcessor>()
                    {
                        rg.configure(dsp_rg_mode, gain_db.unwrap_or(0.0));
                    }
                }
                return Ok(());
            }
            _ => return Ok(()),
        }
    }

    // ── Wasm：沿用原有持久化路径 ──────────────────────────────
    state.plugin_runtime.set_plugin_config(&name, &config);
    Ok(())
}

/// Call a custom command on a loaded WASM plugin.
///
/// The plugin must export `plugin_command`. The input and output are JSON strings.
/// This is intended for non-real-time operations (computation, analysis, etc.).
#[tauri::command]
pub fn call_plugin_command(
    state: State<'_, AppState>,
    plugin_name: String,
    command: String,
    input_json: String,
) -> Result<String, String> {
    state
        .plugin_runtime
        .call_plugin_command(&plugin_name, &command, &input_json)
        .map_err(|e| e.to_string())
}

/// 在 DSP 链中按插件名开关启用状态。
///
/// · **Builtin**：按 name 前缀直接命中的 DSP id（5 个固定）
/// · **Wasm**：遍历 DSP 链上所有处理器，`DspProcessor::name()` 恰好返回
///   `add_wasm_dsp(name, …)` 注册时传入的插件名，无需 downcast 到私有字段。
///   若插件类型为非 DSP（如 TimeStretch / AudioDecoder）则返回明确错误。
#[tauri::command]
pub fn toggle_plugin_in_dsp(
    state: State<'_, AppState>,
    name: String,
    enabled: bool,
) -> Result<(), String> {
    let builtin_id: Option<&str> = match name.as_str() {
        "phonon_equalizer" => Some("equalizer"),
        "phonon_smart_effect" => Some("smart_effect"),
        "phonon_surround_sound" => Some("surround_sound"),
        "phonon_downmix" => Some("downmix"),
        "phonon_replaygain" => Some("replaygain"),
        _ => None,
    };

    if let Some(id) = builtin_id {
        let mut chain = state.dsp_chain.lock().unwrap();
        if chain.find_mut(id).is_none() {
            return Err(format!(
                "Builtin DSP processor '{}' not registered in chain",
                id
            ));
        }
        chain.set_enabled(id, enabled);
        log::info!("[builtin:{id}] set_enabled = {enabled}");
        return Ok(());
    }

    // ── Wasm：通过公共 trait 方法 name() 匹配 ─────────────
    // 一次短锁：用 list_processors().id 走 find_mut(...) → proc.name() == name → set_enabled。
    let ids: Vec<String> = state
        .dsp_chain
        .lock()
        .unwrap()
        .list_processors()
        .into_iter()
        .map(|p| p.id)
        .collect();
    let mut chain = state.dsp_chain.lock().unwrap();
    for id in ids {
        let matched = {
            let proc = chain
                .find_mut(&id)
                .expect("just got id from list_processors, should still exist");
            proc.name() == name
        };
        if matched {
            chain.set_enabled(&id, enabled);
            log::info!("[wasm plugin {name}] (id={id}) set_enabled = {enabled}");
            return Ok(());
        }
    }

    // 找不到这个 name 对应的 WasmDspProcessor → 区分插件存在但不是 DSP 的情况
    let plugin_list = state.plugin_runtime.list_plugins();
    if let Some(info) = plugin_list.iter().find(|p| p.manifest.name == name) {
        use phonon_plugin::PluginType;
        match info.manifest.plugin_type {
            PluginType::DspProcessor => Err(format!(
                "Wasm DSP plugin '{}' not yet added to DSP chain. Call `add_wasm_dsp` first.",
                name
            )),
            PluginType::TimeStretch => Err(format!(
                "Plugin '{}' is a TimeStretch factory (not a DSP processor), cannot toggle_in_dsp.",
                name
            )),
            PluginType::SourceProvider => Err(format!(
                "Plugin '{}' is a SourceProvider (not a DSP processor), cannot toggle_in_dsp.",
                name
            )),
            PluginType::AudioDecoder => Err(format!(
                "Plugin '{}' is an AudioDecoder (not a DSP processor), cannot toggle_in_dsp.",
                name
            )),
            PluginType::Visualizer => Err(format!(
                "Plugin '{}' is a Visualizer (not a DSP processor), cannot toggle_in_dsp.",
                name
            )),
        }
    } else {
        Err(format!("Unknown plugin: {}", name))
    }
}

// ── Time-Stretch Plugin ───────────────────────────────────────

#[tauri::command]
pub fn load_time_stretch_plugin(
    state: State<'_, AppState>,
    file_path: String,
) -> Result<(), String> {
    // Load the WASM plugin first (by file path)
    state
        .plugin_runtime
        .load(&file_path)
        .map_err(|e| e.to_string())?;
    // Get the plugin name from the loaded state
    let name = {
        let plugins = state.plugin_runtime.list_plugins();
        plugins
            .iter()
            .find(|p| p.file_path == file_path)
            .map(|p| p.manifest.name.clone())
            .ok_or_else(|| format!("Plugin not found: {}", file_path))?
    };
    // Create the factory and store in engine
    let factory = state
        .plugin_runtime
        .create_time_stretch_factory(&name)
        .map_err(|e| e.to_string())?;
    state.engine.set_time_stretch_factory(Some(factory));
    log::info!("TimeStretch plugin '{}' loaded and activated", name);
    Ok(())
}

#[tauri::command]
pub fn unload_time_stretch_plugin(
    state: State<'_, AppState>,
    file_path: String,
) -> Result<(), String> {
    state.engine.set_time_stretch_factory(None);
    state
        .plugin_runtime
        .unload(&file_path)
        .map_err(|e| e.to_string())?;
    log::info!("TimeStretch plugin unloaded: {}", file_path);
    Ok(())
}

// ── Settings ──────────────────────────────────────────────────

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    Ok(state.settings.lock().unwrap().clone())
}

#[tauri::command]
pub fn take_settings_corrupt_notice(state: State<'_, AppState>) -> Option<String> {
    state.settings_corrupt_notice.lock().unwrap().take()
}

#[tauri::command]
pub fn update_settings(state: State<'_, AppState>, settings: AppSettings) -> Result<(), String> {
    // Apply buffer size
    state.engine.set_buffer_size(settings.buffer_size);
    // Apply resampler quality
    let quality = match settings.resampler_quality.as_str() {
        "Fast" => phonon_core::ResamplerQuality::Fast,
        "Balanced" => phonon_core::ResamplerQuality::Balanced,
        "High" => phonon_core::ResamplerQuality::High,
        _ => phonon_core::ResamplerQuality::Balanced,
    };
    state.engine.set_resampler_quality(quality);
    // Apply bit depth
    state.engine.set_bit_depth(settings.bit_depth);
    // Apply output sample rate
    state
        .engine
        .set_output_sample_rate(settings.output_sample_rate);
    // Apply DSD mode
    state
        .engine
        .set_dsd_mode(dsd_mode_from_u8(settings.dsd_mode));
    // Apply exclusive mode + volume mode — these were previously persisted
    // but never applied here, so they only took effect after an app restart.
    state.engine.set_exclusive(settings.exclusive_mode);
    state
        .device_manager
        .lock()
        .unwrap()
        .set_exclusive_mode(settings.exclusive_mode);
    // VolumeMode (state) is Copy; map to the core VolumeMode for set_mode.
    let vm = match settings.volume_mode {
        crate::state::VolumeMode::Hardware => phonon_core::volume::VolumeMode::Hardware,
        crate::state::VolumeMode::Software => phonon_core::volume::VolumeMode::Software,
    };
    state.volume.lock().unwrap().set_mode(vm);
    // Apply debug log
    if settings.debug_log {
        log::set_max_level(log::LevelFilter::Debug);
    } else {
        log::set_max_level(log::LevelFilter::Info);
    }
    // Extract time-stretch mode before moving settings into the Mutex.
    let ts_mode = settings.dsp.time_stretch_mode;
    *state.settings.lock().unwrap() = settings;

    // ── C: time-stretch mode change → swap built-in factory ──
    // 只在没有插件 time-stretch 工厂时替换内置工厂。
    // 如果插件已加载，内置工厂被忽略，UI 切换不影响状态。
    // NOTE: `set_time_stretch_factory` 内部已经将 `speed_changed` 置 true，
    // 触发解码线程在下一帧重建 stretcher，因此这里不需要再访问私有字段。
    {
        let has_plugin_factory = state.engine.get_time_stretch_factory().is_some();
        if !has_plugin_factory {
            let new_factory = phonon_core::engine::time_stretch_factory(ts_mode);
            state.engine.set_time_stretch_factory(Some(new_factory));
            log::info!(
                "[update_settings] built-in time-stretch mode = {:?}",
                ts_mode
            );
        } else {
            log::info!(
                "[update_settings] plugin time-stretch loaded; ignoring UI mode = {:?}",
                ts_mode
            );
        }
    }

    // Persist the full AppSettings to dedicated settings.json (atomic write).
    let settings_snapshot = state.settings.lock().unwrap().clone();
    if let Err(e) = save_settings_sync(&settings_snapshot) {
        log::error!("[update_settings] save_settings_sync failed: {}", e);
    }

    Ok(())
}

// ── Buffer Size ───────────────────────────────────────────────

#[tauri::command]
pub fn set_buffer_size(state: State<'_, AppState>, size: usize) -> Result<(), String> {
    state.engine.set_buffer_size(size);
    persist_settings_after(&state, |s| s.buffer_size = size)?;
    log::info!("Buffer size set to {} samples", size);
    Ok(())
}

#[tauri::command]
pub fn get_buffer_size(state: State<'_, AppState>) -> Result<usize, String> {
    Ok(state.engine.buffer_size())
}

// ── Resampler Quality ─────────────────────────────────────────

#[tauri::command]
pub fn set_resampler_quality(state: State<'_, AppState>, quality: String) -> Result<(), String> {
    let q = match quality.as_str() {
        "Fast" => phonon_core::ResamplerQuality::Fast,
        "Balanced" => phonon_core::ResamplerQuality::Balanced,
        "High" => phonon_core::ResamplerQuality::High,
        _ => return Err(format!("Invalid quality: {}", quality)),
    };
    state.engine.set_resampler_quality(q);
    persist_settings_after(&state, |s| s.resampler_quality = quality)?;
    log::info!("Resampler quality set to {:?}", q);
    Ok(())
}

#[tauri::command]
pub fn get_resampler_quality(state: State<'_, AppState>) -> Result<String, String> {
    Ok(format!("{:?}", state.engine.resampler_quality()))
}

// ── Theme ─────────────────────────────────────────────────────

#[tauri::command]
pub fn set_theme(state: State<'_, AppState>, theme: String) -> Result<(), String> {
    persist_settings_after(&state, |s| s.theme = theme)
}

#[tauri::command]
pub fn get_theme(state: State<'_, AppState>) -> Result<String, String> {
    Ok(state.settings.lock().unwrap().theme.clone())
}

// ── Debug Log ─────────────────────────────────────────────────

#[tauri::command]
pub fn set_debug_log(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    persist_settings_after(&state, |s| s.debug_log = enabled)?;
    if enabled {
        log::set_max_level(log::LevelFilter::Debug);
    } else {
        log::set_max_level(log::LevelFilter::Info);
    }
    log::info!("Debug logging: {}", if enabled { "ON" } else { "OFF" });
    Ok(())
}

#[tauri::command]
pub fn get_debug_log(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.settings.lock().unwrap().debug_log)
}

#[tauri::command]
pub fn open_log_dir(app: tauri::AppHandle) -> Result<(), String> {
    let log_dir = app.path().app_log_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&log_dir).map_err(|e| e.to_string())?;
    open_path_in_explorer(&app, &log_dir.to_string_lossy())
}

// ── Lyrics API URL ────────────────────────────────────────────

#[tauri::command]
pub fn get_lyrics_api_url(state: State<'_, AppState>) -> Result<String, String> {
    Ok(state.settings.lock().unwrap().lyrics_api_url.clone())
}

#[tauri::command]
pub fn set_lyrics_api_url(state: State<'_, AppState>, url: String) -> Result<(), String> {
    persist_settings_after(&state, |s| s.lyrics_api_url = url)
}

// ── Startup Behavior ──────────────────────────────────────────

#[tauri::command]
pub fn set_startup_behavior(state: State<'_, AppState>, behavior: String) -> Result<(), String> {
    persist_settings_after(&state, |s| s.startup_behavior = behavior)
}

#[tauri::command]
pub fn get_startup_behavior(state: State<'_, AppState>) -> Result<String, String> {
    Ok(state.settings.lock().unwrap().startup_behavior.clone())
}

// ── Session Save/Load ─────────────────────────────────────────

#[tauri::command]
pub fn save_session(state: State<'_, AppState>) -> Result<(), String> {
    let settings = state.settings.lock().unwrap();
    if settings.startup_behavior != "Remember" {
        return Ok(());
    }
    drop(settings);

    let queue = state.engine.queue_snapshot();
    // Don't save empty queue to avoid overwriting a valid session
    if queue.is_empty() {
        return Ok(());
    }
    let position = state.engine.position();
    let queue_idx = state.engine.queue_index();

    // Snapshot enabled DSP processor IDs so their enable state survives restart
    let dsp_enabled: Vec<String> = state
        .dsp_chain
        .lock()
        .unwrap()
        .list_processors()
        .iter()
        .filter(|p| p.enabled)
        .map(|p| p.id.clone())
        .collect();

    // Snapshot smart effect mode
    let smart_effect_mode = {
        let mut chain = state.dsp_chain.lock().unwrap();
        let mut mode = "off".to_string();
        if let Some(proc) = chain.find_mut("smart_effect") {
            if let Some(se) = proc
                .as_any_mut()
                .downcast_mut::<phonon_core::dsp::SmartEffectProcessor>()
            {
                mode = se.mode().as_str().to_string();
            }
        }
        mode
    };

    // Snapshot surround sound mode
    let surround_sound_mode = {
        let mut chain = state.dsp_chain.lock().unwrap();
        let mut mode = "off".to_string();
        if let Some(proc) = chain.find_mut("surround_sound") {
            if let Some(ss) = proc
                .as_any_mut()
                .downcast_mut::<phonon_core::dsp::SurroundSoundProcessor>()
            {
                mode = ss.mode().as_str().to_string();
            }
        }
        mode
    };

    // Snapshot WASM DSP plugins so they can be re-added on restart
    let wasm_dsp_plugins: Vec<serde_json::Value> = state
        .dsp_chain
        .lock()
        .unwrap()
        .list_processors()
        .iter()
        .filter(|p| p.id.starts_with("wasm_"))
        .map(|p| {
            serde_json::json!({
                "name": p.name,
                "id": p.id,
                "latency": p.latency,
            })
        })
        .collect();

    // NOTE: settings are no longer snapshotted into session.json — they live
    // in the dedicated settings.json (see save_settings_sync / load_settings_sync).
    // Legacy session.json "settings" object is still honored on load (load_session
    // does a selective merge) for backward compatibility with old builds.

    let session = serde_json::json!({
        "queue": queue,
        "position": position,
        "queue_index": queue_idx,
        "playback_mode": format!("{:?}", state.engine.get_playback_mode()),
        "sw_volume": state.volume.lock().unwrap().get_sw_level(),
        "hw_volume": state.volume.lock().unwrap().get_hw_level(),
        "active_playlist": state.active_playlist.lock().unwrap().clone(),
        "playlists": state.playlists.lock().unwrap().clone(),
        "playlist_order": state.playlist_order.lock().unwrap().clone(),
        "dsp_enabled": dsp_enabled,
        "wasm_dsp_plugins": wasm_dsp_plugins,
        "smart_effect_mode": smart_effect_mode,
        "surround_sound_mode": surround_sound_mode,
        "synced_folders": state.synced_folders.lock().unwrap().clone(),
    });

    let session_path = get_session_path()?;
    if let Some(parent) = session_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create session dir: {}", e))?;
    }
    // Atomic write: tmp file + rename — prevents corrupt session.json on crash
    use std::io::Write;
    let tmp = session_path.with_extension("json.tmp");
    let json = serde_json::to_string(&session).map_err(|e| e.to_string())?;
    let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    f.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
    f.flush().map_err(|e| e.to_string())?;
    drop(f);
    std::fs::rename(&tmp, &session_path).map_err(|e| format!("Failed to save session: {}", e))?;
    log::debug!("Session saved to {}", session_path.display());
    Ok(())
}

/// Load and restore session directly in the engine (called once at startup).
/// This avoids React StrictMode double-invocation issues.
#[tauri::command]
pub fn load_session(state: State<'_, AppState>) -> Result<(), String> {
    // Clear the frontend_reloading flag on every mount — the new frontend
    // is alive and ready to receive events. This is called early in the
    // init sequence so the event-forwarding thread resumes ASAP.
    state
        .frontend_reloading
        .store(false, std::sync::atomic::Ordering::SeqCst);

    // Prevent concurrent/duplicate session loads (e.g. from React StrictMode)
    {
        let mut loaded = state.session_loaded.lock().unwrap();
        if *loaded {
            return Ok(());
        }
        *loaded = true;
    }

    let settings = state.settings.lock().unwrap();
    if settings.startup_behavior != "Remember" {
        return Ok(());
    }
    drop(settings);

    let session_path = get_session_path()?;
    if !session_path.exists() {
        return Ok(());
    }
    let content = std::fs::read_to_string(&session_path)
        .map_err(|e| format!("Failed to read session: {}", e))?;
    let session: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("Failed to parse session: {}", e))?;

    let queue = session["queue"].as_array();
    let _position = session["position"].as_f64().unwrap_or(0.0);
    let queue_idx = session["queue_index"].as_u64().unwrap_or(0) as usize;

    if let Some(queue) = queue {
        if !queue.is_empty() {
            // Filter out non-audio files (.lrc, .cue, .txt, images, etc.)
            // from the saved queue. Old sessions may contain these entries
            // before the extension filter was added.
            let raw_count = queue.len();
            let paths: Vec<String> = queue
                .iter()
                .filter_map(|item| item["path"].as_str().map(String::from))
                .filter(|p| is_audio_path(p))
                .collect();
            let filtered_out = raw_count - paths.len();
            if filtered_out > 0 {
                log::info!(
                    "Session queue: filtered out {} non-audio file(s) ({} → {})",
                    filtered_out,
                    raw_count,
                    paths.len()
                );
            }
            if !paths.is_empty() {
                state.engine.clear_queue();
                // enqueue_fast: no per-track decoder probe (O(N) opens on
                // large sessions); durations fill in lazily on play.
                for path in &paths {
                    state.engine.enqueue_fast(path).map_err(|e| e.to_string())?;
                }
                // Set queue index but do NOT auto-play.
                // Clamp to filtered length in case the saved index pointed
                // to a non-audio entry that was removed.
                let safe_idx = queue_idx.min(paths.len().saturating_sub(1));
                state.engine.set_queue_index(safe_idx);
                log::info!(
                    "Session restored: {} tracks, index={} (not auto-playing)",
                    paths.len(),
                    safe_idx
                );
            }
        }
    }

    // Restore playlists from session (if saved)
    if let Some(pl_json) = session.get("playlists") {
        if let Ok(saved_playlists) =
            serde_json::from_value::<HashMap<String, Vec<String>>>(pl_json.clone())
        {
            // Filter out non-audio paths from restored playlists — old sessions
            // may contain .lrc/.cue/.txt entries that were saved before the
            // extension filter was added.
            let filtered: HashMap<String, Vec<String>> = saved_playlists
                .into_iter()
                .map(|(name, paths)| {
                    let cleaned: Vec<String> =
                        paths.into_iter().filter(|p| is_audio_path(p)).collect();
                    (name, cleaned)
                })
                .collect();
            let mut playlists = state.playlists.lock().unwrap();
            *playlists = filtered;
            if let Some(active) = session.get("active_playlist").and_then(|v| v.as_str()) {
                // Don't restore system playlists as active on startup —
                // they need to be re-queried from the DB via switch_system_playlist.
                // Fall back to "Default" so the user's last user playlist isn't lost.
                if is_system_playlist(active) {
                    *state.active_playlist.lock().unwrap() = "Default".to_string();
                } else {
                    *state.active_playlist.lock().unwrap() = active.to_string();
                }
            }
            // Restore playlist display order
            if let Some(order_json) = session.get("playlist_order") {
                if let Ok(saved_order) = serde_json::from_value::<Vec<String>>(order_json.clone()) {
                    *state.playlist_order.lock().unwrap() = saved_order;
                }
            } else {
                // Legacy session: populate order from playlists
                let playlists = state.playlists.lock().unwrap();
                let mut order = state.playlist_order.lock().unwrap();
                for name in playlists.keys() {
                    if !order.contains(name) {
                        order.push(name.clone());
                    }
                }
            }
            log::info!("Playlists restored from session");
        }
    } else {
        // Auto-create "Default" playlist if no playlists exist yet and queue has songs
        let queue = session["queue"].as_array();
        if let Some(queue) = queue {
            if !queue.is_empty() {
                let paths: Vec<String> = queue
                    .iter()
                    .filter_map(|item| item["path"].as_str().map(String::from))
                    .filter(|p| is_audio_path(p))
                    .collect();
                if !paths.is_empty() {
                    let track_count = paths.len();
                    let mut playlists = state.playlists.lock().unwrap();
                    if playlists.is_empty() {
                        playlists.insert("Default".to_string(), paths);
                        state
                            .playlist_order
                            .lock()
                            .unwrap()
                            .push("Default".to_string());
                        *state.active_playlist.lock().unwrap() = "Default".to_string();
                        log::info!(
                            "Auto-created Default playlist from session ({} tracks)",
                            track_count
                        );
                    }
                }
            }
        }
    }

    // ── Settings restore ────────────────────────────────────────
    // Step A: legacy session.json may contain an inline "settings" object —
    // merge its keys on top of the in-memory settings (which were already
    // populated by load_settings_sync at app start). This handles the case
    // where the user ran an old build that only wrote session.json.
    if let Some(session_settings_val) = session.get("settings") {
        let mut current = serde_json::to_value(&*state.settings.lock().unwrap())
            .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new()));
        if let (Some(current_obj), Some(legacy_obj)) =
            (current.as_object_mut(), session_settings_val.as_object())
        {
            for (k, v) in legacy_obj {
                current_obj.insert(k.clone(), v.clone());
            }
        }
        if let Ok(merged) = serde_json::from_value::<crate::state::AppSettings>(current) {
            *state.settings.lock().unwrap() = merged;
            log::info!("Settings merged from legacy session.json");
        } else {
            log::warn!("legacy session.settings merge failed, keeping in-memory settings");
        }
    }

    // Step B: apply engine-level settings from the (now-merged) in-memory
    // settings. This runs every load_session call, regardless of whether
    // session.json contained a settings object — load_settings_sync only
    // restores the struct, it does NOT call engine.set_*, so this is the
    // single place where the engine picks up persisted audio config.
    {
        let settings = state.settings.lock().unwrap();
        state.engine.set_buffer_size(settings.buffer_size);
        let rq = match settings.resampler_quality.as_str() {
            "Fast" => phonon_core::ResamplerQuality::Fast,
            "Balanced" => phonon_core::ResamplerQuality::Balanced,
            "High" => phonon_core::ResamplerQuality::High,
            _ => phonon_core::ResamplerQuality::Balanced,
        };
        state.engine.set_resampler_quality(rq);
        state.engine.set_bit_depth(settings.bit_depth);
        state
            .engine
            .set_output_sample_rate(settings.output_sample_rate);
        state
            .engine
            .set_dsd_mode(dsd_mode_from_u8(settings.dsd_mode));
        if settings.debug_log {
            log::set_max_level(log::LevelFilter::Debug);
        } else {
            log::set_max_level(log::LevelFilter::Info);
        }
        // Apply volume mode
        let vm = match settings.volume_mode {
            crate::state::VolumeMode::Hardware => phonon_core::volume::VolumeMode::Hardware,
            crate::state::VolumeMode::Software => phonon_core::volume::VolumeMode::Software,
        };
        drop(settings); // release the lock before locking volume
        state.volume.lock().unwrap().set_mode(vm);
        log::info!("Engine settings applied from settings");
    }

    // Restore WASM DSP plugins from session (if saved)
    if let Some(wasm_plugins) = session.get("wasm_dsp_plugins").and_then(|v| v.as_array()) {
        for plugin in wasm_plugins {
            if let (Some(name), Some(id), Some(latency)) = (
                plugin.get("name").and_then(|v| v.as_str()),
                plugin.get("id").and_then(|v| v.as_str()),
                plugin.get("latency").and_then(|v| v.as_f64()),
            ) {
                // Check if already in chain
                {
                    let chain = state.dsp_chain.lock().unwrap();
                    if chain.list_processors().iter().any(|p| p.id == id) {
                        continue;
                    }
                }
                // Try to add the WASM DSP plugin
                let result = || -> Result<(), String> {
                    let plugins = state.plugin_runtime.list_plugins();
                    if let Some(info) = plugins.iter().find(|p| p.manifest.name == name) {
                        use phonon_plugin::PluginState;
                        if info.state != PluginState::Loaded {
                            state
                                .plugin_runtime
                                .load(&info.file_path)
                                .map_err(|e| e.to_string())?;
                        }
                        let process_fn = state
                            .plugin_runtime
                            .create_processor(&name)
                            .map_err(|e| e.to_string())?;
                        let processor = phonon_core::WasmDspProcessor::new(
                            name.to_string(),
                            id.to_string(),
                            latency,
                            process_fn,
                        );
                        let mut chain = state.dsp_chain.lock().unwrap();
                        chain.add(Box::new(processor));
                        Ok(())
                    } else {
                        Err(format!("Plugin '{}' not found", name))
                    }
                }();
                if let Err(e) = result {
                    log::warn!("Failed to restore WASM DSP plugin '{}': {}", name, e);
                } else {
                    log::info!("Restored WASM DSP plugin: {}", name);
                }
            }
        }
    }

    // Restore DSP chain enable states from session (if saved)
    if let Some(dsp_arr) = session.get("dsp_enabled").and_then(|v| v.as_array()) {
        let enabled_ids: Vec<String> = dsp_arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        let mut chain = state.dsp_chain.lock().unwrap();
        for info in chain.list_processors() {
            let should_enable = enabled_ids.iter().any(|id| id == &info.id);
            chain.set_enabled(&info.id, should_enable);
        }
        log::info!("DSP enable states restored from session: {:?}", enabled_ids);
    }

    // Restore smart effect mode from session (if saved)
    if let Some(se_mode) = session.get("smart_effect_mode").and_then(|v| v.as_str()) {
        let mut chain = state.dsp_chain.lock().unwrap();
        if let Some(proc) = chain.find_mut("smart_effect") {
            if let Some(se) = proc
                .as_any_mut()
                .downcast_mut::<phonon_core::dsp::SmartEffectProcessor>()
            {
                let mode = phonon_core::dsp::SmartEffectMode::from_str(se_mode);
                se.set_mode(mode);
                log::info!("Smart effect mode restored from session: {}", se_mode);
            }
        }
    }

    // Restore surround sound mode from session (if saved)
    if let Some(ss_mode) = session.get("surround_sound_mode").and_then(|v| v.as_str()) {
        let mut chain = state.dsp_chain.lock().unwrap();
        if let Some(proc) = chain.find_mut("surround_sound") {
            if let Some(ss) = proc
                .as_any_mut()
                .downcast_mut::<phonon_core::dsp::SurroundSoundProcessor>()
            {
                let mode = phonon_core::dsp::SurroundSoundMode::from_str(ss_mode);
                ss.set_mode(mode);
                log::info!("Surround sound mode restored from session: {}", ss_mode);
            }
        }
    }

    // Restore playback mode from session (if saved)
    if let Some(pm_str) = session.get("playback_mode").and_then(|v| v.as_str()) {
        use phonon_core::engine::PlaybackMode;
        let mode = match pm_str {
            "Normal" => PlaybackMode::Normal,
            "RepeatOne" => PlaybackMode::RepeatOne,
            "RepeatAll" => PlaybackMode::RepeatAll,
            _ => PlaybackMode::Normal,
        };
        state.engine.set_playback_mode(mode);
        log::info!("Playback mode restored from session: {:?}", mode);
    }

    // Restore volume level from session (if saved)
    // For Hardware mode, always sync from device instead of using saved level,
    // because HW volume is system-wide and should reflect the current device state.
    #[allow(unused_variables)]
    if let Some(level) = session.get("volume_level").and_then(|v| v.as_f64()) {
        #[cfg(windows)]
        {
            // Sync from the device via the attached controller (no per-call
            // device enumeration).
            if let Ok(mut vc) = state.volume.lock() {
                if vc.mode() == phonon_core::volume::VolumeMode::Hardware {
                    if let Some(v) = vc.sync_from_device() {
                        log::info!("HW volume synced from device: {:.2}", v);
                    }
                }
            }
        }
        #[cfg(not(windows))]
        {
            let vol = level as f32;
            let mut vc = state.volume.lock().unwrap();
            vc.set_volume(vol);
            log::info!("Volume level restored from session: {:.2}", vol);
        }
    }

    // Restore synced folders from session (if saved)
    if let Some(synced) = session.get("synced_folders").and_then(|v| v.as_object()) {
        if !synced.is_empty() {
            log::info!(
                "[folder-sync] Restoring {} synced folder binding(s) from session",
                synced.len()
            );
            let mut guard = state.synced_folders.lock().unwrap();
            for (k, v) in synced {
                if let Some(path) = v.as_str() {
                    guard.insert(k.clone(), path.to_string());
                }
            }
        }
    }

    Ok(())
}

fn get_session_path() -> Result<std::path::PathBuf, String> {
    Ok(data_dir_pub().join("session.json"))
}

// ── Spectrum Settings ─────────────────────────────────────────

#[tauri::command]
pub fn set_spectrum_settings(
    state: State<'_, AppState>,
    smoothing: f32,
    decay: f32,
    fps: u32,
) -> Result<(), String> {
    persist_settings_after(&state, |s| {
        s.spectrum_smoothing = smoothing.clamp(0.01, 0.5);
        s.spectrum_decay = decay.clamp(0.5, 0.99);
        s.spectrum_fps = fps.clamp(5, 60);
    })
}

#[tauri::command]
pub fn get_spectrum_settings(state: State<'_, AppState>) -> Result<(f32, f32, u32), String> {
    let s = state.settings.lock().unwrap();
    Ok((s.spectrum_smoothing, s.spectrum_decay, s.spectrum_fps))
}

#[tauri::command]
pub fn set_replay_gain(state: State<'_, AppState>, mode: String) -> Result<(), String> {
    let rg_mode = match mode.as_str() {
        "Off" => ReplayGainMode::Off,
        "Track" => ReplayGainMode::Track,
        "Album" => ReplayGainMode::Album,
        _ => return Err(format!("Invalid ReplayGain mode: {}", mode)),
    };

    // Update settings
    persist_settings_after(&state, |s| s.replay_gain_mode = rg_mode)?;

    // Configure the ReplayGain processor in the DSP chain
    let dsp_rg_mode = match rg_mode {
        ReplayGainMode::Off => DspReplayGainMode::Off,
        ReplayGainMode::Track => DspReplayGainMode::Track,
        ReplayGainMode::Album => DspReplayGainMode::Album,
    };

    // Read ReplayGain metadata from the current track (skip for Off)
    let gain_db = if dsp_rg_mode == DspReplayGainMode::Off {
        None
    } else if let Some(current) = state.engine.current_track() {
        match phonon_codec::metadata::extract_metadata(&current) {
            Ok(meta) => {
                if let Some(ref rg_data) = meta.replay_gain {
                    match dsp_rg_mode {
                        DspReplayGainMode::Track => rg_data.track_gain,
                        DspReplayGainMode::Album => rg_data.album_gain,
                        DspReplayGainMode::Off => None,
                    }
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    } else {
        None
    };

    let mut dsp = state.dsp_chain.lock().unwrap();
    if let Some(proc) = dsp.find_mut("replaygain") {
        if let Some(rg) = proc
            .as_any_mut()
            .downcast_mut::<phonon_core::dsp::ReplayGainProcessor>()
        {
            rg.configure(dsp_rg_mode, gain_db.unwrap_or(0.0));
        }
    }
    Ok(())
}

#[tauri::command]
pub fn get_replay_gain(state: State<'_, AppState>) -> Result<String, String> {
    let mode = state.settings.lock().unwrap().replay_gain_mode;
    Ok(format!("{:?}", mode))
}

#[tauri::command]
pub fn set_volume_mode(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    mode: String,
) -> Result<(), String> {
    let vm = match mode.as_str() {
        "Hardware" => VolumeMode::Hardware,
        "Software" => VolumeMode::Software,
        _ => return Err(format!("Invalid volume mode: {}", mode)),
    };

    // Update settings
    persist_settings_after(&state, |s| s.volume_mode = vm)?;

    // Update VolumeControl — set mode first, then sync HW volume
    let core_vm = match vm {
        VolumeMode::Hardware => CoreVolumeMode::Hardware,
        VolumeMode::Software => CoreVolumeMode::Software,
    };
    state.volume.lock().unwrap().set_mode(core_vm);

    // Sync HW volume from device when switching to Hardware mode.
    // Lock device_manager first, then volume — matching the lock order
    // used by sync_volume_from_device and load_session to avoid deadlock
    // with start_playback (which holds device_manager while audio callback
    // locks volume).
    #[cfg(windows)]
    if core_vm == CoreVolumeMode::Hardware {
        ensure_com_initialized();
        // Use the polling task's attached controller — no per-call device
        // enumeration.
        if let Ok(mut vc) = state.volume.lock() {
            if let Some(v) = vc.sync_from_device() {
                log::info!("HW volume synced on mode switch: {:.2}", v);
            }
        }
    }

    // Emit current volume level so frontend sliders (PlayerBar) stay in sync
    // after mode switch, regardless of which component triggered the change.
    let level = state.volume.lock().unwrap().level();
    let _ = app.emit("hw-volume", level.clamp(0.0, 1.0));

    Ok(())
}

// ── Exclusive Mode ────────────────────────────────────────────

#[tauri::command]
pub fn get_exclusive_mode(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.device_manager.lock().unwrap().is_exclusive())
}

#[tauri::command]
pub fn set_exclusive_mode(enabled: bool, state: State<'_, AppState>) -> Result<(), String> {
    state
        .device_manager
        .lock()
        .unwrap()
        .set_exclusive_mode(enabled);
    state.engine.set_exclusive(enabled);
    log::info!("Exclusive mode: {}", if enabled { "ON" } else { "OFF" });
    // Persist to settings.json
    if let Err(e) = persist_settings_after(&state, |s| s.exclusive_mode = enabled) {
        log::error!("[set_exclusive_mode] failed to persist: {}", e);
    }

    // Force output stream recreation so the new mode takes effect
    state.engine.set_output_running(false);

    // If currently playing, restart playback to apply the new mode
    let was_playing = {
        let s = state.engine.state();
        s == PlaybackState::Playing || s == PlaybackState::Paused
    };
    if was_playing {
        let pos = state.engine.position();
        let queue_idx = state.engine.queue_index();
        state.engine.stop();
        // Restore queue index (stop() no longer resets it, but be safe)
        state.engine.set_queue_index(queue_idx);
        if let Err(e) = state.engine.start_playback() {
            log::error!("Failed to restart playback after exclusive mode change: {e}");
            // Revert exclusive mode flags
            state.engine.set_exclusive(!enabled);
            state
                .device_manager
                .lock()
                .unwrap()
                .set_exclusive_mode(!enabled);
            return Err(e.to_string());
        }
        if pos > 0.0 {
            state.engine.seek(pos);
        }
    }

    // If exclusive mode was requested but failed to activate, stop playback
    // and return an error so the frontend toggle reverts.
    if enabled && !state.device_manager.lock().unwrap().is_exclusive() {
        let queue_idx = state.engine.queue_index();
        state.engine.set_exclusive(false);
        state
            .device_manager
            .lock()
            .unwrap()
            .set_exclusive_mode(false);
        state.engine.stop();
        state.engine.set_queue_index(queue_idx);
        return Err("Exclusive mode not supported by this device".into());
    }

    Ok(())
}

// ── Reset to Defaults ─────────────────────────────────────────

#[tauri::command]
pub fn reset_to_defaults(state: State<'_, AppState>) -> Result<(), String> {
    let defaults = AppSettings::default();
    let settings_snapshot = {
        let mut settings = state.settings.lock().unwrap();
        // Preserve appearance-related settings
        let preserved_theme = settings.theme.clone();
        let preserved_spectrum_colorscheme = settings.spectrum_colorscheme.clone();
        let preserved_spectrum_smoothing = settings.spectrum_smoothing;
        let preserved_spectrum_decay = settings.spectrum_decay;
        let preserved_spectrum_fps = settings.spectrum_fps;
        let preserved_language = settings.language.clone();
        *settings = defaults.clone();
        settings.theme = preserved_theme;
        settings.spectrum_colorscheme = preserved_spectrum_colorscheme;
        settings.spectrum_smoothing = preserved_spectrum_smoothing;
        settings.spectrum_decay = preserved_spectrum_decay;
        settings.spectrum_fps = preserved_spectrum_fps;
        settings.language = preserved_language;
        settings.clone()
    };
    if let Err(e) = save_settings_sync(&settings_snapshot) {
        log::error!("[reset_to_defaults] save_settings_sync failed: {}", e);
    }

    // Apply engine-level settings
    state.engine.set_buffer_size(defaults.buffer_size);
    let rq = match defaults.resampler_quality.as_str() {
        "Fast" => phonon_core::ResamplerQuality::Fast,
        "Balanced" => phonon_core::ResamplerQuality::Balanced,
        "High" => phonon_core::ResamplerQuality::High,
        _ => phonon_core::ResamplerQuality::Balanced,
    };
    state.engine.set_resampler_quality(rq);
    state.engine.set_bit_depth(defaults.bit_depth);
    state
        .engine
        .set_output_sample_rate(defaults.output_sample_rate);
    state
        .engine
        .set_dsd_mode(dsd_mode_from_u8(defaults.dsd_mode));
    if defaults.debug_log {
        log::set_max_level(log::LevelFilter::Debug);
    } else {
        log::set_max_level(log::LevelFilter::Info);
    }

    // Apply volume mode
    let vm = match defaults.volume_mode {
        crate::state::VolumeMode::Hardware => phonon_core::volume::VolumeMode::Hardware,
        crate::state::VolumeMode::Software => phonon_core::volume::VolumeMode::Software,
    };
    state.volume.lock().unwrap().set_mode(vm);

    log::info!("Settings reset to defaults (preserving appearance) and applied");
    Ok(())
}

// ── Hotplug Monitor ───────────────────────────────────────────

#[tauri::command]
pub fn start_hotplug_monitor(
    _app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.start_hotplug_monitor();
    Ok(())
}

// ── User Data Cleanup ─────────────────────────────────────────
/// Clean user data with three modes:
/// - `"keep_tracks"` — keep playlists / favorites / recent, delete settings, session state, DSP, plugins, ReplayGain
/// - `"clear_all"` — delete everything
/// - `"keep_all"` — no-op (just confirms data dir path)
#[tauri::command]
pub fn clean_user_data(state: State<'_, AppState>, mode: String) -> Result<String, String> {
    let data_dir = data_dir_pub();
    if !data_dir.exists() {
        return Err("Data directory does not exist".to_string());
    }

    match mode.as_str() {
        "keep_all" => Ok(format!("Data dir: {}", data_dir.display())),
        "keep_tracks" => {
            // 1. Save playlists from current memory state as a minimal session
            let playlists_snapshot = state.playlists.lock().unwrap().clone();
            let order_snapshot = state.playlist_order.lock().unwrap().clone();
            let active_snapshot = state.active_playlist.lock().unwrap().clone();

            // 2. Stop playback and clear engine state
            state.engine.stop();
            state.engine.clear_queue();

            // 3. Reset DSP chain state (disable all, keep processors)
            {
                let mut chain = state.dsp_chain.lock().unwrap();
                for info in chain.list_processors() {
                    chain.set_enabled(&info.id, false);
                }
            }

            // 4. Reset settings to defaults (in-memory)
            let defaults = crate::state::AppSettings::default();
            {
                let mut settings = state.settings.lock().unwrap();
                *settings = defaults.clone();
            }

            // 5. Delete everything in data_dir except we'll write a minimal session back
            let items = std::fs::read_dir(&data_dir)
                .map_err(|e| format!("Failed to read data dir: {}", e))?;
            for entry in items.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                // Skip the plugins dir — user installs plugins manually, don't delete them
                if name_str == "plugins" {
                    continue;
                }
                if path.is_dir() {
                    let _ = std::fs::remove_dir_all(&path);
                } else {
                    let _ = std::fs::remove_file(&path);
                }
            }

            // 6. Write back minimal session with only playlists
            let session_path = data_dir.join("session.json");
            let minimal = serde_json::json!({
                "queue": [],
                "position": 0.0,
                "queue_index": 0,
                "playback_mode": "Normal",
                "active_playlist": active_snapshot,
                "playlists": playlists_snapshot,
                "playlist_order": order_snapshot,
                "dsp_enabled": [],
                "smart_effect_mode": "off",
                "surround_sound_mode": "off",
                "synced_folders": {},
            });
            if let Some(parent) = session_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(
                &session_path,
                serde_json::to_string_pretty(&minimal).unwrap_or_default(),
            )
            .map_err(|e| format!("Failed to write minimal session: {}", e))?;

            // 7. Write back default settings
            let _ = save_settings_sync(&defaults);

            // 8. Re-apply engine-level defaults
            state.engine.set_buffer_size(defaults.buffer_size);
            let rq = match defaults.resampler_quality.as_str() {
                "Fast" => phonon_core::ResamplerQuality::Fast,
                "Balanced" => phonon_core::ResamplerQuality::Balanced,
                "High" => phonon_core::ResamplerQuality::High,
                _ => phonon_core::ResamplerQuality::Balanced,
            };
            state.engine.set_resampler_quality(rq);
            state.engine.set_bit_depth(defaults.bit_depth);
            state
                .engine
                .set_output_sample_rate(defaults.output_sample_rate);
            state
                .engine
                .set_dsd_mode(dsd_mode_from_u8(defaults.dsd_mode));
            state.engine.set_exclusive(defaults.exclusive_mode);
            if defaults.debug_log {
                log::set_max_level(log::LevelFilter::Debug);
            } else {
                log::set_max_level(log::LevelFilter::Info);
            }
            let vm = match defaults.volume_mode {
                crate::state::VolumeMode::Hardware => phonon_core::volume::VolumeMode::Hardware,
                crate::state::VolumeMode::Software => phonon_core::volume::VolumeMode::Software,
            };
            state.volume.lock().unwrap().set_mode(vm);

            log::info!(
                "[clean_user_data] mode=keep_tracks — playlists preserved, all other data cleared"
            );
            Ok(format!("Kept playlists only — {}", data_dir.display()))
        }
        "clear_all" => {
            // Stop playback
            state.engine.stop();
            state.engine.clear_queue();

            // Delete everything in data_dir (including playlists, settings, etc.)
            let items = std::fs::read_dir(&data_dir)
                .map_err(|e| format!("Failed to read data dir: {}", e))?;
            for entry in items.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let _ = std::fs::remove_dir_all(&path);
                } else {
                    let _ = std::fs::remove_file(&path);
                }
            }

            // Reset in-memory state to defaults
            let defaults = crate::state::AppSettings::default();
            {
                let mut settings = state.settings.lock().unwrap();
                *settings = defaults.clone();
            }
            {
                let mut playlists = state.playlists.lock().unwrap();
                playlists.clear();
                playlists.insert("Default".to_string(), Vec::new());
            }
            *state.active_playlist.lock().unwrap() = "Default".to_string();
            *state.playlist_order.lock().unwrap() = vec!["Default".to_string()];
            {
                let mut chain = state.dsp_chain.lock().unwrap();
                for info in chain.list_processors() {
                    chain.set_enabled(&info.id, false);
                }
            }

            // Write back default settings so next startup is clean
            let _ = save_settings_sync(&defaults);

            log::info!("[clean_user_data] mode=clear_all — all user data cleared");
            Ok(format!("All data cleared — {}", data_dir.display()))
        }
        _ => Err(format!("Unknown clean mode: {}", mode)),
    }
}

// ── Metadata ──────────────────────────────────────────────────

#[derive(Serialize)]
pub struct SearchResultJson {
    pub uri: String,
    pub name: String,
}

#[tauri::command]
pub fn scan_folder(path: String) -> Result<Vec<SearchResultJson>, String> {
    let source = phonon_source::LocalFileSource::new();
    let results = source.search(&path).map_err(|e| e.to_string())?;
    /// Extensions that must NEVER appear in scan results.
    const NON_AUDIO_EXTS: &[&str] = &[
        "lrc", "txt", "srt", "ass", "ssa", "vtt", "jpg", "jpeg", "png", "gif", "bmp", "webp",
        "svg", "pdf", "doc", "docx", "xls", "xlsx", "zip", "rar", "7z", "log", "cue",
    ];
    Ok(results
        .into_iter()
        .filter(|r| {
            // Final safety net: reject any URI whose extension is in the
            // non-audio blocklist, even if it slipped through local.rs.
            let ext = std::path::Path::new(&r.uri)
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .unwrap_or_default();
            !NON_AUDIO_EXTS.contains(&ext.as_str())
        })
        .map(|r| SearchResultJson {
            uri: r.uri,
            name: r.name,
        })
        .collect())
}

// ── Folder Sync (per-playlist auto-sync) ─────────────────────

/// Audio file extensions that trigger folder-sync events.
const WATCH_AUDIO_EXTS: &[&str] = &[
    "flac", "alac", "m4a", "wav", "wave", "mp3", "aac", "dsf", "dff",
];

/// Bind a folder to a playlist for auto-sync. Each playlist can bind at most
/// one folder; if a folder was already bound, it is replaced. When audio files
/// in the folder change externally, only the bound playlist is updated.
#[tauri::command]
pub fn sync_folder(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    playlist: String,
    path: String,
) -> Result<(), String> {
    use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

    // If this playlist was already bound to a different folder, unwatch the
    // old one — but only if no other playlist still uses it.
    {
        let synced = state.synced_folders.lock().unwrap();
        if let Some(old_path) = synced.get(&playlist) {
            if old_path != &path {
                let old_path = old_path.clone();
                drop(synced);
                // Check if any other playlist still uses the old folder
                let still_used = {
                    let synced = state.synced_folders.lock().unwrap();
                    synced.values().filter(|v| **v == old_path).count() > 1
                };
                if !still_used {
                    let mut watcher_guard = state.folder_watcher.lock().unwrap();
                    if let Some(ref mut watcher) = *watcher_guard {
                        let _ = watcher.unwatch(std::path::Path::new(&old_path));
                    }
                }
            } else {
                return Ok(()); // Same folder already bound
            }
        }
    }

    // Insert/update the binding (one playlist → one folder).
    // A folder CAN be bound to multiple playlists, so we don't remove
    // other playlists' bindings to the same folder.
    {
        let mut synced = state.synced_folders.lock().unwrap();
        synced.insert(playlist.clone(), path.clone());
    }

    // Create watcher if it doesn't exist yet
    let mut watcher_guard = state.folder_watcher.lock().unwrap();
    if watcher_guard.is_none() {
        let app_for_cb = app.clone();
        let watcher = RecommendedWatcher::new(
            move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
                    if !matches!(event.kind, EventKind::Create(_) | EventKind::Remove(_)) {
                        return;
                    }
                    let has_audio = event.paths.iter().any(|p| {
                        p.extension()
                            .and_then(|e| e.to_str())
                            .map(|e| WATCH_AUDIO_EXTS.contains(&e.to_lowercase().as_str()))
                            .unwrap_or(false)
                    });
                    if has_audio {
                        let folder_hint = event
                            .paths
                            .first()
                            .and_then(|p| p.to_str())
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        log::debug!("[folder-sync] audio change in '{}'", folder_hint);
                        let _ = app_for_cb.emit("folder-changed", folder_hint);
                    }
                }
            },
            Config::default(),
        )
        .map_err(|e| format!("Failed to create folder watcher: {}", e))?;
        *watcher_guard = Some(watcher);
    }

    // Watch the path recursively. If another playlist already bound this
    // folder, the watcher is already watching it — ignore the duplicate error.
    if let Some(ref mut watcher) = *watcher_guard {
        if let Err(e) = watcher.watch(std::path::Path::new(&path), RecursiveMode::Recursive) {
            // Check if this folder is already bound by another playlist
            let already_watched = {
                let synced = state.synced_folders.lock().unwrap();
                synced.values().filter(|v| **v == path).count() > 1
            };
            if !already_watched {
                return Err(format!("Failed to watch folder '{}': {}", path, e));
            }
        }
    }
    drop(watcher_guard);

    log::info!("[folder-sync] Playlist '{}' synced to: {}", playlist, path);
    Ok(())
}

/// Unbind a playlist's synced folder and stop watching it (if no other playlist uses it).
#[tauri::command]
pub fn unsync_folder(state: State<'_, AppState>, playlist: String) -> Result<(), String> {
    use notify::Watcher;

    let removed_path = {
        let mut synced = state.synced_folders.lock().unwrap();
        synced.remove(&playlist)
    };

    if let Some(ref path) = removed_path {
        // Check if any other playlist still uses this folder
        let still_used = {
            let synced = state.synced_folders.lock().unwrap();
            synced.values().any(|v| v == path)
        };
        if !still_used {
            let mut watcher_guard = state.folder_watcher.lock().unwrap();
            if let Some(ref mut watcher) = *watcher_guard {
                let _ = watcher.unwatch(std::path::Path::new(path));
            }
            let synced = state.synced_folders.lock().unwrap();
            if synced.is_empty() {
                *watcher_guard = None;
            }
        }
        log::info!(
            "[folder-sync] Playlist '{}' unsynced from: {}",
            playlist,
            path
        );
    }

    Ok(())
}

/// Get all playlist->folder sync bindings.
#[tauri::command]
pub fn get_synced_folders(
    state: State<'_, AppState>,
) -> Result<std::collections::HashMap<String, String>, String> {
    let guard = state.synced_folders.lock().unwrap();
    Ok(guard.clone())
}

/// Rescan the folder bound to `playlist` and sync that playlist's entries:
/// - Add new audio files that appeared on disk but aren't in the playlist
/// - Remove playlist entries whose files no longer exist on disk
/// If the synced playlist is currently active, also update the engine queue.
#[tauri::command]
pub fn sync_folder_for_playlist(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    playlist: String,
) -> Result<serde_json::Value, String> {
    let folder = {
        let synced = state.synced_folders.lock().unwrap();
        synced.get(&playlist).cloned()
    };
    let folder = match folder {
        Some(f) => f,
        None => return Ok(serde_json::json!({"added": 0, "removed": 0})),
    };

    // Scan the bound folder for current audio files
    let mut current_files: std::collections::HashSet<String> = std::collections::HashSet::new();
    let source = phonon_source::LocalFileSource::new();
    if let Ok(results) = source.search(&folder) {
        for r in &results {
            let ext = std::path::Path::new(&r.uri)
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .unwrap_or_default();
            if is_audio_path(&r.uri) || WATCH_AUDIO_EXTS.contains(&ext.as_str()) {
                current_files.insert(r.uri.clone());
            }
        }
    }

    // Get current playlist paths
    let pl_paths: Vec<String> = {
        let playlists = state.playlists.lock().unwrap();
        playlists.get(&playlist).cloned().unwrap_or_default()
    };
    let pl_set: std::collections::HashSet<String> = pl_paths.iter().cloned().collect();

    // Files to add: on disk but not in playlist
    let to_add: Vec<String> = current_files
        .iter()
        .filter(|p| !pl_set.contains(*p))
        .cloned()
        .collect();

    // Files to remove: in playlist but no longer on disk (and under this folder)
    let to_remove: Vec<String> = pl_paths
        .iter()
        .filter(|p| p.starts_with(&folder) && !current_files.contains(*p))
        .cloned()
        .collect();

    // Update the playlist's path list
    {
        let mut playlists = state.playlists.lock().unwrap();
        if let Some(pl) = playlists.get_mut(&playlist) {
            pl.retain(|p| !to_remove.contains(p));
            pl.extend(to_add.iter().cloned());
        }
    }

    // If this playlist is currently active, also sync the engine queue
    let is_active = {
        let active = state.active_playlist.lock().unwrap();
        *active == playlist
    };

    if is_active {
        let snapshot = state.engine.queue_snapshot();

        let mut to_remove_indices: Vec<usize> = Vec::new();
        for (i, item) in snapshot.iter().enumerate() {
            if to_remove.contains(&item.path) {
                to_remove_indices.push(i);
            }
        }
        if !to_remove_indices.is_empty() {
            state.engine.remove_from_queue(&to_remove_indices);
            log::info!(
                "[folder-sync] Removed {} deleted file(s) from queue",
                to_remove_indices.len()
            );
        }

        let mut added = 0usize;
        for path in &to_add {
            match state.engine.enqueue(path) {
                Ok(true) => added += 1,
                Ok(false) => {}
                Err(e) => log::warn!("[folder-sync] Failed to enqueue '{}': {}", path, e),
            }
        }
        if added > 0 {
            log::info!("[folder-sync] Added {} new file(s) to queue", added);
        }

        if !to_remove_indices.is_empty() || added > 0 {
            emit_state_snapshot(&app, &state);
        }
    }

    Ok(serde_json::json!({"added": to_add.len(), "removed": to_remove.len()}))
}

#[derive(Serialize)]
pub struct TrackMetadataJson {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub track_number: Option<u32>,
    pub total_tracks: Option<u32>,
    pub disc_number: Option<u32>,
    pub total_discs: Option<u32>,
    pub duration: Option<f64>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u16>,
    pub channels: Option<u16>,
    pub bitrate: Option<u32>,
    pub format: Option<String>,
    pub has_album_art: bool,
}

#[tauri::command]
pub fn get_track_metadata(path: String) -> Result<TrackMetadataJson, String> {
    let meta = phonon_codec::metadata::extract_metadata(&path).map_err(|e| e.to_string())?;
    Ok(TrackMetadataJson {
        title: meta.title,
        artist: meta.artist,
        album: meta.album,
        album_artist: meta.album_artist,
        genre: meta.genre,
        year: meta.year,
        track_number: meta.track_number,
        total_tracks: meta.total_tracks,
        disc_number: meta.disc_number,
        total_discs: meta.total_discs,
        duration: meta.duration,
        sample_rate: meta.sample_rate,
        bit_depth: meta.bit_depth,
        channels: meta.channels,
        bitrate: meta.bitrate,
        format: meta.format.map(|f| format!("{:?}", f)),
        has_album_art: meta.album_art.is_some(),
    })
}

#[derive(Serialize)]
pub struct AlbumArtResponse {
    pub data: Vec<u8>,
    pub mime_type: String,
}

#[tauri::command]
pub fn get_album_art(path: String) -> Result<Option<AlbumArtResponse>, String> {
    match phonon_codec::metadata::extract_album_art(&path) {
        Some(art) => Ok(Some(AlbumArtResponse {
            data: art.data,
            mime_type: art.mime_type,
        })),
        None => Ok(None),
    }
}

// ── Lyrics ────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct LyricWordJson {
    pub time_ms: u64,
    pub text: String,
}

#[derive(Serialize)]
pub struct LyricLineJson {
    pub time_ms: u64,
    pub text: String,
    pub words: Vec<LyricWordJson>,
}

#[derive(Serialize)]
#[serde(tag = "type")]
pub enum LyricDataJson {
    None,
    Unsynced { text: String },
    Synced { lines: Vec<LyricLineJson> },
}

#[tauri::command]
pub fn get_lyrics(path: String) -> Result<LyricDataJson, String> {
    let data = phonon_codec::metadata::extract_lyrics(&path);
    Ok(convert_lyric_data(data))
}

#[tauri::command]
pub fn load_lrc_file(path: String) -> Result<LyricDataJson, String> {
    let data = phonon_codec::metadata::load_lrc_file(&path);
    Ok(convert_lyric_data(data))
}

/// Search for synced lyrics online via lrclib.net.
/// Returns LRC data if found, or None if not available.
#[tauri::command]
pub async fn search_lyrics(
    state: State<'_, AppState>,
    artist: String,
    title: String,
) -> Result<LyricDataJson, String> {
    let base = state.settings.lock().unwrap().lyrics_api_url.clone();
    let url = format!(
        "{}/api/get?artist_name={}&track_name={}",
        base.trim_end_matches('/'),
        urlencoding(&artist),
        urlencoding(&title),
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;
    let resp = client
        .get(&url)
        .header("User-Agent", "Phonon/1.0")
        .send()
        .await
        .map_err(|e| format!("Failed to search lyrics: {}", e))?;
    if !resp.status().is_success() {
        return Ok(LyricDataJson::None);
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;
    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse response: {}", e))?;
    // lrclib returns syncedLyrics (LRC) or plainLyrics
    let lrc_text = json["syncedLyrics"]
        .as_str()
        .or_else(|| json["plainLyrics"].as_str())
        .unwrap_or("");
    if lrc_text.is_empty() {
        return Ok(LyricDataJson::None);
    }
    let data = phonon_codec::metadata::parse_lyrics_for_search(lrc_text);
    Ok(convert_lyric_data(data))
}

/// A search result item from lrclib.net.
#[derive(Serialize)]
pub struct SearchResultItem {
    pub id: u64,
    pub track_name: String,
    pub artist_name: String,
    pub has_synced: bool,
}

/// Search for lyrics by a single keyword via lrclib.net search API.
/// Returns a list of results for the user to choose from.
#[tauri::command]
pub async fn search_lyrics_by_keyword(
    state: State<'_, AppState>,
    keyword: String,
) -> Result<Vec<SearchResultItem>, String> {
    let base = state.settings.lock().unwrap().lyrics_api_url.clone();
    let url = format!(
        "{}/api/search?q={}",
        base.trim_end_matches('/'),
        urlencoding(&keyword),
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;
    let resp = client
        .get(&url)
        .header("User-Agent", "Phonon/1.0")
        .send()
        .await
        .map_err(|e| format!("Failed to search lyrics: {}", e))?;
    if !resp.status().is_success() {
        return Ok(Vec::new());
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;
    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse response: {}", e))?;
    let results = match json.as_array() {
        Some(arr) => arr,
        None => return Ok(Vec::new()),
    };
    let items: Vec<SearchResultItem> = results
        .iter()
        .filter_map(|item| {
            let id = item["id"].as_u64()?;
            let track_name = item["trackName"].as_str().unwrap_or("Unknown").to_string();
            let artist_name = item["artistName"].as_str().unwrap_or("Unknown").to_string();
            let has_synced = item["syncedLyrics"]
                .as_str()
                .map(|s| !s.is_empty())
                .unwrap_or(false);
            Some(SearchResultItem {
                id,
                track_name,
                artist_name,
                has_synced,
            })
        })
        .collect();
    Ok(items)
}

/// Fetch lyrics for a specific track by its lrclib.net ID.
#[tauri::command]
pub async fn fetch_lyrics_by_id(
    state: State<'_, AppState>,
    track_id: u64,
) -> Result<LyricDataJson, String> {
    let base = state.settings.lock().unwrap().lyrics_api_url.clone();
    let url = format!("{}/api/get/{}", base.trim_end_matches('/'), track_id);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;
    let resp = client
        .get(&url)
        .header("User-Agent", "Phonon/1.0")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch lyrics: {}", e))?;
    if !resp.status().is_success() {
        return Ok(LyricDataJson::None);
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;
    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse response: {}", e))?;
    let lrc_text = json["syncedLyrics"]
        .as_str()
        .or_else(|| json["plainLyrics"].as_str())
        .unwrap_or("");
    if lrc_text.is_empty() {
        return Ok(LyricDataJson::None);
    }
    let data = phonon_codec::metadata::parse_lyrics_for_search(lrc_text);
    Ok(convert_lyric_data(data))
}

fn urlencoding(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            b' ' => result.push('+'),
            _ => {
                result.push('%');
                result.push(hex_char(byte >> 4));
                result.push(hex_char(byte & 0x0F));
            }
        }
    }
    result
}

fn hex_char(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'A' + (n - 10)) as char,
        _ => '0',
    }
}

fn convert_lyric_data(data: phonon_codec::LyricData) -> LyricDataJson {
    match data {
        phonon_codec::LyricData::None => LyricDataJson::None,
        phonon_codec::LyricData::Unsynced(text) => LyricDataJson::Unsynced { text },
        phonon_codec::LyricData::Synced(lines) => LyricDataJson::Synced {
            lines: lines
                .into_iter()
                .map(|l| LyricLineJson {
                    time_ms: l.time_ms,
                    text: l.text,
                    words: l
                        .words
                        .into_iter()
                        .map(|w| LyricWordJson {
                            time_ms: w.time_ms,
                            text: w.text,
                        })
                        .collect(),
                })
                .collect(),
        },
    }
}

/// Parse raw LRC text into LyricData. Used by lyric source scripts.
#[tauri::command]
pub fn parse_lrc_text(lrc_text: String) -> LyricDataJson {
    let data = phonon_codec::metadata::parse_lyrics_for_search(&lrc_text);
    convert_lyric_data(data)
}

// ── Lyric Source Scripts ─────────────────────────────────────

/// A lyric source script file entry.
#[derive(Serialize)]
pub struct LyricSourceEntry {
    pub name: String,
    pub file_name: String,
}

/// Scan the plugins/lyrics/ directory for .js lyric source scripts.
#[tauri::command]
pub fn scan_lyric_sources() -> Result<Vec<LyricSourceEntry>, String> {
    let mut entries = Vec::new();
    // Try multiple candidate directories
    let candidates = vec![
        std::path::PathBuf::from("plugins/lyrics"),
        std::path::PathBuf::from("phonon-tauri/plugins/lyrics"),
        std::env::current_dir()
            .unwrap_or_default()
            .join("plugins/lyrics"),
    ];
    for dir in &candidates {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for entry in rd.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "js") {
                    let file_name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    entries.push(LyricSourceEntry {
                        name: file_name.trim_end_matches(".js").to_string(),
                        file_name,
                    });
                }
            }
            if !entries.is_empty() {
                break;
            }
        }
    }
    log::info!("scan_lyric_sources: found {} scripts", entries.len());
    Ok(entries)
}

/// Read the content of a lyric source script file.
#[tauri::command]
pub fn read_lyric_script(file_name: String) -> Result<String, String> {
    // Prevent path traversal
    if file_name.contains('/') || file_name.contains('\\') || file_name.contains("..") {
        return Err("Invalid file name".to_string());
    }
    let candidates = vec![
        std::path::PathBuf::from("plugins/lyrics").join(&file_name),
        std::path::PathBuf::from("phonon-tauri/plugins/lyrics").join(&file_name),
        std::env::current_dir()
            .unwrap_or_default()
            .join("plugins/lyrics")
            .join(&file_name),
    ];
    for path in &candidates {
        if path.exists() {
            return std::fs::read_to_string(path)
                .map_err(|e| format!("Failed to read script: {}", e));
        }
    }
    Err(format!("Script not found: {}", file_name))
}

/// Fetch a URL and return the response body as text.
/// Used by lyric source scripts to bypass CORS restrictions.
#[tauri::command]
pub async fn fetch_url(
    url: String,
    method: Option<String>,
    headers: Option<String>,
    body: Option<String>,
) -> Result<String, String> {
    let method = method.unwrap_or_else(|| "GET".to_string());
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;
    let mut req = match method.to_uppercase().as_str() {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        _ => return Err(format!("Unsupported method: {}", method)),
    };
    req = req.header("User-Agent", "Phonon/1.0");
    if let Some(h) = &headers {
        if let Ok(map) = serde_json::from_str::<std::collections::HashMap<String, String>>(h) {
            for (k, v) in map {
                req = req.header(&k, &v);
            }
        }
    }
    if let Some(b) = body {
        req = req.body(b);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("Fetch failed: {}", e))?;
    resp.text()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))
}

// ── Visualization Scripts ────────────────────────────────────

/// A visualization script file entry.
#[derive(Serialize)]
pub struct VisSourceEntry {
    pub name: String,
    pub file_name: String,
}

/// Scan the plugins/vis/ directory for .js visualization scripts.
#[tauri::command]
pub fn scan_vis_sources(app: tauri::AppHandle) -> Result<Vec<VisSourceEntry>, String> {
    let mut entries = Vec::new();
    let mut candidates = vis_scan_dirs(&app);
    candidates.extend(vec![
        std::path::PathBuf::from("plugins/vis"),
        std::path::PathBuf::from("phonon-tauri/plugins/vis"),
        std::path::PathBuf::from("../plugins/vis"),
        std::path::PathBuf::from("../../plugins/vis"),
        std::env::current_dir()
            .unwrap_or_default()
            .join("plugins/vis"),
    ]);
    for dir in &candidates {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for entry in rd.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "js") {
                    let file_name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    entries.push(VisSourceEntry {
                        name: file_name.trim_end_matches(".js").to_string(),
                        file_name,
                    });
                }
            }
            if !entries.is_empty() {
                break;
            }
        }
    }
    Ok(entries)
}

/// Candidate directories holding visualization scripts, most specific first:
/// the bundled resource dir (production installs) and the per-user data dir
/// (lets users drop their own scripts without touching the install).
fn vis_scan_dirs(app: &tauri::AppHandle) -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(rd) = app.path().resource_dir() {
        dirs.push(rd.join("plugins").join("vis"));
    }
    if let Ok(dd) = app.path().app_data_dir() {
        dirs.push(dd.join("plugins").join("vis"));
    }
    dirs
}

/// Read the content of a visualization script file.
#[tauri::command]
pub fn read_vis_script(app: tauri::AppHandle, file_name: String) -> Result<String, String> {
    if file_name.contains('/') || file_name.contains('\\') || file_name.contains("..") {
        return Err("Invalid file name".to_string());
    }
    let mut candidates: Vec<std::path::PathBuf> = vis_scan_dirs(&app)
        .into_iter()
        .map(|d| d.join(&file_name))
        .collect();
    candidates.extend(vec![
        std::path::PathBuf::from("plugins/vis").join(&file_name),
        std::path::PathBuf::from("phonon-tauri/plugins/vis").join(&file_name),
        std::path::PathBuf::from("../plugins/vis").join(&file_name),
        std::path::PathBuf::from("../../plugins/vis").join(&file_name),
        std::env::current_dir()
            .unwrap_or_default()
            .join("plugins/vis")
            .join(&file_name),
    ]);
    for path in &candidates {
        if path.exists() {
            return std::fs::read_to_string(path)
                .map_err(|e| format!("Failed to read script: {}", e));
        }
    }
    Err(format!("Script not found: {}", file_name))
}

/// Tracks the most recent stream temp file for cleanup-on-next-play.
/// Using a Mutex<Option<PathBuf>> ensures we only keep one "active" ref;
/// the previous file is deleted before a new one is created, which covers
/// the common play_url → play_url sequential use case.
static LAST_STREAM_TMP: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

/// Download a URL to a temporary file and play it.
#[tauri::command]
pub async fn play_url(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    url: String,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;
    let resp = client
        .get(&url)
        .header("User-Agent", "Phonon/1.0")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch URL: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read body: {}", e))?;
    // Write to temp file — clean up old stream files first
    let temp_dir = std::env::temp_dir().join("phonon_streams");
    std::fs::create_dir_all(&temp_dir).ok();

    // ① Delete the previously-tracked stream temp file (if any).
    //    This handles the common sequential play_url → play_url pattern
    //    so we don't leak one file per stream play.
    if let Ok(mut guard) = LAST_STREAM_TMP.lock() {
        if let Some(old_path) = guard.take() {
            let _ = std::fs::remove_file(&old_path);
        }
    }

    // ② Best-effort cleanup of files older than 10 minutes (safety net
    //    for edge cases like app crash mid-play or queued streams).
    if let Ok(entries) = std::fs::read_dir(&temp_dir) {
        let cutoff = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .saturating_sub(600); // 10 minutes
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|e| e.to_str()) == Some("tmp") {
                if let Ok(meta) = entry.metadata() {
                    if let Ok(mtime) = meta.modified() {
                        let mtime_secs = mtime
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        if mtime_secs < cutoff {
                            let _ = std::fs::remove_file(entry.path());
                        }
                    }
                }
            }
        }
    }
    // Use a unique counter + timestamp to avoid filename collisions
    static STREAM_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let file_name = format!(
        "stream_{}_{}.tmp",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        STREAM_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let temp_path = temp_dir.join(&file_name);
    std::fs::write(&temp_path, &bytes).map_err(|e| format!("Failed to write temp file: {}", e))?;
    let path_str = temp_path.to_string_lossy().to_string();

    // ③ Remember this temp file so the next play_url call can clean it up.
    if let Ok(mut guard) = LAST_STREAM_TMP.lock() {
        *guard = Some(temp_path.clone());
    }

    // Enqueue and play
    let engine = state.engine.clone();
    engine.enqueue(&path_str).map_err(|e| e.to_string())?;
    engine.start_playback().map_err(|e| e.to_string())?;
    // Send metadata event
    let _ = app.emit("queue-changed", ());
    Ok(())
}

// ── Equalizer ─────────────────────────────────────────────────

#[derive(Serialize)]
pub struct EqState {
    pub bands: Vec<EqBand>,
    pub mode: EqMode,
    pub presets: Vec<String>,
    pub geq_band_count: usize,
    pub preamp_db: f32,
}

#[tauri::command]
pub fn get_eq_state(state: State<'_, AppState>) -> Result<EqState, String> {
    let mut dsp = state.dsp_chain.lock().unwrap();
    if let Some(proc) = dsp.find_mut("equalizer") {
        if let Some(eq) = proc
            .as_any_mut()
            .downcast_mut::<phonon_core::eq::EqualizerProcessor>()
        {
            return Ok(EqState {
                bands: eq.get_bands().to_vec(),
                mode: eq.mode(),
                presets: eq.get_presets(),
                geq_band_count: eq.geq_band_count(),
                preamp_db: eq.preamp(),
            });
        }
    }
    Err("Equalizer not found".into())
}

#[tauri::command]
pub fn set_eq_band(
    state: State<'_, AppState>,
    index: usize,
    frequency: f32,
    gain_db: f32,
    q: f32,
    filter_type: String,
) -> Result<(), String> {
    let ft = match filter_type.as_str() {
        "Peaking" => FilterType::Peaking,
        "LowShelf" => FilterType::LowShelf,
        "HighShelf" => FilterType::HighShelf,
        "LowPass" => FilterType::LowPass,
        "HighPass" => FilterType::HighPass,
        _ => return Err(format!("Invalid filter type: {}", filter_type)),
    };
    let band = EqBand {
        frequency,
        gain_db,
        q,
        filter_type: ft,
    };
    let mut dsp = state.dsp_chain.lock().unwrap();
    if let Some(proc) = dsp.find_mut("equalizer") {
        if let Some(eq) = proc
            .as_any_mut()
            .downcast_mut::<phonon_core::eq::EqualizerProcessor>()
        {
            eq.set_band(index, band, state.engine.sample_rate());
            drop(dsp);
            save_eq_state(&state);
            return Ok(());
        }
    }
    Err("Equalizer not found".into())
}

#[tauri::command]
pub fn set_eq_mode(state: State<'_, AppState>, mode: String) -> Result<(), String> {
    let m = match mode.as_str() {
        "Graphic" => EqMode::Graphic,
        "Parametric" => EqMode::Parametric,
        _ => return Err(format!("Invalid EQ mode: {}", mode)),
    };
    let mut dsp = state.dsp_chain.lock().unwrap();
    if let Some(proc) = dsp.find_mut("equalizer") {
        if let Some(eq) = proc
            .as_any_mut()
            .downcast_mut::<phonon_core::eq::EqualizerProcessor>()
        {
            eq.set_mode(m, state.engine.sample_rate());
            drop(dsp);
            save_eq_state(&state);
            return Ok(());
        }
    }
    Err("Equalizer not found".into())
}

#[tauri::command]
pub fn load_eq_preset(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let mut dsp = state.dsp_chain.lock().unwrap();
    if let Some(proc) = dsp.find_mut("equalizer") {
        if let Some(eq) = proc
            .as_any_mut()
            .downcast_mut::<phonon_core::eq::EqualizerProcessor>()
        {
            if eq.load_preset(&name, state.engine.sample_rate()) {
                drop(dsp);
                save_eq_state(&state);
                return Ok(());
            }
            return Err(format!("Preset not found: {}", name));
        }
    }
    Err("Equalizer not found".into())
}

#[tauri::command]
pub fn save_eq_preset(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let mut dsp = state.dsp_chain.lock().unwrap();
    if let Some(proc) = dsp.find_mut("equalizer") {
        if let Some(eq) = proc
            .as_any_mut()
            .downcast_mut::<phonon_core::eq::EqualizerProcessor>()
        {
            eq.save_preset(&name);
            return Ok(());
        }
    }
    Err("Equalizer not found".into())
}

#[tauri::command]
pub fn delete_eq_preset(state: State<'_, AppState>, name: String) -> Result<bool, String> {
    let mut dsp = state.dsp_chain.lock().unwrap();
    if let Some(proc) = dsp.find_mut("equalizer") {
        if let Some(eq) = proc
            .as_any_mut()
            .downcast_mut::<phonon_core::eq::EqualizerProcessor>()
        {
            return Ok(eq.delete_preset(&name));
        }
    }
    Err("Equalizer not found".into())
}

#[tauri::command]
pub fn add_eq_band(
    state: State<'_, AppState>,
    frequency: f32,
    gain_db: f32,
    q: f32,
    filter_type: String,
) -> Result<(), String> {
    let ft = match filter_type.as_str() {
        "Peaking" => FilterType::Peaking,
        "LowShelf" => FilterType::LowShelf,
        "HighShelf" => FilterType::HighShelf,
        "LowPass" => FilterType::LowPass,
        "HighPass" => FilterType::HighPass,
        _ => return Err(format!("Invalid filter type: {}", filter_type)),
    };
    let band = EqBand {
        frequency,
        gain_db,
        q,
        filter_type: ft,
    };
    let mut dsp = state.dsp_chain.lock().unwrap();
    if let Some(proc) = dsp.find_mut("equalizer") {
        if let Some(eq) = proc
            .as_any_mut()
            .downcast_mut::<phonon_core::eq::EqualizerProcessor>()
        {
            if eq.add_band(band, state.engine.sample_rate()) {
                drop(dsp);
                save_eq_state(&state);
                return Ok(());
            }
            return Err("Maximum 10 bands reached".into());
        }
    }
    Err("Equalizer not found".into())
}

#[tauri::command]
pub fn remove_eq_band(state: State<'_, AppState>, index: usize) -> Result<(), String> {
    let mut dsp = state.dsp_chain.lock().unwrap();
    if let Some(proc) = dsp.find_mut("equalizer") {
        if let Some(eq) = proc
            .as_any_mut()
            .downcast_mut::<phonon_core::eq::EqualizerProcessor>()
        {
            if eq.remove_band(index) {
                drop(dsp);
                save_eq_state(&state);
                return Ok(());
            }
            return Err("Invalid band index".into());
        }
    }
    Err("Equalizer not found".into())
}

#[tauri::command]
pub fn set_geq_band_count(state: State<'_, AppState>, count: usize) -> Result<(), String> {
    let mut dsp = state.dsp_chain.lock().unwrap();
    if let Some(proc) = dsp.find_mut("equalizer") {
        if let Some(eq) = proc
            .as_any_mut()
            .downcast_mut::<phonon_core::eq::EqualizerProcessor>()
        {
            eq.set_geq_bands(count, state.engine.sample_rate());
            drop(dsp);
            save_eq_state(&state);
            return Ok(());
        }
    }
    Err("Equalizer not found".into())
}

#[tauri::command]
pub fn set_eq_preamp(state: State<'_, AppState>, db: f32) -> Result<(), String> {
    let mut dsp = state.dsp_chain.lock().unwrap();
    if let Some(proc) = dsp.find_mut("equalizer") {
        if let Some(eq) = proc
            .as_any_mut()
            .downcast_mut::<phonon_core::eq::EqualizerProcessor>()
        {
            eq.set_preamp(db);
            drop(dsp);
            save_eq_state(&state);
            return Ok(());
        }
    }
    Err("Equalizer not found".into())
}

// ── Hotplug Notifications ─────────────────────────────────────

#[tauri::command]
pub fn get_hotplug_notifications(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.settings.lock().unwrap().hotplug_notifications)
}

#[tauri::command]
pub fn set_hotplug_notifications(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    persist_settings_after(&state, |s| s.hotplug_notifications = enabled)
}

// ── Close to Tray ─────────────────────────────────────────────

#[tauri::command]
pub fn get_close_to_tray(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.settings.lock().unwrap().close_to_tray)
}

#[tauri::command]
pub fn set_close_to_tray(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    persist_settings_after(&state, |s| s.close_to_tray = enabled)
}

// ── ReplayGain Pre-amp ────────────────────────────────────────

#[tauri::command]
pub fn get_replay_gain_preamp(state: State<'_, AppState>) -> Result<f32, String> {
    Ok(state.settings.lock().unwrap().replay_gain_preamp)
}

#[tauri::command]
pub fn set_replay_gain_preamp(state: State<'_, AppState>, preamp_db: f32) -> Result<(), String> {
    let clamped = preamp_db.clamp(-12.0, 12.0);
    persist_settings_after(&state, |s| s.replay_gain_preamp = clamped)?;
    let mut dsp = state.dsp_chain.lock().unwrap();
    if let Some(proc) = dsp.find_mut("replaygain") {
        if let Some(rg) = proc
            .as_any_mut()
            .downcast_mut::<phonon_core::dsp::ReplayGainProcessor>()
        {
            rg.set_preamp(clamped);
        }
    }
    Ok(())
}

// ── Spectrum Color Scheme ─────────────────────────────────────

#[tauri::command]
pub fn get_spectrum_colorscheme(state: State<'_, AppState>) -> Result<String, String> {
    Ok(state.settings.lock().unwrap().spectrum_colorscheme.clone())
}

#[tauri::command]
pub fn set_spectrum_colorscheme(state: State<'_, AppState>, scheme: String) -> Result<(), String> {
    let valid = ["aurora", "warm", "cool", "mono", "custom"];
    if !valid.contains(&scheme.as_str()) {
        return Err(format!("Invalid scheme: {}. Valid: {:?}", scheme, valid));
    }
    persist_settings_after(&state, |s| s.spectrum_colorscheme = scheme)
}

// ── Plugin Limits ─────────────────────────────────────────────

#[derive(Serialize)]
pub struct PluginLimits {
    pub max_memory_mb: u32,
    pub call_timeout_ms: u64,
}

#[tauri::command]
pub fn get_plugin_limits(state: State<'_, AppState>) -> Result<PluginLimits, String> {
    let s = state.settings.lock().unwrap();
    Ok(PluginLimits {
        max_memory_mb: s.plugin_max_memory_mb,
        call_timeout_ms: s.plugin_call_timeout_ms,
    })
}

#[tauri::command]
pub fn set_plugin_limits(
    state: State<'_, AppState>,
    max_memory_mb: u32,
    call_timeout_ms: u64,
) -> Result<(), String> {
    let mm = max_memory_mb.clamp(16, 512);
    let ct = call_timeout_ms.clamp(1000, 30000);
    let settings_snapshot = {
        let mut s = state.settings.lock().unwrap();
        s.plugin_max_memory_mb = mm;
        s.plugin_call_timeout_ms = ct;
        s.clone()
    }; // <-- settings lock DROPPED here before I/O
       // Persist to settings.json
    if let Err(e) = save_settings_sync(&settings_snapshot) {
        log::error!("[set_plugin_limits] save_settings_sync failed: {}", e);
    }
    // Apply to runtime
    let limits = phonon_plugin::PluginResourceLimits {
        max_memory: (mm as usize) * 1024 * 1024,
        max_call_timeout_ms: ct,
    };
    state.plugin_runtime.set_resource_limits(limits);
    state.plugin_runtime.set_call_timeout(ct);
    log::info!("Plugin limits: {}MB / {}ms timeout", mm, ct);

    // Reload all currently loaded plugins so they pick up the new memory limit
    // (Wasmtime Store memory limits are set at creation time).
    let loaded_paths: Vec<String> = state
        .plugin_runtime
        .list_plugins()
        .into_iter()
        .filter(|p| {
            p.state == phonon_plugin::PluginState::Loaded
                || p.state == phonon_plugin::PluginState::Running
        })
        .map(|p| p.file_path.clone())
        .collect();
    let mut reloaded = 0usize;
    for path in &loaded_paths {
        match state.plugin_runtime.reload(path) {
            Ok(()) => reloaded += 1,
            Err(e) => log::warn!("Failed to reload plugin {} on limit change: {}", path, e),
        }
    }
    if reloaded > 0 {
        log::info!("Reloaded {} plugins with new memory limits", reloaded);
    }

    // Persist to an independent file so the limit survives restart even when
    // startup_behavior != "Remember" (session.json is only saved/loaded then).
    {
        let limits_path = data_dir_pub().join("plugin_limits.json");
        if let Some(parent) = limits_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log::warn!("Failed to create plugin_limits dir: {}", e);
            }
        }
        let payload = serde_json::json!({
            "plugin_max_memory_mb": mm,
            "plugin_call_timeout_ms": ct,
        });
        if let Err(e) = std::fs::write(&limits_path, payload.to_string()) {
            log::warn!("Failed to persist plugin limits: {}", e);
        } else {
            log::debug!("Persisted plugin limits to {}", limits_path.display());
        }
    }
    Ok(())
}

// ── Language ───────────────────────────────────────────────────

#[tauri::command]
pub fn get_language(state: State<'_, AppState>) -> Result<String, String> {
    Ok(state.settings.lock().unwrap().language.clone())
}

#[tauri::command]
pub fn set_language(state: State<'_, AppState>, language: String) -> Result<(), String> {
    let valid = ["zh-CN", "en"];
    if !valid.contains(&language.as_str()) {
        return Err(format!(
            "Invalid language: {}. Valid: {:?}",
            language, valid
        ));
    }
    persist_settings_after(&state, |s| s.language = language)
}

// ── Bit Depth ─────────────────────────────────────────────────

#[tauri::command]
pub fn set_bit_depth(state: State<'_, AppState>, depth: String) -> Result<(), String> {
    let depth_num = match depth.as_str() {
        "Float32" => 0,
        "I16" => 16,
        "I24" => 24,
        _ => {
            return Err(format!(
                "Invalid bit depth: {}. Valid: Float32, I16, I24",
                depth
            ))
        }
    };
    state.engine.set_bit_depth(depth_num);
    persist_settings_after(&state, |s| s.bit_depth = depth_num)?;
    log::info!("Bit depth set to {}", depth);
    Ok(())
}

#[tauri::command]
pub fn get_bit_depth(state: State<'_, AppState>) -> Result<String, String> {
    match state.settings.lock().unwrap().bit_depth {
        0 => Ok("Float32".to_string()),
        16 => Ok("I16".to_string()),
        24 => Ok("I24".to_string()),
        d => Ok(format!("{}", d)),
    }
}

// ── Output Sample Rate ────────────────────────────────────────

const SAMPLE_RATES: &[u32] = &[
    0, 44100, 48000, 88200, 96000, 176400, 192000, 352800, 384000,
];

#[tauri::command]
pub fn set_output_sample_rate(state: State<'_, AppState>, rate: u32) -> Result<(), String> {
    if !SAMPLE_RATES.contains(&rate) {
        return Err(format!(
            "Invalid sample rate: {}. Valid: {:?}",
            rate, SAMPLE_RATES
        ));
    }
    state.engine.set_output_sample_rate(rate);
    persist_settings_after(&state, |s| s.output_sample_rate = rate)?;
    log::info!(
        "Output sample rate set to {}",
        if rate == 0 {
            "auto".to_string()
        } else {
            format!("{} Hz", rate)
        }
    );
    Ok(())
}

#[tauri::command]
pub fn get_output_sample_rate(state: State<'_, AppState>) -> Result<u32, String> {
    Ok(state.settings.lock().unwrap().output_sample_rate)
}

// ── DSD Mode ──────────────────────────────────────────────────

pub fn dsd_mode_from_u8(mode: u8) -> DsdMode {
    match mode {
        1 => DsdMode::DopDsd64,
        2 => DsdMode::DopDsd128,
        3 => DsdMode::DopDsd256,
        _ => DsdMode::Off,
    }
}

#[tauri::command]
pub fn set_dsd_mode(state: State<'_, AppState>, mode: String) -> Result<(), String> {
    let mode_num = match mode.as_str() {
        "Off" => 0,
        "DoP_Dsd64" => 1,
        "DoP_Dsd128" => 2,
        "DoP_Dsd256" => 3,
        _ => {
            return Err(format!(
                "Invalid DSD mode: {}. Valid: Off, DoP_Dsd64, DoP_Dsd128, DoP_Dsd256",
                mode
            ))
        }
    };
    let dm = dsd_mode_from_u8(mode_num);
    state.engine.set_dsd_mode(dm);
    persist_settings_after(&state, |s| s.dsd_mode = mode_num)?;
    log::info!("DSD mode set to {}", mode);
    Ok(())
}

#[tauri::command]
pub fn get_dsd_mode(state: State<'_, AppState>) -> Result<String, String> {
    match state.settings.lock().unwrap().dsd_mode {
        0 => Ok("Off".to_string()),
        1 => Ok("DoP_Dsd64".to_string()),
        2 => Ok("DoP_Dsd128".to_string()),
        3 => Ok("DoP_Dsd256".to_string()),
        m => Ok(format!("{}", m)),
    }
}

// ── App Control ────────────────────────────────────────────────

#[tauri::command]
pub fn quit_app(app: tauri::AppHandle, state: State<'_, AppState>) {
    // Persist window geometry first — the tray-quit path bypasses
    // CloseRequested, so without this the next launch restores stale bounds.
    crate::save_main_window_geometry(&app);
    // Release the audio device (incl. exclusive-mode WASAPI) BEFORE the hard
    // exit — std::process::exit skips destructors.
    state.engine.stop();
    let _ = save_session(state);
    std::process::exit(0);
}

/// Set whether the desktop lyrics window ignores mouse events (click-through)
#[tauri::command]
pub fn set_ignore_cursor_events(app: tauri::AppHandle, ignore: bool) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("desktop-lyrics") {
        window
            .set_ignore_cursor_events(ignore)
            .map_err(|e| e.to_string())
    } else {
        Ok(())
    }
}

/// Adjust window position so it stays within the monitor work area
/// (the portion of the screen not occupied by the taskbar / tray).
/// Returns the adjusted (x, y) position.
fn adjust_window_position(
    window: &tauri::WebviewWindow,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> (i32, i32) {
    // Default: return original position
    let mut ax = x;
    let mut ay = y;

    // Try to get the monitor the window would appear on
    if let Ok(Some(monitor)) = window.current_monitor() {
        let mon_x = monitor.position().x;
        let mon_y = monitor.position().y;
        let mon_w = monitor.size().width as i32;
        let mon_h = monitor.size().height as i32;

        // On Windows, we can use SystemParametersInfo to get the work area
        // (screen area minus taskbar). Fall back to the full monitor size.
        #[cfg(target_os = "windows")]
        {
            use windows::Win32::Foundation::RECT;
            use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETWORKAREA};

            let mut work_rect = RECT {
                left: mon_x,
                top: mon_y,
                right: mon_x + mon_w,
                bottom: mon_y + mon_h,
            };

            let ok = unsafe {
                SystemParametersInfoW(
                    SPI_GETWORKAREA,
                    0,
                    Some(&mut work_rect as *mut _ as *mut _),
                    windows::Win32::UI::WindowsAndMessaging::SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
                )
                .is_ok()
            };

            if ok {
                let work_left = work_rect.left;
                let work_top = work_rect.top;
                let work_right = work_rect.right;
                let work_bottom = work_rect.bottom;
                let work_w = work_right - work_left;
                let work_h = work_bottom - work_top;

                // Clamp width/height to work area
                let eff_w = w.min(work_w);
                let eff_h = h.min(work_h);

                // Horizontal: ensure window is fully within work area
                if ax + eff_w > work_right {
                    ax = work_right - eff_w;
                }
                if ax < work_left {
                    ax = work_left;
                }

                // Vertical: ensure window is fully within work area
                if ay + eff_h > work_bottom {
                    ay = work_bottom - eff_h;
                }
                if ay < work_top {
                    ay = work_top;
                }

                return (ax, ay);
            }
        }

        // Fallback: clamp to full monitor bounds
        if ax + w > mon_x + mon_w {
            ax = mon_x + mon_w - w;
        }
        if ax < mon_x {
            ax = mon_x;
        }
        if ay + h > mon_y + mon_h {
            ay = mon_y + mon_h - h;
        }
        if ay < mon_y {
            ay = mon_y;
        }
    }

    (ax, ay)
}

/// Open desktop lyrics floating window.
#[tauri::command]
pub async fn open_desktop_lyrics(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    use tauri::WebviewUrl;
    use tauri::WebviewWindowBuilder;

    // If window already exists, just show and restore position
    if let Some(existing) = app.get_webview_window("desktop-lyrics") {
        // Apply click-through BEFORE showing to avoid unlocked flash
        {
            let s = state.settings.lock().map_err(|e| e.to_string())?;
            if s.desktop_lyrics_locked {
                let _ = existing.set_ignore_cursor_events(true);
            }
        }
        let _ = existing.show();
        // Restore saved position if available
        let (sx, sy, sw, sh, pos_set) = {
            let s = state.settings.lock().map_err(|e| e.to_string())?;
            (
                s.desktop_lyrics_x,
                s.desktop_lyrics_y,
                s.desktop_lyrics_width,
                s.desktop_lyrics_height,
                s.desktop_lyrics_position_set,
            )
        };
        if pos_set {
            use tauri::PhysicalPosition;
            let (adj_x, adj_y) =
                adjust_window_position(&existing, sx as i32, sy as i32, sw as i32, sh as i32);
            let _ = existing.set_position(PhysicalPosition::new(adj_x, adj_y));
        }
        let _ = app.emit("desktop-lyrics-visibility", true);
        return Ok(());
    }

    log::info!("[desktop-lyrics] Creating floating lyrics window");

    // Read saved position/size from settings
    let (saved_x, saved_y, saved_w, saved_h, position_set) = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        (
            s.desktop_lyrics_x,
            s.desktop_lyrics_y,
            s.desktop_lyrics_width,
            s.desktop_lyrics_height,
            s.desktop_lyrics_position_set,
        )
    };

    let mut builder = WebviewWindowBuilder::new(
        &app,
        "desktop-lyrics",
        WebviewUrl::App("desktop-lyrics.html".into()),
    )
    .title("Phonon Lyrics")
    .inner_size(900.0, 120.0)
    .min_inner_size(300.0, 80.0)
    .decorations(false)
    .always_on_top(true)
    .resizable(true)
    .visible(false);

    #[allow(unused_mut)]
    #[cfg(not(target_os = "macos"))]
    {
        builder = builder.transparent(true).skip_taskbar(true).shadow(false);
    }
    #[cfg(target_os = "macos")]
    {
        // macOS: no transparent() API in Tauri 2.x
        // desktop-lyrics remains opaque on macOS; functionality preserved
        // via global CSS data-vis-mode for visual distinction.
    }

    let window = builder.build().map_err(|e| {
        log::error!("[desktop-lyrics] Failed to create window: {}", e);
        format!("Failed to create window: {}", e)
    })?;

    // Restore saved size and position using PhysicalSize to match what was saved
    if position_set {
        use tauri::PhysicalSize;
        let _ = window.set_size(PhysicalSize::new(saved_w as u32, saved_h as u32));
        use tauri::PhysicalPosition;

        // Adjust position to ensure the window is visible and not obscured
        // by the taskbar / system tray area.
        let (adjusted_x, adjusted_y) = adjust_window_position(
            &window,
            saved_x as i32,
            saved_y as i32,
            saved_w as i32,
            saved_h as i32,
        );
        let _ = window.set_position(PhysicalPosition::new(adjusted_x, adjusted_y));
    } else {
        // Center horizontally near the bottom of the screen by default
        if let Ok(monitor) = window.current_monitor() {
            if let Some(m) = monitor {
                let screen_w = m.size().width as f64;
                let screen_h = m.size().height as f64;
                let pos_x = (screen_w - 900.0) / 2.0 + m.position().x as f64;
                let pos_y = screen_h * 0.8 - 120.0 + m.position().y as f64;
                use tauri::PhysicalPosition;
                let _ = window.set_position(PhysicalPosition::new(pos_x as i32, pos_y as i32));
            }
        }
    }

    // Apply click-through BEFORE showing the window so there's no flash
    // of the unlocked state when the saved setting is locked=true.
    {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        if s.desktop_lyrics_locked {
            let _ = window.set_ignore_cursor_events(true);
        }
    }

    // Show window after positioning and click-through are done
    let _ = window.show();

    let _ = app.emit("desktop-lyrics-visibility", true);

    log::info!("[desktop-lyrics] Window created and visible");

    // Windows: background monitor that temporarily disables click-through
    // when the cursor hovers the top 32px strip of the window, so users can
    // click the unlock button even when the window is fully click-through.
    #[cfg(target_os = "windows")]
    {
        let win_monitor = window.clone();
        let app_monitor = app.clone();
        std::thread::spawn(move || {
            use windows::Win32::Foundation::POINT;
            use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

            let mut last_hover = false;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(80));

                // Check if window still exists by testing if is_visible succeeds
                if win_monitor.is_visible().is_err() {
                    break;
                }

                // Read locked state from AppHandle's state
                let locked = {
                    let state_handle = app_monitor.state::<AppState>();
                    let lock_result = state_handle.settings.lock();
                    match lock_result {
                        Ok(s) => s.desktop_lyrics_locked,
                        Err(_) => break,
                    }
                };

                if !locked {
                    // When unlocked, ensure click-through is disabled
                    let _ = win_monitor.set_ignore_cursor_events(false);
                    last_hover = false;
                    continue;
                }

                // Get cursor position (screen coordinates)
                let mut cursor = POINT { x: 0, y: 0 };
                let cursor_ok = unsafe { GetCursorPos(&mut cursor) }.is_ok();
                if !cursor_ok {
                    continue;
                }

                // Get window position and size
                let (win_x, win_y, win_w) =
                    match (win_monitor.outer_position(), win_monitor.inner_size()) {
                        (Ok(pos), Ok(size)) => (pos.x, pos.y, size.width as i32),
                        _ => continue,
                    };

                // Check if cursor is inside the top strip of the window
                let in_top_strip = cursor.x >= win_x
                    && cursor.x <= win_x + win_w
                    && cursor.y >= win_y
                    && cursor.y <= win_y + 36; // slightly larger hit area

                if in_top_strip != last_hover {
                    let _ = win_monitor.set_ignore_cursor_events(!in_top_strip);
                    last_hover = in_top_strip;
                }
            }
        });
    }

    // Emit event to main window when desktop lyrics is closed.
    // Save position and size on close.
    // Use a weak reference-style approach: clone the AppHandle and look up
    // the main window inside the callback instead of cloning the WebviewWindow
    // directly, to avoid potential reference cycles.
    let app_handle = app.clone();
    let win_for_save = window.clone();
    window.on_window_event(move |event| {
        match event {
            tauri::WindowEvent::CloseRequested { .. } => {
                // Save position and size before closing (use inner size for consistency with inner_size builder)
                if let (Ok(pos), Ok(size)) =
                    (win_for_save.outer_position(), win_for_save.inner_size())
                {
                    let s_clone = {
                        let state_handle = app_handle.state::<AppState>();
                        let lock_result = state_handle.settings.lock();
                        if let Ok(mut s) = lock_result {
                            s.desktop_lyrics_x = pos.x as f64;
                            s.desktop_lyrics_y = pos.y as f64;
                            s.desktop_lyrics_width = size.width as f64;
                            s.desktop_lyrics_height = size.height as f64;
                            s.desktop_lyrics_position_set = true;
                            Some(s.clone())
                        } else {
                            None
                        }
                    };
                    if let Some(s) = s_clone {
                        if let Err(e) = save_settings_sync(&s) {
                            log::error!("[desktop lyrics close] save_settings_sync failed: {}", e);
                        }
                    }
                }
            }
            tauri::WindowEvent::Destroyed => {
                let _ = app_handle.emit("desktop-lyrics-visibility", false);
                if let Some(main_win) = app_handle.get_webview_window("main") {
                    let _ = main_win.emit("desktop-lyrics-closed", ());
                }
            }
            _ => {}
        }
    });

    Ok(())
}

/// Close desktop lyrics window.
#[tauri::command]
pub async fn close_desktop_lyrics(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("desktop-lyrics") {
        window.close().map_err(|e| format!("{}", e))?;
    }
    Ok(())
}

/// Desktop lyrics settings payload.
/// `shadow_enabled` is retained for backward compatibility with older
/// persisted settings but is no longer used by the frontend — text shadow
/// is always disabled in the floating window.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DesktopLyricsSettings {
    pub font_size: f64,
    pub opacity: f64,
    #[serde(default = "default_text_color")]
    pub text_color: String,
    pub locked: bool,
    pub bg_opacity: f64,
    #[serde(default)]
    pub shadow_enabled: bool,
    pub always_on_top: bool,
    pub played_color: String,
    pub unplayed_color: String,
    pub bg_mode: String,
    pub bg_path: String,
    #[serde(default = "default_line_mode")]
    pub line_mode: String,
    #[serde(default = "default_scroll_fx")]
    pub scroll_fx: bool,
    #[serde(default = "default_equal_size")]
    pub equal_size: bool,
    #[serde(default = "default_line_style")]
    pub line_style: String,
}

fn default_line_mode() -> String {
    "single".to_string()
}

fn default_text_color() -> String {
    "#ffffff".to_string()
}

// ── Media Library ─────────────────────────────────────────────
//
// All library_* commands operate on the lazily-initialized
// `MediaLibrary`. The first call that needs the library opens it
// at `<cwd>/.phonon_data/library.sqlite`. Subsequent calls reuse
// the open handle.
//
// Compliance note: search/edit_metadata never invoke any third-party
// metadata/cover/lyrics API. All input is local files or manual text.

/// Resolve the on-disk path for the library database.
/// Stored under `.phonon_data/library.sqlite` next to the executable
/// or in the current working directory.
fn library_db_path() -> std::path::PathBuf {
    data_dir_pub().join("library.sqlite")
}

/// Ensure the media library is opened. Subsequent calls are no-ops.
///
/// Opening happens outside any guard to avoid holding the lock during IO,
/// and the final assignment re-checks `is_none()` to handle the race where
/// another thread opened the library in the meantime.
fn ensure_library_open(state: &State<'_, AppState>) -> Result<(), String> {
    let already_open = {
        let guard = state.media_library.lock().map_err(|e| e.to_string())?;
        guard.is_some()
    };
    if already_open {
        return Ok(());
    }
    let path = library_db_path();
    log::info!("[library] opening media library at {}", path.display());
    let lib = phonon_media::MediaLibrary::open(&path)
        .map_err(|e| format!("Failed to open media library: {e}"))?;
    let mut guard = state.media_library.lock().map_err(|e| e.to_string())?;
    if guard.is_none() {
        *guard = Some(lib);
    }
    Ok(())
}

/// Helper: ensure the library is open, then run a closure with it.
fn with_library<R>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&phonon_media::MediaLibrary) -> Result<R, String>,
) -> Result<R, String> {
    ensure_library_open(state)?;
    let guard = state.media_library.lock().map_err(|e| e.to_string())?;
    let lib = guard
        .as_ref()
        .ok_or_else(|| "library not open".to_string())?;
    f(lib)
}

/// Configure the library roots. Replaces any previously configured roots.
#[tauri::command]
pub fn library_set_roots(state: State<'_, AppState>, roots: Vec<String>) -> Result<(), String> {
    let paths: Vec<std::path::PathBuf> = roots.into_iter().map(std::path::PathBuf::from).collect();
    let mut guard = state.library_roots.lock().map_err(|e| e.to_string())?;
    *guard = paths;
    Ok(())
}

/// Get the currently configured library roots.
#[tauri::command]
pub fn library_get_roots(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let guard = state.library_roots.lock().map_err(|e| e.to_string())?;
    Ok(guard
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect())
}

/// Trigger an incremental scan over the configured roots.
///
/// Emits `library-scan-progress` events to all windows as the scan proceeds.
/// The command blocks until the scan completes; the frontend is expected to
/// invoke it from a worker context or display a busy indicator.
#[tauri::command]
pub fn library_scan(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<phonon_media::ScanStats, String> {
    let roots: Vec<std::path::PathBuf> = {
        let guard = state.library_roots.lock().map_err(|e| e.to_string())?;
        guard.clone()
    };
    if roots.is_empty() {
        return Err("No library roots configured. Call library_set_roots first.".into());
    }
    ensure_library_open(&state)?;
    let guard = state.media_library.lock().map_err(|e| e.to_string())?;
    let lib = guard
        .as_ref()
        .ok_or_else(|| "library not open".to_string())?;

    let app_for_cb = app.clone();
    let progress: std::sync::Arc<dyn Fn(phonon_media::ScanProgress) + Send + Sync> =
        std::sync::Arc::new(move |p: phonon_media::ScanProgress| {
            let _ = app_for_cb.emit(
                "library-scan-progress",
                serde_json::json!({
                    "total": p.total,
                    "processed": p.processed,
                    "current_path": p.current_path,
                    "phase": p.phase,
                }),
            );
        });
    let opts = phonon_media::ScanOptions::default();
    lib.scan(&roots, &opts, &*progress)
        .map_err(|e| e.to_string())
}

/// Full-text search the library.
#[tauri::command]
pub fn library_search(
    state: State<'_, AppState>,
    query: String,
    limit: Option<i64>,
) -> Result<Vec<phonon_media::SearchResult>, String> {
    with_library(&state, |lib| {
        lib.search(
            &query,
            phonon_media::SearchMode::FtsPrefix,
            limit.unwrap_or(200),
        )
        .map_err(|e| e.to_string())
    })
}

/// List tracks with optional filter/pagination.
#[tauri::command]
pub fn library_get_tracks(
    state: State<'_, AppState>,
    filter: Option<phonon_media::TrackFilter>,
) -> Result<Vec<phonon_media::TrackInfo>, String> {
    with_library(&state, |lib| {
        lib.get_tracks(&filter.unwrap_or_default())
            .map_err(|e| e.to_string())
    })
}

/// Get a single track by id.
#[tauri::command]
pub fn library_get_track(
    state: State<'_, AppState>,
    id: i64,
) -> Result<Option<phonon_media::TrackInfo>, String> {
    with_library(&state, |lib| lib.get_track(id).map_err(|e| e.to_string()))
}

/// Get a single track by its file path. Returns None if not in library.
#[tauri::command]
pub fn library_get_track_by_path(
    state: State<'_, AppState>,
    path: String,
) -> Result<Option<phonon_media::TrackInfo>, String> {
    with_library(&state, |lib| {
        lib.get_track_by_path(&path).map_err(|e| e.to_string())
    })
}

/// Toggle favorite flag. Returns the new state.
#[tauri::command]
pub fn library_toggle_favorite(state: State<'_, AppState>, id: i64) -> Result<bool, String> {
    with_library(&state, |lib| {
        lib.toggle_favorite(id).map_err(|e| e.to_string())
    })
}

/// Set rating (0..=5).
#[tauri::command]
pub fn library_set_rating(state: State<'_, AppState>, id: i64, rating: u8) -> Result<(), String> {
    with_library(&state, |lib| {
        lib.set_rating(id, rating).map_err(|e| e.to_string())
    })
}

/// Record a play event (bumps play_count + appends to play_history).
#[tauri::command]
pub fn library_record_play(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    with_library(&state, |lib| lib.record_play(id).map_err(|e| e.to_string()))
}

/// Manually edit a track's metadata. Only the supplied fields are updated.
///
/// Compliance: only manual text input is accepted. No third-party metadata
/// API is invoked.
#[tauri::command]
pub fn library_edit_metadata(
    state: State<'_, AppState>,
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
) -> Result<phonon_media::TrackInfo, String> {
    with_library(&state, |lib| {
        lib.edit_metadata(
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
        )
        .map_err(|e| e.to_string())
    })
}

/// Bind a local cover file (JPEG/PNG) to a track.
///
/// Compliance: only local file paths supplied by the user are accepted.
#[tauri::command]
pub fn library_set_track_cover(
    state: State<'_, AppState>,
    id: i64,
    cover_path: String,
) -> Result<phonon_media::TrackInfo, String> {
    with_library(&state, |lib| {
        lib.set_track_cover_from_file(id, std::path::Path::new(&cover_path))
            .map_err(|e| e.to_string())
    })
}

/// Physically remove soft-deleted rows. Returns the number of rows deleted.
#[tauri::command]
pub fn library_cleanup_deleted(state: State<'_, AppState>) -> Result<u64, String> {
    with_library(&state, |lib| {
        lib.cleanup_deleted().map_err(|e| e.to_string())
    })
}

/// Batch-scan ReplayGain for all tracks (Mode A: sidecar SQLite, no file
/// modification). Emits `library-replaygain-progress` events per track.
///
/// `force` = true rescans all tracks even if sidecar-cached.
#[tauri::command]
pub fn library_batch_scan_replaygain(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    force: Option<bool>,
) -> Result<phonon_media::ReplayGainScanStats, String> {
    ensure_library_open(&state)?;
    let guard = state.media_library.lock().map_err(|e| e.to_string())?;
    let lib = guard
        .as_ref()
        .ok_or_else(|| "library not open".to_string())?;

    let sidecar_path = { data_dir_pub().join("replaygains.sqlite") };

    let app_for_cb = app.clone();
    let progress: std::sync::Arc<dyn Fn(phonon_media::ReplayGainScanProgress) + Send + Sync> =
        std::sync::Arc::new(move |p: phonon_media::ReplayGainScanProgress| {
            let _ = app_for_cb.emit(
                "library-replaygain-progress",
                serde_json::json!({
                    "total": p.total,
                    "processed": p.processed,
                    "current_path": p.current_path,
                    "phase": p.phase,
                }),
            );
        });

    lib.batch_scan_replaygain(&sidecar_path, force.unwrap_or(false), &*progress)
        .map_err(|e| e.to_string())
}

/// List all albums.
#[tauri::command]
pub fn library_list_albums(
    state: State<'_, AppState>,
) -> Result<Vec<phonon_media::AlbumInfo>, String> {
    with_library(&state, |lib| lib.list_albums().map_err(|e| e.to_string()))
}

/// List all artists.
#[tauri::command]
pub fn library_list_artists(
    state: State<'_, AppState>,
) -> Result<Vec<phonon_media::ArtistInfo>, String> {
    with_library(&state, |lib| lib.list_artists().map_err(|e| e.to_string()))
}

/// List all genres.
#[tauri::command]
pub fn library_list_genres(
    state: State<'_, AppState>,
) -> Result<Vec<phonon_media::GenreInfo>, String> {
    with_library(&state, |lib| lib.list_genres().map_err(|e| e.to_string()))
}

/// Retrieve a cover thumbnail by hash and size (256 or 512).
/// Returns the WEBP bytes as a `Vec<u8>`; the frontend typically wraps
/// these in a `Blob` URL for `<img>` consumption.
#[tauri::command]
pub fn library_get_thumbnail(
    state: State<'_, AppState>,
    hash: String,
    size: u32,
) -> Result<Option<phonon_media::ThumbnailInfo>, String> {
    with_library(&state, |lib| {
        lib.get_thumbnail(&hash, size).map_err(|e| e.to_string())
    })
}

fn default_scroll_fx() -> bool {
    true
}

fn default_equal_size() -> bool {
    false
}

fn default_line_style() -> String {
    "scroll".to_string()
}

impl Default for DesktopLyricsSettings {
    fn default() -> Self {
        Self {
            font_size: 22.0,
            opacity: 1.0,
            text_color: "#ffffff".to_string(),
            locked: false,
            bg_opacity: 0.45,
            shadow_enabled: true,
            always_on_top: true,
            played_color: "#ffffff".to_string(),
            unplayed_color: "#888888".to_string(),
            bg_mode: "hidden".to_string(),
            bg_path: String::new(),
            line_mode: "single".to_string(),
            scroll_fx: true,
            equal_size: false,
            line_style: "scroll".to_string(),
        }
    }
}

/// Get desktop lyrics settings.
#[tauri::command]
pub fn get_desktop_lyrics_settings(
    state: State<'_, AppState>,
) -> Result<DesktopLyricsSettings, String> {
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    Ok(DesktopLyricsSettings {
        font_size: settings.desktop_lyrics_font_size,
        opacity: settings.desktop_lyrics_opacity,
        text_color: settings.desktop_lyrics_text_color.clone(),
        locked: settings.desktop_lyrics_locked,
        bg_opacity: settings.desktop_lyrics_bg_opacity,
        shadow_enabled: settings.desktop_lyrics_shadow_enabled,
        always_on_top: settings.desktop_lyrics_always_on_top,
        played_color: settings.desktop_lyrics_played_color.clone(),
        unplayed_color: settings.desktop_lyrics_unplayed_color.clone(),
        bg_mode: settings.desktop_lyrics_bg_mode.clone(),
        bg_path: settings.desktop_lyrics_bg_path.clone(),
        line_mode: settings.desktop_lyrics_line_mode.clone(),
        scroll_fx: settings.desktop_lyrics_scroll_fx,
        equal_size: settings.desktop_lyrics_equal_size,
        line_style: settings.desktop_lyrics_line_style.clone(),
    })
}

/// Update desktop lyrics settings and broadcast to all windows.
#[tauri::command]
pub fn set_desktop_lyrics_settings(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    settings: DesktopLyricsSettings,
) -> Result<(), String> {
    // Save old always_on_top value before updating settings
    let old_always_on_top = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        s.desktop_lyrics_always_on_top
    };

    {
        let mut s = state.settings.lock().map_err(|e| e.to_string())?;
        s.desktop_lyrics_font_size = settings.font_size;
        s.desktop_lyrics_opacity = settings.opacity;
        s.desktop_lyrics_text_color = settings.text_color.clone();
        s.desktop_lyrics_locked = settings.locked;
        s.desktop_lyrics_bg_opacity = settings.bg_opacity;
        s.desktop_lyrics_shadow_enabled = settings.shadow_enabled;
        s.desktop_lyrics_always_on_top = settings.always_on_top;
        s.desktop_lyrics_played_color = settings.played_color.clone();
        s.desktop_lyrics_unplayed_color = settings.unplayed_color.clone();
        s.desktop_lyrics_bg_mode = settings.bg_mode.clone();
        s.desktop_lyrics_bg_path = settings.bg_path.clone();
        s.desktop_lyrics_line_mode = settings.line_mode.clone();
        s.desktop_lyrics_scroll_fx = settings.scroll_fx;
        s.desktop_lyrics_equal_size = settings.equal_size;
        s.desktop_lyrics_line_style = settings.line_style.clone();
    }

    // Apply always-on-top to the window if it exists (only when value actually changes)
    if settings.always_on_top != old_always_on_top {
        if let Some(win) = app.get_webview_window("desktop-lyrics") {
            let _ = win.set_always_on_top(settings.always_on_top);
        }
    }

    // Broadcast settings change to all windows
    let _ = app.emit("desktop-lyrics-settings-updated", &settings);

    // When switching to double-line mode, ensure the window has enough height
    // to show both lines. If the saved height is too small, bump it.
    if settings.line_mode == "double" {
        if let Some(win) = app.get_webview_window("desktop-lyrics") {
            let mut s = state.settings.lock().map_err(|e| e.to_string())?;
            // Minimum sensible height for double-line mode: ~3x font size + padding.
            // Default font size is 22 → 22 * 3.2 ≈ 70 + top bar ≈ 110, keep 140 minimum.
            let min_double_height = (s.desktop_lyrics_font_size * 3.2 + 40.0).max(150.0);
            if s.desktop_lyrics_height < min_double_height {
                s.desktop_lyrics_height = min_double_height;
                let physical_w = s.desktop_lyrics_width.max(400.0) as u32;
                let physical_h = s.desktop_lyrics_height as u32;
                use tauri::PhysicalSize;
                let _ = win.set_size(PhysicalSize::new(physical_w, physical_h));
            }
        }
    }

    let s_clone = state.settings.lock().map_err(|e| e.to_string())?.clone();
    if let Err(e) = save_settings_sync(&s_clone) {
        log::error!(
            "[set_desktop_lyrics_settings] save_settings_sync failed: {}",
            e
        );
    }

    Ok(())
}

/// Save desktop lyrics window position and size.
#[tauri::command]
pub fn save_desktop_lyrics_position(
    state: State<'_, AppState>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let s_clone = {
        let mut s = state.settings.lock().map_err(|e| e.to_string())?;
        s.desktop_lyrics_x = x;
        s.desktop_lyrics_y = y;
        s.desktop_lyrics_width = width;
        s.desktop_lyrics_height = height;
        s.desktop_lyrics_position_set = true;
        s.clone()
    };
    if let Err(e) = save_settings_sync(&s_clone) {
        log::error!(
            "[save_desktop_lyrics_position] save_settings_sync failed: {}",
            e
        );
    }
    Ok(())
}

/// Resolve a writable path for the desktop lyrics background image inside
/// the app's data directory. Kept for backward compatibility; the frontend
/// now uses `save_desktop_lyrics_bg` to write bytes directly via IPC, which
/// avoids the need for the `tauri-plugin-fs` plugin and its scope config.
#[tauri::command]
pub fn resolve_desktop_lyrics_bg_path(app: tauri::AppHandle) -> Result<String, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir failed: {}", e))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("create_dir_all failed: {}", e))?;
    let file_path = dir.join("phonon-dl-bg.jpg");
    Ok(file_path.to_string_lossy().to_string())
}

/// Write cropped background image bytes (JPEG) to the app data dir and
/// return the resulting absolute path. Used by the desktop lyrics settings
/// page after CropEditor produces a cropped image.
#[tauri::command]
pub fn save_desktop_lyrics_bg(app: tauri::AppHandle, bytes: Vec<u8>) -> Result<String, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir failed: {}", e))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("create_dir_all failed: {}", e))?;
    // Clean up old background images to prevent resource leak
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with("phonon-dl-bg-") && name.ends_with(".jpg") {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
    let filename = "phonon-dl-bg-current.jpg".to_string();
    let file_path = dir.join(&filename);
    std::fs::write(&file_path, &bytes).map_err(|e| format!("write failed: {}", e))?;
    // Return path with forward slashes so convertFileSrc in frontend
    // can generate a valid asset URL (Windows backslashes break the URL).
    let path_str = file_path.to_string_lossy().replace('\\', "/");
    log::info!("[save_desktop_lyrics_bg] saved to: {}", path_str);
    Ok(path_str)
}

/// Reset all desktop lyrics settings (including window position) to their
/// defaults. Also repositions the live desktop-lyrics window to a centered
/// default size and broadcasts the new settings so all windows update.
#[tauri::command]
pub fn reset_desktop_lyrics_settings(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<DesktopLyricsSettings, String> {
    let default = DesktopLyricsSettings::default();
    let s_clone = {
        let mut s = state.settings.lock().map_err(|e| e.to_string())?;
        s.desktop_lyrics_font_size = default.font_size;
        s.desktop_lyrics_opacity = default.opacity;
        s.desktop_lyrics_text_color = default.text_color.clone();
        s.desktop_lyrics_locked = default.locked;
        s.desktop_lyrics_bg_opacity = default.bg_opacity;
        s.desktop_lyrics_shadow_enabled = default.shadow_enabled;
        s.desktop_lyrics_always_on_top = default.always_on_top;
        s.desktop_lyrics_played_color = default.played_color.clone();
        s.desktop_lyrics_unplayed_color = default.unplayed_color.clone();
        s.desktop_lyrics_bg_mode = default.bg_mode.clone();
        s.desktop_lyrics_bg_path = default.bg_path.clone();
        s.desktop_lyrics_line_mode = default.line_mode.clone();
        s.desktop_lyrics_scroll_fx = default.scroll_fx;
        s.desktop_lyrics_equal_size = default.equal_size;
        s.desktop_lyrics_line_style = default.line_style.clone();
        s.desktop_lyrics_x = 0.0;
        s.desktop_lyrics_y = 0.0;
        s.desktop_lyrics_width = 900.0;
        s.desktop_lyrics_height = 120.0;
        s.desktop_lyrics_position_set = false;
        s.clone()
    };
    if let Err(e) = save_settings_sync(&s_clone) {
        log::error!(
            "[reset_desktop_lyrics_settings] save_settings_sync failed: {}",
            e
        );
    }
    // Reset the live window: always-on-top, click-through off, size & center.
    if let Some(win) = app.get_webview_window("desktop-lyrics") {
        let _ = win.set_always_on_top(default.always_on_top);
        let _ = win.set_ignore_cursor_events(false);
        let _ = win.set_size(tauri::LogicalSize::new(900.0_f64, 120.0_f64));
        let _ = win.center();
    }
    let _ = app.emit("desktop-lyrics-settings-updated", &default);
    Ok(default)
}

/// Save main window position and size.
#[tauri::command]
pub fn save_main_window_position(
    state: State<'_, AppState>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    maximized: Option<bool>,
) -> Result<(), String> {
    let s_clone = {
        let mut s = state.settings.lock().map_err(|e| e.to_string())?;
        s.main_window_x = x;
        s.main_window_y = y;
        s.main_window_width = width;
        s.main_window_height = height;
        s.main_window_position_set = true;
        if let Some(max) = maximized {
            s.main_window_maximized = max;
        }
        s.clone()
    };
    if let Err(e) = save_settings_sync(&s_clone) {
        log::error!(
            "[save_main_window_position] save_settings_sync failed: {}",
            e
        );
    }
    Ok(())
}

// ── COM Helpers ────────────────────────────────────────────────

/// Ensure COM is initialized on the current thread (Windows only).
/// Multiple calls are safe (returns S_FALSE if already initialized).
#[cfg(windows)]
fn ensure_com_initialized() {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
}

// ── Window helpers ─────────────────────────────────────────────

/// Set the window title of the ROOT top-level window via raw Win32 SetWindowTextW.
///
/// Tauri's WebViewWindow::set_title() updates the WebView document title, but
/// on some Tauri 2.x builds on Windows it doesn't propagate the change to the
/// actual top-level HWND that Windows reads for the taskbar thumbnail header,
/// taskbar tooltip, and Alt+Tab. This command writes the title directly to
/// the GWL_HWND root window caption so the taskbar always shows the song name.
#[cfg(windows)]
#[tauri::command]
pub fn set_window_title(app: tauri::AppHandle, title: String) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, SetWindowTextW, GA_ROOTOWNER};

    let Some(webview) = app.get_webview_window("main") else {
        return Err("main window not found".to_string());
    };
    let hwnd_any = webview.hwnd().map_err(|e| e.to_string())?;
    let hwnd_ptr = hwnd_any.0 as isize;
    let hwnd = windows::Win32::Foundation::HWND(hwnd_ptr);
    let root = unsafe { GetAncestor(hwnd, GA_ROOTOWNER) };
    let effective = if root.0 != 0 { root } else { hwnd };

    // Encode to UTF-16 null-terminated, then wrap as PCWSTR for windows-rs
    // SetWindowTextW. windows-rs 0.54 expects IntoParam<PCWSTR> not &Vec<u16>.
    let wstr: Vec<u16> = OsStr::new(&title)
        .encode_wide()
        .chain(std::iter::once(0u16))
        .collect();

    unsafe {
        let pcw = windows::core::PCWSTR(wstr.as_ptr());
        let _ = SetWindowTextW(effective, pcw);
    }
    log::info!(
        "[set_window_title] Set title on HWND {:?}: {}",
        effective,
        title
    );
    Ok(())
}

#[cfg(not(windows))]
#[tauri::command]
pub fn set_window_title(app: tauri::AppHandle, title: String) -> Result<(), String> {
    if let Some(webview) = app.get_webview_window("main") {
        let _ = webview.set_title(&title);
    }
    Ok(())
}

// ──────────────────────────────────────────────────────────────
// Settings persistence (A/B/C shared).
// Whole AppSettings struct is serialized as JSON because
// AppSettings has #[derive(Serialize, Deserialize)] (state.rs).
// ──────────────────────────────────────────────────────────────
/// Global data directory, set once in Tauri setup() via set_data_dir().
static DATA_DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Set the global data directory from Tauri's app_data_dir().
/// Called once during app setup().
pub fn set_data_dir(dir: std::path::PathBuf) {
    let _ = std::fs::create_dir_all(&dir);
    let _ = DATA_DIR.set(dir);
}

/// Resolve the data directory. Uses the global path set by set_data_dir()
/// if available (preferred — works in packaged builds), otherwise falls
/// back to CWD/.phonon_data (dev mode).
pub fn data_dir_pub() -> std::path::PathBuf {
    if let Some(d) = DATA_DIR.get() {
        return d.clone();
    }
    // Fallback for dev mode before setup() runs
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let dir = cwd.join(".phonon_data");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub(crate) fn settings_path() -> Result<std::path::PathBuf, String> {
    Ok(data_dir_pub().join("settings.json"))
}

/// Synchronous, atomic-write settings persist. Always runs — no gate on
/// startup_behavior or empty queue (unlike save_session). This guarantees
/// that B's `last_used_device_id` / C's `dsp.time_stretch_mode` /
/// A's `library.auto_sync_playlist_folder_to_roots` always survive a crash.
pub(crate) fn save_settings_sync(settings: &crate::state::AppSettings) -> Result<(), String> {
    use std::io::Write;
    let final_path = settings_path()?;
    let tmp = final_path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    f.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
    f.flush().map_err(|e| e.to_string())?;
    drop(f);
    std::fs::rename(&tmp, &final_path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Load settings from disk (if file present & parseable).
/// Returns `Ok(None)` if settings.json does NOT exist (legacy session.json-only state).
/// Returns `Err` if it exists but is corrupt (caller decides recovery).
pub(crate) fn load_settings_sync() -> Result<Option<crate::state::AppSettings>, String> {
    let path = settings_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let parsed = serde_json::from_slice::<crate::state::AppSettings>(&bytes)
        .map_err(|e| format!("settings.json corrupt: {}", e))?;
    Ok(Some(parsed))
}

/// Lock settings, apply a mutation, then persist the snapshot to disk.
/// Use this in single-field setters to guarantee immediate persistence.
pub(crate) fn persist_settings_after<F>(state: &State<'_, AppState>, f: F) -> Result<(), String>
where
    F: FnOnce(&mut crate::state::AppSettings),
{
    let s_clone = {
        let mut s = state.settings.lock().map_err(|e| e.to_string())?;
        f(&mut s);
        s.clone()
    };
    if let Err(e) = save_settings_sync(&s_clone) {
        log::error!("[persist_settings_after] save_settings_sync failed: {}", e);
    }
    Ok(())
}

/// Save current EQ state from the DSP processor to settings.json.
/// Called after any EQ-modifying command to persist changes.
fn save_eq_state(state: &State<'_, AppState>) {
    let eq_snapshot = {
        let mut dsp = state.dsp_chain.lock().unwrap();
        dsp.find_mut("equalizer")
            .and_then(|proc| {
                proc.as_any_mut()
                    .downcast_mut::<phonon_core::eq::EqualizerProcessor>()
            })
            .map(|eq| crate::state::EqSettings {
                bands: eq.get_bands().to_vec(),
                mode: eq.mode(),
                geq_band_count: eq.geq_band_count(),
                preamp_db: eq.preamp(),
            })
    };
    if let Some(eq_snap) = eq_snapshot {
        if let Err(e) = persist_settings_after(state, |s| s.dsp.eq = eq_snap.clone()) {
            log::error!("[save_eq_state] failed to persist: {}", e);
        }
    }
}

/// Restore EQ state from settings to the DSP processor on startup.
pub(crate) fn restore_eq_state(state: &crate::state::AppState) {
    let eq_settings = state.settings.lock().unwrap().dsp.eq.clone();
    let sr = state.engine.sample_rate();
    let mut dsp = state.dsp_chain.lock().unwrap();
    if let Some(proc) = dsp.find_mut("equalizer") {
        if let Some(eq) = proc
            .as_any_mut()
            .downcast_mut::<phonon_core::eq::EqualizerProcessor>()
        {
            eq.set_mode(eq_settings.mode, sr);
            eq.set_geq_bands(eq_settings.geq_band_count, sr);
            eq.set_preamp(eq_settings.preamp_db);
            for (i, band) in eq_settings.bands.iter().enumerate() {
                eq.set_band(i, band.clone(), sr);
            }
        }
    }
}

// ══════════════════════════════════════════════════════════════════
// Plan A Task A5: Media Library — 6 new commands
// (delegate to phonon-media crate via media_library state handle)
// ══════════════════════════════════════════════════════════════════
//
// NOTE on lazy-open pattern: the library singleton is stored in
// AppState.media_library (type MediaLibraryHandle =
// Arc<Mutex<Option<MediaLibrary>>>). commands.rs already exposes TWO helpers
// for it (around L4195/4215) — DO NOT invent a second one:
//   • ensure_library_open(state) → opens the DB on first call (idempotent)
//   • with_library(state, |lib| { ... }) → run closure with &MediaLibrary
//     (returns Result<R, String>)
// Both are used by library_scan / library_get_roots / library_set_roots
// today. We reuse them.
//
// RENAME NOTE: the new edit_metadata / set_track_cover commands would
// collide at the Tauri command-name level with the existing per-field
// `library_edit_metadata` (L4352) and `library_set_track_cover` (L4386)
// commands. We add suffixes `_partial` / `_bytes` to the new ones so both
// can coexist; the frontend imports them under the suffixed names.

/// Resolve a batch of file paths to library track rows (max 500 per call).
/// Returns rows in input order; missing paths are silently dropped.
#[tauri::command]
pub fn library_get_tracks_by_paths(
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<Vec<phonon_media::LibraryTrack>, String> {
    if paths.len() > 500 {
        return Err("paths batch limit 500 (use multiple calls for larger sets)".to_string());
    }
    ensure_library_open(&state)?;
    with_library(&state, |lib| {
        lib.get_tracks_by_paths(&paths).map_err(|e| e.to_string())
    })
}

/// Omnisearch suggest: top 5 tracks + 3 albums + 3 artists matching `query`.
#[tauri::command]
pub fn library_search_suggest(
    state: State<'_, AppState>,
    query: String,
) -> Result<phonon_media::LibrarySuggest, String> {
    if query.trim().is_empty() {
        return Ok(Default::default());
    }
    ensure_library_open(&state)?;
    with_library(&state, |lib| {
        lib.search_suggest(&query).map_err(|e| e.to_string())
    })
}

/// Apply a partial metadata patch (PartialMeta) to a track. Library row is
/// always updated before the file-write attempt, so a file-write failure
/// NEVER rolls back the library change. The returned track row reflects
/// the new library state.
#[tauri::command]
pub fn library_edit_metadata_partial(
    state: State<'_, AppState>,
    track_id: i64,
    partial: phonon_codec::PartialMeta,
) -> Result<phonon_media::LibraryTrack, String> {
    ensure_library_open(&state)?;
    with_library(&state, |lib| {
        lib.edit_metadata_partial(track_id, &partial)
            .map_err(|e| e.to_string())
    })
}

/// Bind raw cover bytes (JPEG/PNG) as the cover for a track. Returns the
/// SHA1 hash of the encoded WEBP thumbnail.
#[tauri::command]
pub fn library_set_track_cover_bytes(
    state: State<'_, AppState>,
    track_id: i64,
    cover_bytes: Vec<u8>,
    mime: String,
) -> Result<String, String> {
    ensure_library_open(&state)?;
    with_library(&state, |lib| {
        lib.set_track_cover_bytes(track_id, cover_bytes, &mime)
            .map_err(|e| e.to_string())
    })
}

/// Try to recover tracks from a corrupt-library backup file. RC1 stub:
/// returns 0 (full recovery planned for RC2). Runs in the calling thread
/// (no &MediaLibrary access needed — try_recover_corrupt re-opens via
/// rusqlite Connection::open).
#[tauri::command]
pub fn library_try_recover_corrupt(
    state: State<'_, AppState>,
    backup_path: String,
) -> Result<u32, String> {
    let _ = state; // silence unused (keeps signature consistent with other library_* cmds)
    phonon_media::library::try_recover_corrupt(&backup_path).map_err(|e| e.to_string())
}

/// Retry writing a track's library metadata back into the actual audio file.
/// Used by the "稍后重试写回" toast action after an earlier
/// library_edit_metadata_partial call failed at the file-write step.
/// Returns the codec error string directly so the frontend can distinguish
/// "library error" from "codec/file-write error".
#[tauri::command]
pub fn library_write_metadata_back_to_file(
    state: State<'_, AppState>,
    track_id: i64,
) -> Result<(), String> {
    ensure_library_open(&state)?;
    with_library(&state, |lib| {
        lib.write_metadata_back_to_file(track_id)
            .map_err(|e| e.to_string())
    })
}

// ──────────────────────────────────────────────────────────────────────
// PEQ (Parametric EQ) commands — general-purpose biquad EQ control
// Available to plugins and the DSP chain for manual EQ adjustments.
// ──────────────────────────────────────────────────────────────────────

/// Manually set PEQ bands for the parametric EQ DSP processor.
/// Uses the current engine sample rate and stereo (2ch) configuration.
#[tauri::command]
pub fn set_peq_bands(
    state: State<'_, AppState>,
    bands_l: Vec<EqBand>,
    bands_r: Vec<EqBand>,
) -> Result<(), String> {
    let sr = state.engine.sample_rate();
    state
        .peq_handle
        .update_bands(bands_l, bands_r, sr)
        .map_err(|e| e.to_string())
}

/// Get the current PEQ state (bands + bypass flag).
/// Returns a JSON object for the frontend to render.
#[tauri::command]
pub fn get_peq_state(state: State<'_, AppState>) -> serde_json::Value {
    let bypass = state
        .peq_handle
        .bypass
        .load(std::sync::atomic::Ordering::Relaxed);
    let snap = state.peq_handle.last_applied_snapshot();
    match snap {
        Some(s) => serde_json::json!({
            "enabled": !bypass,
            "bands_L": s.bands_L,
            "bands_R": s.bands_R,
        }),
        None => serde_json::json!({
            "enabled": !bypass,
            "bands_L": [],
            "bands_R": [],
        }),
    }
}

/// Toggle PEQ bypass on/off. When bypassed, the PEQ processor skips processing.
#[tauri::command]
pub fn set_peq_bypass(state: State<'_, AppState>, bypass: bool) -> Result<(), String> {
    state
        .peq_handle
        .bypass
        .store(bypass, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}
