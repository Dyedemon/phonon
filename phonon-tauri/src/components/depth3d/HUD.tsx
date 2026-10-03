// HUD 组件 — 沉浸式界面控制
// 包含：底部居中播放栏、左上角返回按钮、右上角菜单按钮、右侧滑入控制面板
// 交互逻辑：鼠标移动 → 四角按钮浮现；hover 底部 → 播放栏浮出；静止 2s → 淡出
import { useState, useEffect, useRef, useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useProgress } from '../../api/progressBus'

// ========= SVG 图标 =========
const IconPrev = () => (
  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <polygon points="19 20 9 12 19 4 19 20" />
    <line x1="5" y1="19" x2="5" y2="5" />
  </svg>
)

const IconPlay = () => (
  <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
    <polygon points="6 4 20 12 6 20 6 4" />
  </svg>
)

const IconPause = () => (
  <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
    <rect x="6" y="4" width="4" height="16" rx="1" />
    <rect x="14" y="4" width="4" height="16" rx="1" />
  </svg>
)

const IconNext = () => (
  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <polygon points="5 4 15 12 5 20 5 4" />
    <line x1="19" y1="5" x2="19" y2="19" />
  </svg>
)

const IconBack = () => (
  <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <line x1="19" y1="12" x2="5" y2="12" />
    <polyline points="12 19 5 12 12 5" />
  </svg>
)

const IconMenu = () => (
  <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
    <line x1="3" y1="6" x2="21" y2="6" />
    <line x1="3" y1="12" x2="21" y2="12" />
    <line x1="3" y1="18" x2="21" y2="18" />
  </svg>
)

const IconClose = () => (
  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round">
    <line x1="18" y1="6" x2="6" y2="18" />
    <line x1="6" y1="6" x2="18" y2="18" />
  </svg>
)

const IconMusic = () => (
  <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
    <path d="M9 18V5l12-2v13" />
    <circle cx="6" cy="18" r="3" />
    <circle cx="18" cy="16" r="3" />
  </svg>
)

const IconSparkles = () => (
  <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
    <path d="M12 3l1.9 5.8a2 2 0 0 0 1.3 1.3L21 12l-5.8 1.9a2 2 0 0 0-1.3 1.3L12 21l-1.9-5.8a2 2 0 0 0-1.3-1.3L3 12l5.8-1.9a2 2 0 0 0 1.3-1.3L12 3z" />
  </svg>
)

const IconSpeaker = () => (
  <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
    <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
    <path d="M19.07 4.93a10 10 0 0 1 0 14.14M15.54 8.46a5 5 0 0 1 0 7.07" />
  </svg>
)

const IconList = () => (
  <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
    <line x1="8" y1="6" x2="21" y2="6" />
    <line x1="8" y1="12" x2="21" y2="12" />
    <line x1="8" y1="18" x2="21" y2="18" />
    <line x1="3" y1="6" x2="3.01" y2="6" />
    <line x1="3" y1="12" x2="3.01" y2="12" />
    <line x1="3" y1="18" x2="3.01" y2="18" />
  </svg>
)

const IconSettings = () => (
  <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
    <circle cx="12" cy="12" r="3" />
    <path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" />
  </svg>
)

const IconExit = () => (
  <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
    <path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4" />
    <polyline points="16 17 21 12 16 7" />
    <line x1="21" y1="12" x2="9" y2="12" />
  </svg>
)

// ========= 底部居中播放栏 =========
interface PlayBarProps {
  visible: boolean
  onHoverChange?: (hovered: boolean) => void
  playback?: {
    state: string
    position_secs: number
    duration_secs: number | null
    current_track: string | null
  }
  volume: number
  onVolumeChange?: (v: number) => void
}

