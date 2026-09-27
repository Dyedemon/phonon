//! Application state management.
//!
//! Holds all shared state for the Tauri application,
//! wrapped in Arc<Mutex<>> for thread-safe access.

use phonon_core::peq::PeqUpdateHandle;
use phonon_core::dsp::DownmixProcessor;
use phonon_core::dsp::ReplayGainProcessor;
use phonon_core::dsp::SmartEffectProcessor;
use phonon_core::dsp::SurroundSoundProcessor;
use phonon_core::eq::{EqualizerProcessor, EqBand, EqMode};
use phonon_core::{AppEvent, PeqProcessor, DeviceManager, DspChain, EngineConfig, PlaybackEngine, VolumeControl};
use phonon_plugin::PluginRuntime;
use phonon_source::Playlist;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tauri::Emitter;

/// Lazily-initialized media library handle.
///
/// `Option` because the library is opened on demand (first scan or query),
/// not at app startup — this keeps cold start fast and avoids creating the
/// SQLite file before the user has configured any library roots.
pub type MediaLibraryHandle = Arc<Mutex<Option<phonon_media::MediaLibrary>>>;

/// Application-level settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub replay_gain_mode: ReplayGainMode,
    pub volume_mode: VolumeMode,
    pub plugin_directory: String,
    pub hotplug_poll_interval_ms: u64,
    /// Buffer size in samples: 960 (10ms), 1920 (20ms), 4800 (50ms), 9600 (100ms).
    pub buffer_size: usize,
    /// Resampling quality: "Fast", "Balanced", "High".
    pub resampler_quality: String,
    /// UI theme: "Dark" or "Light".
    pub theme: String,
    /// Whether debug logging is enabled.
    pub debug_log: bool,
    /// Startup behavior: "Remember" or "Clear".
    pub startup_behavior: String,
    /// Spectrum smoothing factor (0.0–1.0).
    pub spectrum_smoothing: f32,
    /// Spectrum decay speed (0.0–1.0).
    pub spectrum_decay: f32,
    /// Spectrum animation FPS target (10–60).
    pub spectrum_fps: u32,
    /// Whether to show hotplug notifications.
    pub hotplug_notifications: bool,
    /// ReplayGain pre-amp gain in dB.
    pub replay_gain_preamp: f32,
    /// Spectrum color scheme: "aurora", "warm", "cool", "mono".
    pub spectrum_colorscheme: String,
    /// Plugin max memory in MB.
    pub plugin_max_memory_mb: u32,
    /// Plugin call timeout in ms.
    pub plugin_call_timeout_ms: u64,
    /// UI language: "zh-CN" or "en".
    pub language: String,
    /// Output sample rate (0 = auto-detect from device).
    pub output_sample_rate: u32,
    /// Bit depth: 0 = float32 passthrough, 16, 24.
    pub bit_depth: u16,
    /// DSD output mode: 0 = Off, 1 = DoP DSD64, 2 = DoP DSD128, 3 = DoP DSD256.
    pub dsd_mode: u8,
    /// Whether exclusive mode (WASAPI) is enabled.
    #[serde(default)]
    pub exclusive_mode: bool,
    /// Whether closing the window minimizes to tray (true) or quits directly (false).
    pub close_to_tray: bool,
    /// Base URL for the lyrics search API (e.g. https://lrclib.net).
    pub lyrics_api_url: String,
    /// Desktop lyrics font size in pixels.
    pub desktop_lyrics_font_size: f64,
    /// Desktop lyrics overall opacity (0.0-1.0).
    pub desktop_lyrics_opacity: f64,
    /// Desktop lyrics text color (hex string).
    pub desktop_lyrics_text_color: String,
    /// Whether desktop lyrics window is locked (cannot be dragged).
    pub desktop_lyrics_locked: bool,
    /// Desktop lyrics background opacity (0.0-1.0).
    pub desktop_lyrics_bg_opacity: f64,
    /// Whether text shadow is enabled for desktop lyrics.
    pub desktop_lyrics_shadow_enabled: bool,
    /// Whether desktop lyrics window is always on top.
    pub desktop_lyrics_always_on_top: bool,
    /// Desktop lyrics played text color (hex string).
    pub desktop_lyrics_played_color: String,
    /// Desktop lyrics unplayed text color (hex string).
    pub desktop_lyrics_unplayed_color: String,
    /// Desktop lyrics background mode: "hidden", "image", "video".
    pub desktop_lyrics_bg_mode: String,
    /// Desktop lyrics custom background path (image or video file path).
    pub desktop_lyrics_bg_path: String,
    /// Desktop lyrics line display mode: "single" or "double".
    pub desktop_lyrics_line_mode: String,
    /// Desktop lyrics double-line mode: enable rolling transition effect.
    pub desktop_lyrics_scroll_fx: bool,
    /// Desktop lyrics double-line mode: both lines use same font size.
    pub desktop_lyrics_equal_size: bool,
    /// Desktop lyrics double-line style: "normal", "centered", or "split".
    pub desktop_lyrics_line_style: String,
    /// Desktop lyrics window X position.
    pub desktop_lyrics_x: f64,
    /// Desktop lyrics window Y position.
    pub desktop_lyrics_y: f64,
    /// Desktop lyrics window width.
    pub desktop_lyrics_width: f64,
    /// Desktop lyrics window height.
    pub desktop_lyrics_height: f64,
    /// Whether desktop lyrics position/size has been set by user.
    pub desktop_lyrics_position_set: bool,
    /// Main window X position.
    pub main_window_x: f64,
    /// Main window Y position.
    pub main_window_y: f64,
    /// Main window width.
    pub main_window_width: f64,
    /// Main window height.
    pub main_window_height: f64,
    /// Whether main window position/size has been set.
    pub main_window_position_set: bool,
    /// Whether main window was maximized.
    pub main_window_maximized: bool,
    /// DSP sub-config (nested; #[serde(default)] fills legacy defaults).
    #[serde(default)]
    pub dsp: DspSettings,
    /// Hotplug sub-config (nested; serde(default) fills legacy defaults).
    #[serde(default)]
    pub hotplug: HotplugSettings,
    /// Library sub-config (nested; #[serde(default)] fills legacy defaults).
    /// Plan A Task A6.
    #[serde(default)]
    pub library: LibrarySettings,
}

