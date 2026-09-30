import { useState, useEffect, useCallback, useRef, lazy, Suspense } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen, emitTo } from '@tauri-apps/api/event'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { register, unregisterAll } from '@tauri-apps/plugin-global-shortcut'
import { getCurrentWindow } from '@tauri-apps/api/window'
import Player from './components/Player'
import Playlist from './components/Playlist'
import PlaylistToolbar from './components/PlaylistToolbar'
import Spectrum from './components/Spectrum'
import DeviceSelector from './components/DeviceSelector'
import DspPanel from './components/DspPanel'
import ExtensionsPanel from './components/ExtensionsPanel'
import Settings from './components/Settings'
import TitleBar from './components/TitleBar'
import LyricsPage from './components/LyricsPage'
import VisualizerPage from './components/VisualizerPage'
import VisModeButton, { type VisMode } from './components/VisModeButton'
import { syncVisPluginStyles, startVisLoop, stopVisLoop } from './api/visModeHost'
// three.js/R3F stack is only needed on the Depth3D tab — keep it out of the
// initial bundle (roughly halves the main chunk size).
const Depth3DPage = lazy(() => import('./components/Depth3DPage'))
import { I18nContext, getTranslation, useI18n } from './i18n'
import type { Lang } from './i18n'
import { useThrottledCallback } from './hooks/useThrottle'
import './App.css'

// Run localStorage schema migration before any component reads a key.
// This is module-level (runs once on import, before the first render) so
// every useState initializer below sees a validated, migrated store.
// Spec §12: "localStorage schema 迁移 + 启动校验 + 降级".
import { migrateStorage, readStorage, writeStorage } from './api/storage'
import { publishSpectrumBands } from './api/spectrumBus'
import { publishProgress, resetProgress, getProgress, useProgress } from './api/progressBus'
migrateStorage()

// ── Vite-only 安全包装 ────────────────────────────────────────────────────
// @tauri-apps/api 的 invoke/listen 在 __TAURI_INTERNALS__ 缺失（Vite dev /
// Playwright 纯 chromium 模式）时，会在同步代码里访问 undefined 的属性并 throw，
// 这直接让 React commit 阶段把整棵组件树 teardown（tab 都渲染不出来）。
// 我们在这里用 try/catch 包一层，同步失败时返回一个永远 resolve(undefined) 的
// Promise，这样应用其他部分照常运行，插件 UI 也能在无后端环境下正常挂载，
// 方便 E2E 做渲染级断言。对于真正的 tauri dev / tauri build，包装是零成本透传。
const hasTauri = typeof (window as any).__TAURI_INTERNALS__ !== 'undefined'
  && typeof (window as any).__TAURI_INTERNALS__.invoke === 'function'

/** Safe invoke — 当 Tauri 后端不存在时静默 resolve(undefined)。 */
async function safeInvoke<T = any>(cmd: string, args?: Record<string, unknown>): Promise<T | undefined> {
  if (!hasTauri) return undefined
  try {
    return await invoke<T>(cmd, args as any)
  } catch {
    return undefined
  }
}

/** Safe listen — 当 Tauri 后端不存在时返回 no-op unlisten。 */
async function safeListen<T = string>(event: string, handler: (event: { payload: T }) => void): Promise<() => void> {
  if (!hasTauri) return () => {}
  try {
    const cb = (e: any) => {
      try { handler(e) } catch { /* swallow handler errors */ }
    }
    const unlisten = await listen<T>(event, cb)
    return () => { try { unlisten() } catch {} }
  } catch {
    return () => {}
  }
}

/** Safe emitTo — 当 Tauri 后端不存在时静默 no-op。 */
async function safeEmitTo(target: string, event: string, payload?: unknown): Promise<void> {
  if (!hasTauri) return
  try { await emitTo(target, event, payload) } catch {}
}

/**
 * Safe getCurrentWindow — 返回 Tauri Window 实例或 null。
 * Vite-only 下 getCurrentWindow() 同步访问 __TAURI_INTERNALS__.metadata
 * 直接 throw（这就是 console 里的 `Cannot read ... 'metadata'` 根因），
 * React 会在 commit phase 把整棵组件树 teardown 导致 tabs 完全不渲染。
 */
function safeGetCurrentWindow(): ReturnType<typeof getCurrentWindow> | null {
  if (!hasTauri) return null
  try { return getCurrentWindow() } catch { return null }
}

type Tab = 'player' | 'devices' | 'dsp' | 'extensions' | 'settings' | 'visualizer' | 'depth3d'

interface PlaybackState {
  state: string
  position_secs: number
  duration_secs: number | null
  buffer_fill: number
  current_track: string | null
  queue_length: number
  speed: number
}

interface AlbumArtResponse {
  data: number[]
  mime_type: string
}

interface LyricWordJson {
  time_ms: number
  text: string
}

interface LyricLineJson {
  time_ms: number
  text: string
  words: LyricWordJson[]
}

// ── Unified AppEvent types (mirrors Rust AppEvent) ─────────────

type AppEventPayload =
  | {
      type: 'DeviceHotplug' | 'PlaybackProgress' | 'SpectrumData' | 'AudioFeaturesData' | 'PluginStatus' | 'PlaybackStateChanged'
      event?: string
      device_name?: string
      is_default?: boolean
      position_secs?: number
      duration_secs?: number | null
      values?: number[]
      id?: string
      status?: string
      message?: string
      state?: string
      // AudioFeaturesData fields
      spectrum?: number[]
      waveform?: number[]
      rms?: number
      peak?: number
      spectral_centroid_hz?: number
      onset?: number
      beat?: boolean
      bpm?: number
      chroma?: number[]
      sample_rate?: number
      channels?: number
    }
  | {
      type: 'PluginUserEvent'
      /** Plugin 自定义事件名（任意字符串），用于区分 payload 语义。 */
      name: string
      /** Plugin 自定义 payload（通常是 JSON 字符串，插件自行解析）。 */
      payload: string
    }

interface Toast {
  id: number
  message: string
  kind: 'info' | 'warn' | 'error'
}

// Helper: lighten a hex color by a ratio
function lighten(hex: string, amount: number): string {
  const num = parseInt(hex.replace('#', ''), 16)
  const r = Math.min(255, (num >> 16) + Math.round(255 * amount))
  const g = Math.min(255, ((num >> 8) & 0x00FF) + Math.round(255 * amount))
  const b = Math.min(255, (num & 0x0000FF) + Math.round(255 * amount))
  return `#${(r << 16 | g << 8 | b).toString(16).padStart(6, '0')}`
}

