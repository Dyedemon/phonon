/**
 * Settings API — application settings, session persistence, window position,
 * desktop lyrics settings, and app lifecycle.
 *
 * Wraps the `get_settings`/`update_settings`/`save_session`/... commands.
 */

import { invoke } from '@tauri-apps/api/core';

// ---------------------------------------------------------------------------
// Types — mirrors `state::AppSettings` (80+ fields).
// ---------------------------------------------------------------------------

/** Application settings (mirrors `state::AppSettings`). */
export interface AppSettings {
  replay_gain_mode: 'off' | 'track' | 'album';
  volume_mode: 'hardware' | 'software';
  plugin_directory: string;
  hotplug_poll_interval_ms: number;
  /** Buffer size in samples: 960 (10ms), 1920 (20ms), 4800 (50ms), 9600 (100ms). */
  buffer_size: number;
  resampler_quality: string;
  theme: string;
  debug_log: boolean;
  startup_behavior: string;
  spectrum_smoothing: number;
  spectrum_decay: number;
  spectrum_fps: number;
  hotplug_notifications: boolean;
  replay_gain_preamp: number;
  spectrum_colorscheme: string;
  plugin_max_memory_mb: number;
  plugin_call_timeout_ms: number;
  language: string;
  output_sample_rate: number;
  bit_depth: number;
  dsd_mode: number;
  close_to_tray: boolean;
  lyrics_api_url: string;
  // Desktop lyrics
  desktop_lyrics_font_size: number;
  desktop_lyrics_opacity: number;
  desktop_lyrics_text_color: string;
  desktop_lyrics_locked: boolean;
  desktop_lyrics_bg_opacity: number;
  desktop_lyrics_shadow_enabled: boolean;
  desktop_lyrics_always_on_top: boolean;
  desktop_lyrics_played_color: string;
  desktop_lyrics_unplayed_color: string;
  desktop_lyrics_bg_mode: string;
  desktop_lyrics_bg_path: string;
  desktop_lyrics_line_mode: string;
  desktop_lyrics_scroll_fx: boolean;
  desktop_lyrics_equal_size: boolean;
  desktop_lyrics_line_style: string;
  desktop_lyrics_x: number;
  desktop_lyrics_y: number;
  desktop_lyrics_width: number;
  desktop_lyrics_height: number;
  desktop_lyrics_position_set: boolean;
  // Main window
  main_window_x: number;
  main_window_y: number;
  main_window_width: number;
  main_window_height: number;
  main_window_position_set: boolean;
  main_window_maximized: boolean;
  /** DSP 子配置（嵌套；旧 JSON 由 Rust 端 #[serde(default)] 自动补全）。 */
  dsp: DspSettings;
  /** Hotplug 子配置（嵌套；旧 JSON 由 Rust 端 #[serde(default)] 自动补全）。 */
  hotplug: HotplugSettings;
  /** 媒体库子配置（嵌套；旧 JSON 由 Rust 端 #[serde(default)] 自动补全）。 */
  library: LibrarySettings;
}

/** Plugin resource limits (mirrors `commands::PluginLimits`). */
export interface PluginLimits {
  max_memory_mb: number;
  call_timeout_ms: number;
}

/** Spectrum settings tuple: (smoothing, decay, fps). */
export type SpectrumSettings = [number, number, number];

/** Desktop lyrics background mode. */
export type DesktopLyricsBgMode = 'hidden' | 'image' | 'video';

/** Desktop lyrics line mode. */
export type DesktopLyricsLineMode = 'single' | 'double';

/** Desktop lyrics line style (only active in double-line mode). */
export type DesktopLyricsLineStyle = 'scroll' | 'centered' | 'split';

/** TimeStretch 算法模式（mirrors `phonon_core::engine::TimeStretchMode`，serde snake_case）。 */
export type TimeStretchMode = 'auto' | 'wsola' | 'phase_vocoder';

/** 嵌套 DSP 子配置（mirrors `state::DspSettings`）。 */
export interface DspSettings {
  time_stretch_mode: TimeStretchMode;
}

/** 设备拔出策略（mirrors `state::DeviceRemovedStrategy`，serde snake_case）。 */
export type DeviceRemovedStrategy = 'pause' | 'switch_default' | 'switch_last_used';

/** 设备插入策略（mirrors `state::DeviceInsertedStrategy`，serde snake_case）。 */
export type DeviceInsertedStrategy = 'ignore' | 'auto_switch';

/** 嵌套 Hotplug 子配置（mirrors `state::HotplugSettings`）。 */
export interface HotplugSettings {
  on_device_removed: DeviceRemovedStrategy;
  on_new_device_inserted: DeviceInsertedStrategy;
  /** 单写者字段：仅 commands::set_device 在硬件切换前写入。 */
  last_used_device_id: string | null;
}

/**
 * 嵌套媒体库子配置（mirrors `state::LibrarySettings`）。
 *
 * `auto_sync_playlist_folder_to_roots` 是 §5.8.1 的双向同步开关：
 * - true  → PlaylistToolbar.syncFolder 成功后自动把该 folder 加入 library roots 并触发扫描
 * - false → 仅创建同步歌单，不动 roots（opt-out）
 */
export interface LibrarySettings {
  auto_sync_playlist_folder_to_roots: boolean;
}

