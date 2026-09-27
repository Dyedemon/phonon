import { useState, useEffect, useRef, useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useI18n } from '../i18n'
import { useProgress } from '../api/progressBus'

interface LyricWord {
  time_ms: number
  text: string
}

interface LyricLine {
  time_ms: number
  text: string
  words: LyricWord[]
}

type LyricData =
  | { type: 'None' }
  | { type: 'Unsynced'; text: string }
  | { type: 'Synced'; lines: LyricLine[] }

interface Props {
  trackPath: string | null
  durationSecs: number | null
  playbackState: string
  coverUrl: string | null
  onClose: () => void
  onPlayPause: () => void
  onPrevious: () => void
  onNext: () => void
  onSeek: (positionSecs: number) => void
}

const AUTO_SYNC_DELAY_MS = 3000

export default function LyricsPage({
  trackPath, durationSecs, playbackState,
  coverUrl, onClose, onPlayPause, onPrevious, onNext, onSeek,
}: Props) {
  const { t } = useI18n()
  // Position streams through the progress bus — self-subscribe instead of
  // receiving it through App state at ~20 Hz.
  const positionSecs = useProgress().position
  const [lyricData, setLyricData] = useState<LyricData>({ type: 'None' })
  const [activeIndex, setActiveIndex] = useState(-1)
  const containerRef = useRef<HTMLDivElement>(null)
  const userScrollingRef = useRef(false)
  const scrollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const lineRefs = useRef<Map<number, HTMLDivElement>>(new Map())

  // ── Seek bar state ──────────────────────────────────────
  const [seekPct, setSeekPct] = useState(0)
  const [dragging, setDragging] = useState(false)
  const draggingRef = useRef(false)
  const seekPctRef = useRef(0)
  const [tooltipTime, setTooltipTime] = useState('')
  const [controlCollapsed, setControlCollapsed] = useState(false)

  // Esc key to close
  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', handleKey)
    return () => window.removeEventListener('keydown', handleKey)
  }, [onClose])

  const formatTime = (secs: number) => {
    const m = Math.floor(secs / 60)
    const s = Math.floor(secs % 60)
    return `${m}:${s.toString().padStart(2, '0')}`
  }

  const handleSeekStart = (e: React.MouseEvent<HTMLDivElement>) => {
    if (!durationSecs) return
    const rect = e.currentTarget.getBoundingClientRect()
    if (rect.width <= 0) return

    const calcPct = (clientX: number) =>
      Math.max(0, Math.min(100, ((clientX - rect.left) / rect.width) * 100))

    draggingRef.current = true
    const pct = calcPct(e.clientX)
    seekPctRef.current = pct
    setSeekPct(pct)
    setDragging(true)
    setTooltipTime(formatTime((pct / 100) * durationSecs))

    const onMove = (ev: MouseEvent) => {
      const p = calcPct(ev.clientX)
      seekPctRef.current = p
      setSeekPct(p)
      setTooltipTime(formatTime((p / 100) * (durationSecs ?? 0)))
    }
    const onUp = () => {
      draggingRef.current = false
      setDragging(false)
      setTooltipTime('')
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
      if (durationSecs) {
        onSeek((seekPctRef.current / 100) * durationSecs)
      }
    }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
  }

  // Load lyrics when track changes
  useEffect(() => {
    setLyricData({ type: 'None' })
    setActiveIndex(-1)
    lineRefs.current.clear()

    if (!trackPath) return
    invoke<LyricData>('get_lyrics', { path: trackPath })
      .then((data) => setLyricData(data))
      .catch(() => setLyricData({ type: 'None' }))
  }, [trackPath])

  // Handle user scroll
  const handleUserScroll = useCallback(() => {
    userScrollingRef.current = true
    if (scrollTimerRef.current) clearTimeout(scrollTimerRef.current)
    scrollTimerRef.current = setTimeout(() => {
      userScrollingRef.current = false
    }, AUTO_SYNC_DELAY_MS)
  }, [])

  useEffect(() => {
    return () => {
      if (scrollTimerRef.current) clearTimeout(scrollTimerRef.current)
    }
  }, [])

  // Track active line — syncs with playback or seek drag
  useEffect(() => {
    if (lyricData.type !== 'Synced') return
    const lines = lyricData.lines
    if (lines.length === 0) return

    const posMs = dragging
      ? (seekPctRef.current / 100) * (durationSecs ?? 0) * 1000
      : positionSecs * 1000
    let idx = -1
    for (let i = 0; i < lines.length; i++) {
      if (lines[i].time_ms <= posMs) {
        idx = i
      } else {
        break
      }
    }
    setActiveIndex(idx)

    // During drag, always sync scroll; otherwise respect user scroll state
    if (idx >= 0 && (dragging || !userScrollingRef.current)) {
      const lineEl = lineRefs.current.get(idx)
      const container = containerRef.current
      if (lineEl && container) {
        const containerRect = container.getBoundingClientRect()
        const lineRect = lineEl.getBoundingClientRect()
        const relativeTop = lineRect.top - containerRect.top + container.scrollTop
        const targetScroll = relativeTop - container.clientHeight / 2 + lineEl.offsetHeight / 2
        container.scrollTo({ top: Math.max(0, targetScroll), behavior: dragging ? 'auto' : 'smooth' })
      }
    }
  }, [positionSecs, lyricData, dragging, durationSecs])

  const setLineRef = (i: number) => (el: HTMLDivElement | null) => {
    if (el) lineRefs.current.set(i, el)
    else lineRefs.current.delete(i)
  }

  const isPlaying = playbackState === 'Playing'

  return (
    <div
      className="lyrics-page"
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 1000,
        background: '#000',
        display: 'flex',
        flexDirection: 'column',
        color: '#fff',
        fontFamily: 'var(--lyrics)',
        userSelect: 'none',
      }}
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose()
      }}
    >
      {/* Blurred album art background */}
      {coverUrl && (
        <div style={{ position: 'absolute', inset: 0, zIndex: 0, overflow: 'hidden' }}>
          <img
            src={coverUrl} alt=""
            style={{
              width: '100%', height: '100%', objectFit: 'cover',
              filter: 'blur(60px) brightness(0.3) saturate(1.5)',
              transform: 'scale(1.1)',
            }}
          />
        </div>
      )}

      {/* Overlay gradient */}
      <div style={{
        position: 'absolute', inset: 0, zIndex: 1,
        background: coverUrl
          ? 'linear-gradient(180deg, rgba(0,0,0,0.4) 0%, rgba(0,0,0,0.6) 50%, rgba(0,0,0,0.85) 100%)'
          : 'rgba(0,0,0,0.92)',
      }} />

      {/* Header */}
      <div style={{
        display: 'flex', alignItems: 'center', padding: '16px 24px',
        gap: 16, flexShrink: 0, position: 'relative', zIndex: 2,
      }}>
        <button
          onClick={onClose}
          style={{
            background: 'rgba(255,255,255,0.1)', border: 'none',
            borderRadius: '50%', width: 36, height: 36,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            cursor: 'pointer', color: 'rgba(255,255,255,0.8)', fontSize: 'var(--text-xl)',
            transition: 'background 0.2s',
          }}
          onMouseEnter={(e) => (e.currentTarget.style.background = 'rgba(255,255,255,0.2)')}
          onMouseLeave={(e) => (e.currentTarget.style.background = 'rgba(255,255,255,0.1)')}
        >
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M19 12H5M12 19l-7-7 7-7" />
          </svg>
        </button>
        <span style={{ fontSize: 'var(--text-sm)', opacity: 0.5, fontWeight: 500 }}>
          {lyricData.type === 'Synced' ? t('lyrics.synced') : lyricData.type === 'Unsynced' ? t('lyrics.unsynced') : ''}
        </span>
        {trackPath && (
          <button
            onClick={async (e) => {
              e.stopPropagation()
              try {
                const meta = await invoke<{ artist?: string; title?: string }>('get_track_metadata', { path: trackPath })
                if (meta.artist && meta.title) {
                  const data = await invoke<LyricData>('search_lyrics', { artist: meta.artist, title: meta.title })
                  setLyricData(data)
                }
              } catch { /* ignore */ }
            }}
            style={{
              marginLeft: 'auto', background: 'rgba(255,255,255,0.1)',
              border: '1px solid rgba(255,255,255,0.15)', borderRadius: 8,
              padding: '6px 14px', color: 'rgba(255,255,255,0.7)',
              fontSize: 'var(--text-sm)', cursor: 'pointer', transition: 'background 0.2s',
            }}
            onMouseEnter={(e) => (e.currentTarget.style.background = 'rgba(255,255,255,0.2)')}
            onMouseLeave={(e) => (e.currentTarget.style.background = 'rgba(255,255,255,0.1)')}
            title={t('lyrics.searchOnline')}
          >
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={{ marginRight: 4 }}>
              <circle cx="11" cy="11" r="8"/><path d="M21 21l-4.35-4.35"/>
            </svg>
            {t('lyrics.searchOnline')}
          </button>
        )}
      </div>

      {/* Lyrics content */}
      <div
        ref={containerRef}
        onWheel={handleUserScroll}
        onScroll={handleUserScroll}
        onTouchMove={handleUserScroll}
        style={{
          flex: 1, overflowY: 'auto', overflowX: 'hidden',
          padding: '20vh 24px 100px',
          scrollBehavior: 'smooth', position: 'relative', zIndex: 2,
        }}
      >
        {lyricData.type === 'None' && (
          <div style={{
            display: 'flex', flexDirection: 'column', alignItems: 'center',
            justifyContent: 'center', height: '40vh', opacity: 0.4, gap: 12,
          }}>
            <svg viewBox="0 0 48 48" width="48" height="48" fill="none" stroke="currentColor" strokeWidth="1.5">
              <path d="M12 36V12l24-4v28" />
              <circle cx="10" cy="36" r="4" />
              <circle cx="34" cy="32" r="4" />
            </svg>
            <span style={{ fontSize: 'var(--text-base)' }}>{t('lyrics.noLyrics')}</span>
          </div>
        )}

        {lyricData.type === 'Unsynced' && (
          <div style={{
            whiteSpace: 'pre-wrap', fontSize: 22, lineHeight: 1.8,
            textAlign: 'center', opacity: 0.7, padding: '0 32px',
          }}>
            {lyricData.text}
          </div>
        )}

        {lyricData.type === 'Synced' && lyricData.lines.map((line, i) => {
          const isActive = i === activeIndex
          const words = line.words
          const hasWords = words && words.length > 0
          const posMs = positionSecs * 1000

          // Calculate gradient sweep progress for the active line
          const lineGradientStyle: React.CSSProperties = {}
          if (isActive && hasWords && words.length > 1) {
            const firstMs = words[0].time_ms
            const lastMs = words[words.length - 1].time_ms
            const lineDuration = lastMs - firstMs || 1
            const lineProgress = Math.max(0, Math.min(1, (posMs - firstMs) / lineDuration))
            const pct = lineProgress * 100
            // Smooth gradient with 8% transition zone
            const softStart = Math.max(0, pct - 8)
            const softEnd = Math.min(100, pct + 8)
            lineGradientStyle.backgroundImage = `linear-gradient(to right, var(--lyrics-accent, #fff) ${softStart}%, var(--lyrics-accent, #fff) ${pct}%, rgba(255,255,255,0.25) ${softEnd}%, rgba(255,255,255,0.25) 100%)`
            lineGradientStyle.backgroundClip = 'text'
            lineGradientStyle.WebkitBackgroundClip = 'text'
            lineGradientStyle.WebkitTextFillColor = 'transparent'
          }

          return (
            <div
              key={i}
              ref={setLineRef(i)}
              style={{
                textAlign: 'center', padding: '10px 32px',
                fontSize: isActive ? 28 : 20,
                fontWeight: isActive ? 700 : 400,
                lineHeight: 1.8,
                transition: 'all 0.4s cubic-bezier(0.4, 0, 0.2, 1)',
                transform: isActive ? 'scale(1.02)' : 'scale(1)',
                filter: isActive ? 'none' : 'blur(0.5px)',
                letterSpacing: isActive ? '0.02em' : '0',
              }}
            >
              {hasWords ? (
                isActive && words.length > 1 ? (
                  // Active line with words: gradient sweep across the whole line
                  <span style={lineGradientStyle}>
                    {words.map((w, j) => (
                      <span key={j}>{w.text}</span>
                    ))}
                  </span>
                ) : (
                  // Inactive line or single-word line: per-character coloring
                  words.map((w, j) => {
                    const highlighted = isActive && w.time_ms <= posMs
                    return (
                      <span
                        key={j}
                        style={{
                          color: highlighted ? '#fff' : 'rgba(255,255,255,0.25)',
                          fontWeight: highlighted ? 700 : 400,
                          transition: 'color 0.1s ease',
                        }}
                      >
                        {w.text}
                      </span>
                    )
                  })
                )
              ) : (
                <span style={{
                  color: isActive ? '#fff' : 'rgba(255,255,255,0.25)',
                  transition: 'color 0.15s',
                }}>
                  {line.text}
                </span>
              )}
            </div>
          )
        })}
      </div>

      {/* ── Bottom control bar ────────────────────────────── */}
      <div style={{
        position: 'relative', zIndex: 2, flexShrink: 0,
        background: 'rgba(0,0,0,0.7)',
        backdropFilter: 'blur(20px)',
        borderTop: '1px solid rgba(255,255,255,0.06)',
        padding: controlCollapsed ? '8px 24px' : '12px 24px 16px',
        transition: 'padding 0.3s',
      }}>
        {/* Collapse toggle */}
        <div style={{ display: 'flex', justifyContent: 'center', marginBottom: controlCollapsed ? 0 : 4 }}>
          <button
            onClick={() => setControlCollapsed(!controlCollapsed)}
            style={{
              background: 'none', border: 'none', cursor: 'pointer',
              color: 'rgba(255,255,255,0.3)', padding: '2px 12px',
              fontSize: 'var(--text-xs)', transition: 'color 0.2s',
            }}
            onMouseEnter={(e) => (e.currentTarget.style.color = 'rgba(255,255,255,0.6)')}
            onMouseLeave={(e) => (e.currentTarget.style.color = 'rgba(255,255,255,0.3)')}
          >
            {controlCollapsed ? '\u25B2' : '\u25BC'}
          </button>
        </div>

        {!controlCollapsed && (
          <>
            {/* Progress bar */}
            <div
              style={{
                position: 'relative', height: 4, marginBottom: 12,
                background: 'rgba(255,255,255,0.1)', borderRadius: 2,
                cursor: 'pointer',
              }}
              onMouseDown={handleSeekStart}
            >
              <div style={{
                height: '100%', borderRadius: 2,
                width: `${dragging ? Math.min(seekPct, 100) : durationSecs ? Math.min((positionSecs / durationSecs) * 100, 100) : 0}%`,
                background: 'var(--accent, #00ffc8)',
                transition: dragging ? 'none' : 'width 0.1s linear',
              }}>
                <div style={{
                  position: 'absolute', right: -5, top: -3,
                  width: 10, height: 10, borderRadius: '50%',
                  background: '#fff',
                  opacity: dragging ? 1 : 0,
                  transition: 'opacity 0.2s',
                }} />
              </div>
              {dragging && tooltipTime && (
                <div style={{
                  position: 'absolute', top: -28,
                  left: `${Math.min(seekPct, 100)}%`,
                  transform: 'translateX(-50%)',
                  background: 'rgba(0,0,0,0.8)', color: '#fff',
                  padding: '2px 8px', borderRadius: 4,
                  fontSize: 'var(--text-xs)', fontFamily: 'var(--mono)',
                  whiteSpace: 'nowrap',
                }}>
                  {tooltipTime}
                </div>
              )}
            </div>

            {/* Time + controls */}
            <div style={{
              display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 24,
            }}>
              <span style={{ fontSize: 'var(--text-xs)', opacity: 0.4, fontFamily: 'var(--mono)', minWidth: 32, textAlign: 'right' }}>
                {formatTime(dragging ? (seekPct / 100) * (durationSecs ?? 0) : positionSecs)}
              </span>

              <button onClick={onPrevious} style={ctrlBtnStyle} title={t('player.previous')}>
                <svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor"><path d="M6 6h2v12H6zm3.5 6l8.5 6V6z"/></svg>
              </button>

              <button
                onClick={onPlayPause}
                style={{
                  ...ctrlBtnStyle,
                  width: 44, height: 44, borderRadius: '50%',
                  background: 'rgba(255,255,255,0.15)',
                }}
                title={isPlaying ? t('player.pause') : t('player.play')}
              >
                {isPlaying ? (
                  <svg viewBox="0 0 24 24" width="20" height="20" fill="currentColor"><path d="M6 19h4V5H6v14zm8-14v14h4V5h-4z"/></svg>
                ) : (
                  <svg viewBox="0 0 24 24" width="20" height="20" fill="currentColor"><path d="M8 5v14l11-7z"/></svg>
                )}
              </button>

              <button onClick={onNext} style={ctrlBtnStyle} title={t('player.next')}>
                <svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor"><path d="M6 18l8.5-6L6 6v12zM16 6v12h2V6h-2z"/></svg>
              </button>

              <span style={{ fontSize: 'var(--text-xs)', opacity: 0.4, fontFamily: 'var(--mono)', minWidth: 32 }}>
                {durationSecs ? formatTime(durationSecs) : '--:--'}
              </span>
            </div>
          </>
        )}
      </div>
    </div>
  )
}

const ctrlBtnStyle: React.CSSProperties = {
  background: 'none', border: 'none',
  color: 'rgba(255,255,255,0.7)', cursor: 'pointer',
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  padding: 4, borderRadius: '50%',
  transition: 'color 0.2s, background 0.2s',
}