/// Nested DSP-specific settings. All fields #[serde(default)] so old JSON configs
/// (saved before this struct was introduced) parse cleanly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DspSettings {
    /// Time-stretch algorithm selector used when no plugin stretcher is loaded.
    #[serde(default)]
    pub time_stretch_mode: phonon_core::engine::TimeStretchMode,
    /// Equalizer state persistence.
    #[serde(default)]
    pub eq: EqSettings,
    /// Acoustic calibration plugin persistence.
    #[serde(default)]
    pub calibration: CalibrationSettings,
}

/// Saved acoustic-calibration state (PEQ bands + FIR plugin config).
///
/// The calibration plugin is an external wasm module — the main app has no
/// build-time dependency on it. These records are only re-applied at startup
/// when the plugin is present again, so removing the plugin cleanly removes
/// its effects from the audio path.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CalibrationSettings {
    pub peq_bands_l: Vec<EqBand>,
    pub peq_bands_r: Vec<EqBand>,
    pub bypass: bool,
    /// Raw FirConfig JSON last pushed to the calibration wasm plugin.
    pub fir_config: Option<String>,
    /// Fit id of the last applied result (display only).
    pub fit_id: Option<String>,
}

/// EQ state saved to settings.json so it survives restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EqSettings {
    pub bands: Vec<EqBand>,
    pub mode: EqMode,
    pub geq_band_count: usize,
    pub preamp_db: f32,
}

impl Default for EqSettings {
    fn default() -> Self {
        Self {
            bands: Vec::new(),
            mode: EqMode::Parametric,
            geq_band_count: 10,
            preamp_db: 0.0,
        }
    }
}
impl Default for DspSettings {
    fn default() -> Self {
        Self {
            time_stretch_mode: phonon_core::engine::TimeStretchMode::Auto,
            eq: EqSettings::default(),
            calibration: CalibrationSettings::default(),
        }
    }
}

// ── Hotplug policy enums + nested struct (Plan B) ────────────────────────
/// What to do when the currently-selected output device disappears.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceRemovedStrategy {
    /// Freeze playback; user picks new device manually. Spec default.
    #[default]
    Pause,
    /// Silently switch to system-default device. If fails → Pause.
    SwitchDefault,
    /// Try switch_last_used → switch_default → Pause (daisy chain).
    SwitchLastUsed,
}

/// What to do when a NEW device appears on the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceInsertedStrategy {
    /// Do nothing. Spec default.
    #[default]
    Ignore,
    /// If `info.is_default == true`, silently switch to it.
    AutoSwitch,
}

/// Nested hotplug sub-config. All fields have #[serde(default)] on container
/// level (via AppSettings) so legacy flat settings.json parse cleanly.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct HotplugSettings {
    pub on_device_removed: DeviceRemovedStrategy,
    pub on_new_device_inserted: DeviceInsertedStrategy,
    /// Device ID (= name today; real cpal ID in the future) of the
    /// "previous good output". Used for `switch_last_used` fallback.
    /// SINGLE WRITER: ONLY commands::set_device mutates this BEFORE a switch.
    /// Hotplug callbacks NEVER mutate this field.
    pub last_used_device_id: Option<String>,
    /// Last explicitly selected output device. Restored on startup.
    #[serde(default)]
    pub last_output_device_id: Option<String>,
}
impl Default for HotplugSettings {
    fn default() -> Self {
        Self {
            on_device_removed: DeviceRemovedStrategy::Pause,
            on_new_device_inserted: DeviceInsertedStrategy::Ignore,
            last_used_device_id: None,
            last_output_device_id: None,
        }
    }
}