// ---------------------------------------------------------------------------
// Settings CRUD
// ---------------------------------------------------------------------------

export function getSettings(): Promise<AppSettings> {
  return invoke('get_settings');
}

export function updateSettings(settings: Partial<AppSettings>): Promise<void> {
  return invoke('update_settings', { settings });
}

export function resetToDefaults(): Promise<void> {
  return invoke('reset_to_defaults');
}

/** Clean user data. Mode: 'keep_all' | 'keep_tracks' | 'clear_all' */
export function cleanUserData(mode: 'keep_all' | 'keep_tracks' | 'clear_all'): Promise<string> {
  return invoke('clean_user_data', { mode })
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

export function saveSession(): Promise<void> {
  return invoke('save_session');
}

export function loadSession(): Promise<void> {
  return invoke('load_session');
}

// ---------------------------------------------------------------------------
// Audio output
// ---------------------------------------------------------------------------

export function setBufferSize(size: number): Promise<void> {
  return invoke('set_buffer_size', { size });
}

export function getBufferSize(): Promise<number> {
  return invoke('get_buffer_size');
}

export function setResamplerQuality(quality: string): Promise<void> {
  return invoke('set_resampler_quality', { quality });
}

export function getResamplerQuality(): Promise<string> {
  return invoke('get_resampler_quality');
}

export function setBitDepth(depth: string): Promise<void> {
  return invoke('set_bit_depth', { depth });
}

export function getBitDepth(): Promise<string> {
  return invoke('get_bit_depth');
}

export function setOutputSampleRate(rate: number): Promise<void> {
  return invoke('set_output_sample_rate', { rate });
}

export function getOutputSampleRate(): Promise<number> {
  return invoke('get_output_sample_rate');
}

export function setDsdMode(mode: string): Promise<void> {
  return invoke('set_dsd_mode', { mode });
}

export function getDsdMode(): Promise<string> {
  return invoke('get_dsd_mode');
}

// ---------------------------------------------------------------------------
// Appearance
// ---------------------------------------------------------------------------

export function setTheme(theme: string): Promise<void> {
  return invoke('set_theme', { theme });
}

export function getTheme(): Promise<string> {
  return invoke('get_theme');
}

export function setLanguage(language: string): Promise<void> {
  return invoke('set_language', { language });
}

export function getLanguage(): Promise<string> {
  return invoke('get_language');
}

export function setDebugLog(enabled: boolean): Promise<void> {
  return invoke('set_debug_log', { enabled });
}

export function getDebugLog(): Promise<boolean> {
  return invoke('get_debug_log');
}

// ---------------------------------------------------------------------------
// Behavior
// ---------------------------------------------------------------------------

export function setStartupBehavior(behavior: string): Promise<void> {
  return invoke('set_startup_behavior', { behavior });
}

export function getStartupBehavior(): Promise<string> {
  return invoke('get_startup_behavior');
}

export function getCloseToTray(): Promise<boolean> {
  return invoke('get_close_to_tray');
}

export function setCloseToTray(enabled: boolean): Promise<void> {
  return invoke('set_close_to_tray', { enabled });
}

// ---------------------------------------------------------------------------
// Spectrum
// ---------------------------------------------------------------------------

export function setSpectrumSettings(
  smoothing: number,
  decay: number,
  fps: number,
): Promise<void> {
  return invoke('set_spectrum_settings', { smoothing, decay, fps });
}

export function getSpectrumSettings(): Promise<SpectrumSettings> {
  return invoke('get_spectrum_settings');
}

export function setSpectrumColorscheme(scheme: string): Promise<void> {
  return invoke('set_spectrum_colorscheme', { scheme });
}

export function getSpectrumColorscheme(): Promise<string> {
  return invoke('get_spectrum_colorscheme');
}

// ---------------------------------------------------------------------------
// Lyrics API
// ---------------------------------------------------------------------------

export function getLyricsApiUrl(): Promise<string> {
  return invoke('get_lyrics_api_url');
}

export function setLyricsApiUrl(url: string): Promise<void> {
  return invoke('set_lyrics_api_url', { url });
}

// ---------------------------------------------------------------------------
// Plugin limits
// ---------------------------------------------------------------------------

export function getPluginLimits(): Promise<PluginLimits> {
  return invoke('get_plugin_limits');
}

export function setPluginLimits(
  max_memory_mb: number,
  call_timeout_ms: number,
): Promise<void> {
  // Tauri v2 将 Rust snake_case 形参映射为 camelCase key，必须传 camelCase。
  return invoke('set_plugin_limits', {
    maxMemoryMb: max_memory_mb,
    callTimeoutMs: call_timeout_ms,
  });
}

// ---------------------------------------------------------------------------
// Window position
// ---------------------------------------------------------------------------

export function saveMainWindowPosition(pos: {
  x: number;
  y: number;
  width: number;
  height: number;
  maximized: boolean;
}): Promise<void> {
  return invoke('save_main_window_position', pos);
}

// ---------------------------------------------------------------------------
// App lifecycle
// ---------------------------------------------------------------------------

export function quitApp(): Promise<void> {
  return invoke('quit_app');
}

export function setIgnoreCursorEvents(ignore: boolean): Promise<void> {
  return invoke('set_ignore_cursor_events', { ignore });
}
