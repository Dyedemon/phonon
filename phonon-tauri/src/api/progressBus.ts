// Playback progress fan-out without React state.
//
// The backend pushes PlaybackProgress ~20x/s. Routing that stream through
// App-level setState re-rendered the whole component tree at the same rate;
// instead the handler publishes here and only the components that actually
// display position (progress bar, time text, lyrics) subscribe via
// useProgress(). Low-frequency fields (duration, track, state) stay in App
// state. Mirrors api/spectrumBus.ts.

import { useEffect, useState } from 'react'

export interface PlaybackProgress {
  position: number
  duration: number | null
}

let latest: PlaybackProgress = { position: 0, duration: null }

export function publishProgress(
  position: number | null,
  duration: number | null,
): void {
  if (position != null) latest = { ...latest, position }
  if (duration != null) latest = { ...latest, duration }
  window.dispatchEvent(new CustomEvent('playback-progress'))
}

/** Reset position to 0 (track change / stop) keeping the current duration. */
export function resetProgress(): void {
  latest = { ...latest, position: 0 }
  window.dispatchEvent(new CustomEvent('playback-progress'))
}

export function getProgress(): PlaybackProgress {
  return latest
}

/**
 * Re-render the calling component at progress-event rate (~20 Hz,
 * rAF-coalesced). Only the calling component re-renders — not the tree.
 */
export function useProgress(): PlaybackProgress {
  const [p, setP] = useState<PlaybackProgress>(getProgress())
  useEffect(() => {
    let raf = 0
    const on = () => {
      if (raf) return
      raf = requestAnimationFrame(() => {
        raf = 0
        setP(getProgress())
      })
    }
    window.addEventListener('playback-progress', on)
    return () => {
      window.removeEventListener('playback-progress', on)
      if (raf) cancelAnimationFrame(raf)
    }
  }, [])
  return p
}
