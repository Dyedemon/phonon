import { useEffect, useRef, useMemo } from 'react'
import { getSpectrumBands } from '../api/spectrumBus'

interface Props {
  /** When false the incoming band stream is ignored (frozen display). */
  active: boolean
  smoothing: number
  decay: number
  fps: number
  colorScheme: string
  customColors?: { bottom: string; top: string; peak: string; grid: string }
}

const BANDS = 32
const BAR_GAP = 2
const PAD = { top: 6, bottom: 26, left: 6, right: 46 }

const SCHEMES: Record<string, { bottom: [number,number,number]; top: [number,number,number]; peak: string; grid: string }> = {
  aurora: { bottom: [0, 49, 83], top: [0, 191, 255], peak: '#00FFFF', grid: '#404040' },
  warm:   { bottom: [74, 21, 0],  top: [255, 85, 0],  peak: '#FFAA00', grid: '#4a3030' },
  cool:   { bottom: [26, 0, 80],  top: [119, 68, 255], peak: '#AA88FF', grid: '#303050' },
  mono:   { bottom: [26, 26, 26], top: [204, 204, 204], peak: '#FFFFFF', grid: '#404040' },
}

const FREQ_LABELS = [
  { label: '31', freq: 31 }, { label: '63', freq: 63 },
  { label: '125', freq: 125 }, { label: '250', freq: 250 },
  { label: '500', freq: 500 }, { label: '1k', freq: 1000 },
  { label: '2k', freq: 2000 }, { label: '4k', freq: 4000 },
  { label: '8k', freq: 8000 }, { label: '16k', freq: 16000 },
  { label: '20k', freq: 20000 },
]

const DB_LABELS = ['0', '-20', '-40', '-60', '-80', '-96']

function freqToX(freq: number, nyquist: number, barW: number, barGap: number, padLeft: number): number {
  const bin = freq * nyquist * 2 / 48000
  const band = Math.log(bin) / Math.log(nyquist) * BANDS
  return padLeft + band * (barW + barGap)
}

function dbColor(v: number, scheme: { bottom: [number,number,number]; top: [number,number,number] }): string {
  const r = Math.round(scheme.bottom[0] + v * (scheme.top[0] - scheme.bottom[0]))
  const g = Math.round(scheme.bottom[1] + v * (scheme.top[1] - scheme.bottom[1]))
  const b = Math.round(scheme.bottom[2] + v * (scheme.top[2] - scheme.bottom[2]))
  return `rgb(${r},${g},${b})`
}

function getScheme(name: string, customColors?: { bottom: string; top: string; peak: string; grid: string }) {
  if (name === 'custom' && customColors) {
    return {
      bottom: hexToRgb(customColors.bottom),
      top: hexToRgb(customColors.top),
      peak: customColors.peak,
      grid: customColors.grid,
    }
  }
  return SCHEMES[name] ?? SCHEMES.aurora
}

function hexToRgb(hex: string): [number, number, number] {
  const num = parseInt(hex.replace('#', ''), 16)
  return [(num >> 16) & 0xFF, (num >> 8) & 0xFF, num & 0xFF]
}