export function BottomPlayBar({ visible, onHoverChange, playback, volume, onVolumeChange }: PlayBarProps) {
  const liveProgress = useProgress()
  const pos = liveProgress.position
  const dur = liveProgress.duration ?? playback?.duration_secs ?? null
  const progress = dur ? Math.max(0, Math.min(1, pos / dur)) : 0
  const localVol = volume

  const fmt = (s: number | null | undefined) => {
    if (s == null) return '00:00'
    const m = Math.floor(s / 60)
    const sec = Math.floor(s % 60)
    return `${String(m).padStart(2, '0')}:${String(sec).padStart(2, '0')}`
  }

  const currentTrack = playback?.current_track
    ? playback.current_track.replace(/^.*[\\/]/, '').replace(/\.[^.]+$/, '')
    : '未在播放'

  const isPlaying = playback?.state === 'Playing'

  const togglePlay = () => {
    invoke('toggle_play_pause').catch(() => {})
  }
  const nextTrack = () => {
    invoke('next').catch(() => {})
  }
  const prevTrack = () => {
    invoke('previous').catch(() => {})
  }
  const setVol = (v: number) => {
    onVolumeChange?.(v)
  }

  // 进度条：点击 + 拖动（拖动时只更新视觉，mouseup 才真正 seek，避免轰炸后端）
  const progressRef = useRef<HTMLDivElement>(null)
  const draggingProgressRef = useRef(false)
  const dragProgressRef = useRef(0)
  const progressCleanupRef = useRef<(() => void) | null>(null)
  const [, forceProgressUpdate] = useState(0)

  const seekToClientX = useCallback((clientX: number, commit = false) => {
    if (!dur || !progressRef.current) return
    const rect = progressRef.current.getBoundingClientRect()
    const ratio = Math.max(0, Math.min(1, (clientX - rect.left) / rect.width))
    dragProgressRef.current = ratio
    if (commit) {
      invoke('seek', { position_secs: ratio * dur }).catch(() => {})
    }
  }, [dur])

  useEffect(() => {
    return () => { progressCleanupRef.current?.() }
  }, [])

  const handleProgressMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault()
    e.stopPropagation()
    draggingProgressRef.current = true
    seekToClientX(e.clientX, false)
    forceProgressUpdate((n) => n + 1)

    const onMove = (ev: MouseEvent) => {
      if (!draggingProgressRef.current) return
      seekToClientX(ev.clientX, false)
      forceProgressUpdate((n) => n + 1)
    }
    const onUp = (ev: MouseEvent) => {
      if (!draggingProgressRef.current) return
      draggingProgressRef.current = false
      seekToClientX(ev.clientX, true)
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
      progressCleanupRef.current = null
      forceProgressUpdate((n) => n + 1)
    }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
    progressCleanupRef.current = () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
    }
  }, [seekToClientX])

  // 显示的进度：拖动时显示拖动位置，否则显示真实播放进度
  const displayProgress = draggingProgressRef.current ? dragProgressRef.current : progress

  // 自定义滑块拖拽（音量）— 用 ref 管理，避免重渲染导致事件反复挂载
  const volRef = useRef<HTMLDivElement>(null)
  const draggingVolRef = useRef(false)
  const volCleanupRef = useRef<(() => void) | null>(null)
  const [, forceVolUpdate] = useState(0)

  const updateVolFromEvent = useCallback((clientX: number) => {
    if (!volRef.current) return
    const rect = volRef.current.getBoundingClientRect()
    const ratio = Math.max(0, Math.min(1, (clientX - rect.left) / rect.width))
    setVol(Math.round(ratio * 100))
  }, [setVol])

  useEffect(() => {
    return () => { volCleanupRef.current?.() }
  }, [])

  const handleVolMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault()
    e.stopPropagation()
    draggingVolRef.current = true
    updateVolFromEvent(e.clientX)
    forceVolUpdate((n) => n + 1)
    document.body.style.cursor = 'grabbing'
    document.body.style.userSelect = 'none'

    const onMove = (ev: MouseEvent) => {
      if (!draggingVolRef.current) return
      updateVolFromEvent(ev.clientX)
    }
    const onUp = () => {
      draggingVolRef.current = false
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
      volCleanupRef.current = null
      forceVolUpdate((n) => n + 1)
    }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
    volCleanupRef.current = () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
    }
  }, [updateVolFromEvent])

  return (
    <div
      onMouseEnter={() => onHoverChange?.(true)}
      onMouseLeave={() => onHoverChange?.(false)}
      style={{
        position: 'absolute',
        left: '50%',
        bottom: visible ? 24 : -70,
        transform: 'translateX(-50%)',
        width: '50%',
        maxWidth: 600,
        minWidth: 360,
        opacity: visible ? 1 : 0,
        pointerEvents: visible ? 'auto' : 'none',
        transition: 'opacity 0.35s cubic-bezier(0.22, 1, 0.36, 1), bottom 0.35s cubic-bezier(0.22, 1, 0.36, 1)',
        zIndex: 15,
      }}
    >
      <div style={{
        height: 56,
        borderRadius: 16,
        background: 'rgba(8, 10, 24, 0.75)',
        backdropFilter: 'blur(20px) saturate(180%)',
        border: '1px solid rgba(120, 160, 255, 0.18)',
        boxShadow: '0 8px 40px rgba(0, 0, 0, 0.5), 0 0 30px rgba(96, 165, 250, 0.08), inset 0 1px 0 rgba(255, 255, 255, 0.06)',
        display: 'flex',
        alignItems: 'center',
        padding: '0 18px',
        gap: 14,
        overflow: 'hidden',
        position: 'relative',
      }}>
        {/* 顶部微光带 */}
        <div style={{
          position: 'absolute',
          top: 0,
          left: '20%',
          right: '20%',
          height: 1,
          background: 'linear-gradient(90deg, transparent, rgba(120, 180, 255, 0.5), transparent)',
        }} />

        {/* 左侧：歌曲名 */}
        <div style={{
          flex: 1,
          minWidth: 0,
          overflow: 'hidden',
          whiteSpace: 'nowrap',
          textOverflow: 'ellipsis',
          fontSize: 12,
          color: 'rgba(220, 230, 255, 0.85)',
          fontWeight: 500,
          letterSpacing: 0.3,
        }}>
          {currentTrack}
        </div>

        {/* 中间：控制按钮 */}
        <div style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
        }}>
          <button onClick={prevTrack} style={iconBtnStyle}>
            <IconPrev />
          </button>
          <button
            onClick={togglePlay}
            style={{
              width: 38,
              height: 38,
              borderRadius: '50%',
              border: 'none',
              background: 'radial-gradient(circle at 30% 30%, rgba(140, 180, 255, 0.9), rgba(100, 140, 255, 0.7) 50%, rgba(80, 100, 200, 0.5) 100%)',
              color: '#fff',
              cursor: 'pointer',
              display: 'inline-flex',
              alignItems: 'center',
              justifyContent: 'center',
              boxShadow: '0 0 16px rgba(100, 150, 255, 0.5), 0 2px 8px rgba(0, 0, 0, 0.3), inset 0 1px 0 rgba(255, 255, 255, 0.3)',
              transition: 'all 0.2s ease',
              padding: 0,
            }}
            onMouseEnter={(e) => {
              e.currentTarget.style.transform = 'scale(1.1)'
              e.currentTarget.style.boxShadow = '0 0 24px rgba(100, 150, 255, 0.7), 0 4px 12px rgba(0, 0, 0, 0.35), inset 0 1px 0 rgba(255, 255, 255, 0.3)'
            }}
            onMouseLeave={(e) => {
              e.currentTarget.style.transform = 'scale(1)'
              e.currentTarget.style.boxShadow = '0 0 16px rgba(100, 150, 255, 0.5), 0 2px 8px rgba(0, 0, 0, 0.3), inset 0 1px 0 rgba(255, 255, 255, 0.3)'
            }}
          >
            {isPlaying ? <IconPause /> : <IconPlay />}
          </button>
          <button onClick={nextTrack} style={iconBtnStyle}>
            <IconNext />
          </button>
        </div>

        {/* 进度条 */}
        <div style={{
          flex: 3,
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          minWidth: 0,
        }}>
          <span style={{
            fontSize: 10,
            color: 'rgba(180, 200, 240, 0.5)',
            fontVariantNumeric: 'tabular-nums',
            minWidth: 32,
            textAlign: 'right',
            flexShrink: 0,
          }}>{fmt(draggingProgressRef.current ? dragProgressRef.current * (dur || 0) : pos)}</span>
          <div
            ref={progressRef}
            onMouseDown={handleProgressMouseDown}
            style={{
              flex: 1,
              height: 3,
              borderRadius: 1.5,
              background: 'rgba(80, 110, 180, 0.18)',
              cursor: 'pointer',
              position: 'relative',
              minWidth: 0,
            }}
          >
            <div style={{
              position: 'absolute',
              left: 0,
              top: 0,
              height: '100%',
              width: `${displayProgress * 100}%`,
              borderRadius: 1.5,
              background: 'linear-gradient(90deg, #7cb0ff, #c8a0ff)',
              boxShadow: '0 0 8px rgba(124, 176, 255, 0.6)',
              pointerEvents: 'none',
            }} />
            <div style={{
              position: 'absolute',
              left: `${displayProgress * 100}%`,
              top: '50%',
              transform: 'translate(-50%, -50%)',
              width: 7,
              height: 7,
              borderRadius: '50%',
              background: '#fff',
              boxShadow: '0 0 6px rgba(200, 220, 255, 0.8)',
              opacity: progress > 0 ? 1 : 0,
              pointerEvents: 'none',
            }} />
          </div>
          <span style={{
            fontSize: 10,
            color: 'rgba(180, 200, 240, 0.5)',
            fontVariantNumeric: 'tabular-nums',
            minWidth: 32,
            flexShrink: 0,
          }}>{fmt(dur)}</span>
        </div>

        {/* 右侧：音量（自定义滑块） */}
        <div style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          color: 'rgba(200, 220, 255, 0.7)',
          flexShrink: 0,
        }}>
          <IconSpeaker />
          <div
            ref={volRef}
            onMouseDown={handleVolMouseDown}
            style={{
              width: 70,
              height: 3,
              borderRadius: 1.5,
              background: 'rgba(80, 110, 180, 0.18)',
              position: 'relative',
              cursor: 'grab',
              userSelect: 'none',
            }}
          >
            <div style={{
              position: 'absolute',
              left: 0,
              top: 0,
              height: '100%',
              width: `${localVol}%`,
              borderRadius: 1.5,
              background: 'linear-gradient(90deg, #7cb0ff, #c8a0ff)',
              boxShadow: '0 0 6px rgba(124, 176, 255, 0.5)',
              pointerEvents: 'none',
            }} />
            <div style={{
              position: 'absolute',
              left: `${localVol}%`,
              top: '50%',
              transform: 'translate(-50%, -50%)',
              width: 7,
              height: 7,
              borderRadius: '50%',
              background: '#fff',
              boxShadow: '0 0 6px rgba(200, 220, 255, 0.7)',
              pointerEvents: 'none',
            }} />
          </div>
          <span style={{
            fontSize: 10,
            color: 'rgba(180, 200, 240, 0.5)',
            minWidth: 26,
            fontVariantNumeric: 'tabular-nums',
          }}>
            {Math.round(localVol)}
          </span>
        </div>
      </div>
    </div>
  )
}

