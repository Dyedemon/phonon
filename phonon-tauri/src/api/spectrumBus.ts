// Spectrum bands fan-out without React state.
//
// The backend pushes SpectrumData ~40x/s. Routing that stream through
// App-level setState re-rendered the whole component tree at the same
// rate; instead the publisher dispatches a 'spectrum-bands' window event
// and consumers read the latest array via getSpectrumBands() inside their
// own rAF loops (mirrors the audio-features bridge in components/depth3d).

let latest: number[] = []

export function publishSpectrumBands(values: number[]): void {
  latest = values
  window.dispatchEvent(new CustomEvent('spectrum-bands'))
}

export function getSpectrumBands(): number[] {
  return latest
}