function App() {
  const [lang, setLang] = useState<Lang>(() => (localStorage.getItem('phonon-language') as Lang) || 'zh-CN')
  const [isMaximized, setIsMaximized] = useState(false)

  const [activeTab, setActiveTab] = useState<Tab>('player')

  // ── 启动层交接 ──
  // 静态启动层（index.html，毛玻璃 + 极光漂移）在首帧即可见；此处等 React
  // 首帧提交（双 rAF）且满足最短展示时长后：标记 app-booted（启动层淡出、
  // 主界面淡入），延时把启动层从 DOM 移除——backdrop-filter 常驻会持续
  // 消耗合成资源，移除后零残留。幂等：HMR/StrictMode 重复执行无副作用。
  useEffect(() => {
    let raf2 = 0
    let t1: ReturnType<typeof setTimeout> | undefined
    let t2: ReturnType<typeof setTimeout> | undefined
    const raf1 = requestAnimationFrame(() => {
      raf2 = requestAnimationFrame(() => {
        const splash = () => document.getElementById('boot-splash')
        const booted = () => document.documentElement.classList.contains('app-booted')
        if (booted() || !splash()) return // 已交接（HMR 场景）
        const started = (window as unknown as { __bootStart?: number }).__bootStart ?? 0
        const remain = Math.max(0, 600 - (performance.now() - started))
        t1 = setTimeout(() => {
          document.documentElement.classList.add('app-booted')
          splash()?.classList.add('boot-out')
          t2 = setTimeout(() => splash()?.remove(), 900)
        }, remain)
      })
    })
    return () => {
      cancelAnimationFrame(raf1)
      cancelAnimationFrame(raf2)
      if (t1) clearTimeout(t1)
      if (t2) clearTimeout(t2)
    }
  }, [])

  // ═══════════════════════════════════════════════════════════════
  //  E2E 测试专用钩子（零副作用）
  //
  //  关键设计约束（最终版，经过 4 次迭代验证）：
  //
  //  1. 必须在 useEffect（render COMMIT 之后）注册钩子，不能在
  //     render 体赋值：render 体赋值 + 外部调用 setState = React 19
  //     StrictMode 识别为 "set during render" → 同步重跑 render
  //     50 次，main thread 占满，CDP Runtime.evaluate 无法返回。
  //
  //  2. setState 必须用 setTimeout(0) 推到**下一事件循环宏任务**：
  //     queueMicrotask / Promise.then().finally() 仍然走 microtask
  //     checkpoint，而 V8/HTML spec 要求 Runtime.evaluate 返回前
  //     必须清空 microtask 队列，导致 React 重渲染在 checkpoint 中
  //     发生。若 React 子树 commit 后某 microtask → setState →
  //     re-render → 新 microtask → 循环 → checkpoint 永远无法清空
  //     → Runtime.evaluate 永远 pending → Playwright 看到 "evaluate
  //     timeout 60s" 但 pageErrors=0。
  //     推到宏任务（setTimeout(fn, 0)）保证 React 的 setState dispatch
  //     在 Runtime.evaluate 返回值之后才发生，后续 CDP 命令先发出去，
  //     就算 React 重渲染 100% CPU 占用的同步阶段让后续 evaluate
  //     timeout，前面的返回值已经到 Playwright，可诊断。
  //
  //  3. 返回值 true 仅代表"调度成功"，不代表 active 已经切换。
  //     调用方需用独立的 waitForFunction/waitForTimeout 轮询 DOM。
  // ═══════════════════════════════════════════════════════════════
  useEffect(() => {
    const allowed: Tab[] = ['player', 'devices', 'dsp', 'extensions', 'settings']
    // @ts-expect-error - test-only global
    window.__phononTestSetActiveTab = (tab: Tab): boolean => {
      if (!allowed.includes(tab)) return false
      // setTimeout(0) → 下一事件循环宏任务 → 不在任何 Runtime.evaluate
      // microtask checkpoint 内 → 保证 Runtime.evaluate 同步返回后
      // React 才开始 setState 流程。
      setTimeout(() => setActiveTab(tab), 0)
      return true
    }
    return () => {
      // @ts-expect-error - cleanup
      delete window.__phononTestSetActiveTab
    }
  }, [])

  // ── Global backend event listeners ─────────────────────────────────
  useEffect(() => {
    let unlisteners: Array<() => void> = []

    // phonon:toast — backend-initiated toast notifications
    const setupToast = async () => {
      try {
        const unlisten = await listen<{ msg: string; level?: string }>('phonon:toast', (e) => {
          const level = e.payload?.level || 'info'
          const kind = level === 'warn' ? 'warn' : level === 'error' ? 'error' : 'info'
          addToast(e.payload.msg || String(e.payload), kind)
        })
        unlisteners.push(unlisten)
      } catch {}
    }
    setupToast()

    // Check for startup-time corrupt-settings notice (may have fired
    // before listeners were registered)
    ;(async () => {
      try {
        const notice = await invoke<string | null>('take_settings_corrupt_notice')
        if (notice) addToast(notice, 'warn')
      } catch {}
    })()

    return () => {
      unlisteners.forEach((fn) => fn())
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])


  const [visPluginEnabled, setVisPluginEnabled] = useState<boolean>(() => {
    try {
      const raw = localStorage.getItem('phonon-enabled-vis-scripts')
      const arr = raw ? JSON.parse(raw) : []
      // 只认视觉升华插件本身（vis_mode.js）：启用其他可视化脚本
      // （纯 canvas 2D 脚本）不应召唤视觉升华按钮，也不该注入它的样式。
      return Array.isArray(arr) && arr.includes('vis_mode.js')
    } catch { return false }
  })
  const [visMode, setVisModeState] = useState<VisMode>(() => {
    try {
      const saved = localStorage.getItem('phonon.visMode')
      if (saved === 'classic' || saved === 'depth3d' || saved === 'off') return saved
    } catch { return 'classic' }
    return 'classic'
  })
  // 视觉增强总开关：每次启动应用都强制默认关闭（用户明确要求），与 visMode 的"上次选择"分离
  const [visActive, setVisActive] = useState<boolean>(false)
  const [showLyricsPage, setShowLyricsPage] = useState(false)
  const [coverUrl, setCoverUrl] = useState<string | null>(null)
  const [desktopLyricsOpen, setDesktopLyricsOpen] = useState(false)

  // ── EULA 首次启动弹窗 ────────────────────────────────────────────
  // 只有在用户从未接受过 EULA（eulaAccepted === false）时才显示。
  // 用户必须勾选同意才能点击"启动 Phonon"。
  const [showEulaModal, setShowEulaModal] = useState<boolean>(() => !readStorage('eulaAccepted'))
  const [eulaChecked, setEulaChecked] = useState<boolean>(false)
  const desktopLyricsOpenRef = useRef(false)
  useEffect(() => { desktopLyricsOpenRef.current = desktopLyricsOpen }, [desktopLyricsOpen])
  // Global dispose flag. Set by beforeunload so every ongoing async
  // operation (setInterval callbacks, refreshPlayback calls, promise
  // continuations) can short-circuit before making a new invoke or
  // a setState update on a torn-down webview.
  const appDisposedRef = useRef(false)
  // Central registry of all running intervals/timeouts that are NOT
  // owned by a single useEffect's return cleanup. HMR destroys the
  // cleanup-after-mount chain earlier than beforeunload, so we need
  // a second, independent registry that runs on beforeunload first.
  const openTimersRef = useRef<Set<ReturnType<typeof setInterval> | ReturnType<typeof setTimeout>>>(new Set())
  const registerTimer = (t: ReturnType<typeof setInterval> | ReturnType<typeof setTimeout>) => {
    openTimersRef.current.add(t)
    return t
  }
  const unregisterTimer = (t: ReturnType<typeof setInterval> | ReturnType<typeof setTimeout>) => {
    openTimersRef.current.delete(t)
  }
  const lyricsDataRef = useRef<{ lines: { time_ms: number; text: string; words: { time_ms: number; text: string }[] }[] } | null>(null)
  // Track the path of the lyrics currently being fetched, so that if the
  // user switches songs while get_lyrics is in flight, the stale result
  // can be discarded instead of overwriting the new song's (empty) state.
  const lyricsFetchPathRef = useRef<string | null>(null)
  /// Last track path that was recorded to play history — prevents duplicate records.
  const lastRecordedTrackRef = useRef<string | null>(null)
  const [playback, setPlayback] = useState<PlaybackState>({
    state: 'Idle',
    position_secs: 0,
    duration_secs: null,
    buffer_fill: 0,
    current_track: null,
    queue_length: 0,
    speed: 1.0,
  })
  const [toasts, setToasts] = useState<Toast[]>([])
  const toastIdRef = useRef(0)
  const [spectrumSettings, setSpectrumSettings] = useState({ smoothing: 0.35, decay: 0.90, fps: 60 })
  const [spectrumColorScheme, setSpectrumColorScheme] = useState<string>(() => localStorage.getItem('phonon-spectrum-colorscheme') || 'aurora')
  const [customSpectrumColors, setCustomSpectrumColors] = useState<{ bottom: string; top: string; peak: string; grid: string }>(() => {
    const saved = localStorage.getItem('phonon-spectrum-custom-colors')
    if (saved) {
      try { return JSON.parse(saved) }
      catch { /* ignore */ }
    }
    return { bottom: '#003153', top: '#00BFFF', peak: '#00FFFF', grid: '#404040' }
  })
  const [volumeMode, setVolumeModeRaw] = useState<string>(() => localStorage.getItem('phonon-volume-mode') || 'Software')
  const setVolumeMode = (mode: string) => {
    setVolumeModeRaw(mode)
    localStorage.setItem('phonon-volume-mode', mode)
  }
  // 音量状态（App 层统一管理，供主界面和 3D 页面共享）
  const [volume, setVolume] = useState(() => {
    const saved = localStorage.getItem('phonon-volume')
    return saved ? Number(saved) : 80
  })
  const throttledSetVolume = useThrottledCallback((level: number) => {
    invoke('set_volume', { level }).catch(() => {})
  }, 50)
  const handleVolume = useCallback((v: number) => {
    setVolume(v)
    localStorage.setItem('phonon-volume', String(v))
    throttledSetVolume(v / 100)
  }, [throttledSetVolume])
  // Persist playback mode to localStorage so it survives HMR refresh.
  // The Rust side also keeps it in memory, but `load_session` (called on
  // every frontend mount) reloads the *stale* session file and would
  // otherwise reset the in-memory mode to whatever was saved on last app
  // exit. localStorage holds the *current* user choice.
  const [playbackMode, setPlaybackModeState] = useState<string>(() => localStorage.getItem('phonon-playback-mode') || 'normal')
  const setPlaybackMode = (mode: string) => {
    setPlaybackModeState(mode)
    localStorage.setItem('phonon-playback-mode', mode)
  }
  const [refreshKey, setRefreshKey] = useState(0)

  const t = useCallback((key: string, fallback?: string) => getTranslation(lang, key, fallback), [lang])

  const addToast = (message: string, kind: Toast['kind'] = 'info') => {
    const id = ++toastIdRef.current
    setToasts((prev) => [...prev, { id, message, kind }])
    setTimeout(() => setToasts((prev) => prev.filter((t) => t.id !== id)), 3000)
  }

  const refreshPlayback = useCallback(async () => {
    // Never fire a new invoke once the page has started unloading:
    // Tauri's callback id table is cleared at the JS context teardown,
    // and any response would produce "[TAURI] Couldn't find callback id".
    if (appDisposedRef.current) return
    try {
      const state = await safeInvoke<PlaybackState>('get_playback_state')
      if (appDisposedRef.current) return
      // Guard：safeInvoke 在 vite-only / stub 下返回 undefined。
      // 不做 partial setState，避免访问 undefined.duration_secs 导致 React fatal。
      if (!state || typeof state !== 'object') return
      setPlayback((prev) => ({
        ...state,
        // Preserve duration_secs from event if poll returns null
        duration_secs: state.duration_secs ?? prev.duration_secs,
      }))
    } catch (_) { /* ignore (e.g. HMR reload cancels pending invoke) */ }
  }, [])

  const triggerRefresh = useCallback(() => {
    refreshPlayback()
    setRefreshKey((k) => k + 1)
  }, [refreshPlayback])

  // Split layout sizes — read from localStorage synchronously so the
  // very first render already has the persisted width. Prevents the
  // "flash to default size, then resize" on refresh, and also protects
  // against the init useEffect being skipped (e.g. if load_session throws).
  const [sidebarWidth, setSidebarWidth] = useState(() => {
    try {
      const s = localStorage.getItem('phonon-sidebar-width')
      const w = s ? parseFloat(s) : NaN
      return !isNaN(w) && w >= 180 && w <= 400 ? w : 220
    } catch {
      return 220
    }
  })
  const [playerMainWidth, setPlayerMainWidth] = useState(() => {
    try {
      const s = localStorage.getItem('phonon-player-main-width')
      const w = s ? parseFloat(s) : NaN
      return !isNaN(w) && w >= 200 && w <= 600 ? w : 360
    } catch {
      return 360
    }
  })
  const dragSplitRef = useRef<string | null>(null)
  const dragStartRef = useRef({ x: 0, y: 0, sidebarWidth: 0, playerMainWidth: 0 })
  const sidebarRef = useRef<HTMLDivElement>(null)
  const playerMainRef = useRef<HTMLDivElement>(null)
  const rafRef = useRef<number | null>(null)
  const pendingWidthRef = useRef<{ sidebar?: number; playerMain?: number } | null>(null)

  const applyPendingWidth = (target: 'sidebar' | 'playerMain', w: number) => {
    if (!pendingWidthRef.current) pendingWidthRef.current = {}
    pendingWidthRef.current[target] = w
    if (rafRef.current !== null) return
    rafRef.current = requestAnimationFrame(() => {
      rafRef.current = null
      if (pendingWidthRef.current) {
        const p = pendingWidthRef.current
        if (p.sidebar !== undefined && sidebarRef.current) {
          sidebarRef.current.style.width = `${p.sidebar}px`
        }
        if (p.playerMain !== undefined && playerMainRef.current) {
          playerMainRef.current.style.width = `${p.playerMain}px`
        }
        pendingWidthRef.current = null
      }
    })
  }

  const onDrag = useCallback((e: MouseEvent) => {
    e.preventDefault()
    const dx = e.clientX - dragStartRef.current.x
    if (dragSplitRef.current === 'sidebar') {
      const newW = Math.max(180, Math.min(400, dragStartRef.current.sidebarWidth + dx))
      applyPendingWidth('sidebar', newW)
    } else if (dragSplitRef.current === 'playlist') {
      const newW = Math.max(200, Math.min(600, dragStartRef.current.playerMainWidth - dx))
      applyPendingWidth('playerMain', newW)
    }
  }, [])

  const stopDrag = useCallback(() => {
    const splitId = dragSplitRef.current
    dragSplitRef.current = null
    document.removeEventListener('mousemove', onDrag)
    document.removeEventListener('mouseup', stopDrag)

    if (rafRef.current !== null) {
      cancelAnimationFrame(rafRef.current)
      rafRef.current = null
    }

    if (splitId === 'sidebar' && sidebarRef.current) {
      const w = parseFloat(sidebarRef.current.style.width) || 220
      setSidebarWidth(w)
      localStorage.setItem('phonon-sidebar-width', String(w))
      sidebarRef.current.classList.remove('sidebar-dragging')
    } else if (splitId === 'playlist' && playerMainRef.current) {
      const w = parseFloat(playerMainRef.current.style.width) || 360
      setPlayerMainWidth(w)
      localStorage.setItem('phonon-player-main-width', String(w))
      playerMainRef.current.classList.remove('player-main-dragging')
    }
    pendingWidthRef.current = null
  }, [onDrag])

  const startDrag = (splitId: string, e: React.MouseEvent) => {
    e.preventDefault()
    e.stopPropagation()
    dragSplitRef.current = splitId
    dragStartRef.current = {
      x: e.clientX,
      y: e.clientY,
      sidebarWidth,
      playerMainWidth,
    }
    if (splitId === 'sidebar' && sidebarRef.current) {
      sidebarRef.current.classList.add('sidebar-dragging')
    } else if (splitId === 'playlist' && playerMainRef.current) {
      playerMainRef.current.classList.add('player-main-dragging')
    }
    document.addEventListener('mousemove', onDrag)
    document.addEventListener('mouseup', stopDrag)
  }

  useEffect(() => {
    const savedPreset = localStorage.getItem('phonon-theme-preset')
    if (savedPreset) {
      document.documentElement.setAttribute('data-theme', savedPreset)
    }

    const savedAccent = localStorage.getItem('phonon-accent-color')
    const savedAccent2 = localStorage.getItem('phonon-accent2-color')
    const savedBg = localStorage.getItem('phonon-bg-color')
    const savedCardBg = localStorage.getItem('phonon-card-bg-color')
    const savedBorder = localStorage.getItem('phonon-border-color')
    const savedAccentOpacity = parseFloat(localStorage.getItem('phonon-accent-opacity') || '1')
    const savedAccent2Opacity = parseFloat(localStorage.getItem('phonon-accent2-opacity') || String(savedAccentOpacity))
    const savedBgOpacity = parseFloat(localStorage.getItem('phonon-bg-opacity') || '1')
    const savedCardBgOpacity = parseFloat(localStorage.getItem('phonon-cardbg-opacity') || '1')
    const savedBorderOpacity = parseFloat(localStorage.getItem('phonon-border-opacity') || '1')

    const hexToRgba = (hex: string, opacity: number): string => {
      const num = parseInt(hex.replace('#', ''), 16)
      const r = (num >> 16) & 0xFF
      const g = (num >> 8) & 0xFF
      const b = num & 0xFF
      return `rgba(${r}, ${g}, ${b}, ${opacity})`
    }

    const root = document.documentElement
    const parts: string[] = []

    if (savedAccent) {
      root.style.setProperty('--accent', hexToRgba(savedAccent, savedAccentOpacity), 'important')
      root.style.setProperty('--accent-hover', hexToRgba(lighten(savedAccent, 0.15), savedAccentOpacity), 'important')
      root.style.setProperty('--accent-glow', hexToRgba(savedAccent, 0.2), 'important')
      parts.push(`--accent:${hexToRgba(savedAccent, savedAccentOpacity)}!important`)
      parts.push(`--accent-hover:${hexToRgba(lighten(savedAccent, 0.15), savedAccentOpacity)}!important`)
      parts.push(`--accent-glow:${hexToRgba(savedAccent, 0.2)}!important`)
    }
    if (savedAccent2) {
      root.style.setProperty('--accent2', hexToRgba(savedAccent2, savedAccent2Opacity), 'important')
      root.style.setProperty('--accent2-hover', hexToRgba(lighten(savedAccent2, 0.15), savedAccent2Opacity), 'important')
      parts.push(`--accent2:${hexToRgba(savedAccent2, savedAccent2Opacity)}!important`)
      parts.push(`--accent2-hover:${hexToRgba(lighten(savedAccent2, 0.15), savedAccent2Opacity)}!important`)
    }
    if (savedBg) {
      root.style.setProperty('--bg', hexToRgba(savedBg, savedBgOpacity), 'important')
      parts.push(`--bg:${hexToRgba(savedBg, savedBgOpacity)}!important`)
    }
    if (savedCardBg) {
      root.style.setProperty('--bg-card', hexToRgba(savedCardBg, savedCardBgOpacity), 'important')
      parts.push(`--bg-card:${hexToRgba(savedCardBg, savedCardBgOpacity)}!important`)
      // Derive --bg-hover and --bg-active from card bg so custom colors propagate everywhere
      const hoverColor = lighten(savedCardBg, 0.06)
      const activeColor = lighten(savedCardBg, 0.12)
      root.style.setProperty('--bg-hover', hexToRgba(hoverColor, savedCardBgOpacity), 'important')
      root.style.setProperty('--bg-active', hexToRgba(activeColor, savedCardBgOpacity), 'important')
      parts.push(`--bg-hover:${hexToRgba(hoverColor, savedCardBgOpacity)}!important`)
      parts.push(`--bg-active:${hexToRgba(activeColor, savedCardBgOpacity)}!important`)
    }
    if (savedBorder) {
      root.style.setProperty('--border', hexToRgba(savedBorder, savedBorderOpacity), 'important')
      // Derive --border-glow from border color
      const glowColor = lighten(savedBorder, 0.15)
      root.style.setProperty('--border-glow', hexToRgba(glowColor, savedBorderOpacity), 'important')
      parts.push(`--border:${hexToRgba(savedBorder, savedBorderOpacity)}!important`)
      parts.push(`--border-glow:${hexToRgba(glowColor, savedBorderOpacity)}!important`)
    }

    if (parts.length > 0) {
      const existingEl = document.getElementById('phonon-color-overrides') as HTMLStyleElement | null
      if (existingEl) {
        existingEl.textContent = `:root{${parts.join(';')}}`
      } else {
        const styleEl = document.createElement('style')
        styleEl.id = 'phonon-color-overrides'
        styleEl.textContent = `:root{${parts.join(';')}}`
        document.head.appendChild(styleEl)
      }
    }

    const savedTextColor = localStorage.getItem('phonon-text-color')
    const savedTextBright = localStorage.getItem('phonon-text-bright-color')
    const savedTextDim = localStorage.getItem('phonon-text-dim-color')
    const savedTextDimOpacity = parseFloat(localStorage.getItem('phonon-text-dim-opacity') || '1')
    const savedPlaceholder = localStorage.getItem('phonon-placeholder-color')
    const savedPlaceholderOpacity = parseFloat(localStorage.getItem('phonon-placeholder-opacity') || '0.5')
    if (savedTextColor) root.style.setProperty('--text', savedTextColor, 'important')
    if (savedTextBright) root.style.setProperty('--text-bright', savedTextBright, 'important')
    if (savedTextDim) root.style.setProperty('--text-dim', hexToRgba(savedTextDim, savedTextDimOpacity), 'important')
    if (savedPlaceholder) root.style.setProperty('--placeholder-text', hexToRgba(savedPlaceholder, savedPlaceholderOpacity), 'important')

    const savedRadius = localStorage.getItem('phonon-album-art-radius')
    const savedBlur = localStorage.getItem('phonon-album-art-blur')
    if (savedRadius) document.documentElement.style.setProperty('--album-art-radius', `${savedRadius}px`)
    if (savedBlur) document.documentElement.style.setProperty('--album-art-blur', savedBlur === 'true' ? '1' : '0')

    // Use readStorage for validation + degradation (spec §12).
    const savedCss = readStorage('customCss')
    if (savedCss) {
      let el = document.getElementById('phonon-custom-css') as HTMLStyleElement | null
      if (!el) {
        el = document.createElement('style')
        el.id = 'phonon-custom-css'
        document.head.appendChild(el)
      }
      el.textContent = savedCss
    }

    const appEl = document.querySelector('.app') as HTMLElement | null
    if (appEl) {
      const bgEnabled = localStorage.getItem('phonon-bg-enabled') !== 'false'
      if (bgEnabled) {
        // Try Settings.tsx multi-image system first
        let applied = false
        try {
          const savedImages = localStorage.getItem('phonon-bg-images')
          const savedIndex = Number(localStorage.getItem('phonon-bg-image-index') || '0')
          if (savedImages) {
            const images: string[] = JSON.parse(savedImages)
            if (images.length > 0 && savedIndex >= 0 && savedIndex < images.length) {
              appEl.style.backgroundImage = `url("${images[savedIndex]}")`
              applied = true
            }
          }
        } catch { /* ignore */ }
        // Fallback to ExtensionsPanel single-image system
        if (!applied) {
          const savedData = localStorage.getItem('phonon-bg-image-data')
          const savedUrl = localStorage.getItem('phonon-bg-image-url')
          if (savedData) {
            appEl.style.backgroundImage = `url("${savedData}")`
          } else if (savedUrl) {
            appEl.style.backgroundImage = `url("${savedUrl}")`
          }
        }
      }
    }

    // readStorage clamps font-size to [10, 32] and degrades to 14 on corrupt
    // values (spec §12 — "启动校验 + 降级").
    const savedFontSize = readStorage('fontSize')
    if (savedFontSize) {
      document.documentElement.style.setProperty('--font-size', `${savedFontSize}px`)
      document.body.style.fontSize = `${savedFontSize}px`
    }

    const savedFontFamily = localStorage.getItem('phonon-font-family')
    if (savedFontFamily) {
      document.documentElement.style.setProperty('--sans', savedFontFamily)
    }

    // 初始化视觉增强模式：
    // - localStorage 仅用于"记住用户上次选了哪种模式"（visMode state + phonon-vis-mode 写入）
    // - 是否实际应用样式，完全取决于 React state `visActive`（默认关闭，不读存储）
    // - 为保证刷新后"开关显示关闭 && 效果也关闭"：初始化时永远不把 quality 写进 <html data-vis-mode>，
    //   只有 depthXX（维度/3D 全屏页本身的独立界面）才需要保留 data-vis-mode 用于它们自己的页面标识。
    const vm = localStorage.getItem('phonon-vis-mode')
    if (vm === 'cosmos') localStorage.setItem('phonon-vis-mode', 'quality')
    const norm = vm === 'cosmos' ? 'quality' : vm
    if (norm === 'depth3d' || norm === 'depth4d' || norm === 'depth5d') {
      document.documentElement.setAttribute('data-vis-mode', norm)
    } else {
      document.documentElement.removeAttribute('data-vis-mode')
    }
    document.documentElement.removeAttribute('data-vis-active')

    // 初始化自定义增强项到 <html data-vis-no-xxx>
    const CUSTOM_KEYS = ['glass', 'shadows', 'buttons', 'titlebar', 'nav', 'inputs', 'scrollbar', 'borders']
    try {
      const raw = localStorage.getItem('phonon-vis-custom')
      const arr = raw ? JSON.parse(raw) : CUSTOM_KEYS
      const set = new Set(Array.isArray(arr) ? arr : CUSTOM_KEYS)
      CUSTOM_KEYS.forEach(k => {
        if (set.has(k)) document.documentElement.removeAttribute(`data-vis-no-${k}`)
        else document.documentElement.setAttribute(`data-vis-no-${k}`, 'true')
      })
    } catch { /* ignore */ }
  }, [])

  // 视觉增强模式切换：
  // - 无论何时都同步写入 localStorage（"保存用户上次选择"要求）
  // - 是否把模式应用到 <html data-vis-mode="..."> 进而影响渲染？
  //   * 如果是 depth3d/4d/5d（维度/3D 全屏独立页）：始终同步 attribute（因为 3D 页本身需要识别）
  //   * 如果是 quality 模式：**仅当 visActive=true（总开关已打开）才同步 attribute**，否则只存 storage 不渲染，
  //     这样切换到质感提升不会在总开关关闭时意外"直接出现"效果
  const setVisMode = useCallback((mode: VisMode) => {
    setVisModeState(mode)
    if (mode === 'quality' || mode === 'depth3d' || mode === 'depth4d' || mode === 'depth5d') {
      localStorage.setItem('phonon-vis-mode', mode)
    } else {
      localStorage.setItem('phonon-vis-mode', 'none')
    }
    if (mode === 'depth3d' || mode === 'depth4d' || mode === 'depth5d') {
      document.documentElement.setAttribute('data-vis-mode', mode)
    } else if (mode === 'quality') {
      // 只有总开关开启时才应用 quality 到 HTML，保持"切换时不意外弹出"
      if (visActive) document.documentElement.setAttribute('data-vis-mode', 'quality')
      else document.documentElement.removeAttribute('data-vis-mode')
    } else {
      document.documentElement.removeAttribute('data-vis-mode')
    }
  }, [visActive])

  // 总开关：控制 data-vis-active 属性 + 同步 quality 模式的实际渲染（因为 quality 效果需要在总开关开时才生效）
  // 关闭视觉增强：退出 3D 全屏页（如果在其中），但不改变 visMode 存储/state（保持用户上次选择），只是移除所有会触发样式的 attribute
  const toggleVisActive = useCallback((on: boolean) => {
    setVisActive(on)
    if (on) {
      document.documentElement.setAttribute('data-vis-active', 'true')
      // 打开时：如果上次模式是 quality，现在才把它真正写到 HTML 上
      if (visMode === 'quality') document.documentElement.setAttribute('data-vis-mode', 'quality')
    } else {
      document.documentElement.removeAttribute('data-vis-active')
      // 关闭时：如果是 quality，也移除 data-vis-mode，确保效果完全消失（模式选择仍保留在 visMode 里）
      if (visMode === 'quality') document.documentElement.removeAttribute('data-vis-mode')
      // 关闭视觉增强 → 如果正在 3D 全屏页，退回到主播放器页（但不改 visMode，保留用户上次维度/模式选择）
      if (visMode === 'depth3d' || visMode === 'depth4d' || visMode === 'depth5d') {
        if (activeTab === 'depth3d') {
          setActiveTab('player')
        }
      }
    }
  }, [visMode, activeTab])

  // 进入维度页（深度空间）：关闭主界面 tab，切到 depth3d 全屏 tab
  const handleEnterDimension = useCallback((mode: Exclude<VisMode, 'none' | 'quality'>) => {
    if (mode === 'depth3d') {
      setActiveTab('depth3d')
      if (!visActive) {
        setVisActive(true)
        document.documentElement.setAttribute('data-vis-active', 'true')
      }
      setVisMode('depth3d')
    }
  }, [visActive, setVisMode])

  // ── 视觉升华插件（plugins/vis/vis_mode.js）────────────────────────
  // 插件自带全部 CSS（按钮 / 面板 / 质感模式 / 氛围光斑），宿主只负责
  // 注入/移除样式与驱动帧循环——这就是"视觉"的统一体系：内置 3D 维度页
  // 和插件质感效果共存，插件按同一契约即可扩展。
  // 加载失败（文件缺失/损坏）时自愈为禁用，避免渲染出无样式按钮。
  useEffect(() => {
    void syncVisPluginStyles(visPluginEnabled).then((ok) => {
      if (!ok && visPluginEnabled) {
        console.warn('[vis] 视觉升华插件加载失败，已自动禁用')
        setVisPluginEnabled(false)
      }
    })
  }, [visPluginEnabled])

  // 插件帧循环：质感模式激活期间运行，驱动插件的音频呼吸（--vis-* 变量）
  useEffect(() => {
    if (visActive && visMode === 'quality') startVisLoop()
    else stopVisLoop()
  }, [visActive, visMode])

  // Initialisation: run once after mount with a small delay so Tauri's
  // frontend callback table is fully populated before issuing any commands.
  // Commands are serialised (not Promise.all concurrent) to avoid callback id
  // race conditions with the Rust promise queue.
  const initialisedRef = useRef(false)
  const sessionLoadedRef = useRef(false)
  useEffect(() => {
    if (initialisedRef.current) return
    initialisedRef.current = true
    const timer = setTimeout(() => {
      // Sequential init — each await ensures previous callback is fully
      // resolved before registering the next, keeping the Tauri callback
      // table small and avoiding race-triggered "Couldn't find callback id".
      ;(async () => {
        try {
          await invoke('load_session')
          sessionLoadedRef.current = true
          triggerRefresh()

          // Sync backend volume mode to match frontend's localStorage preference
          const savedMode = localStorage.getItem('phonon-volume-mode') || 'Software'
          try { await invoke('set_volume_mode', { mode: savedMode }) } catch {}

          // Spectrum settings
          try {
            const [s, d, f] = await invoke<[number, number, number]>('get_spectrum_settings')
            setSpectrumSettings({ smoothing: s, decay: d, fps: f })
          } catch {}

          // Theme
          try {
            const t = await invoke<string>('get_theme')
            const savedPreset = localStorage.getItem('phonon-theme-preset')
            if (!savedPreset) {
              if (t === 'System') {
                const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches
                document.documentElement.setAttribute('data-theme', prefersDark ? 'dark' : 'light')
              } else {
                document.documentElement.setAttribute('data-theme', t.toLowerCase())
              }
            }
          } catch {}

          // Playback mode — re-apply from localStorage AFTER load_session,
          // because load_session reloads the (stale) session file and would
          // otherwise reset the in-memory mode. localStorage holds the
          // current user choice. On first app start (empty localStorage),
          // fall back to the Rust value (from session file).
          try {
            const savedMode = localStorage.getItem('phonon-playback-mode')
            if (savedMode) {
              await invoke('set_playback_mode', { mode: savedMode })
              setPlaybackModeState(savedMode)
            } else {
              const mode = await invoke<string>('get_playback_mode')
              setPlaybackModeState(mode)
              localStorage.setItem('phonon-playback-mode', mode)
            }
          } catch {}

          // Spectrum colorscheme
          try {
            const scheme = await invoke<string>('get_spectrum_colorscheme')
            if (!localStorage.getItem('phonon-spectrum-colorscheme')) {
              setSpectrumColorScheme(scheme)
            }
          } catch {}

          // Language
          try {
            const l = await invoke<string>('get_language')
            setLang(l as Lang)
            localStorage.setItem('phonon-language', l)
          } catch {}

          // Restore split layout widths
          const savedSidebar = localStorage.getItem('phonon-sidebar-width')
          if (savedSidebar) {
            const w = parseFloat(savedSidebar)
            if (!isNaN(w) && w >= 180 && w <= 400) setSidebarWidth(w)
          }
          const savedPlayerMain = localStorage.getItem('phonon-player-main-width')
          if (savedPlayerMain) {
            const w = parseFloat(savedPlayerMain)
            if (!isNaN(w) && w >= 200 && w <= 600) setPlayerMainWidth(w)
          }
        } catch {}
      })()
    }, 250)
    return () => clearTimeout(timer)
  }, [triggerRefresh])

  // Page unload cleanup (F5 refresh, HMR).
  // Rust side already handles playback stop + event suppression via
  // `on_page_load(Started)` + `prepare_for_reload` (called from main.tsx).
  // JS-side callback-missing warnings are suppressed by the
  // phonon-callback-guard plugin's initialization script (runs before any
  // page code). Here we do app-internal cleanup: flag disposal, clear timers.
  useEffect(() => {
    const handleBeforeUnload = () => {
      appDisposedRef.current = true
      for (const t of openTimersRef.current) {
        try { clearInterval(t as any) } catch {}
        try { clearTimeout(t as any) } catch {}
      }
      openTimersRef.current.clear()
    }
    window.addEventListener('beforeunload', handleBeforeUnload)
    return () => window.removeEventListener('beforeunload', handleBeforeUnload)
  }, [])

  // Load album art when current track changes
  useEffect(() => {
    let cancelled = false
    let url: string | null = null

    const loadArt = async () => {
      setCoverUrl(null)
      if (!playback.current_track) return

      try {
        const art = await invoke<AlbumArtResponse | null>('get_album_art', {
          path: playback.current_track,
        })
        if (cancelled || !art) return

        const blob = new Blob([new Uint8Array(art.data)], { type: art.mime_type })
        url = URL.createObjectURL(blob)
        setCoverUrl(url)
      } catch (_) {
        /* no art available */
      }
    }

    loadArt()
    return () => {
      cancelled = true
      if (url) URL.revokeObjectURL(url)
    }
  }, [playback.current_track])

  // Save session periodically (every 15s).
  // Only starts after load_session completes to avoid callback id contention.
  // Do NOT call invoke() on unmount: during HMR/reload the frontend callback
  // table is torn down before cleanup, producing "Couldn't find callback id".
  // Session is also saved on every major state change in Rust.
  useEffect(() => {
    let interval: ReturnType<typeof setInterval> | null = null
    const startTimer = setInterval(() => {
      if (!sessionLoadedRef.current) return
      clearInterval(startTimer)
      unregisterTimer(startTimer)
      interval = setInterval(() => {
        if (appDisposedRef.current) return
        invoke('save_session').catch(() => {})
      }, 15000)
      registerTimer(interval)
    }, 500)
    registerTimer(startTimer)
    return () => {
      clearInterval(startTimer)
      unregisterTimer(startTimer)
      if (interval) {
        clearInterval(interval)
        unregisterTimer(interval)
      }
    }
  }, [])

  // Desktop lyrics: cache lyrics data when loaded, and emit to desktop-lyrics window
  useEffect(() => {
    if (!playback.current_track) {
      lyricsDataRef.current = null
      // Notify desktop lyrics window to clear immediately
      if (desktopLyricsOpen) {
        safeEmitTo('desktop-lyrics', 'desktop-lyrics-update', {
          lines: [],
          position_secs: 0,
          track_title: '',
        })
      }
      return
    }

    // IMMEDIATELY clear the cached lyrics and notify the desktop lyrics
    // window with empty lines + new track title. This prevents the previous
    // song's lyrics from being shown during the async get_lyrics invoke
    // (the PlaybackProgress event resets position to 0, which would
    // otherwise highlight line 0 of the OLD lyrics).
    lyricsDataRef.current = null
    const trackTitle = playback.current_track.replace(/^.*[\\/]/, '').replace(/\.[^/.]+$/, '')
    resetProgress()
    if (desktopLyricsOpen) {
      safeEmitTo('desktop-lyrics', 'desktop-lyrics-update', {
        lines: [],
        position_secs: 0,
        track_title: trackTitle,
      })
    }

    // Then fetch the actual lyrics for the new track asynchronously
    const fetchPath = playback.current_track
    lyricsFetchPathRef.current = fetchPath
    safeInvoke<{ type: string; lines?: LyricLineJson[] }>('get_lyrics', { path: fetchPath })
      .then((data) => {
        if (!data) return // Vite-only
        // Guard: track may have changed again while we were fetching.
        // Discard stale results to avoid overwriting the new song's state.
        if (lyricsFetchPathRef.current !== fetchPath) return
        if (data.type === 'Synced' && data.lines && data.lines.length > 0) {
          lyricsDataRef.current = { lines: data.lines }
        } else {
          lyricsDataRef.current = null
        }
        if (desktopLyricsOpen) {
          safeEmitTo('desktop-lyrics', 'desktop-lyrics-update', {
            lines: lyricsDataRef.current?.lines || [],
            position_secs: getProgress().position,
            track_title: trackTitle,
          })
        }
      })
      .catch(() => {})
  }, [playback.current_track, desktopLyricsOpen])

  // ── Smart Effect auto-switching ──
  // When a new track loads, read its genre metadata and automatically
  // select the best Smart Effect preset. Disabled when the user has
  // turned off auto mode (checked via localStorage).
  // Per user request: "根据不同的曲风、流派、设备自动适配最佳效果".
  const smartEffectAutoRef = useRef(localStorage.getItem('phonon-smart-effect-auto') === 'true')
  useEffect(() => {
    const handler = (e: Event) => {
      smartEffectAutoRef.current = (e as CustomEvent).detail as boolean
    }
    window.addEventListener('smart-effect-auto-change', handler)
    return () => window.removeEventListener('smart-effect-auto-change', handler)
  }, [])

  useEffect(() => {
    if (!playback.current_track) return
    if (!smartEffectAutoRef.current) return
    // Map genre string → Smart Effect preset
    const genreToMode = (genre: string): string => {
      const g = genre.toLowerCase()
      if (g.includes('pop') || g.includes('流行')) return 'pop'
      if (g.includes('rock') || g.includes('摇滚')) return 'rock'
      if (g.includes('jazz') || g.includes('爵士')) return 'jazz'
      if (g.includes('classical') || g.includes('古典') || g.includes('symphony')) return 'classical'
      if (g.includes('electronic') || g.includes('edm') || g.includes('dance') || g.includes('house') || g.includes('techno') || g.includes('电子')) return 'electronic'
      if (g.includes('vocal') || g.includes('folk') || g.includes('acoustic') || g.includes('人声') || g.includes('民谣')) return 'vocal'
      if (g.includes('hip') || g.includes('rap') || g.includes('r&b') || g.includes('soul') || g.includes('bass')) return 'bass_boost'
      return 'pop' // default fallback — safe, subtle enhancement
    }
    const trackPath = playback.current_track
    safeInvoke<{ genre?: string }>('get_track_metadata', { path: trackPath })
      .then((meta) => {
        if (!meta) return // Vite-only or backend error — silently no-op
        if (!smartEffectAutoRef.current) return // may have been disabled
        const genre = meta.genre || ''
        const mode = genreToMode(genre)
        safeInvoke('set_smart_effect_mode', { mode })
        // Notify PluginManager so its UI stays in sync
        window.dispatchEvent(new CustomEvent('smart-effect-mode-change', { detail: mode }))
      })
      .catch(() => {
        // Metadata unavailable — use default preset
        if (smartEffectAutoRef.current) {
          safeInvoke('set_smart_effect_mode', { mode: 'pop' })
          window.dispatchEvent(new CustomEvent('smart-effect-mode-change', { detail: 'pop' }))
        }
      })
  }, [playback.current_track])

  // Record play history when a new track starts playing (only if it's in the library).
  useEffect(() => {
    const path = playback.current_track
    if (!path) return
    if (lastRecordedTrackRef.current === path) return
    // Only record when playback is actually playing — avoids counting paused/skipped tracks.
    if (playback.state !== 'Playing') return
    lastRecordedTrackRef.current = path
    ;(async () => {
      try {
        const track = await invoke<{ id: number } | null>('library_get_track_by_path', { path })
        if (track?.id) {
          await invoke('library_record_play', { id: track.id })
        }
      } catch {
        /* not in library or backend error — silently skip */
      }
    })()
  }, [playback.current_track, playback.state])

  // Desktop lyrics: create/destroy window (via Rust for correct URL)
  useEffect(() => {
    if (desktopLyricsOpen) {
      safeInvoke('open_desktop_lyrics').then((ok) => {
        if (ok === undefined) return // Vite-only — no backend
        // After window is created, send current lyrics data with a short delay
        // so the desktop lyrics window has time to set up its event listeners
        setTimeout(() => {
          const lines = lyricsDataRef.current?.lines
          const trackTitle = playback.current_track
            ? playback.current_track.replace(/^.*[\\/]/, '').replace(/\.[^/.]+$/, '')
            : ''
          safeEmitTo('desktop-lyrics', 'desktop-lyrics-update', {
            lines: lines || [],
            position_secs: getProgress().position,
            track_title: trackTitle,
          })
        }, 200)
      }).catch((e) => {
        console.error('[desktop-lyrics] Failed to open:', e)
        setDesktopLyricsOpen(false)
      })
    } else {
      safeInvoke('close_desktop_lyrics').catch((e) => {
        console.error('[desktop-lyrics] Failed to close:', e)
      })
    }
  }, [desktopLyricsOpen])

  // Listen for desktop lyrics window close event from Rust
  useEffect(() => {
    let unlistenFn: (() => void) | null = null
    safeListen('desktop-lyrics-closed', () => {
      setDesktopLyricsOpen(false)
    }).then((fn) => { unlistenFn = fn }).catch(() => {})
    return () => { if (unlistenFn) { try { unlistenFn() } catch {} } }
  }, [])

  // Listen for folder-changed events (external file additions/removals in synced folders).
  // The event payload is a file path hint. We find which playlist is bound to the
  // folder containing that path and call sync_folder_for_playlist for each match.
  // Debounce 2s so a batch of file operations only triggers one sync.
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null
    let pendingPaths: Set<string> = new Set()
    let unlistenFn: (() => void) | null = null
    safeListen<string>('folder-changed', (event) => {
      const hint = event.payload
      if (hint) pendingPaths.add(hint)
      if (timer) clearTimeout(timer)
      timer = setTimeout(async () => {
        timer = null
        if (appDisposedRef.current) return
        const paths = Array.from(pendingPaths)
        pendingPaths.clear()
        try {
          // Get all synced folder bindings
          const synced = await safeInvoke<Record<string, string>>('get_synced_folders')
          if (!synced) return
          // For each binding, check if any changed path is under its folder
          let changed = false
          for (const [playlist, folder] of Object.entries(synced)) {
            const matches = paths.some(p => p.startsWith(folder))
            if (matches) {
              const result = await safeInvoke<{ added: number; removed: number }>(
                'sync_folder_for_playlist', { playlist }
              )
              if (result && (result.added > 0 || result.removed > 0)) changed = true
            }
          }
          if (changed) triggerRefresh()
        } catch { /* ignore */ }
      }, 2000)
    }).then((fn) => { unlistenFn = fn }).catch(() => {})
    return () => {
      if (timer) clearTimeout(timer)
      if (unlistenFn) { try { unlistenFn() } catch {} }
    }
  }, [triggerRefresh])

  // Listen for tray menu "toggle desktop lyrics" click
  useEffect(() => {
    let unlistenFn: (() => void) | null = null
    safeListen('toggle-desktop-lyrics', () => {
      setDesktopLyricsOpen(prev => !prev)
    }).then((fn) => { unlistenFn = fn }).catch(() => {})
    return () => { if (unlistenFn) { try { unlistenFn() } catch {} } }
  }, [])

  // NOTE: Thumbnail toolbar button playback is handled 100% reliably in the
  // Rust WndProc side (phonon_wndproc directly calls engine methods) — which
  // also emits tb-prev / tb-play-pause / tb-next convenience events. We
  // deliberately do NOT re-invoke Rust commands from those events here: doing
  // so caused a double-execute bug where one click ran the engine twice
  // (Playing → Paused → Playing), making buttons appear "unresponsive".
  // If any UI component needs to react to these events for visual updates
  // only (without re-triggering playback), attach listeners elsewhere.

  // Update window title (taskbar thumbnail header, Alt+Tab, tooltip) to show
  // current song name. Uses the Rust set_window_title command which calls
  // SetWindowTextW directly on the ROOT HWND — Tauri's built-in setTitle()
  // on some Windows builds only updates the WebView document title and the
  // taskbar doesn't pick it up.
  useEffect(() => {
    let cancelled = false
    let unlistenFn: (() => void) | null = null

    // When the user clicks the taskbar play button with an empty queue,
    // auto-open a file dialog so they can pick songs immediately.
    safeListen('taskbar-play-empty-queue', async () => {
      addToast('播放队列为空，请选择音频文件', 'info')
      try {
        const selected = await openDialog({
          multiple: true,
          filters: [{ name: 'Audio', extensions: ['mp3', 'flac', 'wav', 'ogg', 'aac', 'm4a', 'wma', 'opus', 'aiff', 'ape', 'wv', 'cue'] }],
        })
        if (!selected) return
        const paths = Array.isArray(selected) ? selected : [selected]
        const result = await invoke<{ added: number; duplicates: number }>('add_to_queue', { paths })
        if (result.added > 0) {
          addToast(`已添加 ${result.added} 首曲目`)
          // Auto-start playback after adding
          await invoke('play_index', { index: 0 })
        }
      } catch (e) {
        addToast(String(e), 'error')
      }
    }).then((fn) => { unlistenFn = fn }).catch(() => {})

    const updateTitle = async () => {
      try {
        let title = 'Phonon'
        if ((playback.state === 'Playing' || playback.state === 'Paused') && playback.current_track) {
          const filename = playback.current_track.split(/[\\/]/).pop() || ''
          const songName = filename.replace(/\.[^.]+$/, '')
          if (songName) title = `Phonon - ${songName}`
        }
        // Rust-side SetWindowTextW writes directly to the root HWND caption,
        // which the taskbar thumbnail/tooltip/Alt+Tab actually read.
        if (!cancelled) {
          await safeInvoke('set_window_title', { title })
        }
      } catch {
        // Fallback: if command fails (e.g. old build / macOS/Linux), try
        // Tauri's built-in setTitle as best effort.
        try {
          if (!cancelled) {
            const win = safeGetCurrentWindow()
            if (win) await win.setTitle(titleFallback(playback))
          }
        } catch { /* ignore */ }
      }
    }
    updateTitle()

    return () => {
      cancelled = true
      if (unlistenFn) { try { unlistenFn() } catch {} }
    }
  }, [playback.state, playback.current_track])

  function titleFallback(p: PlaybackState): string {
    if ((p.state === 'Playing' || p.state === 'Paused') && p.current_track) {
      const filename = p.current_track.split(/[\\/]/).pop() || ''
      const songName = filename.replace(/\.[^.]+$/, '')
      if (songName) return `Phonon - ${songName}`
    }
    return 'Phonon'
  }

  // Re-enable plugins that were enabled in the previous session
  useEffect(() => {
    try {
      const raw = localStorage.getItem('phonon-enabled-plugins')
      if (!raw) return
      const enabled: { file_path: string; plugin_type: string }[] = JSON.parse(raw)
      const validPlugins: { file_path: string; plugin_type: string }[] = []
      const failedPlugins: string[] = []
      const skippedBuiltins: string[] = []
      const promises: Promise<void>[] = []
      for (const { file_path, plugin_type } of enabled) {
        // Builtin 插件是 Rust 原生 DSP，静态加载、永远 enabled；
        // 不走 Wasm runtime 的 enable_plugin，也不在 plugins HashMap 里，跳过避免报错。
        if (file_path.startsWith('builtin:')) {
          skippedBuiltins.push(file_path)
          continue
        }
        if (plugin_type === 'TimeStretch') {
          promises.push(
            invoke('load_time_stretch_plugin', { filePath: file_path })
              .then(() => { validPlugins.push({ file_path, plugin_type }) })
              .catch((e) => {
                console.error('[auto-load] Failed to load TimeStretch plugin:', file_path, e)
                failedPlugins.push(file_path)
              })
          )
        } else {
          promises.push(
            invoke('enable_plugin', { id: file_path })
              .then(() => { validPlugins.push({ file_path, plugin_type }) })
              .catch((e) => {
                console.error('[auto-load] Failed to enable plugin:', file_path, e)
                failedPlugins.push(file_path)
              })
          )
        }
      }
      // Clean up failed plugins & stale Builtin entries from localStorage after ALL promises resolve
      Promise.allSettled(promises).then(() => {
        const toRemove = [...failedPlugins, ...skippedBuiltins]
        if (toRemove.length > 0) {
          try {
            const currentRaw = localStorage.getItem('phonon-enabled-plugins')
            if (currentRaw) {
              const current: { file_path: string; plugin_type: string }[] = JSON.parse(currentRaw)
              const filtered = current.filter(p => !toRemove.includes(p.file_path))
              localStorage.setItem('phonon-enabled-plugins', JSON.stringify(filtered))
            }
          } catch { /* ignore */ }
        }
      })
    } catch { /* ignore malformed localStorage */ }
  }, [])

  // Listen for window maximize/restore to adjust toast position
  // Also save main window position on resize/move
  useEffect(() => {
    const win = safeGetCurrentWindow()
    let unlistenMax: (() => void) | null = null
    let unlistenRes: (() => void) | null = null
    let unlistenResize: (() => void) | null = null
    let unlistenMoved: (() => void) | null = null
    let saveTimer: ReturnType<typeof setTimeout> | null = null
    let disposed = false
    // Throttle flag: skip debounce work if the last event fired < 80ms ago
    // (resize/move fires ~60 events/sec during drag; debouncing at the
    // entry avoids creating+clearing N timers per second.)
    let lastEv = 0

    // Track the last saved (width, height, x, y) to skip redundant saves.
    // Users usually drag 30-100 pixels per drag cycle; comparing against
    // the last saved lets us skip the 900ms write entirely when nothing
    // actually changed from the prior save.
    let lastSaved: { x: number; y: number; w: number; h: number } | null = null

    // Cache whether the window is maximized so savePosition() never has to
    // call win.isMaximized() (which is a cross-boundary invoke — #2 source
    // of pending callbacks on window resize). Updated via event listeners.
    let localMaximized = false

    const savePosition = () => {
      const now = performance.now()
      if (now - lastEv < 80) {
        if (saveTimer) clearTimeout(saveTimer)
      }
      lastEv = now
      if (saveTimer) clearTimeout(saveTimer)
      // 900ms debounce: save once user STOPS dragging for 900ms
      saveTimer = setTimeout(() => {
        if (disposed || appDisposedRef.current) return
        if (localMaximized) return  // client-side cache — NO invoke call
        if (!win) return
        // Client-side innerSize (cheap, same-thread DOM property) for
        // width/height instead of the cross-boundary win.innerSize().
        const w = window.innerWidth
        const h = window.innerHeight
        // Skip entirely if dimensions haven't changed since last save —
        // this avoids a redundant outerPosition() invoke when the resize
        // event fired but the window snapped back to the same size.
        if (lastSaved && lastSaved.w === w && lastSaved.h === h) {
          // Dimensions same — only position may have changed (move, not resize)
          // Still need outerPosition to check, but this is already debounced.
        }
        // outerPosition is a native coordinate so we must query once via
        // Tauri (it IS 1 invoke, but the debounce already guarantees at
        // most 1 invoke per drag cycle — acceptable).
        win.outerPosition().then((pos) => {
          if (disposed || appDisposedRef.current) return
          if (lastSaved &&
            Math.abs(lastSaved.x - pos.x) < 2 &&
            Math.abs(lastSaved.y - pos.y) < 2 &&
            Math.abs(lastSaved.w - w) < 2 &&
            Math.abs(lastSaved.h - h) < 2) {
            return // skip redundant write — nothing meaningful changed
          }
          lastSaved = { x: pos.x, y: pos.y, w, h }
          invoke('save_main_window_position', {
            x: pos.x,
            y: pos.y,
            width: w,
            height: h,
            maximized: localMaximized,
          }).catch(() => {})
        }).catch(() => {})
      }, 900)
    }

    const setupListeners = async () => {
      if (!win) return // Vite-only — skip window listeners entirely
      try {
        const maximized = await win.isMaximized()
        if (disposed) return // unmounted while awaiting
        setIsMaximized(maximized)
        localMaximized = maximized

        unlistenMax = await safeListen('tauri://maximized', () => {
          if (disposed) return
          setIsMaximized(true)
          localMaximized = true
          invoke('save_main_window_position', {
            x: lastSaved?.x ?? 0,
            y: lastSaved?.y ?? 0,
            width: lastSaved?.w ?? window.innerWidth,
            height: lastSaved?.h ?? window.innerHeight,
            maximized: true,
          }).catch(() => {})
        })
        if (disposed) { unlistenMax?.(); return }
        unlistenRes = await safeListen('tauri://restored', () => {
          if (disposed) return
          setIsMaximized(false)
          localMaximized = false
        })
        if (disposed) { unlistenRes?.(); return }

        // Save window position on resize/move.
        // Wrap savePosition in a disposed check so events that fire
        // between unmount and listener teardown don't trigger invokes
        // (the #1 cause of "Couldn't find callback id" warnings during
        // minimize / HMR reload).
        const onResizeMove = () => { if (!disposed) savePosition() }
        const r1 = await win.onResized(onResizeMove)
        if (disposed) { r1(); return }
        unlistenResize = r1
        const r2 = await win.onMoved(onResizeMove)
        if (disposed) { r2(); return }
        unlistenMoved = r2
      } catch (_) { /* ignore in dev / non-tauri env */ }
    }
    setupListeners()

    return () => {
      disposed = true
      unlistenMax?.()
      unlistenRes?.()
      unlistenResize?.()
      unlistenMoved?.()
      if (saveTimer) clearTimeout(saveTimer)
    }
  }, [])

  // Register global shortcuts on startup (from localStorage)
  // Listens for 'shortcuts-changed' event to re-register when settings update.
  useEffect(() => {
    let cancelled = false
    const lastTriggerRef: Record<string, number> = {}
    const DEBOUNCE_MS = 400

    const loadAndRegister = async () => {
      try {
        const raw = localStorage.getItem('phonon_shortcuts')
        if (!raw || cancelled) return
        const bindings: { keys: string; action: string }[] = JSON.parse(raw)
        await unregisterAll().catch(() => {})
        if (cancelled) return
        for (const b of bindings) {
          if (cancelled) return
          if (!b.keys) continue
          try {
            await register(b.keys, (event) => {
              if (event.state !== 'Pressed') return
              const now = Date.now()
              const last = lastTriggerRef[b.action] || 0
              if (now - last < DEBOUNCE_MS) return
              lastTriggerRef[b.action] = now
              switch (b.action) {
                case 'playPause':
                  invoke('toggle_play_pause').catch(() => {})
                  break
                case 'next':
                  invoke('next').catch(() => {})
                  break
                case 'previous':
                  invoke('previous').catch(() => {})
                  break
                case 'stop':
                  invoke('stop').catch(() => {})
                  break
              }
            })
          } catch (e) {
            console.error('[shortcut] register failed:', b.keys, e)
          }
        }
      } catch (e) {
        console.error('[shortcut] loadAndRegister failed:', e)
      }
    }

    const onShortcutsChanged = () => { loadAndRegister() }
    window.addEventListener('shortcuts-changed', onShortcutsChanged)

    loadAndRegister()
    return () => {
      cancelled = true
      window.removeEventListener('shortcuts-changed', onShortcutsChanged)
      unregisterAll().catch(() => {})
    }
  }, [])

  // ── Unified event listener ──────────────────────────────────
  useEffect(() => {
    let disposed = false
    refreshPlayback()
    // Previously this polled every 500ms, which generated ~2 callback ids
    // per second — HMR or any page reload made those pending callbacks
    // produce "Couldn't find callback id" warnings in the console.
    // Now we rely on PlaybackProgress events (emitted from Rust every ~50ms)
    // for position/duration, and only poll the full state lightly every
    // 3000ms to catch state transitions that don't emit events (e.g.
    // queue_length, speed, idle state recovery).
    const interval = setInterval(() => {
      if (!disposed && !appDisposedRef.current) refreshPlayback()
    }, 3000)
    registerTimer(interval)

    // Rust pushes an immediate engine snapshot after every successful taskbar
    // thumbnail action (prev/toggle/next). This eliminates the ~0–3s lag of
    // the polling loop above: the user sees the new song title and play state
    // instantly, instead of "the music started but the UI still shows the
    // previous song". The snapshot has the exact same shape as the poller.
    let unlistenSnapshotFn: (() => void) | null = null
    safeListen<PlaybackState>('playback-state-snapshot', (ev) => {
      const payload = ev.payload
      if (!payload || typeof payload !== 'object') return
      setPlayback((prev) => ({
        ...payload,
        // Polled duration might already be populated (e.g. same song re-toggled)
        duration_secs: payload.duration_secs ?? prev.duration_secs,
      }))
    }).then((fn) => { unlistenSnapshotFn = fn }).catch(() => {})

    let unlistenFn: (() => void) | null = null
    safeListen<AppEventPayload>('app-event', (event) => {
      const { payload } = event
      switch (payload.type) {
        case 'PlaybackProgress':
          // ~20 Hz stream goes through the progress bus so the whole tree
          // stops re-rendering per tick; App state only tracks the rarely
          // changing duration.
          publishProgress(payload.position_secs ?? null, payload.duration_secs ?? null)
          setPlayback((prev) =>
            payload.duration_secs != null && payload.duration_secs !== prev.duration_secs
              ? { ...prev, duration_secs: payload.duration_secs }
              : prev,
          )
          break
        case 'DeviceHotplug':
          // Only show notification if enabled
          safeInvoke<boolean>('get_hotplug_notifications').then((enabled) => {
            if (enabled === true) {
              addToast(
                payload.event === 'connect'
                  ? `${t('toast.deviceConnected')}: ${payload.device_name}`
                  : `${t('toast.deviceDisconnected')}: ${payload.device_name}`,
                payload.event === 'disconnect' ? 'warn' : 'info',
              )
            }
          }).catch(() => {
            addToast(
              payload.event === 'connect'
                ? `${t('toast.deviceConnected')}: ${payload.device_name}`
                : `${t('toast.deviceDisconnected')}: ${payload.device_name}`,
              payload.event === 'disconnect' ? 'warn' : 'info',
            )
          })
          if (payload.event === 'disconnect') {
            setPlayback((prev) => ({ ...prev, state: 'Stopped' }))
            refreshPlayback()
          }
          break
        case 'SpectrumData': {
          // Fan out through the spectrum bus (CustomEvent + shared ref) —
          // never through React state, which would re-render the whole App
          // at ~40 Hz. Consumers (Spectrum canvas, EQ panel) draw via rAF.
          const vals = payload.values ?? []
          publishSpectrumBands(vals)
          // Forward spectrum data to desktop lyrics window if open
          if (desktopLyricsOpenRef.current) {
            safeEmitTo('desktop-lyrics', 'spectrum-data', { values: vals })
          }
          break
        }
        case 'AudioFeaturesData': {
          // Broadcast rich features via CustomEvent so VisualizerPage can read them
          // directly without prop-drilling or extra re-renders on every 40 Hz frame.
          const detail = {
            spectrum: payload.spectrum ?? [],
            waveform: payload.waveform ?? [],
            rms: payload.rms ?? 0,
            peak: payload.peak ?? 0,
            spectral_centroid_hz: payload.spectral_centroid_hz ?? 0,
            onset: payload.onset ?? 0,
            beat: !!payload.beat,
            bpm: payload.bpm ?? 0,
            chroma: payload.chroma ?? [0,0,0,0,0,0,0,0,0,0,0,0],
            sample_rate: payload.sample_rate ?? 44100,
            channels: payload.channels ?? 2,
          }
          window.dispatchEvent(new CustomEvent('audio-features', { detail }))
          // Also forward to desktop lyrics window if open (same payload shape)
          if (desktopLyricsOpenRef.current) {
            safeEmitTo('desktop-lyrics', 'audio-features', detail)
          }
          break
        }
        case 'PlaybackStateChanged':
          setPlayback((prev) => ({ ...prev, state: payload.state ?? prev.state }))
          break
        case 'PluginStatus':
          addToast(
            t('plugins.status').replace('{id}', payload.id || '?').replace('{status}', payload.status || ''),
            payload.status === 'error' ? 'error' : 'info',
          )
          break
        case 'PluginUserEvent':
          // 分发到 window 上的自定义事件，子组件 subscribe：
          //   window.addEventListener('plugin-user-event', (e: CustomEvent) => console.log(e.detail.name, e.detail.payload))
          window.dispatchEvent(new CustomEvent('plugin-user-event', {
            detail: { name: payload.name, payload: payload.payload },
          }))
          break
      }
    }).then((fn) => { unlistenFn = fn }).catch(() => {})

    return () => {
      clearInterval(interval)
      unregisterTimer(interval)
      if (unlistenFn) { try { unlistenFn() } catch {} }
      if (unlistenSnapshotFn) { try { unlistenSnapshotFn() } catch {} }
    }
  }, [refreshPlayback])

  // ── 音量初始化 & 硬件音量同步 ──
  useEffect(() => {
    let cancelled = false
    const timer = setTimeout(() => {
      invoke<number>('get_volume_level').then((l) => {
        if (cancelled) return
        const backendVol = Math.round(l * 100)
        if (backendVol === 100) {
          const saved = localStorage.getItem('phonon-volume')
          if (saved) {
            const localVol = Number(saved)
            setVolume(localVol)
            invoke('set_volume', { level: localVol / 100 }).catch(() => {})
            return
          }
        }
        setVolume(backendVol)
        localStorage.setItem('phonon-volume', String(backendVol))
      }).catch(() => {})
    }, 80)
    return () => { cancelled = true; clearTimeout(timer) }
  }, [])

  // 硬件音量变化同步
  useEffect(() => {
    let unlistenFn: (() => void) | null = null
    safeListen<number>('hw-volume', (event) => {
      const v = Math.round(event.payload * 100)
      setVolume(v)
      localStorage.setItem('phonon-volume', String(v))
    }).then((fn) => { unlistenFn = fn }).catch(() => {})
    return () => { if (unlistenFn) { try { unlistenFn() } catch {} } }
  }, [])

  // Listen for visual enhancement plugin enable/disable changes.
  // 当插件被禁用（总开关关闭）时，必须连带关闭所有视觉增强效果，
  // 否则 data-vis-mode 属性会残留在 <html> 上，界面仍保持提升质感。
  useEffect(() => {
    const handler = () => {
      try {
        const raw = localStorage.getItem('phonon-enabled-vis-scripts')
        const arr = raw ? JSON.parse(raw) : []
        const enabled = Array.isArray(arr) && arr.includes('vis_mode.js')
        setVisPluginEnabled(enabled)
        if (!enabled) {
          // 总开关关闭 → 一键关闭所有视觉增强，恢复默认界面
          setVisActive(false)
          document.documentElement.removeAttribute('data-vis-active')
          setVisMode('none')
          // 若正停留在可视化页，跳回扩展页
          if (activeTab === 'visualizer') {
            setActiveTab('extensions')
          }
        }
      } catch { /* ignore */ }
    }
    window.addEventListener('vis-scripts-enabled-changed', handler)
    return () => window.removeEventListener('vis-scripts-enabled-changed', handler)
  }, [activeTab, setVisMode])

  const tabs: { id: Tab; label: string }[] = [
    { id: 'player', label: t('tab.player') },
    { id: 'devices', label: t('tab.devices') },
    { id: 'dsp', label: t('tab.dsp') },
    { id: 'extensions', label: t('tab.extensions') },
    { id: 'settings', label: t('tab.settings') },
  ]

  return (
    <I18nContext.Provider value={{ lang, setLang, t }}>
    <>
    <div className="app" data-active-tab={activeTab}>
      <TitleBar currentTrack={playback.current_track} />

      {/* Toast notifications */}
      <div className={`toast-container ${isMaximized ? 'toast-maximized' : ''}`}>
        {toasts.map((t) => (
          <div key={t.id} className={`toast toast-${t.kind}`}>
            {t.message}
          </div>
        ))}
      </div>

      <div className="tabs">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            data-tab={tab.id}
            className={`tab ${activeTab === tab.id ? 'active' : ''}`}
            onClick={() => setActiveTab(tab.id)}
            style={{ textAlign: 'left' }}
          >
            {tab.label}
          </button>
        ))}
        {visPluginEnabled && (
          <VisModeButton
            enabled={visPluginEnabled}
            active={visActive}
            mode={visMode}
            onToggle={toggleVisActive}
            onModeChange={setVisMode}
            onEnterDimension={handleEnterDimension}
          />
        )}
      </div>

      <div className="content">
        {activeTab === 'depth3d' && (
          <Suspense fallback={<div style={{ padding: 40, opacity: 0.5 }}>…</div>}>
            <Depth3DPage
              onClose={() => setActiveTab('player')}
              playback={playback}
              volume={volume}
              onVolumeChange={handleVolume}
              coverUrl={coverUrl}
              visActive={visActive}
              visMode={visMode}
              onToggleVisActive={toggleVisActive}
              onVisModeChange={setVisMode}
              onEnterDimension={handleEnterDimension}
            />
          </Suspense>
        )}
        {activeTab === 'player' && (
          <PlayerView
            playback={playback}
            onUpdate={refreshPlayback}
            refreshKey={refreshKey}
            triggerRefresh={triggerRefresh}
            spectrumActive={playback.state === 'Playing'}
            spectrumSettings={spectrumSettings}
            spectrumColorScheme={spectrumColorScheme}
            customSpectrumColors={customSpectrumColors}
            addToast={addToast}
            sidebarWidth={sidebarWidth}
            playerMainWidth={playerMainWidth}
            startDrag={startDrag}
            sidebarRef={sidebarRef}
            playerMainRef={playerMainRef}
          />
        )}
        {activeTab === 'devices' && <DeviceSelector />}
        {activeTab === 'dsp' && <DspPanel onNavigate={(tab) => setActiveTab(tab as Tab)} colorScheme={spectrumColorScheme} customColors={customSpectrumColors} addToast={addToast} />}
        {activeTab === 'extensions' && <ExtensionsPanel addToast={addToast} onNavigate={(tab) => setActiveTab(tab)} />}
        {activeTab === 'visualizer' && (
          <VisualizerPage
            playbackState={playback.state}
            onClose={() => setActiveTab('extensions')}
          />
        )}
        {activeTab === 'settings' && <Settings volumeMode={volumeMode} onVolumeModeChange={setVolumeMode} addToast={addToast} onSpectrumColorSchemeChange={(scheme, customColors) => { setSpectrumColorScheme(scheme); if (customColors) setCustomSpectrumColors(customColors) }} onSpectrumSettingsChange={setSpectrumSettings} onNavigate={(tab) => setActiveTab(tab as Tab)} />}
        
      </div>

      {/* Draggable split bar between content and player bar */}
      <PlayerBar playback={playback} onUpdate={refreshPlayback} volume={volume} onVolumeChange={handleVolume} volumeMode={volumeMode} onVolumeModeChange={setVolumeMode} playbackMode={playbackMode} setPlaybackMode={setPlaybackMode} addToast={addToast} coverUrl={coverUrl} onOpenLyrics={() => setShowLyricsPage(true)} onToggleDesktopLyrics={() => setDesktopLyricsOpen(!desktopLyricsOpen)} desktopLyricsOpen={desktopLyricsOpen} />
    </div>

    {/* ── EULA 首次启动弹窗（§14 合规） ─────────────────────── */}
    {showEulaModal && (
      <div style={{
        position: 'fixed', inset: 0, zIndex: 9999,
        background: 'rgba(0,0,0,0.75)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        backdropFilter: 'blur(6px)',
      }}>
        <div style={{
          width: 'min(560px, 92vw)',
          background: 'var(--card-bg)',
          border: '1px solid var(--border)',
          borderRadius: 12,
          padding: '28px 32px',
          boxShadow: '0 12px 48px rgba(0,0,0,0.5)',
        }}>
          <h2 style={{
            margin: '0 0 12px',
            color: 'var(--accent)',
            fontSize: 'var(--text-xl)',
          }}>
            {t('settings.about.eula.title')}
          </h2>
          <p style={{
            margin: '0 0 20px',
            color: 'var(--text-dim)',
            fontSize: 'var(--text-sm)',
            lineHeight: 1.7,
            whiteSpace: 'pre-wrap',
          }}>
            {t('settings.about.eula.text')}
          </p>
          <label style={{
            display: 'flex', alignItems: 'flex-start', gap: 10,
            cursor: 'pointer', userSelect: 'none',
            marginBottom: 24,
          }}>
            <input
              type="checkbox"
              checked={eulaChecked}
              onChange={(e) => setEulaChecked(e.target.checked)}
              style={{ marginTop: 3 }}
            />
            <span style={{ fontSize: 'var(--text-sm)', color: 'var(--text)' }}>
              {t('eula.confirm')}
            </span>
          </label>
          <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 10 }}>
            <button
              disabled={!eulaChecked}
              onClick={() => {
                writeStorage('eulaAccepted', true)
                setShowEulaModal(false)
              }}
              style={{
                padding: '8px 20px',
                background: eulaChecked ? 'var(--accent)' : 'var(--border)',
                color: eulaChecked ? '#fff' : 'var(--text-dim)',
                border: 'none',
                borderRadius: 6,
                cursor: eulaChecked ? 'pointer' : 'not-allowed',
                fontSize: 'var(--text-sm)',
                fontWeight: 600,
              }}
            >
              {t('eula.agree')}
            </button>
          </div>
        </div>
      </div>
    )}

    {showLyricsPage && (
      <LyricsPage
        trackPath={playback.current_track}
        durationSecs={playback.duration_secs}
        playbackState={playback.state}
        coverUrl={coverUrl}
        onClose={() => setShowLyricsPage(false)}
        onPlayPause={() => {
          (playback.state === 'Playing' ? invoke('pause') : invoke('play'))
            .then(refreshPlayback)
            .catch(() => {})
        }}
        onPrevious={() => invoke('previous').catch(() => {})}
        onNext={() => invoke('next').catch(() => {})}
        onSeek={(pos) => invoke('seek', { positionSecs: pos }).catch(() => {})}
      />)}
    </>
    </I18nContext.Provider>
  )
}

