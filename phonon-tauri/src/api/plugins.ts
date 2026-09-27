/**
 * Plugins API — Wasm plugin lifecycle, TimeStretch, visualizer/lyric scripts,
 * and file scanning/metadata extraction.
 *
 * Wraps the `list_plugins`/`scan_vis_sources`/`get_track_metadata`/... commands.
 */

import { invoke } from '@tauri-apps/api/core';

// ---------------------------------------------------------------------------
// Types — Wasm plugins (mirror `phonon_plugin::types`)
// ---------------------------------------------------------------------------

/** Plugin interface type (mirrors `phonon_plugin::types::PluginType`). */
export type PluginType =
  | 'SourceProvider'
  | 'DspProcessor'
  | 'AudioDecoder'
  | 'TimeStretch'
  | 'Visualizer';

/** Permissions a Wasm plugin can request (mirrors `PluginPermission`). */
export type PluginPermission =
  | 'FileSystem'
  | 'Network'
  | 'AudioOutput'
  | 'AudioAnalysis';

/** Plugin manifest (mirrors `phonon_plugin::types::PluginManifest`). */
export interface PluginManifest {
  name: string;
  version: string;
  author: string;
  description: string;
  plugin_type: PluginType;
  permissions: PluginPermission[];
  min_framework_version: string;
}

/** Plugin lifecycle state (mirrors `phonon_plugin::types::PluginState`). */
export type PluginState = 'Discovered' | 'Loaded' | 'Running' | 'Error' | 'Disabled';

/** Plugin origin / 三层分类（mirrors `phonon_plugin::types::PluginOrigin`）。 */
export type PluginOrigin = 'builtin' | 'wasmExample' | 'wasmExternal';

/** Plugin info (mirrors `phonon_plugin::types::PluginInfo`). */
export interface PluginInfo {
  manifest: PluginManifest;
  state: PluginState;
  file_path: string;
  error_message: string | null;
  origin: PluginOrigin;
}

/** 从 Wasm 插件经 host::emit_event 转发到前端的自定义事件。
 *
 * 由 AppEvent::PluginUserEvent 承载，前端 window 上会派发
 * `CustomEvent<PluginUserEvent>('plugin-user-event')`。
 */
export interface PluginUserEvent {
  name: string;
  payload: string;
}

// ---------------------------------------------------------------------------
// Types — Lyrics
// ---------------------------------------------------------------------------

/** Single word timing entry in an enhanced LRC line. */
export interface LyricWord {
  time_ms: number;
  text: string;
}

/** Single lyric line with optional word-level timings. */
export interface LyricLine {
  time_ms: number;
  text: string;
  words: LyricWord[];
}

/** Parsed LRC / lyrics result. */
export interface LyricsResult {
  lines: LyricLine[];
}

/** Online lyrics search result entry. */
export interface LyricsSearchResult {
  id: number;
  artist: string;
  title: string;
  album?: string;
  duration?: number;
  provider?: string;
}

// ---------------------------------------------------------------------------
// Types — Desktop Lyrics Settings
// ---------------------------------------------------------------------------

export interface DesktopLyricsSettings {
  font_family?: string;
  font_size?: number;
  font_weight?: number;
  stroke_width?: number;
  primary_color?: string;
  secondary_color?: string;
  stroke_color?: string;
  shadow_color?: string;
  shadow_blur?: number;
  opacity?: number;
  line_spacing?: number;
  show_translation?: boolean;
  translation_font_size?: number;
  translation_color?: string;
  always_on_top?: boolean;
  click_through?: boolean;
  lock_position?: boolean;
  show_progress_bar?: boolean;
  progress_bar_color?: string;
  karaoke_mode?: boolean;
  dual_line?: boolean;
  pong_mode?: boolean;
  pong_speed?: number;
  blur_amount?: number;
  background_image?: string;
  background_opacity?: number;
}

// ---------------------------------------------------------------------------
// Types — file scanning & metadata
// ---------------------------------------------------------------------------

/** Scan result entry (mirrors `commands::SearchResultJson`). */
export interface ScanResultEntry {
  uri: string;
  name: string;
}