const iconBtnStyle: React.CSSProperties = {
  width: 32,
  height: 32,
  borderRadius: '50%',
  border: '1px solid rgba(120, 160, 255, 0.15)',
  background: 'rgba(255, 255, 255, 0.04)',
  color: 'rgba(220, 230, 255, 0.75)',
  cursor: 'pointer',
  display: 'inline-flex',
  alignItems: 'center',
  justifyContent: 'center',
  padding: 0,
  transition: 'all 0.2s ease',
}

// ========= 角落按钮 =========
interface CornerButtonProps {
  position: 'top-left' | 'top-right'
  visible: boolean
  onClick: () => void
  icon: React.ReactNode
  highlight?: boolean
}

export function CornerButton({ position, visible, onClick, icon, highlight }: CornerButtonProps) {
  const isLeft = position === 'top-left'

  return (
    <button
      onClick={onClick}
      style={{
        position: 'absolute',
        top: 22,
        [isLeft ? 'left' : 'right']: 26,
        width: 40,
        height: 40,
        borderRadius: '50%',
        border: '1px solid rgba(120, 160, 255, 0.2)',
        background: 'rgba(10, 12, 28, 0.55)',
        backdropFilter: 'blur(14px)',
        color: 'rgba(220, 230, 255, 0.8)',
        cursor: 'pointer',
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        opacity: visible ? (highlight ? 1 : 0.65) : 0,
        pointerEvents: visible ? 'auto' : 'none',
        transition: 'all 0.3s cubic-bezier(0.22, 1, 0.36, 1)',
        zIndex: 20,
        boxShadow: highlight
          ? '0 4px 20px rgba(100, 150, 255, 0.3), inset 0 0 12px rgba(120, 180, 255, 0.15)'
          : '0 4px 16px rgba(0, 0, 0, 0.3)',
      }}
      onMouseEnter={(e) => {
        e.currentTarget.style.opacity = '1'
        e.currentTarget.style.transform = 'scale(1.12)'
        e.currentTarget.style.boxShadow = '0 4px 24px rgba(100, 150, 255, 0.4), inset 0 0 16px rgba(120, 180, 255, 0.2)'
      }}
      onMouseLeave={(e) => {
        e.currentTarget.style.opacity = highlight ? '1' : '0.65'
        e.currentTarget.style.transform = 'scale(1)'
        e.currentTarget.style.boxShadow = highlight
          ? '0 4px 20px rgba(100, 150, 255, 0.3), inset 0 0 12px rgba(120, 180, 255, 0.15)'
          : '0 4px 16px rgba(0, 0, 0, 0.3)'
      }}
    >
      {icon}
    </button>
  )
}

// ========= 右侧控制面板 =========
interface RightPanelProps {
  open: boolean
  onClose: () => void
  onExit3D?: () => void
  playback?: {
    state: string
    position_secs: number
    duration_secs: number | null
    current_track: string | null
  }
  volume: number
  onVolumeChange?: (v: number) => void
  sceneSettings?: SceneSettings
  onSettingChange?: (key: string, value: any) => void
  /** 帧预算自动降载的当前档位（0=满血）；>0 时在设置页显示提示 */
  loadLevel?: number
}

type PanelTab = 'playback' | 'visual' | 'device' | 'queue' | 'settings'

// 数据类型
interface DeviceInfo {
  id: string
  name: string
  is_default: boolean
  sample_rates: number[]
  max_channels: number
  supports_exclusive: boolean
}

interface QueueItem {
  path: string
  title: string
  artist: string | null
  duration: number | null
}

interface DspInfo {
  id: string
  name: string
  enabled: boolean
  latency_ms: number
}

