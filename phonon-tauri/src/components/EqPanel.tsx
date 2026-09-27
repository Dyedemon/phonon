import { useState, useEffect, useCallback, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useI18n } from '../i18n'
import { getSpectrumBands } from '../api/spectrumBus'

interface EqBand {
  frequency: number
  gain_db: number
  q: number
  filter_type: string
}

interface EqState {
  bands: EqBand[]
  mode: string
  presets: string[]
  geq_band_count: number
  preamp_db: number
}

const MAX_BANDS = 20
const FILTER_TYPES = ['Peaking', 'LowShelf', 'HighShelf', 'LowPass', 'HighPass']
const CANVAS_MARGIN = 62
const CANVAS_BOTTOM = 28
const PLOT_TOP = 10

// ── Floating edit panel ───────────────────────────────────
function BandEditPopup({
  band, index, onSave, onClose, onRemove,
  parentRef: _parentRef, pos, setPos,
}: {
  band: EqBand; index: number
  onSave: (index: number, b: EqBand) => void
  onClose: () => void
  onRemove?: (index: number) => void
  parentRef: React.RefObject<HTMLDivElement | null>
  pos: { x: number; y: number } | null
  setPos: (p: { x: number; y: number }) => void
}) {
  const { t } = useI18n()
  const [freq, setFreq] = useState(band.frequency)
  const [gain, setGain] = useState(band.gain_db)
  const [q, setQ] = useState(band.q)
  const [ft, setFt] = useState(band.filter_type)
  const dragRef = useRef<{ sx: number; sy: number; px: number; py: number } | null>(null)

  const handleMouseDown = (e: React.MouseEvent) => {
    // Only allow dragging from header
    if ((e.target as HTMLElement).closest('.band-edit-header') === null) return
    e.preventDefault()
    dragRef.current = { sx: e.clientX, sy: e.clientY, px: pos?.x ?? 8, py: pos?.y ?? 8 }
    const onMove = (ev: MouseEvent) => {
      if (!dragRef.current) return
      const dx = ev.clientX - dragRef.current.sx
      const dy = ev.clientY - dragRef.current.sy
      setPos({ x: dragRef.current.px + dx, y: dragRef.current.py + dy })
    }
    const onUp = () => {
      dragRef.current = null
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
    }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
  }

  return (
    <div style={{
      position: 'absolute', top: pos?.y ?? 8, left: pos?.x ?? 8, zIndex: 100,
      background: 'var(--bg-card)', border: '1px solid var(--border-glow)',
      borderRadius: 'var(--radius)', padding: 0, minWidth: 210,
      boxShadow: '0 8px 32px rgba(0,0,0,0.6)', backdropFilter: 'blur(20px)',
      fontSize: 'var(--text-sm)', overflow: 'hidden',
      cursor: dragRef.current ? 'grabbing' : 'default',
    }}>
      {/* Draggable header */}
      <div
        className="band-edit-header"
        onMouseDown={handleMouseDown}
        style={{
          padding: '8px 14px',
          background: 'var(--bg-active)',
          cursor: 'grab',
          borderBottom: '1px solid var(--border)',
        }}
      >
        <div style={{ fontWeight: 600, color: 'var(--text-bright)', fontSize: 'var(--text-sm)', letterSpacing: 0.5 }}>
          {t('eq.bandTitle').replace('{n}', String(index + 1))}
        </div>
      </div>
      <div style={{ padding: '0 14px 14px 14px' }}>
      <div style={{ display: 'grid', gridTemplateColumns: '56px 1fr', gap: '6px 8px', alignItems: 'center' }}>
        <label style={{ color: 'var(--text-dim)', fontSize: 'var(--text-xs)' }}>{t('eq.freq')}</label>
        <input type="number" min={20} max={20000} step={1} value={freq}
          onChange={e => setFreq(+e.target.value)} />
        <label style={{ color: 'var(--text-dim)', fontSize: 'var(--text-xs)' }}>{t('eq.gain')}</label>
        <input type="number" min={-24} max={24} step={0.5} value={gain}
          onChange={e => setGain(+e.target.value)} />
        <label style={{ color: 'var(--text-dim)', fontSize: 'var(--text-xs)' }}>{t('eq.q')}</label>
        <input type="number" min={0.1} max={10} step={0.1} value={q}
          onChange={e => setQ(+e.target.value)} />
        <label style={{ color: 'var(--text-dim)', fontSize: 'var(--text-xs)' }}>{t('eq.filterType')}</label>
        <select value={ft} onChange={e => setFt(e.target.value)} style={{ cursor: 'pointer' }}>
          {FILTER_TYPES.map(t => <option key={t} value={t}>{t}</option>)}
        </select>
      </div>
      <div style={{ display: 'flex', gap: 6, marginTop: 10 }}>
        <button className="btn btn-primary btn-sm" onClick={() => {
          onSave(index, { frequency: freq, gain_db: gain, q, filter_type: ft }); onClose()
        }}>{t('common.apply')}</button>
        <button className="btn btn-outline btn-sm" onClick={onClose}>{t('common.cancel')}</button>
        {onRemove && (
          <button className="btn btn-danger btn-sm"
            style={{ marginLeft: 'auto' }}
            onClick={() => { onRemove(index); onClose() }}>{t('common.remove')}</button>
        )}
      </div>
      </div>
    </div>
  )
}

