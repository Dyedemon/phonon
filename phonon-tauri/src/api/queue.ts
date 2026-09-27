/**
 * Queue & Playlists API — queue management and user playlist CRUD.
 *
 * Wraps the `add_to_queue`/`reorder_queue`/`create_playlist`/... commands.
 */

import { invoke } from '@tauri-apps/api/core';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** Result of adding tracks to the queue (mirrors `commands::AddToQueueResult`). */
export interface AddToQueueResult {
  added: number;
  duplicates: number;
}

/** A queued track (mirrors `phonon_core::engine::QueueItem`). */
export interface QueueItem {
  path: string;
  title: string;
  artist: string | null;
  duration: number | null;
}

/** User-created playlist summary (mirrors `commands::UserPlaylist`). */
export interface UserPlaylist {
  name: string;
  track_count: number;
  /** Stable identifier for system playlists: "favorites" | "recent" | null */
  system_id?: string | null;
}

/** Playlist file format for save/load. */
export type PlaylistFormat = 'm3u' | 'm3u8' | 'pls' | 'xspf';

// ---------------------------------------------------------------------------
// Queue operations
// ---------------------------------------------------------------------------

/** Add audio file paths to the front of the queue. */
export function addToQueue(paths: string[]): Promise<AddToQueueResult> {
  return invoke('add_to_queue', { paths });
}

/** Add CUE sheet virtual tracks to the queue. */
export function addCueToQueue(path: string): Promise<AddToQueueResult> {
  return invoke('add_cue_to_queue', { path });
}

/** Remove tracks at the given indices from the queue. */
export function removeFromQueue(indices: number[]): Promise<void> {
  return invoke('remove_from_queue', { indices });
}

/** Move a queue item from `from` index to `to` index. */
export function reorderQueue(from: number, to: number): Promise<void> {
  return invoke('reorder_queue', { from, to });
}

/** Get the current queue contents. */
export function getQueue(): Promise<QueueItem[]> {
  return invoke('get_queue');
}

/** Clear the entire queue. */
export function clearQueue(): Promise<void> {
  return invoke('clear_queue');
}

// ---------------------------------------------------------------------------
// User playlists (in-app playlist management)
// ---------------------------------------------------------------------------

export function createPlaylist(name: string): Promise<void> {
  return invoke('create_playlist', { name });
}

export function deletePlaylist(name: string): Promise<void> {
  return invoke('delete_playlist', { name });
}

export function switchPlaylist(name: string): Promise<void> {
  return invoke('switch_playlist', { name });
}

export function listPlaylists(): Promise<UserPlaylist[]> {
  return invoke('list_playlists');
}

export function getActivePlaylist(): Promise<string> {
  return invoke('get_active_playlist');
}

export function renamePlaylist(oldName: string, newName: string): Promise<void> {
  // Tauri v2 camelCase key mapping (Rust params old_name / new_name).
  return invoke('rename_playlist', { oldName, newName });
}

// ---------------------------------------------------------------------------
// Playlist file I/O (M3U/M3U8/PLS/XSPF)
// ---------------------------------------------------------------------------

export function savePlaylist(filePath: string, format: PlaylistFormat): Promise<void> {
  return invoke('save_playlist', { filePath, format });
}

export function loadPlaylist(filePath: string): Promise<QueueItem[]> {
  return invoke('load_playlist', { filePath });
}
