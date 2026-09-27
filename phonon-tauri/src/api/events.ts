/**
 * Event system — central event-name constants and the global `useListen` hook.
 *
 * Per spec §12:
 * - Global `useListen<T>` Hook must handle React StrictMode double-invocation
 *   and call `unlisten` in the effect cleanup to avoid duplicate listeners.
 * - Event names are declared here as constants to eliminate magic strings
 *   scattered across components.
 */

import { useEffect, useRef } from 'react';
import { listen, type EventCallback, type UnlistenFn } from '@tauri-apps/api/event';

// ---------------------------------------------------------------------------
// Event name constants.
// ---------------------------------------------------------------------------

/** Emitted by Rust after a playback action succeeds (optimistic UI update). */
export const EVT_PLAYBACK_STATE_SNAPSHOT = 'playback-state-snapshot';
/** Generic app event (device hotplug, playback state changes, etc.). */
export const EVT_APP_EVENT = 'app-event';
/** Spectrum data (32 bands), emitted ~20Hz during playback. */
export const EVT_SPECTRUM_DATA = 'spectrum-data';
/** Audio features (spectrum/waveform/rms/peak/onset/beat/bpm/chroma). */
export const EVT_AUDIO_FEATURES = 'audio-features';
/** Hardware volume change notification. */
export const EVT_HW_VOLUME = 'hw-volume';
/** Playback state update (legacy, used by tray popup). */
export const EVT_PLAYBACK_STATE = 'playback-state';
/** Directed to the desktop-lyrics window with lyrics/progress payload. */
export const EVT_DESKTOP_LYRICS_UPDATE = 'desktop-lyrics-update';
/** Desktop lyrics settings changed (broadcast to desktop-lyrics window). */
export const EVT_DESKTOP_LYRICS_SETTINGS_UPDATED = 'desktop-lyrics-settings-updated';
/** Desktop lyrics visibility toggled. */
export const EVT_DESKTOP_LYRICS_VISIBILITY = 'desktop-lyrics-visibility';
/** Directed to main window to toggle desktop lyrics. */
export const EVT_TOGGLE_DESKTOP_LYRICS = 'toggle-desktop-lyrics';
/** Directed to main window to show it from the tray. */
export const EVT_SHOW_MAIN_WINDOW = 'show-main-window';
/** Media library scan progress (emitted during `library_scan`). */
export const EVT_LIBRARY_SCAN_PROGRESS = 'library-scan-progress';
/** ReplayGain batch scan progress (emitted during `library_batch_scan_replaygain`). */
export const EVT_LIBRARY_REPLAYGAIN_PROGRESS = 'library-replaygain-progress';

// ---------------------------------------------------------------------------
// useListen hook — safe Tauri event subscription with StrictMode support.
// ---------------------------------------------------------------------------

/**
 * Subscribe to a Tauri event in a React component, with automatic cleanup.
 *
 * Handles React 18 StrictMode double-invocation correctly: the `listen`
 * promise resolves to an `UnlistenFn` which is stored in a ref and invoked
 * on cleanup. If the component unmounts before `listen` resolves, the
 * cleanup flag prevents the stale listener from being registered.
 *
 * @example
 * useListen(EVT_PLAYBACK_STATE_SNAPSHOT, (payload: PlaybackState) => {
 *   setPlayback(payload);
 * });
 *
 * @param eventName  Event name (use the `EVT_*` constants above).
 * @param handler     Callback receiving the event payload.
 */
export function useListen<T>(
  eventName: string,
  handler: EventCallback<T>,
): void {
  // Keep the latest handler in a ref so the listener doesn't need to be
  // re-registered on every render when the handler identity changes.
  const handlerRef = useRef(handler);
  handlerRef.current = handler;

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;

    listen<T>(eventName, (event) => {
      handlerRef.current(event);
    }).then((fn) => {
      if (cancelled) {
        // Component unmounted before listen resolved — clean up immediately.
        fn();
      } else {
        unlisten = fn;
      }
    });

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
    // eventName is intentionally the only dependency — handler changes are
    // absorbed via handlerRef without re-subscribing.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [eventName]);
}
