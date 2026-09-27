import { useEffect, useRef, useState, useCallback, useMemo } from 'react'
import { invoke } from '@tauri-apps/api/core'

// ── Data shapes (mirrors Rust AppEvent::AudioFeaturesData) ────────────────

export interface AudioFeatures {
  spectrum: number[]
  waveform: number[]
  rms: number
  peak: number
  spectral_centroid_hz: number
  onset: number
  beat: boolean
  bpm: number
  chroma: number[]
  sample_rate: number
  channels: number
}

export interface VisSourceEntry {
  name: string
  file_name: string
}

export interface ExternalScript {
  name: string
  type: '2d' | 'webgl'
  init?: (canvas: HTMLCanvasElement, width: number, height: number) => void
  render?: (canvas: HTMLCanvasElement, features: AudioFeatures, width: number, height: number, time: number) => void
  resize?: (canvas: HTMLCanvasElement, width: number, height: number) => void
  destroy?: () => void
}

interface Props {
  playbackState: string
  onClose?: () => void
}

// localStorage key — shared with PluginManager. Holds the list of
// user-enabled vis script file names: ["bars.js", "wave.js", ...]
const ENABLED_VIS_KEY = 'phonon-enabled-vis-scripts'

function readEnabledVis(): string[] {
  try {
    const raw = localStorage.getItem(ENABLED_VIS_KEY)
    if (!raw) return []
    const arr = JSON.parse(raw)
    return Array.isArray(arr) ? arr.filter(x => typeof x === 'string') : []
  } catch { return [] }
}