export function RightControlPanel({ open, onClose, onExit3D, playback, volume, onVolumeChange, sceneSettings, onSettingChange, loadLevel }: RightPanelProps) {
  const [tab, setTab] = useState<PanelTab>('playback')
  const [devices, setDevices] = useState<DeviceInfo[]>([])
  const [currentDeviceId, setCurrentDeviceId] = useState<string | null>(null)
  const [queue, setQueue] = useState<QueueItem[]>([])
  const [dspList, setDspList] = useState<DspInfo[]>([])
  const [smartMode, setSmartMode] = useState<string>('auto')
  const [surroundMode, setSurroundMode] = useState<string>('off')
  const [pendingQueueIndex, setPendingQueueIndex] = useState<number | null>(null)

  // 设备 / DSP / 增强模式：面板打开时拉取一次（低频变化，不追事件）
  useEffect(() => {
    if (!open) return
    invoke<DeviceInfo[]>('list_devices')
      .then(setDevices)
      .catch(() => {})
    invoke<{ id: string } | null>('get_current_device')
      .then((d) => setCurrentDeviceId(d?.id ?? null))
      .catch(() => {})
    invoke<DspInfo[]>('list_dsp_processors')
      .then(setDspList)
      .catch(() => {})
    invoke<string>('get_smart_effect_mode')
      .then(setSmartMode)
      .catch(() => {})
    invoke<string>('get_surround_sound_mode')
      .then(setSurroundMode)
      .catch(() => {})
  }, [open])

  // 队列：打开时拉取；面板开着期间曲目切换也要刷新，否则当前曲目高亮失准
  const currentTrackForQueue = playback?.current_track
  useEffect(() => {
    if (!open) return
    invoke<QueueItem[]>('get_queue')
      .then(setQueue)
      .catch(() => {})
  }, [open, currentTrackForQueue])

  const handleSetDevice = (id: string) => {
    invoke('set_device', { device_id: id })
      .then(() => {
        setCurrentDeviceId(id)
        // 重新拉取设备列表以更新 is_default 状态
        return invoke<DeviceInfo[]>('list_devices')
      })
      .then((devs) => { if (devs) setDevices(devs) })
      .catch(() => {})
  }

  const handlePlayIndex = (index: number) => {
    // 乐观更新：立即高亮选中项，减少等待感
    setPendingQueueIndex(index)
    invoke('play_index', { index })
      .then(() => {
        // 播放成功后 1.5s 清除 pending，等事件刷新
        setTimeout(() => setPendingQueueIndex(null), 1500)
      })
      .catch(() => setPendingQueueIndex(null))
  }

  const handleToggleDsp = (id: string, enabled: boolean) => {
    // 乐观更新 UI，后端失败则回滚，避免显示与实际状态脱节
    setDspList((prev) => prev.map((d) => (d.id === id ? { ...d, enabled: !enabled } : d)))
    const cmd = enabled ? 'disable_dsp' : 'enable_dsp'
    invoke(cmd, { id }).catch(() => {
      setDspList((prev) => prev.map((d) => (d.id === id ? { ...d, enabled } : d)))
    })
  }

  const tabs: { key: PanelTab; icon: React.ReactNode; label: string }[] = [
    { key: 'playback', icon: <IconMusic />, label: '播放' },
    { key: 'visual', icon: <IconSparkles />, label: '可视化' },
    { key: 'device', icon: <IconSpeaker />, label: '设备' },
    { key: 'queue', icon: <IconList />, label: '队列' },
    { key: 'settings', icon: <IconSettings />, label: '设置' },
  ]

  const currentTrackPath = playback?.current_track
  // 路径归一化后再匹配（Windows 下斜杠可能不一致）
  const normalizePath = (p: string) => p.replace(/\\/g, '/').toLowerCase()
  const currentQueueIndex = pendingQueueIndex !== null
    ? pendingQueueIndex
    : (currentTrackPath
        ? queue.findIndex((q) => normalizePath(q.path) === normalizePath(currentTrackPath))
        : -1)

  return (
    <>
      {/* 遮罩 */}
      <div
        onClick={onClose}
        style={{
          position: 'absolute',
          inset: 0,
          background: 'rgba(0, 0, 0, 0.4)',
          backdropFilter: 'blur(6px)',
          opacity: open ? 1 : 0,
          pointerEvents: open ? 'auto' : 'none',
          transition: 'opacity 0.35s ease',
          zIndex: 25,
        }}
      />

      {/* 面板 */}
      <div style={{
        position: 'absolute',
        top: 0,
        right: 0,
        width: 320,
        height: '100%',
        background: 'linear-gradient(180deg, rgba(14, 16, 36, 0.94), rgba(6, 8, 20, 0.97))',
        backdropFilter: 'blur(24px) saturate(180%)',
        borderLeft: '1px solid rgba(120, 160, 255, 0.18)',
        boxShadow: '-12px 0 48px rgba(0, 0, 0, 0.5), inset 1px 0 0 rgba(255, 255, 255, 0.04)',
        transform: open ? 'translateX(0)' : 'translateX(100%)',
        transition: 'transform 0.4s cubic-bezier(0.22, 1, 0.36, 1)',
        zIndex: 30,
        display: 'flex',
        flexDirection: 'column',
      }}>
        {/* 顶部渐变光带 + 标题 */}
        <div style={{
          position: 'relative',
          padding: '20px 20px 16px',
          borderBottom: '1px solid rgba(120, 160, 255, 0.12)',
          overflow: 'hidden',
        }}>
          {/* 顶部光晕 */}
          <div style={{
            position: 'absolute',
            top: -30,
            left: '20%',
            right: '20%',
            height: 60,
            background: 'radial-gradient(ellipse at center, rgba(120, 160, 255, 0.25) 0%, transparent 70%)',
            pointerEvents: 'none',
          }} />
          <div style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            position: 'relative',
          }}>
            <span style={{
              fontSize: 14,
              fontWeight: 600,
              color: 'rgba(220, 230, 255, 0.92)',
              letterSpacing: 0.8,
            }}>控制台</span>
            <button
              onClick={onClose}
              style={{
                width: 28,
                height: 28,
                borderRadius: '50%',
                border: '1px solid rgba(120, 160, 255, 0.2)',
                background: 'rgba(255, 255, 255, 0.04)',
                color: 'rgba(200, 215, 255, 0.65)',
                cursor: 'pointer',
                display: 'inline-flex',
                alignItems: 'center',
                justifyContent: 'center',
                transition: 'all 0.2s ease',
              }}
              onMouseEnter={(e) => {
                e.currentTarget.style.color = 'rgba(255, 150, 150, 0.9)'
                e.currentTarget.style.borderColor = 'rgba(255, 120, 120, 0.4)'
                e.currentTarget.style.background = 'rgba(255, 100, 100, 0.1)'
              }}
              onMouseLeave={(e) => {
                e.currentTarget.style.color = 'rgba(200, 215, 255, 0.65)'
                e.currentTarget.style.borderColor = 'rgba(120, 160, 255, 0.2)'
                e.currentTarget.style.background = 'rgba(255, 255, 255, 0.04)'
              }}
            ><IconClose /></button>
          </div>
        </div>

        {/* 主体：左侧 tab + 右侧内容 */}
        <div style={{ flex: 1, display: 'flex', minHeight: 0 }}>
          {/* 左侧 tab 栏 */}
          <div style={{
            width: 60,
            borderRight: '1px solid rgba(120, 160, 255, 0.1)',
            padding: '14px 0',
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            gap: 4,
            flexShrink: 0,
            position: 'relative',
          }}>
            {tabs.map((t) => {
              const active = tab === t.key
              return (
                <TabButton
                  key={t.key}
                  active={active}
                  label={t.label}
                  onClick={() => setTab(t.key)}
                >
                  {t.icon}
                </TabButton>
              )
            })}

            <div style={{ flex: 1 }} />

            {/* 退出 3D */}
            <button
              onClick={() => onExit3D?.()}
              style={{
                width: 44,
                height: 44,
                borderRadius: 12,
                border: 'none',
                background: 'transparent',
                color: 'rgba(255, 140, 140, 0.55)',
                cursor: 'pointer',
                display: 'inline-flex',
                alignItems: 'center',
                justifyContent: 'center',
                transition: 'all 0.2s ease',
              }}
              onMouseEnter={(e) => {
                e.currentTarget.style.color = 'rgba(255, 120, 120, 0.9)'
                e.currentTarget.style.background = 'rgba(255, 100, 100, 0.1)'
              }}
              onMouseLeave={(e) => {
                e.currentTarget.style.color = 'rgba(255, 140, 140, 0.55)'
                e.currentTarget.style.background = 'transparent'
              }}
            >
              <IconExit />
            </button>
          </div>

          {/* 右侧内容区 */}
          <div style={{ flex: 1, padding: '18px 18px 28px', overflowY: 'auto', minWidth: 0 }}>
            {tab === 'playback' && <PlaybackTab playback={playback} volume={volume} onVolumeChange={onVolumeChange} />}
            {tab === 'visual' && <VisualTab settings={sceneSettings} onChange={onSettingChange} />}
            {tab === 'device' && (
              <DeviceTab
                devices={devices}
                currentDeviceId={currentDeviceId}
                onSelect={handleSetDevice}
                dspList={dspList}
                smartMode={smartMode}
                surroundMode={surroundMode}
                onToggleDsp={handleToggleDsp}
              />
            )}
            {tab === 'queue' && (
              <QueueTab
                items={queue}
                currentIndex={currentQueueIndex}
                onPlay={handlePlayIndex}
              />
            )}
            {tab === 'settings' && <SettingsTab settings={sceneSettings} onChange={onSettingChange} loadLevel={loadLevel} />}
          </div>
        </div>
      </div>
    </>
  )
}