// ── Library nested sub-config (Plan A Task A6) ──────────────────────────
// CROSS-PLAN ORDER CONVENTION (严格遵守，与 C 计划 Task2 Step2 / B 计划
// Task1 Step1 一致):
//   nested struct 定义顺序: (1) DspSettings (C) → (2) HotplugSettings (B)
//   → (3) LibrarySettings (A/此处)
//   AppSettings struct field 顺序: (1) pub dsp: DspSettings →
//   (2) pub hotplug: HotplugSettings → (3) pub library (A/此处)
//   impl Default 初始化顺序同上。三组插入点都在结构体底部。
/// Nested media-library sub-config. All fields have `#[serde(default)]` on
/// container level (via AppSettings) so legacy flat settings.json parse
/// cleanly.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct LibrarySettings {
    /// When user clicks "同步文件夹" on a playlist, do we auto-add the path
    /// to media library roots? Default true. User opt-out in Settings →
    /// Audio → 媒体库状态卡片 → switch.
    pub auto_sync_playlist_folder_to_roots: bool,
}
impl Default for LibrarySettings {
    fn default() -> Self {
        Self {
            auto_sync_playlist_folder_to_roots: true,
        }
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            replay_gain_mode: ReplayGainMode::Track,
            volume_mode: VolumeMode::Software,
            plugin_directory: "./plugins".to_string(),
            hotplug_poll_interval_ms: 2000,
            buffer_size: 4800, // 50ms Balanced — matches Settings UI BUFFER_OPTIONS
            resampler_quality: "Balanced".to_string(),
            theme: "Dark".to_string(),
            debug_log: false,
            startup_behavior: "Remember".to_string(),
            spectrum_smoothing: 0.35,
            spectrum_decay: 0.90,
            spectrum_fps: 60,
            hotplug_notifications: true,
            replay_gain_preamp: 0.0,
            spectrum_colorscheme: "aurora".to_string(),
            plugin_max_memory_mb: 128,
            plugin_call_timeout_ms: 5000,
            language: "zh-CN".to_string(),
            output_sample_rate: 0, // auto-detect
            bit_depth: 0,          // float32 passthrough
            dsd_mode: 0,           // DSD Off
            exclusive_mode: false,
            close_to_tray: true,
            lyrics_api_url: "https://lrclib.net".to_string(),
            desktop_lyrics_font_size: 22.0,
            desktop_lyrics_opacity: 1.0,
            desktop_lyrics_text_color: "#ffffff".to_string(),
            desktop_lyrics_locked: false,
            desktop_lyrics_bg_opacity: 0.45,
            desktop_lyrics_shadow_enabled: true,
            desktop_lyrics_always_on_top: true,
            desktop_lyrics_played_color: "#ffffff".to_string(),
            desktop_lyrics_unplayed_color: "#888888".to_string(),
            desktop_lyrics_bg_mode: "hidden".to_string(),
            desktop_lyrics_bg_path: String::new(),
            desktop_lyrics_line_mode: "single".to_string(),
            desktop_lyrics_scroll_fx: true,
            desktop_lyrics_equal_size: false,
            desktop_lyrics_line_style: "scroll".to_string(),
            desktop_lyrics_x: 0.0,
            desktop_lyrics_y: 0.0,
            desktop_lyrics_width: 900.0,
            desktop_lyrics_height: 120.0,
            desktop_lyrics_position_set: false,
            main_window_x: 0.0,
            main_window_y: 0.0,
            main_window_width: 1280.0,
            main_window_height: 800.0,
            main_window_position_set: false,
            main_window_maximized: false,
            dsp: DspSettings::default(),
            hotplug: HotplugSettings::default(),
            library: LibrarySettings::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplayGainMode {
    Off,
    Track,
    Album,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VolumeMode {
    Hardware,
    Software,
}

/// DSP processor info for the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DspInfo {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub latency_ms: f64,
}

/// The global application state.
pub struct AppState {
    pub engine: Arc<PlaybackEngine>,
    pub device_manager: Arc<Mutex<DeviceManager>>,
    pub dsp_chain: Arc<Mutex<DspChain>>,
    pub volume: Arc<Mutex<VolumeControl>>,
    pub settings: Arc<Mutex<AppSettings>>,
    pub playlist: Arc<Mutex<Playlist>>,
    pub plugin_runtime: Arc<PluginRuntime>,
    pub session_loaded: Mutex<bool>,
    /// User-created playlists: name -> list of paths.
    pub playlists: Mutex<HashMap<String, Vec<String>>>,
    /// Playlist display order (new playlists appended at the end).
    pub playlist_order: Mutex<Vec<String>>,
    /// Currently active playlist name (empty = default/main queue).
    pub active_playlist: Mutex<String>,
    /// Flag set by `prepare_for_reload` when the frontend is about to
    /// unload (HMR refresh, F5). While true, the event-forwarding thread
    /// discards all events (no `app.emit`) so the dying JS context
    /// doesn't receive callbacks that would produce
    /// "[TAURI] Couldn't find callback id" warnings.
    /// Reset to false by `load_session` on the next mount.
    pub frontend_reloading: Arc<AtomicBool>,
    /// Media library handle (opened lazily on first scan/query).
    /// The inner `Option` is `None` until the user triggers a scan or
    /// library_* command for the first time.
    pub media_library: MediaLibraryHandle,
    /// Configured library roots (absolute paths). Driven by the UI.
    pub library_roots: Mutex<Vec<std::path::PathBuf>>,
    /// Playlist → folder path mapping for auto-sync. Each playlist can
    /// bind at most one folder, and each folder binds to one playlist.
    /// When audio files change in a synced folder, only the bound
    /// playlist is updated.
    pub synced_folders: Mutex<std::collections::HashMap<String, String>>,
    /// Filesystem watcher handle. Owned here to keep it alive.
    pub folder_watcher: Mutex<Option<notify::RecommendedWatcher>>,
    /// Tauri AppHandle, set once in setup() so hotplug callbacks can emit
    /// frontend events (`phonon:toast`, `phonon:audioDeviceSwitch`) and the
    /// debug_inject_hotplug_event command can forward synthetic events.
    /// `None` until main() setup calls `.set(...)`.
    pub app_handle: std::sync::OnceLock<tauri::AppHandle>,
    /// Set during startup if settings.json was found corrupt and had to be
    /// reset to defaults. The frontend picks this up on mount and shows a
    /// warning toast to the user (since the early `phonon:toast` emit may
    /// arrive before listeners are registered).
    pub settings_corrupt_notice: Mutex<Option<String>>,

    // ── PEQ DSP processor (general-purpose parametric EQ) ──────────────
    /// Biquad PEQ update handle from the PEQ DSP processor.
    /// General-purpose parametric EQ available to plugins and DSP chain.
    pub peq_handle: PeqUpdateHandle,
}

impl AppState {
    /// Create a new application state.
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let config = EngineConfig::default();
        let engine = PlaybackEngine::new(config)?;
        let device_manager = engine.device_manager();
        let dsp_chain = engine.dsp_chain();
        let volume = engine.volume();

        // Add ReplayGain processor to DSP chain
        dsp_chain
            .lock()
            .unwrap()
            .add(Box::new(ReplayGainProcessor::new()));
        // Add Smart Effect processor to DSP chain
        dsp_chain
            .lock()
            .unwrap()
            .add(Box::new(SmartEffectProcessor::new()));
        // Add Surround Sound processor to DSP chain
        dsp_chain
            .lock()
            .unwrap()
            .add(Box::new(SurroundSoundProcessor::new()));
        // Add Equalizer processor to DSP chain
        dsp_chain
            .lock()
            .unwrap()
            .add(Box::new(EqualizerProcessor::new()));
        // Add Downmix processor to DSP chain (5.1/7.1 → stereo)
        dsp_chain
            .lock()
            .unwrap()
            .add(Box::new(DownmixProcessor::new()));
        // ── PEQ (Parametric EQ): native DSP processor registered in the chain ──
        //    Position: after Equalizer, before FIR Wasm plugins.
        //    General-purpose biquad PEQ available to plugins via the handle.
        let peq_sr: u32 = engine.sample_rate();
        let (peq_proc, peq_handle) =
            PeqProcessor::create(peq_sr);
        dsp_chain
            .lock()
            .unwrap()
            .add(Box::new(peq_proc));

        let mut settings = AppSettings::default();

        // Apply the default volume mode from settings to the VolumeControl.
        // VolumeControl defaults to Hardware if supported, but AppSettings
        // defaults to Software — sync them so the frontend sees the correct mode.
        {
            use phonon_core::volume::VolumeMode as CoreVolumeMode;
            let core_vm = match settings.volume_mode {
                VolumeMode::Hardware => CoreVolumeMode::Hardware,
                VolumeMode::Software => CoreVolumeMode::Software,
            };
            volume.lock().unwrap().set_mode(core_vm);
        }

        // Load persisted plugin limits (independent of session save which is
        // gated by startup_behavior == "Remember").
        {
            let limits_path = crate::commands::data_dir_pub().join("plugin_limits.json");
            if limits_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&limits_path) {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                        if let Some(mm) = v["plugin_max_memory_mb"].as_u64() {
                            settings.plugin_max_memory_mb = (mm as u32).clamp(16, 512);
                        }
                        if let Some(ct) = v["plugin_call_timeout_ms"].as_u64() {
                            settings.plugin_call_timeout_ms = ct.clamp(1000, 30000);
                        }
                        log::info!(
                            "Loaded persisted plugin limits: {}MB / {}ms",
                            settings.plugin_max_memory_mb,
                            settings.plugin_call_timeout_ms
                        );
                    }
                }
            }
        }

        // Resolve plugin directory: try multiple candidate paths
        let plugin_dir = {
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            log::info!("CWD: {}", cwd.display());

            let exe_dir = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .unwrap_or_default();
            log::info!("EXE dir: {}", exe_dir.display());

            // Compile-time manifest path: src-tauri/../plugins = phonon-tauri/plugins/
            let manifest_plugins = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join(&settings.plugin_directory);

            // Build candidate paths — prefer project-root plugins/ then fall back
            // to compile-time paths. No longer uses plugins-dist/.
            let candidates: Vec<std::path::PathBuf> = vec![
                cwd.join(&settings.plugin_directory), // ./plugins (from cwd)
                exe_dir
                    .join("../../../../")
                    .join(&settings.plugin_directory), // from target/debug/ -> project root/plugins
                manifest_plugins.clone(),             // compile-time: phonon-tauri/plugins
                cwd.join("phonon-tauri").join(&settings.plugin_directory), // phonon-tauri/plugins (from project root)
                exe_dir.join("../../").join(&settings.plugin_directory), // from target/debug/ -> src-tauri/plugins
                exe_dir.join("../../../").join(&settings.plugin_directory), // from target/debug/ -> phonon-tauri/plugins
            ];

            // Prefer directories that contain .wasm files, fall back to first existing dir
            let mut found = None;
            for candidate in &candidates {
                log::info!("Trying plugin dir: {}", candidate.display());
                if candidate.exists() {
                    // Check if this directory has any .wasm files
                    let has_wasm = std::fs::read_dir(candidate)
                        .map(|entries| {
                            entries
                                .filter_map(|e| e.ok())
                                .any(|e| e.path().extension().is_some_and(|ext| ext == "wasm"))
                        })
                        .unwrap_or(false);
                    if has_wasm {
                        log::info!("Found plugin dir with .wasm files: {}", candidate.display());
                        found = Some(candidate.clone());
                        break;
                    }
                    // Remember first existing directory as fallback
                    if found.is_none() {
                        log::info!("Found plugin dir (no .wasm files): {}", candidate.display());
                        found = Some(candidate.clone());
                    }
                }
            }

            found.unwrap_or_else(|| {
                let default = manifest_plugins;
                log::info!(
                    "No existing plugin dir found, using default: {}",
                    default.display()
                );
                default
            })
        };
        log::info!("Plugin directory: {}", plugin_dir.display());

        let plugin_runtime = PluginRuntime::new(&plugin_dir)
            .map_err(|e| format!("Failed to initialize plugin runtime: {}", e))?;
        if let Err(e) = plugin_runtime.scan() {
            log::warn!("Plugin scan failed: {}", e);
        }

        // Apply persisted plugin limits to the runtime (overrides the defaults
        // that PluginRuntime::new set from PluginResourceLimits::default()).
        {
            let limits = phonon_plugin::PluginResourceLimits {
                max_memory: (settings.plugin_max_memory_mb as usize) * 1024 * 1024,
                max_call_timeout_ms: settings.plugin_call_timeout_ms,
            };
            plugin_runtime.set_resource_limits(limits);
            plugin_runtime.set_call_timeout(settings.plugin_call_timeout_ms);
        }

        // Default built-in TimeStretch factory (Auto mode; plugin overrides later)
        engine.set_time_stretch_factory(Some(phonon_core::engine::time_stretch_factory(
            settings.dsp.time_stretch_mode,
        )));

        Ok(Self {
            engine: Arc::new(engine),
            device_manager,
            dsp_chain,
            volume,
            settings: Arc::new(Mutex::new(settings)),
            playlist: Arc::new(Mutex::new(Playlist::new("Untitled"))),
            plugin_runtime: Arc::new(plugin_runtime),
            session_loaded: Mutex::new(false),
            playlists: Mutex::new(HashMap::new()),
            playlist_order: Mutex::new(Vec::new()),
            active_playlist: Mutex::new(String::new()),
            frontend_reloading: Arc::new(AtomicBool::new(false)),
            media_library: Arc::new(Mutex::new(None)),
            library_roots: Mutex::new(Vec::new()),
            synced_folders: Mutex::new(std::collections::HashMap::new()),
            folder_watcher: Mutex::new(None),
            app_handle: std::sync::OnceLock::new(),
            settings_corrupt_notice: Mutex::new(None),
            // ── PEQ DSP processor (general-purpose parametric EQ) ──────────────
            peq_handle: peq_handle,
        })
    }

    /// Start device hotplug monitoring and forward events to the module-level
    /// `hotplug_event_handler`. The handler runs the configured remove/insert
    /// strategy pipeline (Pause / SwitchDefault / SwitchLastUsed + daisy chain
    /// fallback → final Pause; AutoSwitch on insert). `last_used_device_id` is
    /// NEVER mutated here — single-writer rule belongs to `commands::set_device`.
    pub fn start_hotplug_monitor(&self) {
        // BEFORE taking the lock, clone 4 Arcs + try clone AppHandle so the
        // closure captures are cheap and do NOT overlap with the MutexGuard
        // borrow (otherwise rustc fires E0505 "cannot move out of dm because it
        // is borrowed").
        let dm_cb = self.device_manager.clone();
        let engine_cb = self.engine.clone();
        let settings_cb = self.settings.clone();
        let app_handle_cb: Option<tauri::AppHandle> = self.app_handle.get().cloned();
        let interval = self.settings.lock().unwrap().hotplug_poll_interval_ms;

        // Guard lives ONLY in this block. The closure was built from clones
        // taken BEFORE the guard existed, so no borrow/move conflict.
        {
            let mut guard = self.device_manager.lock().unwrap();
            guard.start_hotplug_monitor(interval, move |event| {
                crate::state::hotplug_event_handler(
                    event,
                    dm_cb.clone(),
                    engine_cb.clone(),
                    settings_cb.clone(),
                    app_handle_cb.clone(),
                );
            });
        }
    }
}