function PlayerView({
  playback,
  onUpdate,
  refreshKey,
  triggerRefresh,
  spectrumActive,
  spectrumSettings,
  spectrumColorScheme,
  customSpectrumColors,
  addToast,
  sidebarWidth,
  playerMainWidth,
  startDrag,
  sidebarRef,
  playerMainRef,
}: {
  playback: PlaybackState
  onUpdate: () => void
  refreshKey: number
  triggerRefresh: () => void
  spectrumActive: boolean
  spectrumSettings: { smoothing: number; decay: number; fps: number }
  spectrumColorScheme: string
  customSpectrumColors: { bottom: string; top: string; peak: string; grid: string }
  addToast: (message: string, kind?: 'info' | 'warn' | 'error') => void
  sidebarWidth: number
  playerMainWidth: number
  startDrag: (splitId: string, e: React.MouseEvent) => void
  sidebarRef: React.RefObject<HTMLDivElement | null>
  playerMainRef: React.RefObject<HTMLDivElement | null>
}) {
  const { t } = useI18n()
  const [showPlaylist, setShowPlaylist] = useState(true)

  return (
    <div className="player-layout">
      {/* Edge navigation — only visible in narrow (≤600px) mode */}
      <div className={`nav-edge-left ${!showPlaylist ? 'visible' : ''}`} onClick={() => setShowPlaylist(true)} title={t('player.showPlaylist')}>
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round"><polyline points="9 18 15 12 9 6"/></svg>
      </div>
      <div className={`nav-edge-right ${showPlaylist ? 'visible' : ''}`} onClick={() => setShowPlaylist(false)} title={t('player.showNowPlaying')}>
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round"><polyline points="15 18 9 12 15 6"/></svg>
      </div>

      {/* Top row: sidebar (left) | playlist (middle) | now-playing (right) */}
      <div className="player-top">
        <div 
          ref={sidebarRef}
          className={`player-sidebar ${!showPlaylist ? 'player-col-hidden' : ''}`}
          style={{ width: sidebarWidth }}
        >
          <div className="sidebar-content">
            <PlaylistToolbar refreshTrigger={refreshKey} onUpdate={triggerRefresh} addToast={addToast} />
          </div>
        </div>

        {/* Draggable split bar between sidebar and playlist */}
        <div 
          className={`split-bar vertical ${!showPlaylist ? 'player-col-hidden' : ''}`}
          onMouseDown={(e) => startDrag('sidebar', e)}
          title={t('player.dragToResize')}
        />

        <div className="player-middle-row" style={{ flex: 1, minWidth: 0 }}>
          <div 
            className={`player-playlist-col ${!showPlaylist ? 'player-col-hidden' : ''}`}
          >
            <Playlist key={refreshKey} onUpdate={onUpdate} compact addToast={addToast} />
          </div>

          {/* Draggable split bar between playlist and now-playing */}
          <div 
            className={`split-bar vertical ${!showPlaylist ? 'player-col-hidden' : ''}`}
            onMouseDown={(e) => startDrag('playlist', e)}
            title={t('player.dragToResize')}
          />

          <div 
            ref={playerMainRef}
            className={`player-main ${showPlaylist ? 'player-col-hidden' : ''}`}
            style={{ width: playerMainWidth }}
          >
            <Player playback={playback} onUpdate={onUpdate} />
            <div className="card" style={{ padding: '10px 14px 6px', marginTop: 14 }}>
              <h3 style={{ marginBottom: 6 }}>{t('spectrum.title')}</h3>
              <Spectrum active={spectrumActive} smoothing={spectrumSettings.smoothing} decay={spectrumSettings.decay} fps={spectrumSettings.fps} colorScheme={spectrumColorScheme} customColors={customSpectrumColors} />
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}

interface PlayerBarProps {
  playback: PlaybackState
  onUpdate: () => void
  volume: number
  onVolumeChange: (v: number) => void
  volumeMode: string
  onVolumeModeChange: (mode: string) => void
  playbackMode: string
  setPlaybackMode: (mode: string) => void
  addToast: (message: string, kind?: 'info' | 'warn' | 'error') => void
  coverUrl: string | null
  onOpenLyrics: () => void
  onToggleDesktopLyrics: () => void
  desktopLyricsOpen: boolean
}

function PlayerBar({
  playback,
  onUpdate,
  volume,
  onVolumeChange,
  volumeMode,
  onVolumeModeChange,
  playbackMode,
  setPlaybackMode,
  addToast,
  coverUrl,
  onOpenLyrics,
  onToggleDesktopLyrics,
  desktopLyricsOpen,
}: PlayerBarProps) {
  const { t } = useI18n()
  // Position streams through the progress bus — subscribing here keeps the
  // ~20 Hz re-render local to PlayerBar instead of the whole App tree.
  const progress = useProgress()
  const [seekPct, setSeekPct] = useState(0)
  const seekPctRef = useRef(0)
  const draggingRef = useRef(false)
  const seekLockRef = useRef(false)
  const [dragging, setDragging] = useState(false)
  const [showSpeedList, setShowSpeedList] = useState(false)
  const modeDebounceRef = useRef(0)
  const skipDebounceRef = useRef(0)

  // Close speed list on outside click
  useEffect(() => {
    if (!showSpeedList) return
    const handler = () => setShowSpeedList(false)
    document.addEventListener('click', handler)
    return () => document.removeEventListener('click', handler)
  }, [showSpeedList])
  const [tooltipTime, setTooltipTime] = useState('')
  const durationRef = useRef(playback.duration_secs)
  durationRef.current = progress.duration ?? playback.duration_secs

  // Sync seekPct with playback position (skip during drag + lock period after seek)
  useEffect(() => {
    if (!draggingRef.current && !seekLockRef.current && durationRef.current) {
      setSeekPct((progress.position / durationRef.current) * 100)
    }
  }, [progress])

  const handleVolume = (v: number) => {
    onVolumeChange(v)
  }

  const toggleVolumeMode = async () => {
    const newMode = volumeMode === 'Hardware' ? 'Software' : 'Hardware'
    try {
      await invoke('set_volume_mode', { mode: newMode })
      onVolumeModeChange(newMode)
      // For Hardware mode, sync from device to get current system volume.
      // For Software mode, use the stored level.
      if (newMode === 'Hardware') {
        try {
          const level = await invoke<number>('sync_volume_from_device')
          onVolumeChange(Math.round(level * 100))
        } catch {
          const level = await invoke<number>('get_volume_level')
          onVolumeChange(Math.round(level * 100))
        }
      } else {
        const level = await invoke<number>('get_volume_level')
        onVolumeChange(Math.round(level * 100))
      }
    } catch (e) {
      console.error('Failed to toggle volume mode:', e)
    }
  }

  const formatTime = (secs: number) => {
    const m = Math.floor(secs / 60)
    const s = Math.floor(secs % 60)
    return `${m}:${s.toString().padStart(2, '0')}`
  }

  const handleSeekStart = (e: React.MouseEvent<HTMLDivElement>) => {
    if (!durationRef.current) return
    const rect = e.currentTarget.getBoundingClientRect()
    if (rect.width <= 0) return

    const calcPct = (clientX: number) =>
      Math.max(0, Math.min(100, ((clientX - rect.left) / rect.width) * 100))

    const dur = durationRef.current ?? 0
    const updateTooltip = (pct: number) => {
      const secs = (pct / 100) * dur
      const m = Math.floor(secs / 60)
      const s = Math.floor(secs % 60)
      setTooltipTime(`${m}:${s.toString().padStart(2, '0')}`)
    }

    draggingRef.current = true
    const pct = calcPct(e.clientX)
    seekPctRef.current = pct
    setSeekPct(pct)
    setDragging(true)
    updateTooltip(pct)
    console.log('[seek] drag start at', pct.toFixed(1), '%')

    const onMove = (ev: MouseEvent) => {
      const p = calcPct(ev.clientX)
      seekPctRef.current = p
      setSeekPct(p)
      updateTooltip(p)
    }

    const onUp = () => {
      draggingRef.current = false
      seekLockRef.current = true
      setDragging(false)
      setTooltipTime('')
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
      if (dur) {
        const pos = (seekPctRef.current / 100) * dur
        console.log('[seek] drag end, invoking seek to', pos.toFixed(1), 's')
        invoke('seek', { positionSecs: pos }).catch((err) => console.error('[seek] failed:', err))
      }
      // Release seek lock after 500ms to allow position sync to resume
      setTimeout(() => { seekLockRef.current = false }, 500)
    }

    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
  }

  const isPlaying = playback.state === 'Playing'

  const svgAttrs = (size = 24) => ({
    viewBox: '0 0 24 24', width: size, height: size,
    fill: 'none', stroke: 'currentColor', strokeWidth: '2',
    strokeLinecap: 'round' as const, strokeLinejoin: 'round' as const,
  })

  const modeIcon = (mode: string) => {
    const a = svgAttrs(24)
    switch (mode) {
      case 'normal':
        return (
          <svg {...a}>
            <line x1="4" y1="6" x2="20" y2="6"/>
            <line x1="4" y1="12" x2="20" y2="12"/>
            <line x1="4" y1="18" x2="20" y2="18"/>
          </svg>
        )
      case 'repeat_all':
        return (
          <svg {...a}>
            <path d="M17 2l4 4-4 4"/>
            <path d="M3 11v-1a4 4 0 014-4h14"/>
            <path d="M7 22l-4-4 4-4"/>
            <path d="M21 13v1a4 4 0 01-4 4H3"/>
          </svg>
        )
      case 'repeat_one':
        return (
          <svg {...a}>
            <path d="M17 2l4 4-4 4"/>
            <path d="M3 11v-1a4 4 0 014-4h14"/>
            <path d="M7 22l-4-4 4-4"/>
            <path d="M21 13v1a4 4 0 01-4 4H3"/>
            <path d="M11 10v4"/>
          </svg>
        )
      default:
        return null
    }
  }

  const speakerIcon = (
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" style={{ verticalAlign: 'middle', marginRight: 2 }}>
      <circle cx="10" cy="12" r="8" />
      <circle cx="10" cy="12" r="4" />
      <circle cx="10" cy="12" r="1.5" />
      <path d="M18 8a6 6 0 010 8" />
      <path d="M20 5a10 10 0 010 14" />
    </svg>
  )

  const speakerIconSw = (
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" style={{ verticalAlign: 'middle', marginRight: 2 }}>
      <path d="M11 5L6 9H2v6h4l5 4V5z" />
      <path d="M19.07 4.93a10 10 0 010 14.14M15.54 8.46a5 5 0 010 7.07" />
    </svg>
  )

  return (
    <div className="player-bar">
      <div
        className="progress-bar"
        onMouseDown={handleSeekStart}
      >
        <div className="track">
          <div className="fill" style={{ width: `${Math.min(seekPct, 100)}%` }}>
            <div className={`progress-thumb ${dragging ? 'dragging' : ''}`} />
          </div>
        </div>
        {dragging && tooltipTime && (
          <div className="progress-tooltip" style={{ left: `${Math.min(seekPct, 100)}%` }}>
            {tooltipTime}
          </div>
        )}
      </div>
      <div className="player-info">
        {/* Album Art */}
        <div className="player-cover" onClick={onOpenLyrics} style={{ cursor: 'pointer' }} title={t('player.lyrics')}>
          {coverUrl ? (
            <img src={coverUrl} alt={t('player.albumArt')} className="cover-img" />
          ) : (
            <div className="cover-placeholder">
              <svg viewBox="0 0 48 48" width="48" height="48" fill="none" stroke="currentColor" strokeWidth="1.5" opacity="0.3">
                <circle cx="24" cy="24" r="20" />
                <circle cx="24" cy="24" r="3" />
                <line x1="24" y1="4" x2="24" y2="21" />
                <line x1="24" y1="27" x2="24" y2="44" />
                <line x1="4" y1="24" x2="21" y2="24" />
                <line x1="27" y1="24" x2="44" y2="24" />
              </svg>
            </div>
          )}
        </div>

        {/* Desktop Lyrics toggle */}
        <button
          onClick={onToggleDesktopLyrics}
          className="btn-icon"
          title={t('player.desktopLyrics')}
          style={{
            color: desktopLyricsOpen ? 'var(--accent)' : 'var(--text-dim)',
            background: 'none', border: 'none', cursor: 'pointer',
            padding: 4, fontSize: 'var(--text-lg)', transition: 'color 0.2s',
          }}
        >
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <rect x="2" y="4" width="20" height="16" rx="2" />
            <line x1="8" y1="8" x2="16" y2="8" />
            <line x1="8" y1="12" x2="14" y2="12" />
            <line x1="8" y1="16" x2="12" y2="16" />
          </svg>
        </button>

        {/* Track name */}
        <span className="track-name">
          {playback.current_track ? extractFileName(playback.current_track) : t('player.noTrack')}
        </span>

        {/* Controls + Time (centered) */}
        <div className="player-controls-col">
          <div className="player-controls">
            <button onClick={() => {
              const now = Date.now()
              if (now - skipDebounceRef.current < 150) return
              skipDebounceRef.current = now
              invoke('previous').catch(() => {})
            }} title={t('player.previous')} className="skip-btn">
              <svg viewBox="0 0 24 24" width="22" height="22" fill="currentColor">
                <rect x="4" y="6" width="2" height="12" rx="0.5" />
                <polygon points="20,6 9,12 20,18" />
              </svg>
            </button>
            <button
              className="play-btn"
              onClick={() => (isPlaying ? invoke('pause') : invoke('play'))
                .then(onUpdate)
                .catch(() => {})}
              title={isPlaying ? t('player.pause') : t('player.play')}
            >
              {isPlaying ? (
                <svg viewBox="0 0 24 24" width="22" height="22" fill="currentColor">
                  <rect x="6.5" y="5" width="4" height="14" rx="1" />
                  <rect x="13.5" y="5" width="4" height="14" rx="1" />
                </svg>
              ) : (
                <svg viewBox="0 0 24 24" width="22" height="22" fill="currentColor" style={{ marginLeft: 2 }}>
                  <polygon points="8,5 20,12 8,19" />
                </svg>
              )}
            </button>
            <button onClick={() => {
              const now = Date.now()
              if (now - skipDebounceRef.current < 150) return
              skipDebounceRef.current = now
              invoke('next').catch(() => {})
            }} title={t('player.next')} className="skip-btn">
              <svg viewBox="0 0 24 24" width="22" height="22" fill="currentColor">
                <polygon points="4,6 15,12 4,18" />
                <rect x="18" y="6" width="2" height="12" rx="0.5" />
              </svg>
            </button>
            <button onClick={() => invoke('stop').then(onUpdate).catch(() => {})} title={t('player.stop')}>
              <svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor">
                <rect x="6" y="6" width="12" height="12" rx="1.5" />
              </svg>
            </button>
          </div>
          <div className="time-row">
            <span className="time-left">{formatTime(progress.position)}</span>
            <span className="time-sep">/</span>
            <span className="time-right">{durationRef.current ? formatTime(durationRef.current) : '--:--'}</span>
          </div>
        </div>

        {/* Playback mode — single button that cycles through modes on click */}
        <div className="mode-buttons">
          <button
            className="mode-btn"
            onClick={() => {
              const now = Date.now()
              if (now - modeDebounceRef.current < 300) return
              modeDebounceRef.current = now
              const modes = ['normal', 'repeat_all', 'repeat_one'];
              const labels: Record<string, string> = {
                normal: t('player.mode.sequential.desc'),
                repeat_all: t('player.mode.repeatAll.desc'),
                repeat_one: t('player.mode.repeatOne.desc'),
              };
              const currentIndex = modes.indexOf(playbackMode);
              const nextIndex = (currentIndex + 1) % modes.length;
              const nextMode = modes[nextIndex];
              invoke('set_playback_mode', { mode: nextMode })
                .then(() => {
                  setPlaybackMode(nextMode);
                  addToast(labels[nextMode]);
                })
                .catch((e) => {
                  console.error('Failed to set playback mode:', e);
                  addToast(t('toast.modeSwitchFailed', 'Mode switch failed'), 'error');
                });
            }}
            title={
              playbackMode === 'normal' ? t('player.mode.sequential.desc') :
              playbackMode === 'repeat_all' ? t('player.mode.repeatAll.desc') :
              t('player.mode.repeatOne.desc')
            }
          >
            {modeIcon(playbackMode)}
          </button>
        </div>

        {/* Speed control */}
        <div className="speed-control" style={{ position: 'relative' }}>
          <button
            className="btn btn-outline btn-sm"
            onClick={(e) => { e.stopPropagation(); setShowSpeedList(!showSpeedList) }}
            title={t('player.speed')}
            style={{ padding: '2px 8px', fontSize: 'var(--text-xs)', fontFamily: 'var(--mono)' }}
          >
            {playback.speed.toFixed(2)}x
          </button>
          {showSpeedList && (
            <div style={{
              position: 'absolute', bottom: '100%', left: '50%', transform: 'translateX(-50%)',
              marginBottom: 4, background: 'var(--bg-card)', border: '1px solid var(--border-glow)',
              borderRadius: 'var(--radius)', padding: 4, zIndex: 200,
              boxShadow: '0 4px 16px rgba(0,0,0,0.5)', display: 'flex', flexDirection: 'column', gap: 2,
              maxHeight: 132, overflowY: 'auto',
            }}>
              {[0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0].map((s) => (
                <button
                  key={s}
                  className={`btn btn-sm ${Math.abs(playback.speed - s) < 0.01 ? 'btn-primary' : 'btn-outline'}`}
                  onClick={() => {
                    invoke('set_playback_speed', { speed: s })
                      .then(onUpdate)
                      .catch(() => {})
                    setShowSpeedList(false)
                  }}
                  style={{ padding: '2px 8px', fontSize: 'var(--text-xs)', fontFamily: 'var(--mono)', whiteSpace: 'nowrap' }}
                >
                  {s}x
                </button>
              ))}
            </div>
          )}
        </div>

        {/* Volume */}
        <div className="volume-control">
          <span
            className="volume-mode"
            title={volumeMode === 'Hardware' ? t('settings.volumeControl.hw.desc') : t('settings.volumeControl.sw.desc')}
            onClick={toggleVolumeMode}
            style={{ cursor: 'pointer' }}
          >
            {volumeMode === 'Hardware' ? <>{speakerIcon} HW</> : <>{speakerIconSw} SW</>}
          </span>
          <input
            type="range"
            min="0"
            max="100"
            value={volume}
            onChange={(e) => handleVolume(Number(e.target.value))}
          />
        </div>
      </div>
    </div>
  )
}

function extractFileName(path: string): string {
  const name = path.split(/[\\/]/).pop() || path
  return name.replace(/\.[^.]+$/, '')
}

export default App