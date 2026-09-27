/**
 * Unified throttle/debounce hooks (spec §12).
 *
 * Replaces the ad-hoc `rafRef` + `flushPending*` pattern scattered across
 * EqPanel/Settings with three standardized, type-safe hooks:
 *
 *   useRafThrottle        — rAF batching for high-frequency drag (EQ bands)
 *   useDebouncedCallback  — trailing debounce for commit-on-settle (color picker)
 *   useThrottledCallback  — leading+trailing throttle for live updates (volume)
 *
 * All hooks:
 *   - keep the latest callback in a ref (no listener re-subscription on rerender)
 *   - cancel pending timers/frames on unmount (StrictMode-safe)
 *   - are generic over argument types
 */

import { useCallback, useEffect, useRef } from 'react';

// ---------------------------------------------------------------------------
// useRafThrottle — coalesce multiple calls into one animation frame.
// ---------------------------------------------------------------------------

/**
 * Coalesce rapid successive calls into a single `requestAnimationFrame`.
 *
 * Use case: EQ band sliders that fire `onChange` every pixel of drag — we
 * only need to invoke the (async, Tauri-bound) callback once per frame.
 *
 * The latest arguments are retained and passed to the callback when the
 * frame fires; intermediate arguments are dropped. The pending frame is
 * cancelled on unmount.
 *
 * @example
 * const setBand = useRafThrottle((freq, gain) => invoke('set_eq_band', {...}));
 * <Slider onChange={(v) => setBand(freq, v)} />
 */
export function useRafThrottle<Args extends unknown[]>(
  callback: (...args: Args) => void,
): (...args: Args) => void {
  const cbRef = useRef(callback);
  cbRef.current = callback;

  const frameRef = useRef<number | null>(null);
  const argsRef = useRef<Args | null>(null);

  // Cancel any pending frame on unmount.
  useEffect(() => {
    return () => {
      if (frameRef.current !== null) {
        cancelAnimationFrame(frameRef.current);
        frameRef.current = null;
      }
    };
  }, []);

  return useCallback((...args: Args) => {
    argsRef.current = args;
    if (frameRef.current !== null) return; // already scheduled
    frameRef.current = requestAnimationFrame(() => {
      frameRef.current = null;
      const a = argsRef.current;
      if (a !== null) {
        cbRef.current(...a);
      }
    });
  }, []);
}

// ---------------------------------------------------------------------------
// useDebouncedCallback — trailing debounce.
// ---------------------------------------------------------------------------

/**
 * Invoke `callback` only after `wait` ms have elapsed without further calls.
 *
 * Use case: color picker — commit the selected color to Tauri only after the
 * user releases the picker for 500ms, avoiding a flood of `update_settings`
 * invokes during continuous dragging.
 *
 * The latest arguments are retained. The pending timer is cleared on unmount
 * or on every new call (restart).
 *
 * @example
 * const commitColor = useDebouncedCallback((hex) => applyColor(hex), 500);
 * <ColorPicker onChange={(hex) => commitColor(hex)} />
 */
export function useDebouncedCallback<Args extends unknown[]>(
  callback: (...args: Args) => void,
  wait: number,
): (...args: Args) => void {
  const cbRef = useRef(callback);
  cbRef.current = callback;

  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const argsRef = useRef<Args | null>(null);

  useEffect(() => {
    return () => {
      if (timerRef.current !== null) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    };
  }, []);

  return useCallback((...args: Args) => {
    argsRef.current = args;
    if (timerRef.current !== null) {
      clearTimeout(timerRef.current);
    }
    timerRef.current = setTimeout(() => {
      timerRef.current = null;
      const a = argsRef.current;
      if (a !== null) {
        cbRef.current(...a);
      }
    }, wait);
  }, [wait]);
}

// ---------------------------------------------------------------------------
// useThrottledCallback — leading + trailing throttle.
// ---------------------------------------------------------------------------

/**
 * Invoke `callback` at most once per `wait` ms, with a trailing call to
 * deliver the last arguments that would otherwise be dropped.
 *
 * Use case: volume slider — the user expects immediate feedback on the first
 * move (leading) and a final commit when they stop (trailing), but we don't
 * want to fire 60 `set_volume` invokes per second.
 *
 * - Leading: fires immediately on the first call in a quiet window.
 * - Trailing: if calls arrive during the cooldown, the last arguments fire
 *   exactly once when the window ends.
 *
 * @example
 * const setVol = useThrottledCallback((v: number) => invoke('set_volume', { level: v }), 50);
 * <Slider onChange={(v) => setVol(v)} />
 */
export function useThrottledCallback<Args extends unknown[]>(
  callback: (...args: Args) => void,
  wait: number,
): (...args: Args) => void {
  const cbRef = useRef(callback);
  cbRef.current = callback;

  const lastCallRef = useRef<number>(0);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const argsRef = useRef<Args | null>(null);

  useEffect(() => {
    return () => {
      if (timerRef.current !== null) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    };
  }, []);

  return useCallback((...args: Args) => {
    const now = Date.now();
    const remaining = wait - (now - lastCallRef.current);
    argsRef.current = args;

    // Leading: fire immediately if the cooldown has elapsed.
    if (remaining <= 0) {
      if (timerRef.current !== null) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
      lastCallRef.current = now;
      cbRef.current(...args);
      return;
    }

    // Schedule a trailing call if not already scheduled.
    if (timerRef.current === null) {
      timerRef.current = setTimeout(() => {
        timerRef.current = null;
        lastCallRef.current = Date.now();
        const a = argsRef.current;
        if (a !== null) {
          cbRef.current(...a);
        }
      }, remaining);
    }
  }, [wait]);
}