export default function VisualizerPage({ playbackState, onClose }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const rafRef = useRef<number>(0)
  const startTimeRef = useRef(performance.now())

  // Latest audio features
  const featuresRef = useRef<AudioFeatures>({
    spectrum: new Array(32).fill(0),
    waveform: new Array(512).fill(0),
    rms: 0, peak: 0, spectral_centroid_hz: 0, onset: 0, beat: false, bpm: 0,
    chroma: new Array(12).fill(0),
    sample_rate: 44100, channels: 2,
  })

  // Script state — only one script active at a time
  const scriptRef = useRef<ExternalScript | null>(null)

  const [sources, setSources] = useState<VisSourceEntry[]>([])
  const [activeFileName, setActiveFileName] = useState('')
  const [scriptError, setScriptError] = useState('')
  const [loading, setLoading] = useState(false)

  const [fullscreen, setFullscreen] = useState(false)
  const [activeDisplayName, setActiveDisplayName] = useState('')

  // ── Listen for audio features
  useEffect(() => {
    const handler = (e: Event) => {
      const f = (e as CustomEvent<AudioFeatures>).detail
      if (!f) return
      featuresRef.current = {
        spectrum: f.spectrum ?? featuresRef.current.spectrum,
        waveform: f.waveform ?? featuresRef.current.waveform,
        rms: f.rms ?? 0,
        peak: f.peak ?? 0,
        spectral_centroid_hz: f.spectral_centroid_hz ?? 0,
        onset: f.onset ?? 0,
        beat: !!f.beat,
        bpm: f.bpm ?? 0,
        chroma: f.chroma ?? featuresRef.current.chroma,
        sample_rate: f.sample_rate ?? 44100,
        channels: f.channels ?? 2,
      }
    }
    window.addEventListener('audio-features', handler)
    return () => window.removeEventListener('audio-features', handler)
  }, [])

  // ── Scan available scripts (only those the user has placed in plugins/vis/)
  const refreshSources = useCallback(async () => {
    try {
      const list = await invoke<VisSourceEntry[]>('scan_vis_sources')
      // Filter to only user-enabled scripts. If none are enabled, sources
      // stays empty and the page shows a "please enable in Plugin Manager"
      // placeholder. This enforces the "must be enabled before use" rule.
      const enabled = readEnabledVis()
      const filtered = enabled.length === 0
        ? []
        : list.filter(s => enabled.includes(s.file_name))
      setSources(filtered)
      // Auto-load first enabled script if nothing is active
      if (filtered.length > 0 && !activeFileName && !scriptRef.current) {
        await loadScript(filtered[0])
      } else if (filtered.length === 0) {
        // No enabled scripts — clear any active script
        if (scriptRef.current?.destroy) {
          try { scriptRef.current.destroy() } catch { /* ignore */ }
        }
        scriptRef.current = null
        setActiveFileName('')
        setActiveDisplayName('')
      }
    } catch { /* ignore */ }
  }, [activeFileName])

  useEffect(() => {
    refreshSources()
  }, [refreshSources])

  // ── Reload when the user toggles enable/disable in PluginManager
  useEffect(() => {
    const handler = () => { refreshSources() }
    window.addEventListener('vis-scripts-enabled-changed', handler)
    return () => window.removeEventListener('vis-scripts-enabled-changed', handler)
  }, [refreshSources])

  // ── Load a script by entry
  const loadScript = useCallback(async (entry: VisSourceEntry) => {
    setLoading(true)
    setScriptError('')
    if (scriptRef.current?.destroy) {
      try { scriptRef.current.destroy() } catch { /* ignore */ }
    }
    scriptRef.current = null
    setActiveFileName('')
    setActiveDisplayName('')
    try {
      const content = await invoke<string>('read_vis_script', { fileName: entry.file_name })
      const fn = new Function(content)
      const script: ExternalScript = await fn()
      if (!script || !script.render) throw new Error('脚本需导出 { name, type, render }')
      scriptRef.current = script
      setActiveFileName(entry.file_name)
      setActiveDisplayName(script.name || entry.name)
      const canvas = canvasRef.current
      if (canvas && script.init) {
        script.init(canvas, canvas.clientWidth, canvas.clientHeight)
      }
    } catch (e) {
      setScriptError('加载失败: ' + String(e))
    } finally {
      setLoading(false)
    }
  }, [])

  // ── Resize handling
  useEffect(() => {
    const el = containerRef.current
    if (!el) return
    const canvas = el.querySelector('canvas')
    if (!canvas) return
    const ro = new ResizeObserver(() => {
      const w = canvas.clientWidth
      const h = canvas.clientHeight
      const dpr = window.devicePixelRatio || 1
      canvas.width = Math.max(1, Math.floor(w * dpr))
      canvas.height = Math.max(1, Math.floor(h * dpr))
      const ctx = canvas.getContext('2d')
      if (ctx) ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  // ── Main render loop
  const scriptErrorRef = useRef<string | null>(null)
  useEffect(() => { scriptErrorRef.current = scriptError }, [scriptError])
  useEffect(() => {
    const loop = () => {
      const now = performance.now()
      const tSec = (now - startTimeRef.current) / 1000
      const playing = playbackState === 'Playing'
      const f = { ...featuresRef.current }
      if (!playing) {
        for (let i = 0; i < f.spectrum.length; i++) f.spectrum[i] *= 0.96
        for (let i = 0; i < f.waveform.length; i++) f.waveform[i] *= 0.96
        f.rms *= 0.96
        f.onset *= 0.9
        f.beat = false
      }

      const mainCanvas = canvasRef.current
      if (mainCanvas) {
        const w = mainCanvas.clientWidth
        const h = mainCanvas.clientHeight
        const script = scriptRef.current
        if (script && script.render) {
          try {
            script.render(mainCanvas, f, w, h, tSec)
          } catch (e) {
            if (!scriptErrorRef.current) setScriptError('渲染错误: ' + String(e))
          }
        } else {
          const ctx = mainCanvas.getContext('2d')
          if (ctx) {
            ctx.fillStyle = '#05060a'
            ctx.fillRect(0, 0, w, h)
            // Placeholder when no plugin is enabled
            ctx.fillStyle = 'rgba(200,200,255,0.6)'
            ctx.font = '15px system-ui, sans-serif'
            ctx.textAlign = 'center'
            ctx.fillText('请先在「插件管理」中启用可视化插件', w / 2, h / 2 - 10)
            ctx.fillStyle = 'rgba(200,200,255,0.35)'
            ctx.font = '12px system-ui, sans-serif'
            ctx.fillText('把 .js 脚本放进 plugins/vis/ 目录，然后在插件管理中启用', w / 2, h / 2 + 14)
          }
        }
      }

      rafRef.current = requestAnimationFrame(loop)
    }
    rafRef.current = requestAnimationFrame(loop)
    return () => cancelAnimationFrame(rafRef.current)
  }, [playbackState])

  const fullscreenLabel = useMemo(() => fullscreen ? '退出全屏' : '全屏', [fullscreen])
  const hasEnabledPlugins = sources.length > 0

  return (
    <div
      ref={containerRef}
      className="visualizer-page"
      style={{
        position: fullscreen ? 'fixed' : 'relative',
        inset: fullscreen ? 0 : undefined,
        zIndex: fullscreen ? 9998 : 1,
        width: '100%',
        height: fullscreen ? '100vh' : 'calc(100vh - 130px)',
        minHeight: 380,
        background: '#05060b',
        display: 'flex',
        flexDirection: 'column',
        overflow: 'hidden',
      }}
    >
      {/* Toolbar */}
      <div style={{
        display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: 8,
        padding: '8px 12px',
        background: 'rgba(10,12,22,0.85)',
        backdropFilter: 'blur(6px)',
        borderBottom: '1px solid var(--border, #222)',
        flexShrink: 0,
      }}>
        {/* Script list as horizontal chips — only enabled plugins */}
        <div style={{
          display: 'flex', flexWrap: 'nowrap', gap: 4, overflowX: 'auto',
          padding: 2, borderRadius: 8,
          background: 'rgba(255,255,255,0.03)',
          maxWidth: '40%',
        }}>
          {sources.map((s) => (
            <button
              key={s.file_name}
              onClick={() => loadScript(s)}
              style={{
                fontSize: 'var(--text-sm)',
                padding: '5px 10px',
                borderRadius: 6,
                border: activeFileName === s.file_name
                  ? '1px solid var(--accent, #66f)'
                  : '1px solid transparent',
                background: activeFileName === s.file_name
                  ? 'color-mix(in srgb, var(--accent,#66f) 20%, transparent)'
                  : 'transparent',
                color: activeFileName === s.file_name ? '#fff' : 'rgba(230,230,255,0.75)',
                cursor: 'pointer',
                whiteSpace: 'nowrap',
                flexShrink: 0,
              }}
            >
              {s.name}
            </button>
          ))}
          {sources.length === 0 && (
            <span style={{ fontSize: 'var(--text-xs)', color: '#667', padding: '5px 10px' }}>
              未启用任何可视化插件
            </span>
          )}
        </div>

        {loading && <span style={{ fontSize: 'var(--text-xs)', color: '#889' }}>加载中…</span>}

        <div style={{ flex: 1 }} />

        {/* Active script badge */}
        {activeDisplayName && (
          <span style={{
            fontSize: 'var(--text-xs)', color: 'rgba(200,230,255,0.8)',
            padding: '3px 8px', borderRadius: 999,
            background: 'rgba(120,180,255,0.08)',
            border: '1px solid rgba(120,180,255,0.2)',
          }}>
            {activeDisplayName}
          </span>
        )}

        <button
          onClick={() => setFullscreen(v => !v)}
          style={{
            fontSize: 'var(--text-sm)', padding: '5px 12px', borderRadius: 6,
            border: '1px solid var(--border,#333)',
            background: 'transparent', color: 'var(--text,#ddd)',
            cursor: 'pointer',
          }}
        >
          {fullscreenLabel}
        </button>
        {onClose && (
          <button
            onClick={onClose}
            style={{
              fontSize: 'var(--text-sm)', padding: '5px 12px', borderRadius: 6,
              border: '1px solid var(--border,#333)',
              background: 'transparent', color: 'var(--text,#ddd)',
              cursor: 'pointer',
            }}
          >
            关闭
          </button>
        )}
      </div>

      {scriptError && (
        <div style={{
          fontSize: 'var(--text-sm)', padding: '4px 12px',
          color: '#f66', background: 'rgba(255,80,80,0.06)',
          borderBottom: '1px solid rgba(255,80,80,0.2)',
        }}>{scriptError}</div>
      )}

      {/* Canvas surface */}
      <div style={{
        position: 'relative', flex: 1, minHeight: 0,
        display: 'flex', flexDirection: 'column',
      }}>
        <canvas
          ref={canvasRef}
          style={{
            position: 'absolute', inset: 0, width: '100%', height: '100%',
            display: 'block',
          }}
        />
        <div style={{
          position: 'absolute', left: 12, bottom: 10,
          fontSize: 'var(--text-xs)', color: 'rgba(220,220,255,0.4)',
          fontFamily: 'monospace', pointerEvents: 'none',
          userSelect: 'none',
        }}>
          {hasEnabledPlugins ? (activeDisplayName || '未选择') : '无可用插件'}
          {'  ·  '}
          {playbackState}
        </div>
      </div>
    </div>
  )
}
