import { useEffect, useRef, useState, useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'

interface VisSourceEntry {
  name: string
  file_name: string
}

interface VisScript {
  name: string
  type: '2d' | 'webgl'
  init?(canvas: HTMLCanvasElement, width: number, height: number): void
  render?(canvas: HTMLCanvasElement, spectrum: number[], width: number, height: number, time: number): void
  resize?(canvas: HTMLCanvasElement, width: number, height: number): void
  destroy?(): void
}

interface Props {
  spectrumValues: number[]
  visible: boolean
  onClose: () => void
}

export default function VisPanel({ spectrumValues, visible, onClose }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const scriptRef = useRef<VisScript | null>(null)
  const rafRef = useRef<number>(0)
  const [sources, setSources] = useState<VisSourceEntry[]>([])
  const [activeName, setActiveName] = useState('')
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [fullscreen, setFullscreen] = useState(false)
  const containerRef = useRef<HTMLDivElement>(null)

  const loadSources = useCallback(async () => {
    try {
      const list = await invoke<VisSourceEntry[]>('scan_vis_sources')
      setSources(list)
    } catch { /* ignore */ }
  }, [])

  useEffect(() => {
    if (visible) loadSources()
  }, [visible, loadSources])

  const loadScript = useCallback(async (entry: VisSourceEntry) => {
    setLoading(true)
    setError('')
    // Destroy previous script
    if (scriptRef.current?.destroy) {
      try { scriptRef.current.destroy() } catch { /* ignore */ }
    }
    scriptRef.current = null
    try {
      const content = await invoke<string>('read_vis_script', { fileName: entry.file_name })
      const scriptFn = new Function(content)
      const script: VisScript = await scriptFn()
      if (!script || !script.render) {
        throw new Error('Script must export { name, type, render }')
      }
      scriptRef.current = script
      setActiveName(script.name || entry.name)
      // Initialize canvas
      const canvas = canvasRef.current
      if (canvas && script.init) {
        script.init(canvas, canvas.width, canvas.height)
      }
    } catch (e) {
      console.error('vis: failed to load script:', e)
      setError('Load failed: ' + String(e))
    } finally {
      setLoading(false)
    }
  }, [])

  // Render loop
  useEffect(() => {
    if (!visible || !scriptRef.current) return
    const canvas = canvasRef.current
    if (!canvas) return
    const script = scriptRef.current
    let startTime = performance.now()

    const loop = () => {
      const elapsed = (performance.now() - startTime) / 1000
      try {
        script.render!(canvas, spectrumValues, canvas.width, canvas.height, elapsed)
      } catch { /* ignore */ }
      rafRef.current = requestAnimationFrame(loop)
    }
    rafRef.current = requestAnimationFrame(loop)
    return () => cancelAnimationFrame(rafRef.current)
  }, [visible, spectrumValues])

  // Resize
  useEffect(() => {
    if (!visible) return
    const container = containerRef.current
    const canvas = canvasRef.current
    if (!container || !canvas) return
    const ro = new ResizeObserver(() => {
      const w = container.clientWidth
      const h = container.clientHeight
      canvas.width = w
      canvas.height = h
      if (scriptRef.current?.resize) {
        scriptRef.current.resize(canvas, w, h)
      }
    })
    ro.observe(container)
    return () => ro.disconnect()
  }, [visible])

  const builtInRender = (ctx: CanvasRenderingContext2D, w: number, h: number) => {
    const vals = spectrumValues
    if (!vals || vals.length === 0) {
      ctx.fillStyle = '#1a1a2e'
      ctx.fillRect(0, 0, w, h)
      return
    }
    // Dark background
    ctx.fillStyle = '#1a1a2e'
    ctx.fillRect(0, 0, w, h)
    const barCount = vals.length
    const barWidth = w / barCount
    const grad = ctx.createLinearGradient(0, h, 0, 0)
    grad.addColorStop(0, '#00e5ff')
    grad.addColorStop(0.5, '#7c4dff')
    grad.addColorStop(1, '#ff4081')
    ctx.fillStyle = grad
    for (let i = 0; i < barCount; i++) {
      const barHeight = vals[i] * h * 0.9
      ctx.fillRect(i * barWidth + 1, h - barHeight, barWidth - 2, barHeight)
    }
  }

  if (!visible) return null

  return (
    <div
      ref={containerRef}
      style={{
        position: fullscreen ? 'fixed' : 'relative',
        top: fullscreen ? 0 : undefined,
        left: fullscreen ? 0 : undefined,
        width: fullscreen ? '100vw' : '100%',
        height: fullscreen ? '100vh' : 200,
        zIndex: fullscreen ? 9999 : undefined,
        background: '#000',
        display: 'flex',
        flexDirection: 'column',
      }}
    >
      {/* Controls */}
      <div style={{
        display: 'flex', alignItems: 'center', gap: 8, padding: '4px 8px',
        background: 'rgba(0,0,0,0.8)', flexShrink: 0,
      }}>
        {sources.length > 0 && (
          <select
            value={activeName}
            onChange={(e) => {
              const s = sources.find(x => x.name === e.target.value)
              if (s) loadScript(s)
            }}
            style={{
              fontSize: 'var(--text-xs)', padding: '2px 6px', borderRadius: 4,
              border: '1px solid var(--border)', background: 'var(--input-bg)',
              color: 'var(--text)', cursor: 'pointer',
            }}
          >
            <option value="">Built-in</option>
            {sources.map(s => (
              <option key={s.name} value={s.name}>{s.name}</option>
            ))}
          </select>
        )}
        {loading && <span style={{ fontSize: 'var(--text-xs)', color: '#888' }}>...</span>}
        <div style={{ flex: 1 }} />
        <button
          onClick={() => setFullscreen(!fullscreen)}
          style={{
            fontSize: 'var(--text-xs)', padding: '2px 8px', borderRadius: 4,
            border: '1px solid #555', background: 'transparent', color: '#ccc',
            cursor: 'pointer',
          }}
        >
          {fullscreen ? 'Exit' : 'FS'}
        </button>
        <button
          onClick={onClose}
          style={{
            fontSize: 'var(--text-xs)', padding: '2px 8px', borderRadius: 4,
            border: '1px solid #555', background: 'transparent', color: '#ccc',
            cursor: 'pointer',
          }}
        >
          X
        </button>
      </div>
      {error && (
        <div style={{ fontSize: 'var(--text-xs)', color: '#f44', padding: '2px 8px', flexShrink: 0 }}>{error}</div>
      )}
      {/* Canvas */}
      <canvas
        ref={canvasRef}
        style={{ flex: 1, width: '100%', height: '100%' }}
      />
      {!activeName && canvasRef.current && (() => {
        const ctx = canvasRef.current.getContext('2d')
        if (ctx) builtInRender(ctx, canvasRef.current.width, canvasRef.current.height)
        return null
      })()}
    </div>
  )
}