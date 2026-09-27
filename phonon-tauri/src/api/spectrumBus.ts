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
  // Mirror on window: vis plugins are evaluated via `new Function` and cannot
  // import this module — the 视觉升华 plugin's frame() reads this mirror to
  // drive its audio-reactive ambient layer.
  ;(window as unknown as { __phononSpectrumBands?: number[] }).__phononSpectrumBands = values
  window.dispatchEvent(new CustomEvent('spectrum-bands'))
}

export function getSpectrumBands(): number[] {
  return latest
}