// ===== 各 Tab 内容 =====

function PlaybackTab({ playback, volume, onVolumeChange }: { playback?: any; volume: number; onVolumeChange?: (v: number) => void }) {
  const currentTrack = playback?.current_track
    ? playback.current_track.replace(/^.*[\\/]/, '').replace(/\.[^.]+$/, '')
    : '未在播放'
  const localVol = volume
  const volRef = useRef<HTMLDivElement>(null)
  const draggingVolRef = useRef(false)
  const volCleanupRef = useRef<(() => void) | null>(null)
  const [, forceUpdate] = useState(0)

  const handleVolChange = (v: number) => {
    onVolumeChange?.(v)
  }

  const updateVolFromEvent = useCallback((clientX: number) => {
    if (!volRef.current) return
    const rect = volRef.current.getBoundingClientRect()
    const ratio = Math.max(0, Math.min(1, (clientX - rect.left) / rect.width))
    handleVolChange(Math.round(ratio * 100))
  }, [])

  useEffect(() => {
    return () => { volCleanupRef.current?.() }
  }, [])

  const handleVolMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault()
    e.stopPropagation()
    draggingVolRef.current = true
    updateVolFromEvent(e.clientX)
    forceUpdate((n) => n + 1)
    document.body.style.cursor = 'grabbing'
    document.body.style.userSelect = 'none'

    const onMove = (ev: MouseEvent) => {
      if (!draggingVolRef.current) return
      updateVolFromEvent(ev.clientX)
    }
    const onUp = () => {
      draggingVolRef.current = false
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
      volCleanupRef.current = null
      forceUpdate((n) => n + 1)
    }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
    volCleanupRef.current = () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
    }
  }, [updateVolFromEvent])

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      <SectionTitle>当前播放</SectionTitle>
      <div style={{
        padding: 14,
        borderRadius: 12,
        background: 'linear-gradient(135deg, rgba(100, 140, 255, 0.08), rgba(160, 120, 255, 0.04))',
        border: '1px solid rgba(120, 160, 255, 0.18)',
        boxShadow: 'inset 0 1px 0 rgba(255, 255, 255, 0.04)',
      }}>
        <div style={{
          fontSize: 13,
          color: 'rgba(220, 230, 255, 0.92)',
          fontWeight: 600,
          marginBottom: 6,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
          letterSpacing: 0.3,
        }}>{currentTrack}</div>
        <div style={{ fontSize: 11, color: 'rgba(180, 200, 240, 0.5)' }}>
          状态：<span style={{ color: 'rgba(140, 200, 255, 0.7)' }}>{playback?.state || '未知'}</span>
        </div>
      </div>

      <SectionTitle>音量</SectionTitle>
      <div style={{
        display: 'flex',
        alignItems: 'center',
        gap: 12,
        padding: '10px 14px',
        borderRadius: 10,
        background: 'rgba(255, 255, 255, 0.02)',
        border: '1px solid rgba(120, 160, 255, 0.1)',
      }}>
        <div style={{ color: 'rgba(180, 200, 240, 0.65)', flexShrink: 0 }}><IconSpeaker /></div>
        <div
          ref={volRef}
          onMouseDown={handleVolMouseDown}
          style={{
            flex: 1,
            height: 3,
            borderRadius: 1.5,
            background: 'rgba(80, 110, 180, 0.18)',
            position: 'relative',
            cursor: 'pointer',
          }}
        >
          <div style={{
            position: 'absolute',
            left: 0,
            top: 0,
            height: '100%',
            width: `${localVol}%`,
            borderRadius: 1.5,
            background: 'linear-gradient(90deg, #7cb0ff, #c8a0ff)',
            boxShadow: '0 0 6px rgba(124, 176, 255, 0.5)',
            pointerEvents: 'none',
          }} />
          <div style={{
            position: 'absolute',
            left: `${localVol}%`,
            top: '50%',
            transform: 'translate(-50%, -50%)',
            width: 8,
            height: 8,
            borderRadius: '50%',
            background: '#fff',
            boxShadow: '0 0 6px rgba(200, 220, 255, 0.7)',
            pointerEvents: 'none',
          }} />
        </div>
        <span style={{ fontSize: 11, color: 'rgba(180, 200, 240, 0.55)', minWidth: 34, textAlign: 'right', fontVariantNumeric: 'tabular-nums', flexShrink: 0 }}>
          {Math.round(localVol)}%
        </span>
      </div>
    </div>
  )
}

