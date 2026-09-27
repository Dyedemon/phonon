/**
 * Unit tests for the unified throttle/debounce hooks (spec §12).
 *
 * Uses fake timers to deterministically test debounce/throttle timing, and
 * real rAF (via jsdom polyfill) for useRafThrottle.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import {
  useRafThrottle,
  useDebouncedCallback,
  useThrottledCallback,
} from './useThrottle';

describe('useRafThrottle', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    // Mock rAF as setTimeout(0) so fake timers can control it.
    vi.stubGlobal(
      'requestAnimationFrame',
      ((cb: FrameRequestCallback) => setTimeout(() => cb(0), 0)) as unknown as typeof requestAnimationFrame,
    );
    vi.stubGlobal(
      'cancelAnimationFrame',
      ((id: number) => clearTimeout(id)) as unknown as typeof cancelAnimationFrame,
    );
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('coalesces multiple synchronous calls into one invocation', () => {
    const cb = vi.fn();
    const { result } = renderHook(() => useRafThrottle(cb));

    act(() => {
      result.current('a');
      result.current('b');
      result.current('c');
      // Flush the rAF (setTimeout(0)).
      vi.advanceTimersByTime(1);
    });

    expect(cb).toHaveBeenCalledTimes(1);
    // The latest arguments win.
    expect(cb).toHaveBeenCalledWith('c');
  });

  it('passes the latest arguments when the frame fires', () => {
    const cb = vi.fn();
    const { result } = renderHook(() => useRafThrottle(cb));

    act(() => {
      result.current(1);
      result.current(2);
      result.current(3);
      vi.advanceTimersByTime(1);
    });

    expect(cb).toHaveBeenCalledTimes(1);
    expect(cb).toHaveBeenCalledWith(3);
  });

  it('uses the latest callback identity (ref pattern)', () => {
    const cb1 = vi.fn();
    const cb2 = vi.fn();
    const { result, rerender } = renderHook(
      ({ cb }) => useRafThrottle(cb),
      { initialProps: { cb: cb1 } },
    );

    rerender({ cb: cb2 });

    act(() => {
      result.current('x');
      vi.advanceTimersByTime(1);
    });

    expect(cb1).not.toHaveBeenCalled();
    expect(cb2).toHaveBeenCalledTimes(1);
    expect(cb2).toHaveBeenCalledWith('x');
  });
});

describe('useDebouncedCallback', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('does not invoke immediately; waits for the quiet period', () => {
    const cb = vi.fn();
    const { result } = renderHook(() => useDebouncedCallback(cb, 500));

    act(() => {
      result.current('a');
    });
    expect(cb).not.toHaveBeenCalled();

    act(() => {
      vi.advanceTimersByTime(499);
    });
    expect(cb).not.toHaveBeenCalled();

    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(cb).toHaveBeenCalledTimes(1);
    expect(cb).toHaveBeenCalledWith('a');
  });

  it('restarts the timer on each call (only last args fire)', () => {
    const cb = vi.fn();
    const { result } = renderHook(() => useDebouncedCallback(cb, 500));

    act(() => {
      result.current('a');
      vi.advanceTimersByTime(300);
      result.current('b');
      vi.advanceTimersByTime(300);
      result.current('c');
      vi.advanceTimersByTime(500);
    });

    expect(cb).toHaveBeenCalledTimes(1);
    expect(cb).toHaveBeenCalledWith('c');
  });

  it('cleans up the pending timer on unmount', () => {
    const cb = vi.fn();
    const { result, unmount } = renderHook(() => useDebouncedCallback(cb, 500));

    act(() => {
      result.current('a');
    });
    unmount();

    act(() => {
      vi.advanceTimersByTime(1000);
    });
    expect(cb).not.toHaveBeenCalled();
  });
});

describe('useThrottledCallback', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('fires the leading call immediately', () => {
    const cb = vi.fn();
    const { result } = renderHook(() => useThrottledCallback(cb, 50));

    act(() => {
      result.current(1);
    });
    expect(cb).toHaveBeenCalledTimes(1);
    expect(cb).toHaveBeenCalledWith(1);
  });

  it('suppresses intermediate calls during cooldown, fires trailing with last args', () => {
    const cb = vi.fn();
    const { result } = renderHook(() => useThrottledCallback(cb, 50));

    act(() => {
      result.current(1); // leading fires immediately
    });
    expect(cb).toHaveBeenCalledTimes(1);

    act(() => {
      result.current(2);
      result.current(3);
      result.current(4); // last args during cooldown
    });
    expect(cb).toHaveBeenCalledTimes(1); // still only the leading call

    act(() => {
      vi.advanceTimersByTime(50);
    });
    // Trailing call fires with the last args.
    expect(cb).toHaveBeenCalledTimes(2);
    expect(cb).toHaveBeenLastCalledWith(4);
  });

  it('allows a new leading call after the cooldown expires', () => {
    const cb = vi.fn();
    const { result } = renderHook(() => useThrottledCallback(cb, 50));

    act(() => {
      result.current('a');
    });
    expect(cb).toHaveBeenCalledTimes(1);

    act(() => {
      vi.advanceTimersByTime(60);
      result.current('b');
    });
    expect(cb).toHaveBeenCalledTimes(2);
    expect(cb).toHaveBeenLastCalledWith('b');
  });

  it('cleans up the pending trailing timer on unmount', () => {
    const cb = vi.fn();
    const { result, unmount } = renderHook(() => useThrottledCallback(cb, 50));

    act(() => {
      result.current(1); // leading
      result.current(2); // schedules trailing
    });
    unmount();

    act(() => {
      vi.advanceTimersByTime(100);
    });
    // Only the leading call should have fired.
    expect(cb).toHaveBeenCalledTimes(1);
  });
});
