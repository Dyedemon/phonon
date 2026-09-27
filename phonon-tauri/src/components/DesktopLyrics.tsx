import { useState, useEffect, useRef, Component, useCallback } from 'react'
import type { ReactNode } from 'react'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { convertFileSrc } from '@tauri-apps/api/core'

// Ensure path uses forward slashes for convertFileSrc on Windows
const toFileSrc = (path: string) => convertFileSrc(path.replace(/\\/g, '/'))

interface LyricWord {
  time_ms: number
  text: string
}

interface LyricLine {
  time_ms: number
  text: string
  words: LyricWord[]
}

interface LyricsUpdatePayload {
  lines: LyricLine[]
  position_secs: number
  track_title: string
}

interface AppEventPayload {
  type: string
  position_secs?: number
  duration_secs?: number | null
  state?: string
}

interface LyricsResponse {
  type: string
  lines?: LyricLine[]
}

interface PlaybackStateResponse {
  current_track: string | null
  position_secs: number
  state: string
}

interface DesktopLyricsSettings {
  font_size: number
  opacity: number
  text_color: string
  locked: boolean
  bg_opacity: number
  always_on_top: boolean
  played_color: string
  unplayed_color: string
  bg_mode: string // "hidden" | "image" | "video"
  bg_path: string
  line_mode: string // "single" | "double"
  scroll_fx?: boolean
  equal_size?: boolean
  line_style?: string // "normal" | "centered" | "split"
}

// ============================================================
// i18n translations for desktop lyrics (standalone window)
// ============================================================
const DL_I18N: Record<string, Record<string, string>> = {
  zh: {
    'dl.tooltip.unlock': '解锁（将鼠标移到窗口顶部可点击）',
    'dl.tooltip.prev': '上一首',
    'dl.tooltip.play': '播放',
    'dl.tooltip.pause': '暂停',
    'dl.tooltip.next': '下一首',
    'dl.tooltip.pin': '置顶',
    'dl.tooltip.unpin': '取消置顶',
    'dl.tooltip.lock': '锁定',
    'dl.tooltip.close': '关闭',
    'dl.menu.pin': '置顶',
    'dl.menu.unpin': '取消置顶',
    'dl.menu.lock': '锁定',
    'dl.menu.unlock': '解锁',
    'dl.menu.singleLine': '单行显示',
    'dl.menu.doubleLine': '双行显示',
    'dl.menu.close': '关闭',
  },
  en: {
    'dl.tooltip.unlock': 'Unlock (move mouse to window top to click)',
    'dl.tooltip.prev': 'Previous',
    'dl.tooltip.play': 'Play',
    'dl.tooltip.pause': 'Pause',
    'dl.tooltip.next': 'Next',
    'dl.tooltip.pin': 'Pin on top',
    'dl.tooltip.unpin': 'Unpin',
    'dl.tooltip.lock': 'Lock',
    'dl.tooltip.close': 'Close',
    'dl.menu.pin': 'Pin on top',
    'dl.menu.unpin': 'Unpin',
    'dl.menu.lock': 'Lock',
    'dl.menu.unlock': 'Unlock',
    'dl.menu.singleLine': 'Single line',
    'dl.menu.doubleLine': 'Double line',
    'dl.menu.close': 'Close',
  },
}

const DEFAULT_SETTINGS: DesktopLyricsSettings = {
  font_size: 22,
  opacity: 1,
  text_color: '#ffffff',
  locked: false,
  bg_opacity: 0.45,
  always_on_top: true,
  played_color: '#ffffff',
  unplayed_color: '#888888',
  bg_mode: 'hidden',
  bg_path: '',
  line_mode: 'single',
  scroll_fx: true,
  equal_size: false,
  line_style: 'scroll',
}

class LyricsErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state: { error: Error | null } = { error: null }
  static getDerivedStateFromError(error: Error) { return { error } }
  componentDidCatch(error: Error, info: { componentStack: string }) {
    console.error('[DesktopLyrics] render crashed:', error, info)
  }
  render() {
    if (this.state.error) {
      return (
        <div style={{
          width: '100vw', height: '100vh',
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          background: 'rgba(0,0,0,0.7)', color: '#f88',
          fontFamily: 'monospace', fontSize: 'var(--text-sm)', padding: 16,
          textAlign: 'center', whiteSpace: 'pre-wrap',
        }} onDoubleClick={() => getCurrentWindow().close()}>
          DesktopLyrics crashed:
          {'\n'}
          {this.state.error.message}
          {'\n\n(double-click to close)'}
        </div>
      )
    }
    return this.props.children
  }
}