// ── Draggable Add-band popup ──────────────────────────────
function AddBandPopup({ onAdd, onClose, parentRef, pos, setPos }: {
  onAdd: (freq: number) => void; onClose: () => void;
  parentRef: React.RefObject<HTMLDivElement | null>;
  pos: { x: number; y: number } | null;
  setPos: (p: { x: number; y: number }) => void;
}) {
  const { t } = useI18n()
  const [freq, setFreq] = useState(1000)
  const presets = [31.5, 63, 125, 250, 500, 1000, 2000, 4000, 8000, 16000]
  const popupRef = useRef<HTMLDivElement>(null)
  const dragRef = useRef<{ sx: number; sy: number; px: number; py: number } | null>(null)

  useEffect(() => {
    if (pos) return
    const parent = parentRef.current
    if (parent) {
      const pr = parent.getBoundingClientRect()
      setPos({ x: pr.width / 2 - 110, y: pr.height / 2 - 80 })
    }
  }, []) // eslint-disable-line react-hooks/exhaustive-deps

  const handleMouseDown = (e: React.MouseEvent) => {
    e.preventDefault()
    dragRef.current = { sx: e.clientX, sy: e.clientY, px: pos?.x ?? 0, py: pos?.y ?? 0 }
    const onMove = (ev: MouseEvent) => {
      if (!dragRef.current) return
      const dx = ev.clientX - dragRef.current.sx
      const dy = ev.clientY - dragRef.current.sy
      setPos({ x: dragRef.current.px + dx, y: dragRef.current.py + dy })
    }
    const onUp = () => {
      dragRef.current = null
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
    }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
  }

  const style: React.CSSProperties = pos ? {
    position: 'absolute', left: pos.x, top: pos.y, zIndex: 100,
    background: 'var(--bg-card)', border: '1px solid var(--border-glow)',
    borderRadius: 'var(--radius)', padding: 14, minWidth: 220,
    boxShadow: '0 8px 32px rgba(0,0,0,0.6)', backdropFilter: 'blur(20px)',
    fontSize: 'var(--text-sm)',
  } : {
    position: 'absolute', left: '50%', top: '50%', transform: 'translate(-50%, -50%)', zIndex: 100,
    background: 'var(--bg-card)', border: '1px solid var(--border-glow)',
    borderRadius: 'var(--radius)', padding: 14, minWidth: 220,
    boxShadow: '0 8px 32px rgba(0,0,0,0.6)', backdropFilter: 'blur(20px)',
    fontSize: 'var(--text-sm)',
  }

  return (
    <div ref={popupRef} style={style}>
      <div onMouseDown={handleMouseDown}
        style={{ cursor: 'move', marginBottom: 10, paddingBottom: 6, borderBottom: '1px solid var(--border)', color: 'var(--text-bright)', fontWeight: 600, fontSize: 'var(--text-sm)', letterSpacing: 0.3 }}>
        {t('eq.addBandTitle')}
      </div>

      <div style={{ display: 'flex', gap: 6, marginBottom: 10, flexWrap: 'wrap' }}>
        {presets.map(f => (
          <button key={f} className={freq === f ? 'btn btn-primary btn-sm' : 'btn btn-outline btn-sm'}
            onClick={() => setFreq(f)}>
            {f >= 1000 ? `${(f / 1000).toFixed(f % 1000 === 0 ? 0 : 1)}k` : f.toFixed(f % 1 === 0 ? 0 : 1)}
          </button>
        ))}
      </div>

      <div style={{ display: 'flex', gap: 8, alignItems: 'center', marginBottom: 10 }}>
        <label style={{ color: 'var(--text-dim)', fontSize: 'var(--text-xs)' }}>{t('eq.custom')}:</label>
        <input type="number" min={20} max={20000} step={1} value={freq}
          onChange={e => setFreq(+e.target.value)} style={{ width: 80 }} />
        <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-dim)' }}>Hz</span>
      </div>

      <div style={{ display: 'flex', gap: 6 }}>
        <button className="btn btn-primary btn-sm" onClick={() => { onAdd(freq); onClose() }}>{t('common.add')}</button>
        <button className="btn btn-outline btn-sm" onClick={onClose}>{t('common.cancel')}</button>
      </div>
    </div>
  )
}

// ── Drag tooltip ──────────────────────────────────────────
function DragTooltip({ text, x, y }: { text: string; x: number; y: number }) {
  return (
    <div style={{
      position: 'absolute', left: x + 8, top: y - 10,
      background: 'var(--bg-active)', color: 'var(--accent)',
      padding: '3px 8px', borderRadius: 'var(--radius-sm)', fontSize: 'var(--text-sm)',
      fontFamily: 'var(--mono)', pointerEvents: 'none',
      whiteSpace: 'nowrap', zIndex: 80,
      border: '1px solid var(--border-glow)',
      boxShadow: '0 4px 12px rgba(0,0,0,0.4)',
    }}>{text}</div>
  )
}

// ── Crosshair readout ─────────────────────────────────────
function CrosshairReadout({ freq, db, x, y }: { freq: string; db: string; x: number; y: number }) {
  return (
    <div style={{
      position: 'absolute', left: x + 12, top: y - 22,
      background: 'var(--bg-active)', color: 'var(--text)',
      padding: '2px 8px', borderRadius: 'var(--radius-sm)', fontSize: 'var(--text-xs)',
      fontFamily: 'var(--mono)', pointerEvents: 'none',
      whiteSpace: 'nowrap', zIndex: 70,
      border: '1px solid var(--border)',
      boxShadow: '0 4px 12px rgba(0,0,0,0.3)',
    }}>{freq} &nbsp; {db}</div>
  )
}

