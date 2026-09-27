/**
 * Player API — playback control, volume, ReplayGain, and playback state.
 *
 * Wraps the `play`/`pause`/`seek`/`set_volume`/... Tauri commands.
 * JS argument keys are camelCase (Tauri v2 maps them to Rust snake_case).
 */

import { invoke } from '@tauri-apps/api/core';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** Playback state snapshot (mirrors `commands::PlaybackStateInfo`). */
export interface PlaybackStateInfo {
  /** PlaybackState enum debug string: "Playing" | "Paused" | "Stopped" | ... */
  state: string;
  position_secs: number;
  duration_secs: number | null;
  /** 0.0–1.0 buffer fill ratio. */
  buffer_fill: number;
  current_track: string | null;
  queue_length: number;
  speed: number;
}

/** ReplayGain mode. */
export type ReplayGainMode = 'off' | 'track' | 'album';

/** Volume control source. */
export type VolumeMode = 'hardware' | 'software';

/** Playback repeat mode. Must match backend PlaybackMode enum. */
export type PlaybackMode = 'normal' | 'repeat_all' | 'repeat_one';

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

export function play(): Promise<void> {
  return invoke('play');
}

export function pause(): Promise<void> {
  return invoke('pause');
}

export function togglePlayPause(): Promise<void> {
  return invoke('toggle_play_pause');
}

export function stop(): Promise<void> {
  return invoke('stop');
}

export function next(): Promise<void> {
  return invoke('next');
}

export function previous(): Promise<void> {
  return invoke('previous');
}

/** Seek to `positionSecs` (seconds, float). */
export function seek(positionSecs: number): Promise<void> {
  // Tauri v2 camelCase key mapping (Rust param position_secs).
  return invoke('seek', { positionSecs });
}

/** Play the track at `index` in the current queue. */
export function playIndex(index: number): Promise<void> {
  return invoke('play_index', { index });
}

// ---------------------------------------------------------------------------
// Volume
// ---------------------------------------------------------------------------

/** Set software/hardware volume. `level` is 0.0–1.0. */
export function setVolume(level: number): Promise<void> {
  return invoke('set_volume', { level });
}

export function getVolumeMode(): Promise<VolumeMode> {
  return invoke('get_volume_mode');
}

export function setVolumeMode(mode: VolumeMode): Promise<void> {
  return invoke('set_volume_mode', { mode });
}

export function getVolumeLevel(): Promise<number> {
  return invoke('get_volume_level');
}

/** Sync volume from the hardware device (returns current level 0.0–1.0). */
export function syncVolumeFromDevice(): Promise<number> {
  return invoke('sync_volume_from_device');
}

// ---------------------------------------------------------------------------
// Playback state & speed
// ---------------------------------------------------------------------------

export function getPlaybackState(): Promise<PlaybackStateInfo> {
  return invoke('get_playback_state');
}

/** Set playback speed (0.25–4.0). Returns the clamped speed. */
export function setPlaybackSpeed(speed: number): Promise<number> {
  return invoke('set_playback_speed', { speed });
}

export function getPlaybackSpeed(): Promise<number> {
  return invoke('get_playback_speed');
}

export function setPlaybackMode(mode: PlaybackMode): Promise<void> {
  return invoke('set_playback_mode', { mode });
}

export function getPlaybackMode(): Promise<PlaybackMode> {
  return invoke('get_playback_mode');
}

// ---------------------------------------------------------------------------
// ReplayGain
// ---------------------------------------------------------------------------

export function setReplayGain(mode: ReplayGainMode): Promise<void> {
  return invoke('set_replay_gain', { mode });
}

export function getReplayGain(): Promise<ReplayGainMode> {
  return invoke('get_replay_gain');
}

/** Set ReplayGain pre-amp gain in dB. */
export function setReplayGainPreamp(preampDb: number): Promise<void> {
  return invoke('set_replay_gain_preamp', { preampDb });
}

export function getReplayGainPreamp(): Promise<number> {
  return invoke('get_replay_gain_preamp');
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/**
 * Signal that the frontend is about to reload (HMR/F5).
 * The event-forwarding thread discards events until `load_session` is called.
 */
export function prepareForReload(): Promise<void> {
  return invoke('prepare_for_reload');
}

/** Set the main window title (used to show current track). */
export function setWindowTitle(title: string): Promise<void> {
  return invoke('set_window_title', { title });
}