export default function Spectrum({ active, smoothing, decay, fps, colorScheme, customColors }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const drawRef = useRef<Float32Array>(new Float32Array(BANDS))
  const peakRef = useRef<Float32Array>(new Float32Array(BANDS))
  const animRef = useRef(0)
  const lastFrameRef = useRef(0)
  const valuesRef = useRef<number[]>([])
  const smoothingRef = useRef(smoothing)
  const decayRef = useRef(decay)
  const fpsRef = useRef(fps)
  const sizeRef = useRef({ w: 0, h: 0 })
  const sizeCheckCounterRef = useRef(0)
  smoothingRef.current = smoothing
  decayRef.current = decay
  fpsRef.current = fps

  const scheme = useMemo(() => getScheme(colorScheme, customColors), [colorScheme, customColors])

  // Subscribe to the spectrum bus instead of taking bands through props —
  // the ~40 Hz stream must not flow through React state.
  useEffect(() => {
    if (!active) {
      // Mirror the previous paused behavior (values=[] → bars freeze).
      valuesRef.current = []
      return
    }
    valuesRef.current = getSpectrumBands()
    const onBands = () => {
      valuesRef.current = getSpectrumBands()
    }
    window.addEventListener('spectrum-bands', onBands)
    return () => window.removeEventListener('spectrum-bands', onBands)
  }, [active])

  // On mount, initialize drawRef with current values
  useEffect(() => {
    const draw = drawRef.current
    const peaks = peakRef.current
    const values = active ? getSpectrumBands() : []
    for (let i = 0; i < BANDS; i++) {
      draw[i] = values[i] ?? 0
      peaks[i] = values[i] ?? 0
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const ctx = canvas.getContext('2d')
    if (!ctx) return

    // Batch canvas state that doesn't change per frame
    ctx.textBaseline = 'alphabetic'

    const render = (timestamp: number) => {
      const fpsTarget = fpsRef.current
      const frameInterval = 1000 / fpsTarget
      if (timestamp - lastFrameRef.current < frameInterval) {
        animRef.current = requestAnimationFrame(render)
        return
      }
      lastFrameRef.current = timestamp

      const dpr = window.devicePixelRatio || 1
      // Only call getBoundingClientRect when size may have changed (every 30 frames or resize)
      sizeCheckCounterRef.current++
      if (sizeCheckCounterRef.current >= 30 || canvas.width === 0) {
        sizeCheckCounterRef.current = 0
        const rect = canvas.getBoundingClientRect()
        sizeRef.current = { w: rect.width * dpr, h: rect.height * dpr }
      }
      const w = sizeRef.current.w
      const h = sizeRef.current.h

      if (canvas.width !== w || canvas.height !== h) {
        canvas.width = w
        canvas.height = h
      }

      ctx.clearRect(0, 0, w, h)

      const padTop = PAD.top * dpr
      const padBottom = PAD.bottom * dpr
      const padLeft = PAD.left * dpr
      const padRight = PAD.right * dpr

      const barAreaW = w - padLeft - padRight
      const barAreaH = h - padTop - padBottom
      const barCount = BANDS
      const barGap = BAR_GAP * dpr
      const barW = Math.max(1.5 * dpr, (barAreaW - (barCount - 1) * barGap) / barCount)

      const bars = drawRef.current
      const peaks = peakRef.current
      const baseline = padTop + barAreaH

      const currentValues = valuesRef.current
      const s = smoothingRef.current
      const d = decayRef.current

      // Smooth interpolation + peak tracking (moved from render body into rAF)
      if (currentValues.length > 0) {
        const maxVal = currentValues.reduce((a, b) => Math.max(a, b), 0)
        if (maxVal < 0.001) {
          for (let i = 0; i < BANDS; i++) {
            bars[i] *= d
            peaks[i] *= 0.98
          }
        } else {
          for (let i = 0; i < BANDS; i++) {
            const target = currentValues[i] ?? 0
            bars[i] += (target - bars[i]) * s
            if (bars[i] >= peaks[i]) {
              peaks[i] = bars[i]
            } else {
              peaks[i] = Math.max(bars[i], peaks[i] * 0.995)
            }
          }
        }
      }

      const gridColor = scheme.grid
      const nyquist = 512

      // Grid lines (horizontal)
      ctx.strokeStyle = gridColor
      ctx.lineWidth = 0.5 * dpr
      ctx.beginPath()
      for (let i = 0; i < DB_LABELS.length; i++) {
        const y = padTop + (i / (DB_LABELS.length - 1)) * barAreaH
        ctx.moveTo(padLeft, y)
        ctx.lineTo(w - padRight, y)
      }
      ctx.stroke()

      // Grid lines (vertical frequency)
      for (const fl of FREQ_LABELS) {
        const x = freqToX(fl.freq, nyquist, barW, barGap, padLeft)
        ctx.beginPath()
        ctx.moveTo(x, padTop)
        ctx.lineTo(x, baseline)
        ctx.stroke()
      }

      // Baseline
      ctx.strokeStyle = gridColor
      ctx.lineWidth = 1 * dpr
      ctx.beginPath()
      ctx.moveTo(padLeft, baseline)
      ctx.lineTo(w - padRight, baseline)
      ctx.stroke()

      // Pre-compute bottom color string
      const bottomColor = `rgb(${scheme.bottom[0]},${scheme.bottom[1]},${scheme.bottom[2]})`

      // Bars
      for (let i = 0; i < barCount; i++) {
        const v = Math.max(0, Math.min(bars[i], 1.0))
        const barH = Math.pow(v, 0.6) * barAreaH
        if (barH < 0.5) continue

        const x = padLeft + i * (barW + barGap)
        const barTop = baseline - barH

        const grad = ctx.createLinearGradient(0, barTop, 0, baseline)
        grad.addColorStop(0, dbColor(v, scheme))
        grad.addColorStop(1, bottomColor)

        ctx.fillStyle = grad
        ctx.fillRect(x, barTop, barW, barH)
      }

      // Peak hold dots
      ctx.fillStyle = scheme.peak
      for (let i = 0; i < barCount; i++) {
        const pv = Math.max(0, Math.min(peaks[i], 1.0))
        const peakH = Math.pow(pv, 0.6) * barAreaH
        if (peakH < 1) continue
        const x = padLeft + i * (barW + barGap)
        const peakY = baseline - peakH
        ctx.fillRect(x, peakY, barW, 2 * dpr)
      }

      // Y-axis dB labels
      ctx.fillStyle = gridColor
      ctx.font = `${10 * dpr}px monospace`
      ctx.textAlign = 'right'
      for (let i = 0; i < DB_LABELS.length; i++) {
        const y = padTop + (i / (DB_LABELS.length - 1)) * barAreaH
        ctx.fillText(DB_LABELS[i], w - 4 * dpr, y + 3 * dpr)
      }

      // X-axis frequency labels
      ctx.fillStyle = gridColor
      ctx.font = `${9 * dpr}px monospace`
      ctx.textAlign = 'center'
      for (const fl of FREQ_LABELS) {
        const x = freqToX(fl.freq, nyquist, barW, barGap, padLeft)
        ctx.fillText(fl.label, x, h - 4 * dpr)
      }

      animRef.current = requestAnimationFrame(render)
    }

    animRef.current = requestAnimationFrame(render)
    return () => cancelAnimationFrame(animRef.current)
  }, [scheme])

  return (
    <div className="spectrum-container">
      <canvas
        ref={canvasRef}
        className="spectrum-canvas"
        style={{
          width: '100%',
          height: 'var(--spectrum-canvas-height, 130px)',
          display: 'block',
        }}
      />
    </div>
  )
}