function VisualTab({ settings, onChange }: { settings?: SceneSettings; onChange?: (key: string, value: any) => void }) {
  const themes = [
    { id: 'nebula', name: '星云行星', desc: '行星自转 + 星云环绕' },
    { id: 'rhythm', name: '节奏音游', desc: '键盘交互，节拍判定' },
  ]
  const currentTheme = settings?.theme ?? 'nebula'

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      <SectionTitle>可视化主题</SectionTitle>
      {themes.map((t) => {
        const active = t.id === currentTheme
        return (
          <button
            key={t.id}
            onClick={() => onChange?.('theme', t.id)}
            style={{
            width: '100%',
            padding: '12px 14px',
            borderRadius: 10,
            border: active
              ? '1px solid rgba(130, 170, 255, 0.45)'
              : '1px solid rgba(120, 160, 255, 0.1)',
            background: active
              ? 'linear-gradient(135deg, rgba(100, 140, 255, 0.15), rgba(160, 120, 255, 0.08))'
              : 'rgba(255, 255, 255, 0.02)',
            color: 'inherit',
            cursor: 'pointer',
            textAlign: 'left',
            transition: 'all 0.2s ease',
            boxShadow: active ? 'inset 0 1px 0 rgba(255, 255, 255, 0.06)' : 'none',
          }}
        >
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            <div style={{
              width: 8,
              height: 8,
              borderRadius: '50%',
              background: active ? '#7cb0ff' : 'rgba(180, 200, 240, 0.2)',
              boxShadow: active ? '0 0 8px rgba(124, 176, 255, 0.6)' : 'none',
              flexShrink: 0,
            }} />
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 12, fontWeight: 600, color: 'rgba(220, 230, 255, 0.9)', marginBottom: 2, letterSpacing: 0.3 }}>
                {t.name}
              </div>
              <div style={{ fontSize: 10, color: 'rgba(180, 200, 240, 0.5)' }}>
                {t.desc}
              </div>
            </div>
          </div>
        </button>
      )})}
    </div>
  )
}

function ToggleRow({ label, defaultChecked, checked, onChange }: { label: string; defaultChecked?: boolean; checked?: boolean; onChange?: (v: boolean) => void }) {
  const isControlled = checked !== undefined
  return (
    <label style={{
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'space-between',
      padding: '10px 14px',
      borderRadius: 10,
      background: 'rgba(255, 255, 255, 0.02)',
      border: '1px solid rgba(120, 160, 255, 0.1)',
      fontSize: 12,
      color: 'rgba(200, 215, 255, 0.75)',
      cursor: 'pointer',
      transition: 'all 0.2s ease',
    }}>
      {label}
      <input
        type="checkbox"
        defaultChecked={isControlled ? undefined : defaultChecked}
        checked={isControlled ? checked : undefined}
        onChange={(e) => onChange?.(e.target.checked)}
        style={{ accentColor: '#7cb0ff' }}
      />
    </label>
  )
}

interface DeviceTabProps {
  devices: DeviceInfo[]
  currentDeviceId: string | null
  onSelect: (id: string) => void
  dspList: DspInfo[]
  smartMode: string
  surroundMode: string
  onToggleDsp: (id: string, enabled: boolean) => void
}

function DeviceTab({ devices, currentDeviceId, onSelect, dspList, smartMode, surroundMode, onToggleDsp }: DeviceTabProps) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      <SectionTitle>输出设备（{devices.length}）</SectionTitle>
      {devices.length === 0 ? (
        <div style={{ fontSize: 11, color: 'rgba(180, 200, 240, 0.4)', padding: '10px 14px' }}>加载中...</div>
      ) : (
        devices.map((d) => {
          const active = d.id === currentDeviceId
          const maxRate = d.sample_rates.length > 0 ? Math.max(...d.sample_rates) : 0
          return (
            <button
              key={d.id}
              onClick={() => onSelect(d.id)}
              style={listItemStyle(active)}
            >
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <div style={{
                  width: 6,
                  height: 6,
                  borderRadius: '50%',
                  background: active ? '#8acfff' : 'rgba(180, 200, 240, 0.2)',
                  boxShadow: active ? '0 0 6px rgba(138, 207, 255, 0.5)' : 'none',
                  flexShrink: 0,
                }} />
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{
                    fontSize: 12,
                    fontWeight: 500,
                    color: active ? '#b8d8ff' : 'rgba(220, 230, 255, 0.85)',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                  }}>
                    {d.name}
                    {d.is_default && <span style={{ color: 'rgba(180, 200, 240, 0.4)', fontSize: 10, marginLeft: 6 }}>（默认）</span>}
                  </div>
                  <div style={{
                    fontSize: 10,
                    color: 'rgba(180, 200, 240, 0.5)',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                  }}>
                    {d.max_channels}ch · {maxRate > 0 ? `${maxRate / 1000}kHz` : '--'}
                    {d.supports_exclusive ? ' · 独占' : ''}
                  </div>
                </div>
              </div>
            </button>
          )
        })
      )}

      <div style={{ height: 2 }} />
      <SectionTitle>DSP 效果</SectionTitle>
      {dspList.length === 0 ? (
        <div style={{ fontSize: 11, color: 'rgba(180, 200, 240, 0.4)', padding: '10px 14px' }}>暂无 DSP 效果器</div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {dspList.map((d) => (
            <button key={d.id} onClick={() => onToggleDsp(d.id, d.enabled)} style={{
              ...listItemStyle(d.enabled),
              padding: '10px 14px',
              display: 'flex',
              flexDirection: 'column',
              gap: 2,
            }}>
              <div style={{
                display: 'flex',
                justifyContent: 'space-between',
                alignItems: 'center',
              }}>
                <span style={{
                  fontSize: 12,
                  fontWeight: 500,
                  color: d.enabled ? '#d4b8ff' : 'rgba(220, 230, 255, 0.85)',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                  flex: 1,
                  marginRight: 10,
                }}>
                  {d.name}
                </span>
                <div style={{
                  width: 36,
                  height: 20,
                  borderRadius: 10,
                  background: d.enabled
                    ? 'linear-gradient(135deg, #8acfff, #b88cff)'
                    : 'rgba(180, 200, 240, 0.12)',
                  position: 'relative',
                  flexShrink: 0,
                  transition: 'all 0.2s ease',
                  boxShadow: d.enabled ? 'inset 0 0 4px rgba(255, 255, 255, 0.2)' : 'none',
                }}>
                  <div style={{
                    position: 'absolute',
                    top: 2,
                    left: d.enabled ? 20 : 2,
                    width: 16,
                    height: 16,
                    borderRadius: '50%',
                    background: d.enabled
                      ? 'radial-gradient(circle at 30% 30%, #fff, #d0e8ff)'
                      : 'rgba(180, 200, 240, 0.35)',
                    boxShadow: d.enabled ? '0 1px 4px rgba(0, 0, 0, 0.3), 0 0 8px rgba(138, 207, 255, 0.4)' : 'none',
                    transition: 'left 0.2s ease',
                  }} />
                </div>
              </div>
              <div style={{
                fontSize: 10,
                color: 'rgba(180, 200, 240, 0.5)',
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
              }}>
                延迟：{d.latency_ms.toFixed(1)} ms
              </div>
            </button>
          ))}
        </div>
      )}

      <div style={{ height: 4 }} />
      <SectionTitle>声音增强</SectionTitle>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
        <div style={{
          padding: '10px 14px',
          borderRadius: 10,
          background: 'rgba(255, 255, 255, 0.02)',
          border: '1px solid rgba(120, 160, 255, 0.1)',
          fontSize: 12,
          color: 'rgba(200, 215, 255, 0.75)',
        }}>
          <div style={{ marginBottom: 6 }}>Smart Effect</div>
          <div style={{ fontSize: 11, color: 'rgba(180, 200, 240, 0.55)' }}>
            模式：<span style={{ color: '#c8a0ff' }}>{smartMode}</span>
          </div>
        </div>
        <div style={{
          padding: '10px 14px',
          borderRadius: 10,
          background: 'rgba(255, 255, 255, 0.02)',
          border: '1px solid rgba(120, 160, 255, 0.1)',
          fontSize: 12,
          color: 'rgba(200, 215, 255, 0.75)',
        }}>
          <div style={{ marginBottom: 6 }}>环绕声</div>
          <div style={{ fontSize: 11, color: 'rgba(180, 200, 240, 0.55)' }}>
            模式：<span style={{ color: '#8acfff' }}>{surroundMode}</span>
          </div>
        </div>
      </div>
    </div>
  )
}