// ──────────────────────────────────────────────────────────────
// Hotplug strategy engine. Module-level (not inside closure)
// so no "define fn inside move closure" compile errors.
//
// Deadlock discipline: `dm` lock is acquired SHORT-TERM only to
// read device names / list devices. The lock is DROPPED
// explicitly via scoped blocks before engine.* calls. Never
// hold dm.lock() across PlaybackEngine::seek / restart_output.
//
// Frontend notification rules:
//   • Toast → app_handle.emit("phonon:toast", {msg, level}); if app_handle
//     is None → degrade to log::info! (typical in unit-test / headless contexts).
//   • Real device connect/disconnect broadcast → use the EXISTING
//     AppEvent::DeviceHotplug variant via global phonon_core::EVENT_TX
//     (a std::sync::OnceLock<broadcast::Sender>, NOT a thread_local! — use
//     EVENT_TX.get().and_then(|tx| tx.send(...).ok())).
//   • NEVER attempt AppEvent::InfoToast — that variant does not exist.
// ──────────────────────────────────────────────────────────────
pub(crate) fn hotplug_event_handler(
    event: phonon_core::device::DeviceEvent,
    dm: Arc<Mutex<phonon_core::DeviceManager>>,
    engine: Arc<phonon_core::PlaybackEngine>,
    settings: Arc<Mutex<crate::state::AppSettings>>,
    app_handle: Option<tauri::AppHandle>,
) {
    use DeviceRemovedStrategy::*;
    use DeviceInsertedStrategy::*;

    // ── Tiny toast helper — encapsulates app_handle-or-log dispatch ───
    fn toast(app: &Option<tauri::AppHandle>, msg: String, level: &str) {
        if let Some(h) = app {
            let _ = h.emit(
                "phonon:toast",
                serde_json::json!({ "msg": msg, "level": level }),
            );
        } else {
            log::info!("[hotplug:toast:{level}] {msg}");
        }
    }
    // ── Tiny DeviceHotplug broadcast helper ────────────────────────────
    fn broadcast_device_hotplug(event_name: &str, device_name: String, is_default: bool) {
        if let Some(tx) = phonon_core::EVENT_TX.get() {
            let _ = tx.send(AppEvent::DeviceHotplug {
                event: event_name.to_string(),
                device_name,
                is_default,
            });
        }
    }

    // ── Collect current playback state ASAP (no locks held) ───
    let current_state = engine.state();
    let was_playing = current_state == phonon_core::PlaybackState::Playing;
    let was_active = current_state == phonon_core::PlaybackState::Playing
        || current_state == phonon_core::PlaybackState::Paused;
    let position = engine.position();
    let queue_idx = engine.queue_index();

    match event {
        phonon_core::device::DeviceEvent::DeviceAdded(info) => {
            // Real device broadcast goes over EVENT_TX (AppEvent::DeviceHotplug is real).
            broadcast_device_hotplug("connect", info.name.clone(), info.is_default);

            // Optional user-facing toast (if notifications enabled).
            let notify = settings
                .lock()
                .ok()
                .map(|s| s.hotplug_notifications)
                .unwrap_or(true);
            if notify {
                toast(
                    &app_handle,
                    format!(
                        "🎧 设备接入: {}{}",
                        info.name,
                        if info.is_default { "（默认）" } else { "" }
                    ),
                    "info",
                );
            }

            let strat = settings
                .lock()
                .ok()
                .map(|s| s.hotplug.on_new_device_inserted)
                .unwrap_or_default();
            if matches!(strat, AutoSwitch) && info.is_default {
                log::info!(
                    "[hotplug:insert] strategy=AutoSwitch, new default {:?} → switching",
                    info.name
                );
                // Drop settings before apply_device_internal (nested mutex paranoia).
                let _ = apply_device_internal(
                    dm.clone(),
                    engine.clone(),
                    info.id,
                    was_active,
                    position,
                    queue_idx,
                    ApplyOpts {
                        persist_last_used: false,
                        emit_frontend: true,
                        app_handle: app_handle.clone(),
                    },
                );
            }
        }

        phonon_core::device::DeviceEvent::DeviceRemoved(removed_id) => {
            // 1) Is the removed device our CURRENT one?
            let is_current = dm
                .lock()
                .ok()
                .and_then(|guard| guard.current_device_name())
                .map(|cur| cur == removed_id)
                .unwrap_or(false);

            broadcast_device_hotplug("disconnect", removed_id.clone(), false);

            let notify = settings
                .lock()
                .ok()
                .map(|s| s.hotplug_notifications)
                .unwrap_or(true);
            if notify {
                toast(
                    &app_handle,
                    format!(
                        "🔌 设备拔出: {}{}",
                        removed_id,
                        if is_current { "（正在使用 — 尝试策略切换）" } else { "" }
                    ),
                    if is_current { "warn" } else { "info" },
                );
            }

            if !is_current {
                return;
            }

            // Clear stale device reference (helps DeviceManager not point to a dead device).
            if let Ok(mut guard) = dm.lock() {
                guard.clear_current_device();
            }

            // 2) Run strategy pipeline with guaranteed final fallback to Pause.
            let strat = settings
                .lock()
                .ok()
                .map(|s| s.hotplug.on_device_removed)
                .unwrap_or_default();
            match strat {
                Pause => {
                    log::info!(
                        "[hotplug:remove] strategy=Pause → stop playback, queue_idx={}, pos_secs={}",
                        queue_idx,
                        position
                    );
                    if was_active {
                        engine.stop();
                        engine.set_queue_index(queue_idx);
                        engine.seek(position);
                    }
                }
                SwitchDefault => {
                    // Try default only. If OK continue; else final Pause.
                    let maybe_default = dm.lock().ok().and_then(|g| g.default_device_name());
                    let r = maybe_default.and_then(|default_id| {
                        log::info!("[hotplug:remove] strategy=SwitchDefault → try {:?}", default_id);
                        apply_device_internal(
                            dm.clone(),
                            engine.clone(),
                            default_id.clone(),
                            was_active,
                            position,
                            queue_idx,
                            ApplyOpts {
                                persist_last_used: false,
                                emit_frontend: true,
                                app_handle: app_handle.clone(),
                            },
                        )
                        .ok()
                        .map(|_| default_id)
                    });
                    if r.is_none() {
                        log::warn!(
                            "[hotplug:remove] strategy=SwitchDefault failed → FINAL FALLBACK pause"
                        );
                        if was_active {
                            engine.stop();
                            engine.set_queue_index(queue_idx);
                            engine.seek(position);
                        }
                    }
                }
                SwitchLastUsed => {
                    // Daisy chain: (A) last_used → (B) default → (C) pause.
                    let last_used = settings
                        .lock()
                        .ok()
                        .and_then(|s| s.hotplug.last_used_device_id.clone());
                    let r_a = last_used.clone().and_then(|id| {
                        log::info!("[hotplug:remove] tier1 switch_last_used → try {:?}", id);
                        apply_device_internal(
                            dm.clone(),
                            engine.clone(),
                            id.clone(),
                            was_active,
                            position,
                            queue_idx,
                            ApplyOpts {
                                persist_last_used: false,
                                emit_frontend: true,
                                app_handle: app_handle.clone(),
                            },
                        )
                        .ok()
                        .map(|_| id)
                    });
                    if r_a.is_some() {
                        return;
                    }

                    let maybe_default = dm.lock().ok().and_then(|g| g.default_device_name());
                    let r_b = maybe_default.and_then(|default_id| {
                        log::info!(
                            "[hotplug:remove] tier2 cascade switch_default → try {:?}",
                            default_id
                        );
                        apply_device_internal(
                            dm.clone(),
                            engine.clone(),
                            default_id.clone(),
                            was_active,
                            position,
                            queue_idx,
                            ApplyOpts {
                                persist_last_used: false,
                                emit_frontend: true,
                                app_handle: app_handle.clone(),
                            },
                        )
                        .ok()
                        .map(|_| default_id)
                    });
                    if r_b.is_some() {
                        return;
                    }

                    log::warn!(
                        "[hotplug:remove] strategy=SwitchLastUsed: tier1(last_used={:?}) + \
                         tier2(default) both failed → FINAL FALLBACK pause",
                        last_used
                    );
                    if was_active {
                        engine.stop();
                        engine.set_queue_index(queue_idx);
                        engine.seek(position);
                    }
                }
            }
        }

        phonon_core::device::DeviceEvent::DefaultDeviceChanged(_info) => {
            // Optional UX: if user has AutoSwitch, changing default in Sound Control
            // Panel without a new device plug also switches. We choose NO — only
            // DeviceAdded triggers AutoSwitch (avoids jitter when user fiddles
            // Sound CPL during playback).
        }

        phonon_core::device::DeviceEvent::DeviceChanged(_info) => {
            // Device capability changed (e.g. sample rate via USB). Today:
            // restart stream so the new capabilities take effect (cheap — not
            // a real device change).
            if was_active {
                let pos = position;
                let was_p = was_playing;
                std::thread::spawn(move || {
                    if !was_p {
                        engine.pause();
                    }
                    engine.seek(pos);
                    let _ = engine.restart_output_stream();
                });
            }
        }
    }
}