/// ScrollingText: when a line overflows the container, scrolls it smoothly
/// left-to-right, FOLLOWING the playback position (KTV style).
///
/// Key design — per user's KTV request:
///  1. Only scroll if the content OVERFLOWS the container.
///  2. Only scroll when `active = true` — i.e. the line is currently
///     BEING SUNG. Preview/next lines with active=false must NOT start
///     scrolling even if they overflow ("下一句还没播就自己滚动了").
///  3. Instead of a single uniform marquee that finishes in lineDuration,
///     we use rAF to compute the target translation every frame based on
///     `activeCharIndex` + `activeCharProgress` so it stays glued to the
///     character that's currently being sung.
///  4. We keep 30% of the container's left side as context (already-sung
///     characters still visible), matching common KTV behaviour.
function ScrollingText({
  children,
  active,
  reverse,
  activeCharIndex = 0,
  activeCharProgress = 0,
}: {
  children: React.ReactNode
  active: boolean
  reverse?: boolean
  activeCharIndex?: number
  activeCharProgress?: number
}) {
  const containerRef = useRef<HTMLDivElement>(null)
  const innerRef = useRef<HTMLSpanElement>(null)
  const offsetRef = useRef(0)
  const targetRef = useRef(0)
  const rafRef = useRef<number | null>(null)
  const lastTsRef = useRef<number>(0)
  const [, force] = useState(0)

  // Measure overflow once when children change. ResizeObserver on inner only.
  useEffect(() => {
    const measure = () => {
      // Trigger a render cycle so the rAF loop (below) sees current dims.
      force((x) => x + 1)
    }
    measure()
    const ro = new ResizeObserver(measure)
    if (innerRef.current) ro.observe(innerRef.current)
    return () => ro.disconnect()
  }, [children])

  // rAF loop: each frame, compute target offset from char DOM widths,
  // then smoothly interpolate current offset toward target using a
  // frame-delta normalized exponential glide (same feel at 60/120fps).
  useEffect(() => {
    lastTsRef.current = 0
    const tick = (ts: number) => {
      const c = containerRef.current
      const i = innerRef.current
      if (!c || !i) {
        rafRef.current = requestAnimationFrame(tick)
        return
      }
      // Frame-delta normalized smoothing so interpolation rate is
      // consistent regardless of refresh rate or dropped frames.
      const dt = lastTsRef.current ? Math.min(64, ts - lastTsRef.current) : 16
      lastTsRef.current = ts
      const halfLifeMs = 90
      const alpha = 1 - Math.exp(-dt / halfLifeMs)

      const overflow = i.scrollWidth - c.clientWidth
      // Never scroll if text fits or line is inactive.
      if (overflow <= 0 || !active) {
        targetRef.current = 0
      } else {
        // Walk the character spans (rendered by renderLyricLine with
        // className .dl-char) and accumulate widths up to activeCharIndex,
        // then add partial width of current active character based on
        // activeCharProgress.
        const spans: NodeListOf<HTMLSpanElement> = i.querySelectorAll('.dl-char')
        let playedWidth = 0
        const n = spans.length
        if (n > 0) {
          const clampedIdx = Math.min(n - 1, Math.max(0, activeCharIndex))
          for (let k = 0; k < clampedIdx; k++) {
            playedWidth += spans[k].offsetWidth
          }
          if (clampedIdx < n) {
            playedWidth += spans[clampedIdx].offsetWidth * activeCharProgress
          }
        } else {
          // No per-char spans — fallback: progress through the whole line.
          playedWidth = i.scrollWidth * (activeCharIndex + activeCharProgress)
        }
        // Keep 30% of the container as sung-character context on the left,
        // so the current character sits ~30% from the left edge.
        const context = c.clientWidth * 0.3
        let raw = playedWidth - context
        if (raw < 0) raw = 0
        if (raw > overflow) raw = overflow
        targetRef.current = raw
      }

      const cur = offsetRef.current
      const diff = targetRef.current - cur
      if (Math.abs(diff) > 0.05) {
        offsetRef.current = cur + diff * alpha
      } else {
        offsetRef.current = targetRef.current
      }
      if (i && i.style) {
        const x = reverse ? offsetRef.current : -offsetRef.current
        i.style.transform = `translate3d(${x}px, 0, 0)`
      }
      rafRef.current = requestAnimationFrame(tick)
    }
    rafRef.current = requestAnimationFrame(tick)
    return () => {
      if (rafRef.current !== null) cancelAnimationFrame(rafRef.current)
    }
  }, [active, reverse, activeCharIndex, activeCharProgress])

  return (
    <div
      ref={containerRef}
      style={{
        overflow: 'hidden',
        maxWidth: '100%',
        position: 'relative',
        flex: '1 1 auto',
        minWidth: 0,
        width: '100%',
        textAlign: 'center',
      }}
    >
      <span
        ref={innerRef}
        style={{
          display: 'inline-block',
          whiteSpace: 'nowrap',
          willChange: 'transform',
          // Inactive lines: clip with ellipsis so overflow shows "…"
          // in the inherited (unplayed) color.
          // Active lines: no max-width so translateX scroll works freely.
          maxWidth: active ? 'none' : '100%',
          overflow: active ? 'visible' : 'hidden',
          textOverflow: active ? 'clip' : 'ellipsis',
          verticalAlign: 'middle',
        }}
      >
        {children}
      </span>
    </div>
  )
}