/** Track metadata (mirrors `commands::TrackMetadataJson`). */
export interface TrackMetadata {
  title: string | null;
  artist: string | null;
  album: string | null;
  album_artist: string | null;
  genre: string | null;
  year: number | null;
  track_number: number | null;
  total_tracks: number | null;
  disc_number: number | null;
  total_discs: number | null;
  duration: number | null;
  sample_rate: number | null;
  bit_depth: number | null;
  channels: number | null;
  bitrate: number | null;
  format: string | null;
  has_album_art: boolean;
}

/** Album art bytes (mirrors `commands::AlbumArtResponse`). */
export interface AlbumArt {
  data: number[];
  mime_type: string;
}

/** Lyric/Vis script entry (mirrors `LyricSourceEntry` / `VisSourceEntry`). */
export interface ScriptEntry {
  name: string;
  file_name: string;
}

// ---------------------------------------------------------------------------
// Wasm plugin lifecycle
// ---------------------------------------------------------------------------

export function importPlugin(sourcePath: string): Promise<PluginInfo[]> {
  // Tauri v2 camelCase key mapping (Rust param source_path).
  return invoke('import_plugin', { sourcePath });
}

export function listPlugins(): Promise<PluginInfo[]> {
  return invoke('list_plugins');
}

export function enablePlugin(id: string): Promise<void> {
  return invoke('enable_plugin', { id });
}

export function disablePlugin(id: string): Promise<void> {
  return invoke('disable_plugin', { id });
}

export function scanPlugins(): Promise<PluginInfo[]> {
  return invoke('scan_plugins');
}

export function getPluginDir(): Promise<string> {
  return invoke('get_plugin_dir');
}

export function openPluginDir(): Promise<void> {
  return invoke('open_plugin_dir');
}

export function removePlugin(filePath: string): Promise<PluginInfo[]> {
  return invoke('remove_plugin', { filePath });
}

export function reloadPlugin(filePath: string): Promise<PluginInfo[]> {
  return invoke('reload_plugin', { filePath });
}

export function getPluginConfig(name: string): Promise<string | null> {
  return invoke('get_plugin_config', { name });
}

export function setPluginConfig(name: string, config: string): Promise<void> {
  return invoke('set_plugin_config', { name, config });
}

/** 在 DSP 链中按插件名 toggle 启用/禁用。
 *
 * 对 Builtin 自动 remap 到 5 个 DSP 处理器 id；
 * 对 Wasm DSP 插件按 `DspProcessor::name() == name` 匹配；
 * 非 DSP 类型插件（TimeStretch / SourceProvider / AudioDecoder / Visualizer）
 * 会抛出带原因的错误字符串。
 */
export function togglePluginInDsp(name: string, enabled: boolean): Promise<void> {
  return invoke('toggle_plugin_in_dsp', { name, enabled });
}

/**
 * Call a custom command on a loaded WASM plugin.
 *
 * The plugin must export `plugin_command`. Input and output are JSON strings.
 * This is intended for non-real-time operations (computation, analysis, etc.).
 *
 * @param pluginName - Plugin name (matches `plugin_name()` export)
 * @param command - Command name the plugin understands
 * @param inputJson - JSON string input for the command
 * @returns JSON string output from the plugin
 */
export function callPluginCommand(pluginName: string, command: string, inputJson: string): Promise<string> {
  return invoke('call_plugin_command', { pluginName, command, inputJson });
}

// ---------------------------------------------------------------------------
// TimeStretch plugins
// ---------------------------------------------------------------------------

export function loadTimeStretchPlugin(filePath: string): Promise<void> {
  return invoke('load_time_stretch_plugin', { filePath });
}

export function unloadTimeStretchPlugin(filePath: string): Promise<void> {
  return invoke('unload_time_stretch_plugin', { filePath });
}

// ---------------------------------------------------------------------------
// Visualization & lyric scripts
// ---------------------------------------------------------------------------

export function scanVisSources(): Promise<ScriptEntry[]> {
  return invoke('scan_vis_sources');
}

export function readVisScript(fileName: string): Promise<string> {
  return invoke('read_vis_script', { fileName });
}

export function scanLyricSources(): Promise<ScriptEntry[]> {
  return invoke('scan_lyric_sources');
}

export function readLyricScript(fileName: string): Promise<string> {
  return invoke('read_lyric_script', { fileName });
}