interface QueueTabProps {
  items: QueueItem[]
  currentIndex: number
  onPlay: (index: number) => void
}

function QueueTab({ items, currentIndex, onPlay }: QueueTabProps) {
  const fmtDur = (s: number | null) => {
    if (s == null) return '--:--'
    const m = Math.floor(s / 60)
    const sec = Math.floor(s % 60)
    return `${String(m).padStart(2, '0')}:${String(sec).padStart(2, '0')}`
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
      <SectionTitle>播放队列（{items.length}）</SectionTitle>
      {items.length === 0 ? (
        <div style={{ fontSize: 11, color: 'rgba(180, 200, 240, 0.4)', padding: '20px 0', textAlign: 'center' }}>队列为空</div>
      ) : (
        items.map((item, i) => {
          const active = i === currentIndex
          return (
            <button
              key={i}
              onClick={() => onPlay(i)}
              style={{
                ...listItemStyle(active),
                padding: '9px 12px',
              }}
            >
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <span style={{
                  fontSize: 10,
                  color: active ? '#ffe08a' : 'rgba(180, 200, 240, 0.35)',
                  minWidth: 22,
                  fontVariantNumeric: 'tabular-nums',
                  textAlign: 'center',
                }}>
                  {active ? '♪' : String(i + 1).padStart(2, '0')}
                </span>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{
                    fontSize: 12,
                    color: active ? '#ffe08a' : 'rgba(220, 230, 255, 0.85)',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                  }}>
                    {item.title || item.path.split(/[\\/]/).pop() || '未知曲目'}
                  </div>
                  <div style={{
                    fontSize: 10,
                    color: 'rgba(180, 200, 240, 0.5)',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                  }}>
                    {item.artist || '未知艺术家'}
                  </div>
                </div>
                <span style={{
                  fontSize: 10,
                  color: 'rgba(180, 200, 240, 0.4)',
                  fontVariantNumeric: 'tabular-nums',
                  flexShrink: 0,
                }}>
                  {fmtDur(item.duration)}
                </span>
              </div>
            </button>
          )
        })
      )}
    </div>
  )
}

function SettingsTab({ settings, onChange, loadLevel }: { settings?: SceneSettings; onChange?: (key: string, value: any) => void; loadLevel?: number }) {
  const qualityLabels = ['低', '中', '高', '极致']
  const qualityKeys = ['low', 'mid', 'high', 'ultra']
  const currentQ = settings?.quality ?? 'high'
  const currentIdx = Math.max(0, qualityKeys.indexOf(currentQ))

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      <SectionTitle>显示</SectionTitle>
      <ToggleRow
        label="后处理效果（极致档含景深）"
        checked={settings?.postProcessing ?? true}
        onChange={(v) => onChange?.('postProcessing', v)}
      />

      <div style={{ height: 4 }} />
      <SectionTitle>性能</SectionTitle>
      <div style={{ fontSize: 11, color: 'rgba(180, 200, 240, 0.5)', marginBottom: 6 }}>
        质量预设：
      </div>
      {(loadLevel ?? 0) > 0 && (
        <div style={{
          fontSize: 10,
          color: 'rgba(255, 205, 130, 0.85)',
          background: 'rgba(255, 180, 80, 0.08)',
          border: '1px solid rgba(255, 190, 100, 0.25)',
          borderRadius: 6,
          padding: '5px 8px',
          marginBottom: 8,
        }}>
          ⚡ 帧预算保护已自动降载到档位 {loadLevel}（稳定后会自动回升）
        </div>
      )}
      <div style={{ display: 'flex', gap: 6 }}>
        {qualityLabels.map((q, i) => (
          <button
            key={q}
            onClick={() => onChange?.('quality', qualityKeys[i])}
            style={{
              flex: 1,
              padding: '7px 0',
              borderRadius: 8,
              border: i === currentIdx
                ? '1px solid rgba(130, 170, 255, 0.5)'
                : '1px solid rgba(120, 160, 255, 0.12)',
              background: i === currentIdx
                ? 'linear-gradient(135deg, rgba(100, 140, 255, 0.2), rgba(160, 120, 255, 0.1))'
                : 'rgba(255, 255, 255, 0.02)',
              color: i === currentIdx ? '#b8d8ff' : 'rgba(200, 215, 255, 0.6)',
              fontSize: 11,
              fontWeight: 500,
              cursor: 'pointer',
              transition: 'all 0.2s ease',
            }}
          >{q}</button>
        ))}
      </div>
    </div>
  )
}

function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <div style={{
      fontSize: 10,
      fontWeight: 600,
      color: 'rgba(150, 180, 240, 0.45)',
      letterSpacing: 1.5,
      textTransform: 'uppercase',
      marginTop: 4,
      padding: '0 2px',
    }}>
      {children}
    </div>
  )
}