function DesktopLyricsInner() {
  const [lines, setLines] = useState<LyricLine[]>([])
  const [positionSecs, setPositionSecs] = useState(0)
  const [trackTitle, setTrackTitle] = useState('')
  const [activeIndex, setActiveIndex] = useState(-1)
  const [bootError, setBootError] = useState<string>('')
  const [settings, setSettings] = useState<DesktopLyricsSettings>(DEFAULT_SETTINGS)
  const [language, setLanguage] = useState<string>('zh')
  // settingsLoaded gates UI rendering so the window doesn't briefly flash
  // the unlocked state (border + controls) before saved settings load.
  const [settingsLoaded, setSettingsLoaded] = useState(false)

  // Lightweight translation function for standalone window
  const t = useCallback((key: string) => {
    const dict = DL_I18N[language] || DL_I18N.zh
    return dict[key] || key
  }, [language])
  const [showContextMenu, setShowContextMenu] = useState(false)
  const [contextMenuPos, setContextMenuPos] = useState({ x: 0, y: 0 })
  const [hovering, setHovering] = useState(false)
  const [isPlaying, setIsPlaying] = useState(false)
  // Track window width so splitMargin recalculates on resize
  const [winWidth, setWinWidth] = useState(typeof window !== 'undefined' ? window.innerWidth : 800)
  // ── Scroll-mode animation state ──
  // ghostLines holds the previously-displayed (active, next) pair so it can
  // animate out (slide up & fade) while the new pair slides in from below.
  // lastScrollIdxRef tracks the index that is currently rendered to the
  // screen, so we know what to put in the ghost when activeIndex changes —
  // even during rapid switches (updated synchronously, not in a timeout).
  const [ghostLines, setGhostLines] = useState<{ active: LyricLine | null; next: LyricLine | null } | null>(null)
  const lastScrollIdxRef = useRef(-1)
  const ghostTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const linesRef = useRef<LyricLine[]>([])
  const clickTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const savePosTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const settingsRef = useRef(settings)
  settingsRef.current = settings

  // Track whether the component is unmounted. Tauri invoke callbacks
  // that fire after unmount are the #1 source of
  // "[TAURI] Couldn't find callback id" warnings during HMR / window close.
  const disposedRef = useRef(false)

  const updateRemoteSettings = useCallback((patch: Partial<DesktopLyricsSettings>) => {
    const next = { ...settingsRef.current, ...patch }
    setSettings(next)
    invoke('set_desktop_lyrics_settings', { settings: next }).catch(() => {})
    // Toggle click-through when locked state changes.
    // Defer enabling click-through so the current click event finishes
    // processing before the window starts ignoring mouse events.
    if (patch.locked !== undefined) {
      if (patch.locked) {
        setTimeout(() => {
          invoke('set_ignore_cursor_events', { ignore: true }).catch(() => {})
        }, 100)
      } else {
        invoke('set_ignore_cursor_events', { ignore: false }).catch(() => {})
      }
    }
  }, [])

  const saveWindowPosition = useCallback(() => {
    if (disposedRef.current) return
    const win = getCurrentWindow()
    // Use client-side dimensions (same-thread, no invoke) for width/height
    // instead of win.innerSize() — halves the number of pending Tauri
    // callbacks during resize, reducing "Couldn't find callback id" errors.
    const w = window.innerWidth
    const h = window.innerHeight
    win.outerPosition().then((pos) => {
      if (disposedRef.current) return
      invoke('save_desktop_lyrics_position', {
        x: pos.x,
        y: pos.y,
        width: w,
        height: h,
      }).catch(() => {})
    }).catch(() => {})
  }, [])

  const scheduleSavePosition = useCallback(() => {
    if (savePosTimerRef.current) clearTimeout(savePosTimerRef.current)
    // 800ms debounce: resize/move fires dozens of events per second;
    // only save once the user stops dragging.
    savePosTimerRef.current = setTimeout(() => {
      saveWindowPosition()
    }, 800)
  }, [saveWindowPosition])

  useEffect(() => {
    document.documentElement.style.background = 'transparent'
    document.body.style.background = 'transparent'
    document.body.style.overflow = 'hidden'
  }, [])

  useEffect(() => {
    // Delay initial invoke calls so Tauri's frontend callback registry
    // is fully ready — prevents "Couldn't find callback id" warnings.
    const timer = setTimeout(() => {
      // Load settings and language in parallel
      Promise.all([
        invoke<DesktopLyricsSettings>('get_desktop_lyrics_settings'),
        invoke<{ language: string }>('get_settings')
          .then(s => s.language)
          .catch(() => 'zh'),
      ])
        .then(([s, lang]) => {
          setSettings(s)
          setLanguage(lang || 'zh')
          setSettingsLoaded(true)
          if (s.locked) {
            invoke('set_ignore_cursor_events', { ignore: true }).catch(() => {})
          }
        })
        .catch(() => {
          // Even on failure, mark as loaded so UI can render
          setSettingsLoaded(true)
        })
    }, 50)
    return () => clearTimeout(timer)
  }, [])

  useEffect(() => {
    let cancelled = false
    const tryBoot = (attempt: number) => {
      if (cancelled) return
      // Add a small delay on first attempt to let the callback registry initialise
      const delay = attempt === 0 ? 150 : 300
      setTimeout(() => {
        if (cancelled) return
        invoke<PlaybackStateResponse>('get_playback_state').then((state) => {
          if (cancelled) return
          // Set initial playing state so the button icon is correct on load
          setIsPlaying(state.state === 'Playing')
          if (state.current_track) {
            setTrackTitle(state.current_track.replace(/^.*[\\/]/, '').replace(/\.[^/.]+$/, ''))
            setPositionSecs(state.position_secs)
            return invoke<LyricsResponse>('get_lyrics', { path: state.current_track })
          }
          return null
        }).then((data) => {
          if (cancelled) return
          if (data?.type === 'Synced' && data.lines && data.lines.length > 0) {
            linesRef.current = data.lines
            setLines(data.lines)
          } else {
            linesRef.current = []
            setLines([])
          }
        }).catch((e) => {
          if (cancelled) return
          if (attempt < 5) {
            tryBoot(attempt + 1)
          } else {
            console.error('[DesktopLyrics] boot failed:', e)
            setBootError(String(e?.message ?? e))
          }
        })
      }, delay)
    }
    tryBoot(0)
    return () => { cancelled = true }
  }, [])

  useEffect(() => {
    const unlisten = listen<AppEventPayload>('app-event', (event) => {
      const payload = event.payload
      if (payload.type === 'PlaybackProgress' && payload.position_secs != null) {
        setPositionSecs(payload.position_secs)
      }
      // PlaybackStateChanged comes through app-event, NOT a separate
      // "playback-state" event (the backend never emits that). Without
      // this, isPlaying stays false forever and the play/pause button
      // icon never switches to the pause icon.
      if (payload.type === 'PlaybackStateChanged' && payload.state) {
        setIsPlaying(payload.state === 'Playing')
      }
    })
    return () => { unlisten.then(fn => fn()).catch(() => {}) }
  }, [])

  useEffect(() => {
    const unlisten = listen<LyricsUpdatePayload>('desktop-lyrics-update', (event) => {
      const { lines: newLines, track_title } = event.payload
      if (newLines && newLines.length > 0) {
        linesRef.current = newLines
        setLines(newLines)
      } else {
        linesRef.current = []
        setLines([])
      }
      if (track_title) setTrackTitle(track_title)
    })
    return () => { unlisten.then(fn => fn()).catch(() => {}) }
  }, [])

  useEffect(() => {
    const unlisten = listen<DesktopLyricsSettings>('desktop-lyrics-settings-updated', (event) => {
      setSettings(event.payload)
    })
    return () => { unlisten.then(fn => fn()).catch(() => {}) }
  }, [])

  const handlePlayPause = () => { invoke('toggle_play_pause').catch(() => {}) }
  const handlePrev = () => { invoke('previous').catch(() => {}) }
  const handleNext = () => { invoke('next').catch(() => {}) }

  useEffect(() => {
    const posMs = positionSecs * 1000
    let idx = -1
    const currentLines = linesRef.current
    for (let i = 0; i < currentLines.length; i++) {
      if (currentLines[i].time_ms <= posMs) idx = i
      else break
    }
    setActiveIndex(idx)
  }, [positionSecs, lines])

  // ── Scroll-mode ghost animation ──
  // ONLY for scroll (滚动切换) mode. In centered/split modes the lines
  // bounce (ping-pong) and should NOT slide up/down — per user request:
  // "滚动切换的这个效果不应该在居中和左右分离里出现".
  // We read lineStyle from a ref so the effect doesn't need it as a dep
  // (avoids re-running when style changes but index doesn't).
  // The .current assignment happens later in the render, after lineStyle
  // is computed — see the assignment near the layout variables section.
  const lineStyleRef = useRef('scroll')
  useEffect(() => {
    // Skip ghost animation entirely for non-scroll modes
    if (lineStyleRef.current !== 'scroll') {
      lastScrollIdxRef.current = activeIndex
      setGhostLines(null)
      return
    }

    if (activeIndex === lastScrollIdxRef.current) return

    // Cancel any pending ghost-clear timer (rapid switch path)
    if (ghostTimerRef.current) {
      clearTimeout(ghostTimerRef.current)
      ghostTimerRef.current = null
    }

    const prevIdx = lastScrollIdxRef.current
    // Update synchronously so the next switch sees the new "old" index
    lastScrollIdxRef.current = activeIndex

    if (activeIndex < 0 || lines.length === 0) {
      setGhostLines(null)
      return
    }

    // Build the ghost from the previously displayed pair
    const oldActive = prevIdx >= 0 && prevIdx < lines.length ? lines[prevIdx] : null
    const oldNext = prevIdx + 1 >= 0 && prevIdx + 1 < lines.length ? lines[prevIdx + 1] : null
    const newActive = lines[activeIndex]

    // If the new active line is the same object as the old active, there's
    // nothing to animate (e.g. index went from -1 to 0 on first load).
    if (oldActive === newActive) {
      setGhostLines(null)
      return
    }

    setGhostLines({ active: oldActive, next: oldNext })
    ghostTimerRef.current = setTimeout(() => {
      setGhostLines(null)
      ghostTimerRef.current = null
    }, 450)
  }, [activeIndex, lines])

  useEffect(() => {
    if (disposedRef.current) return
    const handleResize = () => {
      if (disposedRef.current) return
      // Update winWidth so split layout margins recalculate on resize
      setWinWidth(window.innerWidth)
      scheduleSavePosition()
    }
    const win = getCurrentWindow()
    let unlistenResize: (() => void) | null = null
    let unlistenMoved: (() => void) | null = null
    win.onResized(() => handleResize()).then(fn => {
      if (disposedRef.current) { fn?.(); return }
      unlistenResize = fn
    }).catch(() => {})
    win.onMoved(() => handleResize()).then(fn => {
      if (disposedRef.current) { fn?.(); return }
      unlistenMoved = fn
    }).catch(() => {})
    return () => {
      if (unlistenResize) unlistenResize()
      if (unlistenMoved) unlistenMoved()
    }
  }, [scheduleSavePosition])

  // Cleanup all pending timers on unmount so they don't fire invoke calls
  // after the window is closed (which produces "Couldn't find callback id"
  // warnings in the console).
  useEffect(() => {
    return () => {
      disposedRef.current = true
      if (savePosTimerRef.current) {
        clearTimeout(savePosTimerRef.current)
        savePosTimerRef.current = null
      }
      if (ghostTimerRef.current) {
        clearTimeout(ghostTimerRef.current)
        ghostTimerRef.current = null
      }
      if (clickTimerRef.current) {
        clearTimeout(clickTimerRef.current)
        clickTimerRef.current = null
      }
    }
  }, [])

  const handleDragStart = (e: React.MouseEvent) => {
    if (e.button !== 0) return
    if (settings.locked) return
    e.preventDefault()
    const win = getCurrentWindow()
    win.startDragging().catch(() => {})
  }

  const handleClick = () => {
    if (settings.locked) return
    if (clickTimerRef.current) {
      clearTimeout(clickTimerRef.current)
      clickTimerRef.current = null
      return
    }
    clickTimerRef.current = setTimeout(() => {
      clickTimerRef.current = null
    }, 250)
  }

  const handleDoubleClick = () => {
    if (settings.locked) return
    if (clickTimerRef.current) {
      clearTimeout(clickTimerRef.current)
      clickTimerRef.current = null
    }
    saveWindowPosition()
    getCurrentWindow().close()
  }

  const handleContextMenu = (e: React.MouseEvent) => {
    e.preventDefault()
    setContextMenuPos({ x: e.clientX, y: e.clientY })
    setShowContextMenu(true)
  }

  useEffect(() => {
    if (!showContextMenu) return
    const handler = () => setShowContextMenu(false)
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [showContextMenu])

  const activeLine = activeIndex >= 0 ? lines[activeIndex] : null
  const nextLine = activeIndex >= 0 && activeIndex + 1 < lines.length ? lines[activeIndex + 1] : null
  const lineHeight = settings.font_size * 1.4
  const gap = settings.line_mode === 'double' ? 6 : 0
  // 'normal' was removed from the settings UI; treat it as 'scroll'.
  // Also default to 'scroll' when unset.
  const rawStyle = settings.line_style || 'scroll'
  const lineStyle = rawStyle === 'normal' ? 'scroll' : rawStyle
  // Sync ref for the ghost-animation effect (which runs before this point
  // in the render cycle but reads the ref at effect-execution time).
  lineStyleRef.current = lineStyle
  const isScroll = lineStyle === 'scroll'
  const isSplit = lineStyle === 'split'
  const isPong = lineStyle === 'centered' || lineStyle === 'split'
  // Split layout offsets: active line shifts left, next line shifts right.
  // We use padding (not translateX) to narrow the available width so the
  // text stays within bounds. ScrollingText handles overflow via scroll
  // (active line) or ellipsis (inactive line).
  // Adaptive: wider window → wider separation. Each side uses 26% of the
  // window width, floored at the minimum so narrow windows still look OK.
  const minSplit = 100
  const scaledSplit = Math.floor(winWidth * 0.26)
  const splitOffset = Math.max(minSplit, scaledSplit)
  const rowPx = lineHeight + gap
  // Current play-head position in ms — drives the per-character progress.
  const posMs = positionSecs * 1000

  // ── Ping-pong mode for centered/split ──
  // Top shows even-indexed lines, bottom shows odd-indexed lines.
  // Highlight bounces between top and bottom — no position swapping.
  // When activeIndex is even: top=activeIndex(active), bottom=activeIndex+1
  // When activeIndex is odd:  top=activeIndex+1, bottom=activeIndex(active)
  const pongEven = activeIndex % 2 === 0
  const pongTopIndex = pongEven ? activeIndex : activeIndex + 1
  const pongBottomIndex = pongEven ? activeIndex + 1 : activeIndex
  const pongTopLine = pongTopIndex >= 0 && pongTopIndex < lines.length ? lines[pongTopIndex] : null
  const pongBottomLine = pongBottomIndex >= 0 && pongBottomIndex < lines.length ? lines[pongBottomIndex] : null

  // ── Helpers for "KTV-style progressive character fill" ──
  //
  // Per user request: "桌面歌词高亮我要的是一个字也会慢慢高亮（就是一个字的高亮过程）".
  //
  // When the LRC does NOT ship with native per-word timestamps (the
  // overwhelming majority of LRC files in the wild), we synthesize
  // per-character word entries with uniform timing across the line's
  // lifetime. Each character then renders with a background-clip text
  // gradient: played_color fills from left to right as `posMs` crosses
  // the character's tiny window. This produces a smooth, classic
  // KTV-style "each character gradually lights up" effect.
  //
  // For files that DO have native per-word timestamps we simply respect
  // them — no splitting needed.
  const makeCharLevelWords = (line: LyricLine, lineEndMs: number): { time_ms: number; text: string }[] => {
    if (line.words && line.words.length >= 2) {
      let hasReal = false
      for (let i = 1; i < line.words.length; i++) {
        if (line.words[i].time_ms > line.words[i - 1].time_ms) { hasReal = true; break }
      }
      if (hasReal) return line.words
    }
    const chars = Array.from(line.text || '')
    if (chars.length === 0) return []
    const start = line.time_ms
    const dur = Math.max(1, lineEndMs - start)
    return chars.map((ch, i) => ({
      time_ms: start + Math.floor((dur * i) / chars.length),
      text: ch,
    }))
  }

  const wordProgress = (wStartMs: number, wEndMs: number): number => {
    if (posMs < wStartMs) return 0
    if (posMs >= wEndMs) return 1
    const denom = Math.max(1, wEndMs - wStartMs)
    return Math.max(0, Math.min(1, (posMs - wStartMs) / denom))
  }

  // ── Compute currently-playing char index + fractional progress
  // Used to drive ScrollingText's KTV-style viewport-follows-playback.
  const computeActiveCharInfo = (line: LyricLine | null, lineEndMs?: number): {
    index: number; progress: number
  } => {
    if (!line) return { index: 0, progress: 0 }
    const endMs: number = lineEndMs ?? (line.time_ms + 5000)
    const words = makeCharLevelWords(line, endMs)
    if (words.length === 0) return { index: 0, progress: 0 }
    for (let j = 0; j < words.length; j++) {
      const wStart = words[j].time_ms
      const wEnd = j + 1 < words.length ? words[j + 1].time_ms : endMs
      const p = wordProgress(wStart, wEnd)
      if (p < 1) return { index: j, progress: p }
    }
    return { index: words.length - 1, progress: 1 }
  }

  const renderLyricLine = (line: LyricLine | null, isActive: boolean, lineEndMs?: number): React.ReactNode => {
    if (!line) {
      return <span style={{ opacity: isActive ? 0.3 : 0.2 }}>Phonon</span>
    }

    // Inactive / preview lines render as a single solid-color span — no
    // need for per-character work (they'll never animate anyway since
    // isActive=false skips the progress tracking in ScrollingText too).
    if (!isActive) {
      return (
        <span style={{
          color: settings.unplayed_color,
          opacity: 0.7,
        }}>{line.text}</span>
      )
    }

    const endMs: number = lineEndMs ?? (line.time_ms + 5000)
    const words = makeCharLevelWords(line, endMs)
    const played = settings.played_color
    const unplayed = settings.unplayed_color

    if (words.length === 0) {
      return (
        <span style={{ color: played }}>{line.text}</span>
      )
    }

    const spans: React.ReactNode[] = []
    for (let j = 0; j < words.length; j++) {
      const w = words[j]
      const wStart = w.time_ms
      const wEnd = j + 1 < words.length ? words[j + 1].time_ms : endMs
      const p = wordProgress(wStart, wEnd)
      // Per user request: "当前那一行的字都会亮，然后红色会慢慢过去"
      // — the entire active line is bright (opacity 1, NOT dimmed),
      // and the played_color sweeps across from left→right as a gradient
      // per character. This makes the whole line visible while still
      // showing playback progress via the color sweep.
      const pct = Math.round(p * 100)
      spans.push(
        <span key={j} className="dl-char" data-idx={j} style={{
          display: 'inline-block',
          backgroundImage: `linear-gradient(to right, ${played} ${pct}%, ${unplayed} ${pct}%)`,
          WebkitBackgroundClip: 'text',
          backgroundClip: 'text',
          color: 'transparent',
          WebkitTextFillColor: 'transparent',
          opacity: 1,
        }}>{w.text}</span>
      )
    }
    return <>{spans}</>
  }

  // ── Per-line on-screen window hints (for ScrollingText duration) ──
  //
  // Per user bug report: "上面一行滚动在快速的情况下跟不上" — when a
  // lyric line's total lifetime is short (e.g. rapid-fire rap), the old
  // ScrollingText would happily schedule a 5s+ marquee that never
  // finishes before the line is gone.
  //
  // We pre-compute line lifetimes here so ScrollingText can clamp its
  // animation duration to 90% of the line's visible window.
  const activeEndMs: number | undefined = (() => {
    if (!activeLine) return undefined
    if (activeIndex + 1 < lines.length) return lines[activeIndex + 1].time_ms
    return activeLine.time_ms + 5000
  })()

  const nextEndMs: number | undefined = (() => {
    if (!nextLine) return undefined
    if (activeIndex + 2 < lines.length) return lines[activeIndex + 2].time_ms
    return nextLine.time_ms + 5000
  })()

  const pongTopEndMs: number | undefined = (() => {
    if (!pongTopLine) return undefined
    if (pongTopIndex + 1 < lines.length) return lines[pongTopIndex + 1].time_ms
    if (pongBottomLine) return pongBottomLine.time_ms
    return pongTopLine.time_ms + 5000
  })()

  const pongBottomEndMs: number | undefined = (() => {
    if (!pongBottomLine) return undefined
    if (pongBottomIndex + 1 < lines.length) return lines[pongBottomIndex + 1].time_ms
    return pongBottomLine.time_ms + 5000
  })()

  // ── Per-line active-char index + fractional progress (for KTV scroll)
  const activeCharInfo = computeActiveCharInfo(activeLine, activeEndMs)
  const pongTopCharInfo = computeActiveCharInfo(pongTopLine, pongTopEndMs)
  const pongBottomCharInfo = computeActiveCharInfo(pongBottomLine, pongBottomEndMs)
  // pong 模式下: 高亮行 = pongEven ? top : bottom，只有高亮行才应该滚动
  const pongTopIsActive = isPong && pongEven
  const pongBottomIsActive = isPong && !pongEven

  const getBackgroundStyle = (): React.CSSProperties => {
    const base: React.CSSProperties = {
      position: 'absolute',
      top: 0, left: 0, right: 0, bottom: 0,
      zIndex: 0,
    }
    if (settings.bg_mode === 'hidden') {
      return { ...base, background: 'transparent' }
    }
    if (settings.bg_mode === 'image' && settings.bg_path) {
      return {
        ...base,
        backgroundImage: `url(${toFileSrc(settings.bg_path)})`,
        // Stretch to fill the entire window frame (user can resize window to adjust)
        backgroundSize: 'cover',
        backgroundRepeat: 'no-repeat',
        backgroundPosition: 'center',
        opacity: settings.bg_opacity,
      }
    }
    return { ...base, background: 'transparent' }
  }

  if (bootError) {
    return (
      <div onDoubleClick={handleDoubleClick} style={{
        width: '100vw', height: '100vh',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        background: 'rgba(0,0,0,0.7)', color: '#f88',
        fontFamily: 'monospace', fontSize: 'var(--text-xs)', padding: 16,
        textAlign: 'center', whiteSpace: 'pre-wrap',
      }}>
        boot error: {bootError}
        {'\n(double-click to close)'}
      </div>
    )
  }

  const controlBtnStyle: React.CSSProperties = {
    width: 36,
    height: 36,
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: 8,
    background: 'transparent',
    color: '#fff',
    cursor: 'pointer',
    userSelect: 'none',
    border: 'none',
    padding: 0,
  }

  // Show border only on hover (when not locked)
  const showBorder = hovering && !settings.locked

  // Before settings load, render transparent empty window to avoid
  // flashing the unlocked border/controls state.
  if (!settingsLoaded) {
    return <div style={{ width: '100vw', height: '100vh', background: 'transparent' }} />
  }

  return (
    <div
      onMouseDown={handleDragStart}
      onClick={handleClick}
      onDoubleClick={handleDoubleClick}
      onContextMenu={handleContextMenu}
      onMouseEnter={() => setHovering(true)}
      onMouseLeave={() => setHovering(false)}
      style={{
        width: '100vw', height: '100vh',
        display: 'flex', flexDirection: 'column',
        alignItems: 'center', justifyContent: 'center',
        background: 'transparent',
        fontFamily: 'var(--lyrics)',
        userSelect: 'none',
        overflow: 'hidden',
        opacity: settings.opacity,
        cursor: settings.locked ? 'default' : 'move',
        position: 'relative',
        // Use outline instead of borderRadius + boxShadow on transparent
        // Tauri windows — borderRadius/boxShadow can cause edge rendering
        // artifacts (visible "黑色阴影边框" or flickering) in WebView2.
        // outline draws outside the element box and doesn't trigger
        // compositing issues.
        outline: showBorder ? '1.5px solid rgba(255,255,255,0.5)' : 'none',
        outlineOffset: '-1.5px',
      }}
    >
      {/* Background layer */}
      {settings.bg_mode !== 'hidden' && (
        settings.bg_mode === 'video' && settings.bg_path ? (
          <video
            key={settings.bg_path}
            src={toFileSrc(settings.bg_path)}
            autoPlay
            loop
            muted
            playsInline
            style={{
              position: 'absolute',
              top: 0, left: 0,
              width: '100%', height: '100%',
              // Stretch to fill the entire window frame
              objectFit: 'cover',
              opacity: settings.bg_opacity,
              zIndex: 0,
            }}
          />
        ) : (
          <div style={getBackgroundStyle()} />
        )
      )}

      {/* Top bar: track title + controls (hidden when locked, except lock button) */}
      <div
        style={{
          position: 'absolute',
          top: 0, left: 0, right: 0,
          display: 'flex',
          alignItems: 'center',
          padding: '6px 10px',
          background: 'transparent',
          zIndex: 10,
          justifyContent: settings.locked ? 'flex-end' : undefined,
          opacity: settings.locked ? 0.3 : (hovering ? 1 : 0.4),
          transition: 'opacity 0.2s ease',
        }}
        onMouseDown={(e) => {
          const target = e.target as HTMLElement
          if (target.closest('button')) {
            e.stopPropagation()
          }
        }}
        onDoubleClick={(e) => e.stopPropagation()}
      >
        {settings.locked ? (
          <button
            style={{
              ...controlBtnStyle,
              width: 28, height: 28, fontSize: 'var(--text-base)',
              pointerEvents: 'auto',
              padding: 4,
            }}
            title={t('dl.tooltip.unlock')}
            onClick={() => updateRemoteSettings({ locked: false })}
          >
            🔒
          </button>
        ) : (
          <>
            <div style={{
              flex: '1 1 0',
              fontSize: Math.max(10, settings.font_size * 0.4),
              color: '#fff',
              overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
              textAlign: 'left',
            }}>
              {trackTitle || 'Phonon'}
            </div>

            <div style={{ display: 'flex', gap: 6, alignItems: 'center', flex: '0 0 auto', justifyContent: 'center' }}>
              <button style={{ ...controlBtnStyle }} onClick={handlePrev} title={t('dl.tooltip.prev')}>
                <svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor"><path d="M6 6h2v12H6zm3.5 6l8.5 6V6z"/></svg>
              </button>
              <button style={{ ...controlBtnStyle }} onClick={handlePlayPause} title={isPlaying ? t('dl.tooltip.pause') : t('dl.tooltip.play')}>
                {isPlaying
                  ? <svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor"><path d="M6 19h4V5H6v14zm8-14v14h4V5h-4z"/></svg>
                  : <svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor"><path d="M8 5v14l11-7z"/></svg>
                }
              </button>
              <button style={{ ...controlBtnStyle }} onClick={handleNext} title={t('dl.tooltip.next')}>
                <svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor"><path d="M6 18l8.5-6L6 6v12zM16 6v12h2V6h-2z"/></svg>
              </button>
            </div>

            <div style={{ display: 'flex', gap: 6, flex: '1 1 0', justifyContent: 'flex-end' }}>
              <button
                style={{
                  ...controlBtnStyle,
                  color: settings.always_on_top ? '#06b6d4' : 'rgba(255,255,255,0.5)',
                  transform: settings.always_on_top ? 'none' : 'rotate(45deg)',
                }}
                title={settings.always_on_top ? t('dl.tooltip.unpin') : t('dl.tooltip.pin')}
                onClick={() => updateRemoteSettings({ always_on_top: !settings.always_on_top })}
              >
                <svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor">
                  <path d="M16 9V4h1c.55 0 1-.45 1-1s-.45-1-1-1H7c-.55 0-1 .45-1 1s.45 1 1 1h1v5c0 1.66-1.34 3-3 3v2h5.97v7l1 1 1-1v-7H19v-2c-1.66 0-3-1.34-3-3z"/>
                </svg>
              </button>
              <button
                style={{ ...controlBtnStyle, color: 'rgba(239,68,68,0.8)' }}
                title={t('dl.tooltip.lock')}
                onClick={() => updateRemoteSettings({ locked: true })}
              >
                <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor"><path d="M18 8h-1V6c0-2.76-2.24-5-5-5S7 3.24 7 6v2H6c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V10c0-1.1-.9-2-2-2zm-6 9c-1.1 0-2-.9-2-2s.9-2 2-2 2 .9 2 2-.9 2-2 2zm3.1-9H8.9V6c0-1.71 1.39-3.1 3.1-3.1s3.1 1.39 3.1 3.1v2z"/></svg>
              </button>
              <button
                style={controlBtnStyle}
                title={t('dl.tooltip.close')}
                onClick={() => { saveWindowPosition(); getCurrentWindow().close() }}
              >
                <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor"><path d="M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"/></svg>
              </button>
            </div>
          </>
        )}
      </div>

      {/* Main lyrics display — no text shadow, KTV-style follow-scroll */}
      <div style={{
        fontSize: settings.font_size,
        textAlign: 'center',
        maxWidth: '100%',
        width: '100%',
        boxSizing: 'border-box',
        padding: '8px 16px',
        zIndex: 2,
        position: 'relative',
        color: settings.played_color,
        textShadow: 'none',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap,
        // overflow:hidden so long lyrics or split-shifted content never
        // spills outside the lyrics window. ScrollingText handles its own
        // internal overflow clipping for horizontal scroll.
        overflow: 'hidden',
      }}>
        {settings.line_mode === 'double' ? (
          // ── Double-line mode ──
          // Three sub-modes based on line_style:
          //  - scroll (滚动切换): top=active (big), bottom=next (small, dim).
          //  - centered (居中): ping-pong, both same size, centered.
          //  - split (左右分离): ping-pong, both same size, shifted L/R.
          <div
            style={{
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'center',
              justifyContent: 'center',
              gap,
              width: '100%',
              position: 'relative',
              transform: ghostLines ? 'translateY(0)' : 'translateY(0)',
              animation: ghostLines ? 'dl-slide-in 450ms ease both' : 'none',
            }}
          >
            {/* Ghost (outgoing) pair — slides UP + fades while the new pair slides in */}
            {ghostLines && (
              <div
                aria-hidden
                style={{
                  position: 'absolute',
                  left: 0, right: 0,
                  top: 0,
                  display: 'flex',
                  flexDirection: 'column',
                  alignItems: 'center',
                  justifyContent: 'center',
                  gap,
                  width: '100%',
                  pointerEvents: 'none',
                  animation: 'dl-slide-out 450ms ease both',
                  opacity: 0.6,
                }}
              >
                {/* ghost top — old active (rendered with same size/style as new active) */}
                <div style={{
                  fontSize: isScroll ? settings.font_size : settings.font_size,
                  lineHeight: 1.4,
                  maxWidth: '100%',
                  minHeight: lineHeight,
                  textAlign: 'center',
                  color: settings.unplayed_color,
                  paddingRight: isSplit ? `${splitOffset}px` : 0,
                  boxSizing: 'border-box',
                }}>
                  <ScrollingText
                    active={false}
                    activeCharIndex={0}
                    activeCharProgress={1}
                  >
                    {renderLyricLine(ghostLines.active, false)}
                  </ScrollingText>
                </div>
                {/* ghost bottom — old next */}
                <div style={{
                  fontSize: isScroll ? settings.font_size * 0.75 : settings.font_size,
                  lineHeight: 1.4,
                  maxWidth: '100%',
                  minHeight: lineHeight,
                  textAlign: 'center',
                  color: settings.unplayed_color,
                  paddingLeft: isSplit ? `${splitOffset}px` : 0,
                  boxSizing: 'border-box',
                  opacity: ghostLines.next ? 0.4 : 0.1,
                }}>
                  <ScrollingText active={false}>
                    {ghostLines.next ? renderLyricLine(ghostLines.next, false) : '\u00A0'}
                  </ScrollingText>
                </div>
              </div>
            )}

            {isScroll ? (
              // Scroll mode: active = big, next = small+dimmed
              <>
                <div style={{
                  fontSize: settings.font_size,
                  lineHeight: 1.4,
                  maxWidth: '100%',
                  width: '100%',
                  minHeight: lineHeight,
                  textAlign: 'center',
                  color: settings.played_color,
                }}>
                  <ScrollingText
                    active={!!activeLine}
                    activeCharIndex={activeCharInfo.index}
                    activeCharProgress={activeCharInfo.progress}
                  >
                    {renderLyricLine(activeLine, true, activeEndMs)}
                  </ScrollingText>
                </div>
                <div style={{
                  fontSize: settings.font_size * 0.75,
                  lineHeight: 1.4,
                  maxWidth: '100%',
                  width: '100%',
                  minHeight: lineHeight,
                  textAlign: 'center',
                  color: settings.unplayed_color,
                  opacity: nextLine ? 0.5 : 0.15,
                }}>
                  {/* preview line must NOT start scrolling: active=false */}
                  <ScrollingText active={false}>
                    {nextLine ? renderLyricLine(nextLine, false, nextEndMs) : '\u00A0'}
                  </ScrollingText>
                </div>
              </>
            ) : (
              // Ping-pong mode: both same size, highlight bounces
              <>
                {/* Top line (even) — in split mode, shift left via padding */}
                <div style={{
                  fontSize: settings.font_size,
                  lineHeight: 1.4,
                  maxWidth: '100%',
                  width: '100%',
                  minHeight: lineHeight,
                  textAlign: 'center',
                  color: pongTopIsActive ? settings.played_color : settings.unplayed_color,
                  paddingRight: isSplit ? `${splitOffset}px` : 0,
                  boxSizing: 'border-box',
                  opacity: pongTopLine ? 1 : 0.15,
                  transition: 'padding 0.3s ease, opacity 0.3s ease',
                }}>
                  <ScrollingText
                    active={pongTopIsActive}
                    activeCharIndex={pongTopCharInfo.index}
                    activeCharProgress={pongTopCharInfo.progress}
                  >
                    {pongTopLine ? renderLyricLine(pongTopLine, pongEven, pongTopEndMs) : '\u00A0'}
                  </ScrollingText>
                </div>
                {/* Bottom line (odd) — in split mode, shift right via padding */}
                <div style={{
                  fontSize: settings.font_size,
                  lineHeight: 1.4,
                  maxWidth: '100%',
                  width: '100%',
                  minHeight: lineHeight,
                  textAlign: 'center',
                  color: pongBottomIsActive ? settings.played_color : settings.unplayed_color,
                  paddingLeft: isSplit ? `${splitOffset}px` : 0,
                  boxSizing: 'border-box',
                  opacity: pongBottomLine ? 1 : 0.15,
                  transition: 'padding 0.3s ease, opacity 0.3s ease',
                }}>
                  <ScrollingText
                    active={pongBottomIsActive}
                    activeCharIndex={pongBottomCharInfo.index}
                    activeCharProgress={pongBottomCharInfo.progress}
                  >
                    {pongBottomLine ? renderLyricLine(pongBottomLine, !pongEven, pongBottomEndMs) : '\u00A0'}
                  </ScrollingText>
                </div>
              </>
            )}
          </div>
        ) : (
          // ── Single-line mode ──
          <div style={{
            width: '100%',
            position: 'relative',
            animation: ghostLines ? 'dl-slide-in 450ms ease both' : 'none',
          }}>
            {ghostLines && (
              <div
                aria-hidden
                style={{
                  position: 'absolute',
                  left: 0, right: 0, top: 0,
                  width: '100%',
                  pointerEvents: 'none',
                  animation: 'dl-slide-out 450ms ease both',
                  opacity: 0.6,
                  color: settings.unplayed_color,
                }}
              >
                <ScrollingText active={false}>
                  {renderLyricLine(ghostLines.active, false)}
                </ScrollingText>
              </div>
            )}
            <ScrollingText
              active={!!activeLine}
              activeCharIndex={activeCharInfo.index}
              activeCharProgress={activeCharInfo.progress}
            >
              {renderLyricLine(activeLine, true, activeEndMs)}
            </ScrollingText>
          </div>
        )}

        {/* Slide keyframes — shared between single & double mode */}
        <style>{`
          @keyframes dl-slide-out {
            0%   { transform: translateY(0); opacity: 0.65; }
            100% { transform: translateY(-${rowPx}px); opacity: 0; }
          }
          @keyframes dl-slide-in {
            0%   { transform: translateY(${rowPx}px); opacity: 0; }
            100% { transform: translateY(0); opacity: 1; }
          }
        `}</style>
      </div>

      {showContextMenu && (
        <div style={{
          position: 'fixed',
          left: contextMenuPos.x, top: contextMenuPos.y,
          background: 'rgba(20,20,30,0.95)',
          border: '1px solid rgba(255,255,255,0.15)',
          borderRadius: 6, padding: 4, zIndex: 10000, minWidth: 140,
          fontSize: 'var(--text-sm)', color: '#fff', fontFamily: 'sans-serif',
        }} onMouseDown={(e) => e.stopPropagation()}>
          <div style={{ padding: '6px 12px', cursor: 'pointer', borderRadius: 4 }}
            onMouseEnter={(e) => (e.currentTarget.style.background = 'rgba(255,255,255,0.1)')}
            onMouseLeave={(e) => (e.currentTarget.style.background = 'transparent')}
            onClick={() => { updateRemoteSettings({ always_on_top: !settings.always_on_top }); setShowContextMenu(false) }}>
            {settings.always_on_top ? t('dl.menu.unpin') : t('dl.menu.pin')}
          </div>
          <div style={{ padding: '6px 12px', cursor: 'pointer', borderRadius: 4 }}
            onMouseEnter={(e) => (e.currentTarget.style.background = 'rgba(255,255,255,0.1)')}
            onMouseLeave={(e) => (e.currentTarget.style.background = 'transparent')}
            onClick={() => { updateRemoteSettings({ locked: !settings.locked }); setShowContextMenu(false) }}>
            {settings.locked ? t('dl.menu.unlock') : t('dl.menu.lock')}
          </div>
          <div style={{ padding: '6px 12px', cursor: 'pointer', borderRadius: 4 }}
            onMouseEnter={(e) => (e.currentTarget.style.background = 'rgba(255,255,255,0.1)')}
            onMouseLeave={(e) => (e.currentTarget.style.background = 'transparent')}
            onClick={() => { updateRemoteSettings({ line_mode: settings.line_mode === 'single' ? 'double' : 'single' }); setShowContextMenu(false) }}>
            {settings.line_mode === 'single' ? t('dl.menu.doubleLine') : t('dl.menu.singleLine')}
          </div>
          <div style={{ height: 1, background: 'rgba(255,255,255,0.1)', margin: '2px 0' }} />
          <div style={{ padding: '6px 12px', cursor: 'pointer', borderRadius: 4, color: '#f88' }}
            onMouseEnter={(e) => (e.currentTarget.style.background = 'rgba(255,255,255,0.1)')}
            onMouseLeave={(e) => (e.currentTarget.style.background = 'transparent')}
            onClick={() => { saveWindowPosition(); getCurrentWindow().close(); setShowContextMenu(false) }}>
            {t('dl.menu.close')}
          </div>
        </div>
      )}
    </div>
  )
}

export default function DesktopLyrics() {
  return (
    <LyricsErrorBoundary>
      <DesktopLyricsInner />
    </LyricsErrorBoundary>
  )
}
