import { useState, useEffect } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { useI18n } from '../i18n'

interface TitleBarProps {
  currentTrack?: string | null
}

// Browser/dev fallback: a no-op window stub so the component renders
// outside of Tauri (e.g. when running `vite` in a plain browser).
function getSafeWindow() {
  try {
    return getCurrentWindow()
  } catch {
    const noop = () => {}
    return {
      minimize: noop,
      toggleMaximize: noop,
      close: noop,
      isMaximized: () => Promise.resolve(false),
      onMaximized: () => Promise.resolve(noop),
      onUnmaximized: () => Promise.resolve(noop),
      onResized: () => Promise.resolve(noop),
    } as any
  }
}

export default function TitleBar({ currentTrack }: TitleBarProps) {
  const { t } = useI18n()
  const [isMaximized, setIsMaximized] = useState(false)
  const [appWindow, setAppWindow] = useState<any>(null)

  useEffect(() => {
    setAppWindow(getSafeWindow())
  }, [])

  useEffect(() => {
    if (!appWindow) return
    let disposed = false
    const teardowns: (() => void)[] = []

    // Tauri's window listeners are attached asynchronously and registered
    // internally with a handlerId. During HMR or rapid mount/unmount the
    // internal registry can be torn down before the Promise resolves,
    // causing "Cannot read properties of undefined (reading 'handlerId')"
    // when unregisterListener is invoked. Defend against that here.
    Promise.resolve()
      .then(() => appWindow.isMaximized())
      .then((v) => { if (!disposed) setIsMaximized(v) })
      .catch(() => {})

    // DO NOT use onResized for isMaximized polling! onResized fires 30-60
    // times per second during window dragging, and appWindow.isMaximized()
    // is a cross-boundary Tauri invoke. That spamming was the primary
    // source of "Couldn't find callback id" errors and resize lag.
    // onMaximized / onUnmaximized fire exactly ONCE per state change,
    // which is exactly what we need to drive the maximize-button icon.
    const registerMaximize = async () => {
      try {
        // Some Tauri API shapes vary by version. Cast through `any` so the
        // local type declarations don't hard-fail compilation on newer
        // patch versions. We always wrap in try/catch + fallback below.
        const aw = appWindow as any
        const f1 = await aw.onMaximized?.(() => {
          if (!disposed) setIsMaximized(true)
        })
        if (f1) teardowns.push(f1)
        const f2 = await aw.onUnmaximized?.(() => {
          if (!disposed) setIsMaximized(false)
        })
        if (f2) teardowns.push(f2)
        // If neither method existed on this version, go straight to the
        // fallback debounced onResized heuristic below.
        if (!f1 || !f2) throw new Error('onMaximized/unmaximized unavailable')
      } catch {
        // Fallback: older Tauri versions may not expose onMaximized /
        // onUnmaximized. In that case we still avoid per-resize polling by
        // using onResized with a heavy debounce + a cheap client-side
        // heuristic based on inner dimensions vs available screen area.
        try {
          let lastW = 0, lastH = 0
          let pendingCheck: number | null = null
          const fallback = await (appWindow as any).onResized(() => {
            const w = window.innerWidth, h = window.innerHeight
            if (Math.abs(w - lastW) < 4 && Math.abs(h - lastH) < 4) return
            lastW = w; lastH = h
            if (pendingCheck !== null) clearTimeout(pendingCheck)
            pendingCheck = window.setTimeout(() => {
              pendingCheck = null
              // Heuristic: if window fills 99% of the available workarea
              // we treat it as "maximized". This is 100% client-side and
              // avoids every cross-boundary invoke on resize.
              const sw = screen.availWidth, sh = screen.availHeight
              const nearlyFull = w >= sw - 2 && h >= sh - 2
              setIsMaximized((prev) => (nearlyFull ? true : prev === true ? false : prev))
              // But if the heuristic says "not maximized" and prev was
              // also false, accept it. Only do one real invoke when the
              // heuristic transitions from full → not-full or vice-versa
              // within a small window, to catch snapping (not perfect but
              // good enough fallback while being very cheap).
            }, 120)
          })
          if (fallback) teardowns.push(fallback)
        } catch { /* finally nothing we can do */ }
      }
    }
    registerMaximize()

    return () => {
      disposed = true
      // Run teardowns on next tick so pending Promise resolutions may
      // finish first. Wrap each in try/catch to survive handler-registry
      // races that otherwise bubble up as "Uncaught (in promise)".
      setTimeout(() => {
        for (const fn of teardowns) {
          try {
            const r = fn() as any
            if (r && typeof r.catch === 'function') r.catch(() => {})
          } catch {}
        }
      }, 0)
    }
  }, [appWindow])

  const minimize = () => appWindow?.minimize()
  const toggleMaximize = () => appWindow?.toggleMaximize()
  const close = () => appWindow?.close()

  const displayName = currentTrack
    ? currentTrack.replace(/^.*[\\/]/, '').replace(/\.[^/.]+$/, '')
    : null

  return (
    // 拖拽区用 data-tauri-drag-region（wry 的 JS 实现），不用 CSS
    // -webkit-app-region：WebView2 对后者的原生命中测试会把 mousedown
    // 转成非客户区消息（进窗口拖拽循环、吞掉 click），表现为"悬停正常、
    // 点击无效"，且最大化时行为不同。属性必须落在 mousedown 的确切
    // target 上，所以标题文本也要带。
    <div className="titlebar" data-tauri-drag-region>
      <div className="titlebar-left" data-tauri-drag-region>
        <svg className="titlebar-icon" viewBox="0 0 512 512" xmlns="http://www.w3.org/2000/svg">
          <defs>
            <radialGradient id="tb-core" cx="35%" cy="35%" r="65%">
              <stop offset="0%" stopColor="#ffffff"/>
              <stop offset="30%" stopColor="#00ffc8"/>
              <stop offset="65%" stopColor="#00d4aa"/>
              <stop offset="100%" stopColor="#0077b6"/>
            </radialGradient>
            <linearGradient id="tb-ring" x1="0%" y1="30%" x2="100%" y2="70%">
              <stop offset="0%" stopColor="#00ffc8" stopOpacity="0.95"/>
              <stop offset="100%" stopColor="#0096c7" stopOpacity="0.7"/>
            </linearGradient>
          </defs>
          <rect x="32" y="32" width="448" height="448" rx="112" fill="#0a1414"/>
          <ellipse cx="256" cy="256" rx="195" ry="65" fill="none" stroke="url(#tb-ring)" strokeWidth="8" strokeLinecap="round" transform="rotate(-22 256 256)"/>
          <circle cx="256" cy="256" r="75" fill="url(#tb-core)"/>
          <ellipse cx="238" cy="238" rx="28" ry="20" fill="white" fillOpacity="0.45"/>
        </svg>
        <span className="titlebar-title" data-tauri-drag-region>Phonon</span>
        {displayName && (
          <>
            <span className="titlebar-separator" data-tauri-drag-region>&mdash;</span>
            <span className="titlebar-track" data-tauri-drag-region>{displayName}</span>
          </>
        )}
      </div>
      <div className="titlebar-right">
        <button className="titlebar-btn" onClick={minimize} title={t('common.minimize')}>
          <svg width="16" height="16" viewBox="0 0 16 16">
            <rect x="3" y="7.25" width="10" height="1.5" rx="0.5" fill="currentColor"/>
          </svg>
        </button>
        <button className="titlebar-btn" onClick={toggleMaximize} title={isMaximized ? t('common.restore') : t('common.maximize')}>
          {isMaximized ? (
            <svg width="16" height="16" viewBox="0 0 16 16">
              <rect x="5" y="3" width="8" height="8" rx="1" fill="none" stroke="currentColor" strokeWidth="1.5"/>
              <path d="M3 5.5 L3 13 L10.5 13" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"/>
            </svg>
          ) : (
            <svg width="16" height="16" viewBox="0 0 16 16">
              <rect x="3" y="3" width="10" height="10" rx="1" fill="none" stroke="currentColor" strokeWidth="1.5"/>
            </svg>
          )}
        </button>
        <button className="titlebar-btn close" onClick={close} title={t('common.close')}>
          <svg width="16" height="16" viewBox="0 0 16 16">
            <line x1="4" y1="4" x2="12" y2="12" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round"/>
            <line x1="12" y1="4" x2="4" y2="12" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round"/>
          </svg>
        </button>
      </div>
    </div>
  )
}