// Tab 按钮 — hover 时滑出文字标签，选中态有左侧发光条
function TabButton({ active, label, onClick, children }: {
  active: boolean
  label: string
  onClick: () => void
  children: React.ReactNode
}) {
  const [hovered, setHovered] = useState(false)

  return (
    <button
      onClick={onClick}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      style={{
        width: 44,
        height: 44,
        borderRadius: 12,
        border: 'none',
        background: active
          ? 'linear-gradient(135deg, rgba(100, 140, 255, 0.25), rgba(160, 120, 255, 0.15))'
          : hovered
            ? 'rgba(255, 255, 255, 0.04)'
            : 'transparent',
        color: active ? '#b0cfff' : hovered ? 'rgba(220, 230, 255, 0.75)' : 'rgba(180, 200, 240, 0.5)',
        cursor: 'pointer',
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        transition: 'all 0.2s ease',
        position: 'relative',
        boxShadow: active
          ? 'inset 0 0 12px rgba(120, 170, 255, 0.15), 0 0 10px rgba(100, 140, 255, 0.15)'
          : 'none',
        padding: 0,
      }}
    >
      {children}
      {/* 选中态左侧发光条 */}
      {active && (
        <div style={{
          position: 'absolute',
          left: -8,
          top: '50%',
          transform: 'translateY(-50%)',
          width: 3,
          height: 20,
          borderRadius: 2,
          background: 'linear-gradient(180deg, #7cb0ff, #c8a0ff)',
          boxShadow: '0 0 8px rgba(124, 176, 255, 0.6)',
        }} />
      )}
      {/* hover 时文字标签滑出 */}
      {hovered && (
        <div style={{
          position: 'absolute',
          left: '100%',
          marginLeft: 4,
          padding: '4px 10px',
          borderRadius: 6,
          background: 'rgba(10, 14, 32, 0.92)',
          border: '1px solid rgba(120, 160, 255, 0.25)',
          color: '#b0cfff',
          fontSize: 11,
          fontWeight: 500,
          whiteSpace: 'nowrap',
          pointerEvents: 'none',
          zIndex: 20,
          animation: 'fadeSlideIn 0.15s ease',
          boxShadow: '0 4px 16px rgba(0, 0, 0, 0.4)',
        }}>
          {label}
        </div>
      )}
    </button>
  )
}

function listItemStyle(active: boolean): React.CSSProperties {
  return {
    width: '100%',
    padding: '10px 14px',
    borderRadius: 10,
    border: active
      ? '1px solid rgba(130, 170, 255, 0.35)'
      : '1px solid transparent',
    background: active
      ? 'linear-gradient(135deg, rgba(100, 140, 255, 0.1), rgba(160, 120, 255, 0.04))'
      : 'rgba(255, 255, 255, 0.02)',
    color: 'inherit',
    cursor: 'pointer',
    textAlign: 'left',
    transition: 'all 0.2s ease',
    boxShadow: active ? 'inset 0 1px 0 rgba(255, 255, 255, 0.04)' : 'none',
  }
}

// ========= HUD 主容器（管理显示/隐藏逻辑）=========
interface SceneSettings {
  theme: string
  postProcessing: boolean
  quality: string
}

interface HUDProps {
  playback?: {
    state: string
    position_secs: number
    duration_secs: number | null
    current_track: string | null
  }
  volume: number
  onVolumeChange?: (v: number) => void
  onExit3D?: () => void
  sceneSettings?: SceneSettings
  onSettingChange?: (key: string, value: any) => void
  /** 帧预算自动降载的当前档位（0=满血）；>0 时在设置页显示提示 */
  loadLevel?: number
}

export function HUD({ playback, volume, onVolumeChange, onExit3D, sceneSettings, onSettingChange, loadLevel }: HUDProps) {
  const [cornersVisible, setCornersVisible] = useState(false)
  const [playBarVisible, setPlayBarVisible] = useState(false)
  const [panelOpen, setPanelOpen] = useState(false)
  const [sharedVol, setSharedVol] = useState(volume)
  const hideTimerRef = useRef<any>(null)
  const playBarHideTimerRef = useRef<any>(null)

  // 外部音量变化时同步到共享状态
  useEffect(() => {
    setSharedVol(volume)
  }, [volume])

  // 统一的音量设置函数（两边都调这个）。
  // 后端调用必须交给 App.handleVolume——那边有 50ms 节流；
  // 这里若直接 invoke('set_volume')，拖动时每条 mousemove 都会
  // 绕过节流直达 IPC，且与 App 路径形成双重调用。
  const handleSetVolume = useCallback((v: number) => {
    setSharedVol(v)
    onVolumeChange?.(v)
  }, [onVolumeChange])

  // 鼠标移动 → 显示四角按钮
  const handleMouseMove = useCallback(() => {
    setCornersVisible(true)
    if (hideTimerRef.current) clearTimeout(hideTimerRef.current)
    hideTimerRef.current = setTimeout(() => {
      setCornersVisible(false)
    }, 2000)
  }, [])

  // 播放栏 hover 状态变化（底部触发区 + 播放栏本身都会调用）
  const handlePlayBarHover = useCallback((hovered: boolean) => {
    if (playBarHideTimerRef.current) {
      clearTimeout(playBarHideTimerRef.current)
      playBarHideTimerRef.current = null
    }
    if (hovered) {
      setPlayBarVisible(true)
    } else {
      // 延迟 300ms 再隐藏，防止鼠标在触发区和播放栏之间移动时抖动
      playBarHideTimerRef.current = setTimeout(() => {
        setPlayBarVisible(false)
      }, 300)
    }
  }, [])

  useEffect(() => {
    window.addEventListener('mousemove', handleMouseMove)
    return () => {
      window.removeEventListener('mousemove', handleMouseMove)
      if (hideTimerRef.current) clearTimeout(hideTimerRef.current)
      if (playBarHideTimerRef.current) clearTimeout(playBarHideTimerRef.current)
    }
  }, [handleMouseMove])

  // Esc：控制台面板开着时先关面板；否则退出 3D 页
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      if (panelOpen) setPanelOpen(false)
      else onExit3D?.()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [panelOpen, onExit3D])

  return (
    <>
      {/* 底部 hover 触发区（透明，扩大触发范围） */}
      <div
        onMouseEnter={() => handlePlayBarHover(true)}
        onMouseLeave={() => handlePlayBarHover(false)}
        style={{
          position: 'absolute',
          bottom: 0,
          left: 0,
          right: 0,
          height: 80,
          zIndex: 14,
        }}
      />

      {/* 左上返回按钮（面板打开时保持可见，不随鼠标静止淡出） */}
      <CornerButton
        position="top-left"
        visible={cornersVisible || panelOpen}
        onClick={() => onExit3D?.()}
        icon={<IconBack />}
      />

      {/* 右上按钮（面板打开时变成关闭按钮） */}
      <CornerButton
        position="top-right"
        visible={cornersVisible || panelOpen}
        onClick={() => setPanelOpen(!panelOpen)}
        icon={panelOpen ? <IconClose /> : <IconMenu />}
        highlight={panelOpen}
      />

      <BottomPlayBar
        visible={playBarVisible}
        onHoverChange={handlePlayBarHover}
        playback={playback}
        volume={sharedVol}
        onVolumeChange={handleSetVolume}
      />

      <RightControlPanel
        open={panelOpen}
        onClose={() => setPanelOpen(false)}
        onExit3D={onExit3D}
        playback={playback}
        volume={sharedVol}
        onVolumeChange={handleSetVolume}
        sceneSettings={sceneSettings}
        onSettingChange={onSettingChange}
        loadLevel={loadLevel}
      />
    </>
  )
}