// ---------------------------------------------------------------------------
// File scanning & metadata
// ---------------------------------------------------------------------------

/** Scan a folder for audio files (filters out non-audio extensions). */
export function scanFolder(path: string): Promise<ScanResultEntry[]> {
  return invoke('scan_folder', { path });
}

export function getTrackMetadata(path: string): Promise<TrackMetadata> {
  return invoke('get_track_metadata', { path });
}

export function getAlbumArt(path: string): Promise<AlbumArt | null> {
  return invoke('get_album_art', { path });
}

export function getLyrics(path: string): Promise<LyricsResult | null> {
  return invoke('get_lyrics', { path });
}

export function loadLrcFile(path: string): Promise<LyricsResult | null> {
  return invoke('load_lrc_file', { path });
}

export function parseLrcText(lrcText: string): Promise<LyricsResult> {
  // Tauri v2 camelCase key mapping (Rust param lrc_text).
  return invoke('parse_lrc_text', { lrcText });
}

// ---------------------------------------------------------------------------
// Online lyrics search
// ---------------------------------------------------------------------------

/** Search lyrics by artist + title. Uses the configured lyrics API URL. */
export function searchLyrics(artist: string, title: string): Promise<LyricsSearchResult[]> {
  return invoke('search_lyrics', { artist, title });
}

/** Search lyrics by a freeform keyword (e.g. song snippet). */
export function searchLyricsByKeyword(keyword: string): Promise<LyricsSearchResult[]> {
  return invoke('search_lyrics_by_keyword', { keyword });
}

/** Fetch a specific lyric result by the provider's track id. */
export function fetchLyricsById(trackId: number): Promise<LyricsResult | null> {
  return invoke('fetch_lyrics_by_id', { trackId });
}

// ---------------------------------------------------------------------------
// Network & URL playback
// ---------------------------------------------------------------------------

/** Fetch a URL via the Rust backend (respects plugin network whitelists). */
export function fetchUrl(url: string): Promise<string> {
  return invoke('fetch_url', { url });
}

/** Add a network stream URL to the queue and start playback. */
export function playUrl(url: string): Promise<void> {
  return invoke('play_url', { url });
}

// ---------------------------------------------------------------------------
// Desktop lyrics window controls
// ---------------------------------------------------------------------------

/** Open the desktop-lyrics overlay window. */
export function openDesktopLyrics(): Promise<void> {
  return invoke('open_desktop_lyrics');
}

/** Close the desktop-lyrics overlay window. */
export function closeDesktopLyrics(): Promise<void> {
  return invoke('close_desktop_lyrics');
}

/** Get current desktop lyrics display settings. */
export function getDesktopLyricsSettings(): Promise<DesktopLyricsSettings> {
  return invoke('get_desktop_lyrics_settings');
}

/** Update desktop lyrics display settings (font, colors, opacity, etc.). */
export function setDesktopLyricsSettings(settings: DesktopLyricsSettings): Promise<void> {
  return invoke('set_desktop_lyrics_settings', { settings });
}

/** Persist the desktop lyrics window position/size to session storage. */
export function saveDesktopLyricsPosition(x: number, y: number, width: number, height: number): Promise<void> {
  // NOTE: Rust command takes only x/y/width/height — no position_set flag.
  return invoke('save_desktop_lyrics_position', { x, y, width, height });
}

/** Resolve the path where the custom desktop-lyrics background is saved. */
export function resolveDesktopLyricsBgPath(): Promise<string> {
  return invoke('resolve_desktop_lyrics_bg_path');
}

/** Write a binary blob as the new desktop-lyrics background image. */
export function saveDesktopLyricsBg(bytes: number[]): Promise<string> {
  return invoke('save_desktop_lyrics_bg', { bytes });
}

/** Reset desktop lyrics settings to their defaults. */
export function resetDesktopLyricsSettings(): Promise<void> {
  return invoke('reset_desktop_lyrics_settings');
}

/** Persist the main window position/size to session storage. */
export function saveMainWindowPosition(x: number, y: number, width: number, height: number, maximized: boolean): Promise<void> {
  // NOTE: Rust command takes only x/y/width/height/maximized — no position_set flag.
  return invoke('save_main_window_position', { x, y, width, height, maximized });
}
