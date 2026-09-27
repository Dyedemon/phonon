import { useState, useEffect, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen, emit, emitTo } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

// ============================================================
// i18n translations for tray popup (standalone window)
// ============================================================
const TRAY_I18N: Record<string, Record<string, string>> = {
  zh: {
    'tray.prev': '上一首',
    'tray.play': '播放',
    'tray.pause': '暂停',
    'tray.next': '下一首',
    'tray.desktopLyrics.show': '显示桌面歌词',
    'tray.desktopLyrics.hide': '关闭桌面歌词',
    'tray.quit': '退出',
  },
  en: {
    'tray.prev': 'Previous',
    'tray.play': 'Play',
    'tray.pause': 'Pause',
    'tray.next': 'Next',
    'tray.desktopLyrics.show': 'Show Desktop Lyrics',
    'tray.desktopLyrics.hide': 'Hide Desktop Lyrics',
    'tray.quit': 'Quit',
  },
}

interface PlaybackState {
  state: string
  current_track: string | null
}

export default function TrayPopup() {
  const [isPlaying, setIsPlaying] = useState(false)
  const [trackName, setTrackName] = useState('Phonon')
  const [volume, setVolume] = useState(80)
  const [showVolTooltip, setShowVolTooltip] = useState(false)
  const [desktopLyricsOpen, setDesktopLyricsOpen] = useState(false)
  const [language, setLanguage] = useState<string>('zh')
  const volTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const winRef = useRef(getCurrentWindow())

  // Lightweight translation function for standalone window
  const t = (key: string) => {
    const dict = TRAY_I18N[language] || TRAY_I18N.zh
    return dict[key] || key
  }

  useEffect(() => {
    document.documentElement.style.background = 'transparent'
    document.documentElement.style.margin = '0'
    document.documentElement.style.padding = '0'
    document.documentElement.style.width = '100%'
    document.documentElement.style.height = '100%'
    document.body.style.background = 'transparent'
    document.body.style.margin = '0'
    document.body.style.padding = '0'
    document.body.style.width = '100%'
    document.body.style.height = '100%'
    document.body.style.overflow = 'hidden'

    const root = document.getElementById('root')
    if (root) {
      root.style.width = '100%'
      root.style.height = '100%'
      root.style.margin = '0'
      root.style.padding = '0'
    }

    invoke<PlaybackState>('get_playback_state').then((s) => {
      setIsPlaying(s.state === 'Playing')
      if (s.current_track) {
        setTrackName(s.current_track.replace(/^.*[\\/]/, '').replace(/\.[^/.]+$/, ''))
      }
    }).catch(() => {})

    // Load language setting
    invoke<{ language: string }>('get_settings')
      .then(s => setLanguage(s.language || 'zh'))
      .catch(() => {})

    const unlisten = listen<{ state: string }>('playback-state', (event) => {
      setIsPlaying(event.payload.state === 'Playing')
    })

    const unlistenApp = listen<{ type: string; state?: string }>('app-event', (event) => {
      if (event.payload.type === 'PlaybackStateChanged' && event.payload.state) {
        setIsPlaying(event.payload.state === 'Playing')
      }
    })

    invoke<number>('get_volume_level').then((l) => {
      setVolume(Math.round(l * 100))
    }).catch(() => {})

    const unlistenVol = listen<number>('hw-volume', (event) => {
      setVolume(Math.round(event.payload * 100))
    })

    // Close on focus loss — covers clicking desktop, taskbar, other apps
    const unlistenFocus = winRef.current.onFocusChanged(({ payload: focused }) => {
      if (!focused) {
        winRef.current.hide().catch(() => {})
      }
    })

    // Track desktop lyrics visibility to toggle menu text
    const unlistenDlVis = listen<boolean>('desktop-lyrics-visibility', (event) => {
      setDesktopLyricsOpen(event.payload)
    })

    return () => {
      unlisten.then((fn) => fn()).catch(() => {})
      unlistenApp.then((fn) => fn()).catch(() => {})
      unlistenVol.then((fn) => fn()).catch(() => {})
      unlistenFocus.then((fn) => fn()).catch(() => {})
      unlistenDlVis.then((fn) => fn()).catch(() => {})
      if (volTimerRef.current) {
        clearTimeout(volTimerRef.current)
        volTimerRef.current = null
      }
    }
  }, [])

  const handlePlayPause = () => invoke('toggle_play_pause').catch(() => {})
  const handlePrev = () => invoke('previous').catch(() => {})
  const handleNext = () => invoke('next').catch(() => {})

  const handleVolumeChange = (v: number) => {
    setVolume(v)
    invoke('set_volume', { level: v / 100 }).catch(() => {})
    setShowVolTooltip(true)
    if (volTimerRef.current) clearTimeout(volTimerRef.current)
    volTimerRef.current = setTimeout(() => setShowVolTooltip(false), 800)
  }

  const toggleDesktopLyrics = () => {
    winRef.current.hide().catch(() => {})
    emitTo('main', 'toggle-desktop-lyrics', {}).catch(() => {})
  }
  const handleQuit = () => invoke('quit_app').catch(() => {})
  const handleShowMain = () => {
    winRef.current.hide().catch(() => {})
    emit('show-main-window', {}).catch(() => {})
  }

  const btnStyle: React.CSSProperties = {
    background: 'transparent',
    border: 'none',
    color: '#fff',
    cursor: 'pointer',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    padding: 0,
    transition: 'opacity 0.15s',
  }

  return (
    <div
      style={{
        width: '100%',
        height: '100%',
        background: 'rgba(28,28,38,0.96)',
        borderRadius: 10,
        border: '1px solid rgba(255,255,255,0.08)',
        color: '#fff',
        userSelect: 'none',
        fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 4,
        boxSizing: 'border-box',
        padding: '8px 0',
      }}
      onMouseDown={(e) => {
        const target = e.target as HTMLElement
        if (!target.closest('button') && !target.closest('input') && !target.closest('.clickable')) {
          winRef.current.startDragging().catch(() => {})
        }
      }}
    >
      {/* Row 1: Track name */}
      <div
        className="clickable"
        style={{
          fontSize: 'var(--text-base)',
          fontWeight: 500,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
          opacity: 0.9,
          cursor: 'pointer',
          textAlign: 'center',
          width: '92%',
        }}
        onClick={handleShowMain}
        title={trackName}
      >
        {trackName}
      </div>

      {/* Row 2: Playback controls */}
      <div style={{ display: 'flex', gap: 10, justifyContent: 'center', alignItems: 'center' }}>
        <button style={{ ...btnStyle, width: 32, height: 32 }} onClick={handlePrev} title={t('tray.prev')}>
          <svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor"><path d="M6 6h2v12H6zm3.5 6l8.5 6V6z"/></svg>
        </button>
        <button style={{ ...btnStyle, width: 38, height: 38, color: '#06b6d4' }} onClick={handlePlayPause} title={isPlaying ? t('tray.pause') : t('tray.play')}>
          {isPlaying ? (
            <svg viewBox="0 0 24 24" width="22" height="22" fill="currentColor"><path d="M6 19h4V5H6v14zm8-14v14h4V5h-4z"/></svg>
          ) : (
            <svg viewBox="0 0 24 24" width="22" height="22" fill="currentColor"><path d="M8 5v14l11-7z"/></svg>
          )}
        </button>
        <button style={{ ...btnStyle, width: 32, height: 32 }} onClick={handleNext} title={t('tray.next')}>
          <svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor"><path d="M6 18l8.5-6L6 6v12zM16 6v12h2V6h-2z"/></svg>
        </button>
      </div>

      {/* Row 3: Volume slider — icon at left, slider fills remaining width.
          Tooltip sits directly above the slider thumb and uses a transparent
          background so it doesn't cover other rows visually. */}
      <div style={{ position: 'relative', width: '70%', display: 'flex', alignItems: 'center', gap: 4 }}>
        <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor" opacity={0.6} style={{ flexShrink: 0 }}>
          <path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02z"/>
        </svg>
        <div style={{ position: 'relative', flex: 1, display: 'flex', alignItems: 'center' }}>
          <input
            type="range"
            min={0}
            max={100}
            value={volume}
            onChange={(e) => handleVolumeChange(Number(e.target.value))}
            style={{
              width: '100%',
              cursor: 'pointer',
              height: 3,
              appearance: 'none',
              WebkitAppearance: 'none',
              background: `linear-gradient(to right, rgba(6,182,212,0.6) ${volume}%, rgba(255,255,255,0.1) ${volume}%)`,
              borderRadius: 2,
              outline: 'none',
            }}
          />
          {showVolTooltip && (
            <div
              style={{
                position: 'absolute',
                // Center tooltip directly above the slider thumb (vertical alignment).
                // Slider thumb is ~14px, so we place the tooltip well above it.
                left: `${(volume / 100) * 100}%`,
                top: -20,
                transform: 'translateX(-50%)',
                background: 'transparent',
                color: 'rgba(255,255,255,0.9)',
                fontSize: 'var(--text-xs)',
                padding: '0 2px',
                borderRadius: 3,
                pointerEvents: 'none',
                whiteSpace: 'nowrap',
                textShadow: '0 1px 2px rgba(0,0,0,0.8)',
              }}
            >
              {volume}%
            </div>
          )}
        </div>
      </div>

      {/* Row 4: 桌面歌词 */}
      <div
        className="clickable"
        style={{
          fontSize: 'var(--text-base)',
          color: 'rgba(255,255,255,0.75)',
          cursor: 'pointer',
          padding: '2px 0',
          transition: 'color 0.15s',
          width: '92%',
          textAlign: 'center',
        }}
        onClick={toggleDesktopLyrics}
        onMouseEnter={(e) => (e.currentTarget as HTMLElement).style.color = '#fff'}
        onMouseLeave={(e) => (e.currentTarget as HTMLElement).style.color = 'rgba(255,255,255,0.75)'}
      >
        {desktopLyricsOpen ? t('tray.desktopLyrics.hide') : t('tray.desktopLyrics.show')}
      </div>

      {/* Row 5: 退出 */}
      <div
        className="clickable"
        style={{
          fontSize: 'var(--text-base)',
          color: 'rgba(239,68,68,0.75)',
          cursor: 'pointer',
          padding: '2px 0',
          transition: 'color 0.15s',
          width: '92%',
          textAlign: 'center',
        }}
        onClick={handleQuit}
        onMouseEnter={(e) => (e.currentTarget as HTMLElement).style.color = 'rgba(239,68,68,1)'}
        onMouseLeave={(e) => (e.currentTarget as HTMLElement).style.color = 'rgba(239,68,68,0.75)'}
      >
        {t('tray.quit')}
      </div>
    </div>
  )
}