// ── Main EqPanel ──────────────────────────────────────────
export default function EqPanel({ colorScheme, customColors, addToast }: { colorScheme?: string; customColors?: { bottom: string; top: string; peak: string; grid: string }; addToast?: (msg: string, kind?: 'info' | 'warn' | 'error') => void }) {
  const { t } = useI18n()
  const [expanded, setExpanded] = useState(false)
  const [eqState, setEqState] = useState<EqState | null>(null)
  const [eqEnabled, setEqEnabled] = useState(false)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const parentRef = useRef<HTMLDivElement>(null)
  // Latest spectrum bands from the bus (kept out of React state — the
  // ~40 Hz stream must not re-render the panel).
  const spectrumRef = useRef<number[]>([])
  const dragRef = useRef<{ index: number; startY: number; startGain: number; startQ: number; altKey: boolean } | null>(null)
  const [editPopup, setEditPopup] = useState<{ band: EqBand; index: number } | null>(null)
  const [editPopupPos, setEditPopupPos] = useState<{ x: number; y: number }>({ x: 8, y: 8 })
  const [deleteMode, setDeleteMode] = useState(() => localStorage.getItem('phonon-eq-delete-mode') === 'true')
  const [saveName, setSaveName] = useState('')
  const [showSave, setShowSave] = useState(false)
  const [showAddBand, setShowAddBand] = useState(false)
  const [addBandPos, setAddBandPos] = useState<{ x: number; y: number } | null>(null)
  const [dragTooltip, setDragTooltip] = useState<{ text: string; x: number; y: number } | null>(null)
  const [crosshair, setCrosshair] = useState<{ x: number; y: number; freq: string; db: string } | null>(null)
  const [activeSlider, setActiveSlider] = useState<{ index: number; type: 'gain' | 'q' } | null>(null)
  const [isDraggingOnCanvas, setIsDraggingOnCanvas] = useState(false)
  const [colorVersion, setColorVersion] = useState(0)
  const [spectrumColors, setSpectrumColors] = useState(() => {
    const scheme = localStorage.getItem('phonon-spectrum-colorscheme') || 'aurora'
    const saved = localStorage.getItem('phonon-spectrum-custom-colors')
    if (scheme === 'custom' && saved) {
      try { return JSON.parse(saved) }
      catch { /* ignore */ }
    }
    const SCHEMES: Record<string, { bottom: string; top: string; peak: string; grid: string }> = {
      aurora: { bottom: '#003153', top: '#00BFFF', peak: '#00FFFF', grid: '#404040' },
      warm: { bottom: '#4a1500', top: '#FF5500', peak: '#FFAA00', grid: '#4a3030' },
      cool: { bottom: '#1a0050', top: '#7744FF', peak: '#AA88FF', grid: '#303050' },
      neon: { bottom: '#004000', top: '#00FF00', peak: '#88FF88', grid: '#204020' },
      fire: { bottom: '#300000', top: '#FF3300', peak: '#FF6600', grid: '#402020' },
      ocean: { bottom: '#001040', top: '#0066FF', peak: '#00AAFF', grid: '#203050' },
    }
    return SCHEMES[scheme] || SCHEMES.aurora
  })
  const isDraggingSliderRef = useRef(false)
  const scrollRef = useRef<HTMLDivElement>(null)
  // Smoothed spectrum values to reduce jitter
  const smoothedRef = useRef<number[] | null>(null)

  const refresh = useCallback(async () => {
    try {
      const state = await invoke<EqState>('get_eq_state')
      setEqState(state)
      const list = await invoke<{id: string; enabled: boolean}[]>('list_dsp_processors')
      const eq = list.find((p) => p.id === 'equalizer')
      if (eq) setEqEnabled(eq.enabled)
    } catch (_) { /* ignore */ }
  }, [])

  useEffect(() => { refresh() }, [refresh])

  useEffect(() => {
    if (colorScheme && customColors) {
      const SCHEMES: Record<string, { bottom: string; top: string; peak: string; grid: string }> = {
        aurora: { bottom: '#003153', top: '#00BFFF', peak: '#00FFFF', grid: '#404040' },
        warm: { bottom: '#4a1500', top: '#FF5500', peak: '#FFAA00', grid: '#4a3030' },
        cool: { bottom: '#1a0050', top: '#7744FF', peak: '#AA88FF', grid: '#303050' },
        neon: { bottom: '#004000', top: '#00FF00', peak: '#88FF88', grid: '#204020' },
        fire: { bottom: '#300000', top: '#FF3300', peak: '#FF6600', grid: '#402020' },
        ocean: { bottom: '#001040', top: '#0066FF', peak: '#00AAFF', grid: '#203050' },
      }
      const colors = colorScheme === 'custom' ? customColors : SCHEMES[colorScheme] || SCHEMES.aurora
      setSpectrumColors(colors)
      setColorVersion(v => v + 1)
    }
  }, [colorScheme, customColors])

  const toggleEq = async () => {
    try {
      if (eqEnabled) await invoke('disable_dsp', { id: 'equalizer' })
      else await invoke('enable_dsp', { id: 'equalizer' })
      setEqEnabled(!eqEnabled)
    } catch (e) { console.error('Failed to toggle EQ:', e) }
  }

  const updateBand = async (index: number, band: EqBand) => {
    if (!eqState) return
    try {
      await invoke('set_eq_band', {
        index, frequency: band.frequency, gainDb: band.gain_db,
        q: band.q, filterType: band.filter_type,
      })
      setEqState(prev => {
        if (!prev) return prev
        const newBands = [...prev.bands]
        newBands[index] = band
        return { ...prev, bands: newBands }
      })
    } catch (e) { console.error('Failed to set EQ band:', e) }
  }

  // rAF-batched EQ band update scheduler. Collects the latest value per
  // (index, field) across all slider pixels within a single frame, then
  // issues at most 1 invoke per frame per band-field. This turns a 60 Hz
  // slider drag (which would otherwise flood Tauri's callback table with
  // 60 pending Promises per second — the #1 source of
  // "[TAURI] Couldn't find callback id" warnings on reload) into a
  // steady 1 invoke per frame, which is always safe.
  const pendingBandsRef = useRef<Record<string, { index: number; band: EqBand }>>({})
  const bandRafRef = useRef<number | null>(null)
  const flushPendingBandUpdates = () => {
    bandRafRef.current = null
    const map = pendingBandsRef.current
    pendingBandsRef.current = {}
    for (const key of Object.keys(map)) {
      const { index, band } = map[key]
      invoke('set_eq_band', {
        index,
        frequency: band.frequency,
        gainDb: band.gain_db,
        q: band.q,
        filterType: band.filter_type,
      }).catch(e => console.error('Failed to set EQ band:', e))
    }
  }
  const scheduleBandFlush = () => {
    if (bandRafRef.current !== null) return
    bandRafRef.current = requestAnimationFrame(flushPendingBandUpdates)
  }
  // Same pattern for preamp — a single batched ref + rAF flush.
  const pendingPreampRef = useRef<number | null>(null)
  const preampRafRef = useRef<number | null>(null)
  const flushPendingPreamp = () => {
    preampRafRef.current = null
    const v = pendingPreampRef.current
    pendingPreampRef.current = null
    if (v !== null) {
      invoke('set_eq_preamp', { db: v }).catch(e => console.error('Failed to set preamp:', e))
    }
  }

  // Standardized cleanup (spec §12): cancel any pending rAF on unmount so
  // we never fire an invoke after the component is gone. Without this, HMR
  // or StrictMode double-mount can trigger "Couldn't find callback id".
  useEffect(() => {
    return () => {
      if (bandRafRef.current !== null) {
        cancelAnimationFrame(bandRafRef.current)
        bandRafRef.current = null
      }
      if (preampRafRef.current !== null) {
        cancelAnimationFrame(preampRafRef.current)
        preampRafRef.current = null
      }
    }
  }, [])

  const setGain = (index: number, gain_db: number) => {
    if (!eqState) return
    const band = { ...eqState.bands[index], gain_db }
    setEqState(prev => {
      if (!prev) return prev
      const newBands = [...prev.bands]
      newBands[index] = band
      return { ...prev, bands: newBands }
    })
    pendingBandsRef.current[`g${index}`] = { index, band }
    scheduleBandFlush()
  }

  const setQ = (index: number, q: number) => {
    if (!eqState) return
    const band = { ...eqState.bands[index], q }
    setEqState(prev => {
      if (!prev) return prev
      const newBands = [...prev.bands]
      newBands[index] = band
      return { ...prev, bands: newBands }
    })
    pendingBandsRef.current[`q${index}`] = { index, band }
    scheduleBandFlush()
  }

  const loadPreset = async (name: string) => {
    try { await invoke('load_eq_preset', { name }); refresh() }
    catch (e) { console.error('Failed to load preset:', e) }
  }

  const savePreset = async () => {
    if (!saveName.trim()) return
    try { await invoke('save_eq_preset', { name: saveName.trim() }); setSaveName(''); setShowSave(false); refresh() }
    catch (e) { console.error('Failed to save preset:', e) }
  }

  const deletePreset = async (name: string) => {
    if (!window.confirm(t('eq.deletePresetConfirm', 'Delete preset "{name}"?').replace('{name}', name))) return
    try {
      const success = await invoke('delete_eq_preset', { name })
      if (!success) {
        // Cannot delete built-in preset
        addToast?.(t('eq.cannotDeleteBuiltin', 'Built-in presets cannot be deleted.'), 'warn')
        return
      }
      await invoke('load_eq_preset', { name: 'Flat' })
      refresh()
    } catch (e) { console.error('Failed to delete preset:', e) }
  }

  const toggleMode = async () => {
    if (!eqState) return
    const newMode = eqState.mode === 'Graphic' ? 'Parametric' : 'Graphic'
    try { await invoke('set_eq_mode', { mode: newMode }); refresh() }
    catch (e) { console.error('Failed to set EQ mode:', e) }
  }

  const setPreamp = (db: number) => {
    // Update canvas immediately, sync backend in a rAF-batched invoke.
    setEqState(prev => prev ? { ...prev, preamp_db: db } : prev)
    pendingPreampRef.current = db
    if (preampRafRef.current !== null) return
    preampRafRef.current = requestAnimationFrame(flushPendingPreamp)
  }

  const addBand = async (frequency: number) => {
    try {
      await invoke('add_eq_band', { frequency, gainDb: 0, q: 1.414, filterType: 'Peaking' })
      refresh()
    } catch (e) { console.error('Failed to add band:', e) }
  }

  const removeBand = async (index: number) => {
    try { await invoke('remove_eq_band', { index }); refresh() }
    catch (e) { console.error('Failed to remove band:', e) }
  }

  const setGeqBandCount = async (count: number) => {
    try { await invoke('set_geq_band_count', { count }); refresh() }
    catch (e) { console.error('Failed to set GEQ band count:', e) }
  }

  const freqLabel = (f: number) =>
    f >= 1000 ? `${(f / 1000).toFixed(f % 1000 === 0 ? 0 : 1)}k` : f.toFixed(f % 1 === 0 ? 0 : 1)

  const logMin = Math.log10(20)
  const logMax = Math.log10(20000)

  // ── Canvas rendering ──────────────────────────────────
  const draw = useCallback(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const ctx = canvas.getContext('2d')
    if (!ctx) return

    const dpr = window.devicePixelRatio || 1
    const rect = canvas.getBoundingClientRect()
    const w = rect.width
    const h = rect.height
    const cw = w * dpr
    const ch = h * dpr
    // Only resize canvas when dimensions actually change to avoid clearing every frame
    if (canvas.width !== cw || canvas.height !== ch) {
      canvas.width = cw
      canvas.height = ch
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)

    const plotW = w - CANVAS_MARGIN - 8
    const plotH = h - CANVAS_BOTTOM - PLOT_TOP - 4
    const midY = PLOT_TOP + plotH / 2

    // Background (CSS vars don't work in canvas — use hex)
    ctx.fillStyle = '#08080f'
    ctx.fillRect(0, 0, w, h)

    // Grid — more visible
    ctx.strokeStyle = 'rgba(30, 30, 50, 0.8)'
    ctx.lineWidth = 0.5
    for (let i = 0; i <= 8; i++) {
      const y = PLOT_TOP + (plotH / 8) * i
      ctx.beginPath()
      ctx.moveTo(CANVAS_MARGIN, y)
      ctx.lineTo(w - 4, y)
      ctx.stroke()
    }

    // Y-axis dB labels
    ctx.textAlign = 'right'
    for (let i = 0; i <= 8; i++) {
      const db = 24 - i * 6
      const y = PLOT_TOP + (plotH / 8) * i + 4
      ctx.fillStyle = '#565d79'
      ctx.font = '10px monospace'
      ctx.fillText(`${db > 0 ? '+' : ''}${db}`, CANVAS_MARGIN - 8, y)
    }

    // 0 dB line
    ctx.strokeStyle = 'rgba(30, 30, 50, 0.9)'
    ctx.lineWidth = 1
    ctx.setLineDash([4, 8])
    ctx.beginPath()
    ctx.moveTo(CANVAS_MARGIN, midY)
    ctx.lineTo(w - 4, midY)
    ctx.stroke()
    ctx.setLineDash([])

    // Frequency labels
    const freqLabels = ['20', '50', '100', '200', '500', '1k', '2k', '5k', '10k', '20k']
    const freqVals = [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000]
    freqLabels.forEach((label, i) => {
      const x = CANVAS_MARGIN + ((Math.log10(freqVals[i]) - logMin) / (logMax - logMin)) * plotW
      ctx.fillStyle = '#565d79'
      ctx.font = '10px monospace'
      ctx.textAlign = 'center'
      ctx.fillText(label, x, PLOT_TOP + plotH + 14)
    })

    // Spectrum background — more visible with gradient, smoothed with EMA to reduce jitter
    const spectrumValues = spectrumRef.current
    if (spectrumValues && spectrumValues.length >= 32) {
      // Initialize smoothed values on first frame
      if (!smoothedRef.current || smoothedRef.current.length !== spectrumValues.length) {
        smoothedRef.current = [...spectrumValues]
      }
      const alpha = 0.25 // smaller = smoother, larger = more responsive
      for (let i = 0; i < 32; i++) {
        // Exponential moving average
        smoothedRef.current[i] = smoothedRef.current[i] * (1 - alpha) + (spectrumValues[i] || 0) * alpha
        const v = smoothedRef.current[i]
        const barW = plotW / 32
        const barH = v * plotH * 0.55
        const x = CANVAS_MARGIN + i * barW
        const gradient = ctx.createLinearGradient(x, PLOT_TOP + plotH, x, PLOT_TOP + plotH - barH)
        gradient.addColorStop(0, spectrumColors.bottom + '26')
        gradient.addColorStop(1, spectrumColors.top + 'b4')
        ctx.fillStyle = gradient
        ctx.fillRect(x, PLOT_TOP + plotH - barH, barW - 1, barH)
      }
    }

    // EQ curve
    if (eqState && eqState.bands.length > 0) {
      const dbScale = plotH / 48
      const bandPoints: { x: number; y: number; index: number }[] = []

      for (let i = 0; i < eqState.bands.length; i++) {
        const b = eqState.bands[i]
        const x = CANVAS_MARGIN + ((Math.log10(b.frequency) - logMin) / (logMax - logMin)) * plotW
        const y = midY - b.gain_db * dbScale
        bandPoints.push({ x, y, index: i })
      }

      // Area fill
      ctx.beginPath()
      ctx.moveTo(CANVAS_MARGIN, midY)
      for (const pt of bandPoints) ctx.lineTo(pt.x, pt.y)
      ctx.lineTo(w - 4, midY)
      ctx.closePath()
      const areaGrad = ctx.createLinearGradient(0, PLOT_TOP, 0, PLOT_TOP + plotH)
      areaGrad.addColorStop(0, 'rgba(6, 182, 212, 0.10)')
      areaGrad.addColorStop(0.5, 'rgba(6, 182, 212, 0.02)')
      areaGrad.addColorStop(1, 'rgba(6, 182, 212, 0.10)')
      ctx.fillStyle = areaGrad
      ctx.fill()

      // Curve line
      ctx.beginPath()
      ctx.strokeStyle = '#06b6d4'
      ctx.lineWidth = 2
      ctx.shadowColor = 'rgba(6, 182, 212, 0.4)'
      ctx.shadowBlur = 8
      ctx.moveTo(bandPoints[0].x, bandPoints[0].y)
      for (let i = 1; i < bandPoints.length; i++) ctx.lineTo(bandPoints[i].x, bandPoints[i].y)
      ctx.stroke()
      ctx.shadowBlur = 0

      // Control points
      for (const pt of bandPoints) {
        ctx.beginPath()
        ctx.arc(pt.x, pt.y, 5, 0, Math.PI * 2)
        ctx.fillStyle = '#06b6d4'
        ctx.fill()
        ctx.strokeStyle = '#fff'
        ctx.lineWidth = 1.5
        ctx.stroke()
      }

      (canvas as any).__bandPoints = bandPoints
    } else if (eqState && eqState.bands.length === 0) {
      ctx.strokeStyle = 'rgba(30, 30, 50, 0.5)'
      ctx.lineWidth = 1
      ctx.beginPath()
      ctx.moveTo(CANVAS_MARGIN, midY)
      ctx.lineTo(w - 4, midY)
      ctx.stroke()
    }
  }, [eqState, colorVersion, spectrumColors])

  // Redraw when EQ state / colors change (static content).
  useEffect(() => { draw() }, [draw])

  // Redraw on incoming spectrum frames, rAF-coalesced, straight from the
  // bus — the ~40 Hz stream never flows through React state/props.
  useEffect(() => {
    const onBands = () => {
      spectrumRef.current = getSpectrumBands()
      draw()
    }
    window.addEventListener('spectrum-bands', onBands)
    return () => window.removeEventListener('spectrum-bands', onBands)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [draw])
  useEffect(() => {
    const onResize = () => draw()
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  }, [draw])
  useEffect(() => {
    const onColorsChanged = () => {
      const saved = localStorage.getItem('phonon-spectrum-custom-colors')
      if (saved) {
        try { setSpectrumColors(JSON.parse(saved)) }
        catch { /* ignore */ }
      }
      setColorVersion(v => v + 1)
    }
    window.addEventListener('spectrum-colors-changed', onColorsChanged)
    return () => window.removeEventListener('spectrum-colors-changed', onColorsChanged)
  }, [])

  // ── Mouse interaction ──────────────────────────────────
  const handleCanvasMouseDown = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const canvas = canvasRef.current
    if (!canvas) return
    const rect = canvas.getBoundingClientRect()
    const mx = e.clientX - rect.left
    const my = e.clientY - rect.top

    // Only allow dragging within the plot area
    const plotW = rect.width - CANVAS_MARGIN - 8
    const plotH = rect.height - CANVAS_BOTTOM - PLOT_TOP - 4
    if (mx < CANVAS_MARGIN || mx > CANVAS_MARGIN + plotW || my < PLOT_TOP || my > PLOT_TOP + plotH) return

    // Lock scroll immediately to prevent the canvas drag from moving the container
    setIsDraggingOnCanvas(true)
    if (scrollRef.current) scrollRef.current.style.overflowY = 'hidden'

    const pts = (canvas as any).__bandPoints as { x: number; y: number; index: number }[] | undefined
    if (!pts || !eqState) return

    const hit = pts.find((p) => Math.hypot(p.x - mx, p.y - my) < 14)
    if (hit) {
      const band = eqState.bands[hit.index]
      if (e.altKey) {
        setEditPopup({ band, index: hit.index })
      } else {
        dragRef.current = { index: hit.index, startY: e.clientY, startGain: band.gain_db, startQ: band.q, altKey: e.altKey }
        setDragTooltip({ text: `${band.gain_db > 0 ? '+' : ''}${band.gain_db.toFixed(1)} dB`, x: mx, y: my })
      }
    }
  }

  const handleCanvasMouseMove = (e: React.MouseEvent<HTMLCanvasElement>) => {
    // Prevent parent scroll whenever interacting with the canvas plot area
    if (isDraggingOnCanvas) e.preventDefault()

    const canvas = canvasRef.current
    if (!canvas) return
    const rect = canvas.getBoundingClientRect()
    const mx = e.clientX - rect.left
    const my = e.clientY - rect.top

    if (dragRef.current && eqState) {
      const plotH = rect.height - CANVAS_BOTTOM - PLOT_TOP - 4
      const dy = dragRef.current.startY - e.clientY
      if (e.altKey) {
        const newQ = Math.round((dragRef.current.startQ + dy * 0.1) * 10) / 10
        const clampedQ = Math.max(0.1, Math.min(10, newQ))
        setQ(dragRef.current.index, clampedQ)
        dragRef.current.startY = e.clientY
        dragRef.current.startQ = clampedQ
        setDragTooltip({ text: `Q: ${clampedQ.toFixed(1)}`, x: mx, y: my })
      } else {
        const dbScale = 48 / plotH
        const newGain = Math.round((dragRef.current.startGain + dy * dbScale) * 10) / 10
        const clampedGain = Math.max(-24, Math.min(24, newGain))
        setGain(dragRef.current.index, clampedGain)
        dragRef.current.startY = e.clientY
        dragRef.current.startGain = clampedGain
        setDragTooltip({ text: `${clampedGain > 0 ? '+' : ''}${clampedGain.toFixed(1)} dB`, x: mx, y: my })
      }
    } else {
      const plotW = rect.width - CANVAS_MARGIN - 8
      const plotH = rect.height - CANVAS_BOTTOM - PLOT_TOP - 4
      const midY = PLOT_TOP + plotH / 2
      if (mx >= CANVAS_MARGIN && mx <= CANVAS_MARGIN + plotW && my >= PLOT_TOP && my <= PLOT_TOP + plotH) {
        const freq = Math.pow(10, logMin + ((mx - CANVAS_MARGIN) / plotW) * (logMax - logMin))
        const db = ((midY - my) / plotH) * 48
        setCrosshair({
          x: mx, y: my,
          freq: freq >= 1000 ? `${(freq / 1000).toFixed(1)}kHz` : `${freq.toFixed(0)}Hz`,
          db: `${db > 0 ? '+' : ''}${db.toFixed(1)} dB`,
        })
      } else {
        setCrosshair(null)
      }
    }
  }

  // Prevent document scrolling during canvas drag
  useEffect(() => {
    if (!isDraggingOnCanvas) return

    const preventScroll = (e: Event) => {
      e.preventDefault()
    }

    // Wheel events still cause scrolling — prevent them
    window.addEventListener('wheel', preventScroll, { passive: false })
    return () => window.removeEventListener('wheel', preventScroll)
  }, [isDraggingOnCanvas])

  // Prevent document scrolling during slider drag (ref-based, synchronous)
  useEffect(() => {
    const preventScroll = (e: Event) => {
      if (isDraggingSliderRef.current) {
        e.preventDefault()
      }
    }
    window.addEventListener('wheel', preventScroll, { passive: false })
    window.addEventListener('touchmove', preventScroll, { passive: false })
    return () => {
      window.removeEventListener('wheel', preventScroll)
      window.removeEventListener('touchmove', preventScroll)
    }
  }, [])

  const handleCanvasMouseUp = () => { dragRef.current = null; setDragTooltip(null); setIsDraggingOnCanvas(false); if (scrollRef.current) scrollRef.current.style.overflowY = 'auto' }
  const handleCanvasMouseLeave = () => { handleCanvasMouseUp(); setCrosshair(null) }

  // ── Render ───────────────────────────────────────────
  if (!eqState) return null

  const isPeq = eqState.mode === 'Parametric'

  return (
    <div className="card eq-panel" style={{ marginTop: 'var(--eq-panel-mt, 12px)' }}>
      {/* Header — collapsible style */}
      <div
        className="collapsible-header"
        style={{ padding: '0 0 10px 0', marginBottom: expanded ? 14 : 0, borderBottom: expanded ? '1px solid var(--border)' : 'none' }}
        onClick={() => setExpanded(!expanded)}
      >
        <span className="collapsible-arrow">{expanded ? '\u25BC' : '\u25B6'}</span>
        <span style={{ flex: 1, fontSize: 'var(--text-sm)', fontWeight: 500, letterSpacing: 0.3 }}>{t('eq.title')}</span>
        <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-dim)', marginRight: 10, fontFamily: 'var(--mono)' }}>
          {eqState.mode === 'Graphic' ? t('eq.modeGraphic') : t('eq.modeParametric')}
        </span>
        <div className={`toggle-switch ${eqEnabled ? 'on' : ''}`}
          onClick={(e) => { e.stopPropagation(); toggleEq() }} />
        <span style={{ fontSize: 'var(--text-sm)', marginLeft: 8, color: eqEnabled ? 'var(--accent)' : 'var(--text-dim)' }}>
          {eqEnabled ? t('eq.on') : t('eq.off')}
        </span>
      </div>

      {expanded && (
        <>
          {/* Toolbar */}
          <div className="toolbar" style={{ flexWrap: 'wrap' }}>
            {eqState.presets.map((name) => (
              <div key={name} className="eq-preset-chip">
                <button className="btn btn-outline btn-sm" onClick={() => loadPreset(name)}>
                  {t(`eq.preset.${name}`, name)}
                </button>
                {deleteMode && (
                  <button
                    className="eq-preset-delete-btn"
                    onClick={(e) => { e.stopPropagation(); deletePreset(name) }}
                    title={t('eq.deletePreset')}
                  >×</button>
                )}
              </div>
            ))}
            <div style={{ flex: 1 }} />

            <button className="btn btn-outline btn-sm" onClick={toggleMode}>
              {isPeq ? t('eq.geq') : t('eq.peq')}
            </button>

            {/* GEQ band count selector — 10 / 15 bands.
                Previously a single toggle button that flipped between
                10 and 15. Replaced with a dropdown so the user can
                explicitly pick the target count (the old toggle felt
                ambiguous: clicking "10b" switched to 15, which is
                counter-intuitive). */}
            {!isPeq && (
              <select
                className="btn btn-outline btn-sm"
                value={eqState.geq_band_count}
                onChange={(e) => setGeqBandCount(Number(e.target.value))}
                style={{ cursor: 'pointer', padding: '0 8px', height: 26 }}
                title={t('eq.bandCount', '频段数')}
              >
                <option value={10}>10 {t('eq.bands', '段')}</option>
                <option value={15}>15 {t('eq.bands', '段')}</option>
              </select>
            )}

            <button
              className={`btn btn-outline btn-sm ${deleteMode ? 'btn-danger' : ''}`}
              onClick={() => { const next = !deleteMode; setDeleteMode(next); localStorage.setItem('phonon-eq-delete-mode', String(next)) }}
              title={t('eq.deletePreset')}
            >{t('eq.delete')}</button>

            {showSave ? (
              <div style={{ display: 'flex', gap: 4 }}>
                <input value={saveName} onChange={e => setSaveName(e.target.value)}
                  placeholder={t('eq.presetPlaceholder')}
                  style={{ width: 100, fontSize: 'var(--text-xs)', padding: '4px 8px' }}
                  onKeyDown={e => { if (e.key === 'Enter') savePreset() }} />
                <button className="btn btn-primary btn-sm" onClick={savePreset}>{t('eq.savePreset')}</button>
                <button className="btn btn-outline btn-sm" onClick={() => setShowSave(false)}>{t('eq.close')}</button>
              </div>
            ) : (
              <button className="btn btn-outline btn-sm" onClick={() => setShowSave(true)}>{t('eq.savePreset')}</button>
            )}
          </div>

          {/* Canvas */}
          <div ref={parentRef} style={{ position: 'relative', marginBottom: 6 }}>
            <canvas ref={canvasRef} style={{
              width: '100%', height: 'var(--eq-canvas-height, 250px)', borderRadius: 6,
              cursor: dragRef.current ? (dragRef.current.altKey ? 'ew-resize' : 'grabbing') : 'crosshair',
              background: 'var(--bg)',
            }}
              onMouseDown={handleCanvasMouseDown}
              onMouseMove={handleCanvasMouseMove}
              onMouseUp={handleCanvasMouseUp}
              onMouseLeave={handleCanvasMouseLeave} />

            {/* Add-band button — separated from the toolbar into its own
                floating control below the canvas. Per user request:
                "分离GEQ的「添加频段」按钮". Only shown in PEQ mode since
                GEQ band count is fixed by the 10/15 dropdown above. */}
            {isPeq && (
              <button
                className="btn btn-primary btn-sm"
                onClick={() => setShowAddBand(true)}
                disabled={eqState.bands.length >= MAX_BANDS}
                style={{ position: 'absolute', top: 8, right: 8, zIndex: 10 }}
              >
                + {t('eq.addBand')}
              </button>
            )}

            {editPopup && (
              <BandEditPopup
                band={editPopup.band}
                index={editPopup.index}
                onSave={(idx, b) => updateBand(idx, b)}
                onClose={() => setEditPopup(null)}
                onRemove={isPeq ? removeBand : undefined}
                parentRef={parentRef}
                pos={editPopupPos}
                setPos={setEditPopupPos}
              />
            )}

            {showAddBand && (
              <AddBandPopup
                parentRef={parentRef}
                pos={addBandPos}
                setPos={setAddBandPos}
                onAdd={(freq) => { addBand(freq); setShowAddBand(false) }}
                onClose={() => setShowAddBand(false)}
              />
            )}

            {dragTooltip && <DragTooltip {...dragTooltip} />}
            {crosshair && !dragRef.current && <CrosshairReadout {...crosshair} />}
          </div>

          <div style={{ fontSize: 'var(--text-xs)', color: 'var(--text-dim)', marginBottom: 8, textAlign: 'center', fontFamily: 'var(--mono)' }}>
            {t('eq.shortcuts')}
          </div>

          {/* Preamp Slider */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 10, padding: '0 4px' }}>
            <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-dim)', minWidth: 48 }}>{t('eq.preamp')}</span>
            <input
              type="range"
              min="-12" max="12" step="0.5"
              value={eqState.preamp_db}
              onChange={e => setPreamp(parseFloat(e.target.value))}
              onMouseDown={() => { document.body.style.overflow = 'hidden'; isDraggingSliderRef.current = true }}
              onMouseUp={() => { document.body.style.overflow = ''; isDraggingSliderRef.current = false }}
              className="eq-gain-slider"
              style={{ flex: 1 }}
            />
            <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-bright)', minWidth: 44, textAlign: 'right', fontFamily: 'var(--mono)' }}>
              {eqState.preamp_db > 0 ? '+' : ''}{eqState.preamp_db.toFixed(1)} dB
            </span>
          </div>

          {/* Separator line */}
          <div style={{
            height: 2, background: 'var(--border)', margin: '8px 0 10px',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}>
            <span style={{
              fontSize: 'var(--text-xs)', color: 'var(--text-dim)', background: 'var(--bg)',
              padding: '0 8px', whiteSpace: 'nowrap',
            }}>{t('eq.bands')}</span>
          </div>

          {/* Band sliders — scrollable independently */}
          <div ref={scrollRef} style={{ maxHeight: 'calc(100vh - 520px)', overflowY: (activeSlider || isDraggingOnCanvas) ? 'hidden' : 'auto', overscrollBehavior: 'contain', paddingRight: 4 }}>
            <div style={{ display: 'flex', gap: 2, justifyContent: 'space-between', overflow: 'visible', touchAction: 'none', paddingTop: isPeq ? 6 : 0 }}>
              {eqState.bands.map((band, i) => (
                <div key={i} style={{ flex: '1 0 auto', minWidth: 42, textAlign: 'center', position: 'relative', overflow: 'visible' }}>
                  {/* Delete button (visible in delete mode) */}
                  {deleteMode && (
                    <button onClick={() => removeBand(i)} title={t('eq.removeBand')}
                      style={{
                        position: 'absolute', top: -2, right: 0, zIndex: 10,
                        background: 'var(--danger)', color: '#fff', border: 'none',
                        fontSize: 'var(--text-xs)', cursor: 'pointer', borderRadius: 3,
                        padding: '1px 5px', whiteSpace: 'nowrap',
                      }}
                    >{'×'}</button>
                  )}

                  {/* Gain bar */}
                  <div style={{ height: 100, display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'flex-end', position: 'relative' }}>
                    {activeSlider?.index === i && activeSlider?.type === 'gain' && (
                      <div style={{
                        position: 'absolute', bottom: -24, left: '50%', transform: 'translateX(-50%)',
                        background: 'var(--bg-active)', color: 'var(--accent)',
                        padding: '2px 6px', borderRadius: 'var(--radius-sm)', fontSize: 'var(--text-xs)',
                        fontFamily: 'var(--mono)', whiteSpace: 'nowrap', zIndex: 20,
                        border: '1px solid var(--border-glow)', pointerEvents: 'none',
                        boxShadow: '0 4px 12px rgba(0,0,0,0.4)',
                      }}>{t('eq.gainTooltip').replace('{gain}', band.gain_db.toFixed(1))}</div>
                    )}
                    <div style={{
                      width: 4, height: `${Math.max(3, Math.min(Math.abs(band.gain_db) / 24 * 60, 60))}%`,
                      background: band.gain_db >= 0 ? 'var(--accent)' : 'var(--danger)',
                      borderRadius: 2, transition: 'height 0.1s',
                    }} />
                  </div>

                  {/* Vertical gain slider */}
                  <input type="range" min={-24} max={24} step={0.5} value={band.gain_db}
                    onChange={(e) => setGain(i, parseFloat(e.target.value))}
                    onMouseDown={() => { document.body.style.overflow = 'hidden'; setActiveSlider({ index: i, type: 'gain' }); if (scrollRef.current) scrollRef.current.style.overflowY = 'hidden'; isDraggingSliderRef.current = true }}
                    onMouseUp={() => { document.body.style.overflow = ''; setActiveSlider(null); if (scrollRef.current) scrollRef.current.style.overflowY = 'auto'; isDraggingSliderRef.current = false }}
                    onMouseLeave={() => { document.body.style.overflow = ''; setActiveSlider(null); if (scrollRef.current) scrollRef.current.style.overflowY = 'auto'; isDraggingSliderRef.current = false }}
                    className="eq-gain-slider"
                    style={{
                      writingMode: 'vertical-lr' as any, direction: 'rtl',
                      width: 18, height: 100, margin: '2px auto',
                    }} />

                  {/* Q slider */}
                  <div style={{ width: '60%', margin: '2px auto', position: 'relative' }}>
                    {activeSlider?.index === i && activeSlider?.type === 'q' && (
                      <div style={{
                        position: 'absolute', top: -18, left: '50%', transform: 'translateX(-50%)',
                        background: 'var(--bg-active)', color: 'var(--text)',
                        padding: '2px 6px', borderRadius: 'var(--radius-sm)', fontSize: 'var(--text-xs)',
                        fontFamily: 'var(--mono)', whiteSpace: 'nowrap', zIndex: 20,
                        border: '1px solid var(--border)', pointerEvents: 'none',
                        boxShadow: '0 4px 12px rgba(0,0,0,0.3)',
                      }}>{t('eq.qTooltip').replace('{q}', band.q.toFixed(1))}</div>
                    )}
                    <input type="range" min={0.1} max={10} step={0.1} value={band.q}
                      onChange={(e) => setQ(i, parseFloat(e.target.value))}
                      onMouseDown={() => { document.body.style.overflow = 'hidden'; setActiveSlider({ index: i, type: 'q' }); if (scrollRef.current) scrollRef.current.style.overflowY = 'hidden'; isDraggingSliderRef.current = true }}
                      onMouseUp={() => { document.body.style.overflow = ''; setActiveSlider(null); if (scrollRef.current) scrollRef.current.style.overflowY = 'auto'; isDraggingSliderRef.current = false }}
                      onMouseLeave={() => { document.body.style.overflow = ''; setActiveSlider(null); if (scrollRef.current) scrollRef.current.style.overflowY = 'auto'; isDraggingSliderRef.current = false }}
                      className="eq-q-slider"
                      style={{ width: '100%', height: 4 }}
                      title={t('eq.qTooltip').replace('{q}', band.q.toFixed(1))} />
                  </div>

                  {/* Labels */}
                  <div style={{ fontSize: 'var(--text-xs)', fontFamily: 'var(--mono)', lineHeight: 1.4 }}>
                    <input
                      type="number"
                      value={band.gain_db}
                      min={-24} max={24} step={0.5}
                      onChange={(e) => setGain(i, parseFloat(e.target.value) || 0)}
                      style={{
                        width: 50, fontSize: 'var(--text-xs)', fontFamily: 'var(--mono)',
                        color: 'var(--text)', background: 'transparent',
                        border: '1px solid var(--border)',
                        borderRadius: 3, textAlign: 'center', padding: '1px 2px',
                        outline: 'none', boxSizing: 'border-box',
                      }}
                    />
                  </div>
                  <div style={{ fontSize: 'var(--text-xs)', fontFamily: 'var(--mono)', color: 'var(--text-dim)', lineHeight: 1.4 }}>
                    {freqLabel(band.frequency)}
                  </div>
                  <div style={{ fontSize: 'var(--text-xs)', color: 'var(--text-dim)', lineHeight: 1.4 }}>
                    {band.filter_type.substring(0, 4)}
                  </div>
                </div>
              ))}
            </div>

            {isPeq && eqState.bands.length === 0 && (
              <div className="empty-state" style={{ padding: '24px 16px' }}>
                <p style={{ fontSize: 'var(--text-sm)' }}>{t('eq.noBands')}</p>
              </div>
            )}
          </div>
        </>
      )}
    </div>
  )
}