// ── Options for apply_device_internal ─────────────────────────
struct ApplyOpts {
    /// If true → this call is the top-level "user requested switch" and
    /// should write last_used_device_id via set_device (which calls save_settings).
    /// If false → we are inside hotplug callback, NEVER mutate last_used_device_id
    /// (single-writer rule).
    persist_last_used: bool,
    /// If true → emit phonon:audioDeviceSwitch to frontend so the device
    /// dropdown selection updates.
    emit_frontend: bool,
    /// Forwarded from hotplug_event_handler; used for toasts. None → log only.
    app_handle: Option<tauri::AppHandle>,
}

/// Core device-apply shared between set_device + hotplug strategies.
/// Caller MUST NOT hold `dm.lock()` when this function runs (deadlock-free design).
fn apply_device_internal(
    dm: Arc<Mutex<phonon_core::DeviceManager>>,
    engine: Arc<phonon_core::PlaybackEngine>,
    new_device_id: String,
    was_active: bool,
    position_secs: f64,
    queue_idx: usize,
    opts: ApplyOpts,
) -> Result<(), String> {
    // 1) Verify target device STILL EXISTS (race: device hot-unplugged between poll + apply)
    let verified_device_id: String = {
        let guard = dm.lock().map_err(|e| e.to_string())?;
        let list = guard.list_devices().map_err(|e| e.to_string())?;
        let found = list.iter().find(|d| d.id == new_device_id || d.name == new_device_id);
        found
            .ok_or_else(|| {
                format!(
                    "Device {:?} no longer present in list_devices()",
                    new_device_id
                )
            })?
            .id
            .clone()
    }; // <-- dm lock DROPPED here (critical)

    // 2) Tell DeviceManager to remember the new default
    {
        let mut guard = dm.lock().map_err(|e| e.to_string())?;
        guard.set_default_device(&verified_device_id).map_err(|e| e.to_string())?;
    } // <-- dm lock DROPPED here (critical — before engine long-ops)

    // 3) Restart engine stream (existing flow)
    if was_active {
        let was_playing = engine.state() == phonon_core::PlaybackState::Playing;
        let engine2 = engine.clone();
        std::thread::spawn(move || {
            if !was_playing {
                engine2.pause();
            }
            // Reset queue index so we don't advance songs
            engine2.set_queue_index(queue_idx);
            engine2.seek(position_secs);
            if let Err(e) = engine2.restart_output_stream() {
                log::error!("[apply_device_internal] restart_output_stream failed: {}", e);
            }
        });
    }

    // 4) Optional frontend emit: device dropdown refresh + human toast.
    if opts.emit_frontend {
        if let Some(h) = &opts.app_handle {
            let _ = h.emit(
                "phonon:audioDeviceSwitch",
                serde_json::json!({ "new_device_id": verified_device_id }),
            );
        }
        // Also log a visible banner; app_handle → toast, else → log.
        if let Some(h) = &opts.app_handle {
            let _ = h.emit(
                "phonon:toast",
                serde_json::json!({
                    "msg": format!("♪ 输出切换到: {}", verified_device_id),
                    "level": "info"
                }),
            );
        } else {
            log::info!("[apply_device_internal] 输出切换到: {}", verified_device_id);
        }
    }

    // 5) persist_last_used: only allowed when called DIRECTLY from set_device command.
    if opts.persist_last_used {
        log::error!(
            "[apply_device_internal] persist_last_used=true should never be used here — use \
             commands::set_device instead (it has proper AppSettings write order)"
        );
    }

    Ok(())
}
