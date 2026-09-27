import { useState, useRef, useEffect } from 'react'
import { invoke as tauriInvoke } from '@tauri-apps/api/core'
import PluginManager from './PluginManager'
import CropEditor from './CropEditor'
import { useI18n } from '../i18n'

// ── safeInvoke: vite-only/E2E 环境下 Tauri 运行时不存在时短路返回 ─
// 与 App.tsx 中的 safeInvoke 等价。
// 若 __TAURI_INTERNALS__ 缺失：立即 resolve undefined；
// 若 invoke 抛错：catch 后 resolve undefined。
// 确保 ExtensionsPanel 在无后端环境下不会因 Promise pending 永久占满主线程。
function safeInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T | undefined> {
  try {
    const anyWin = globalThis as any
    if (!anyWin.__TAURI_INTERNALS__) return Promise.resolve(undefined)
    const p = tauriInvoke<T>(cmd, args) as Promise<T>
    return p.catch(() => undefined as any)
  } catch {
    return Promise.resolve(undefined)
  }
}

interface ExtensionsPanelProps {
  addToast: (message: string, kind?: 'info' | 'warn' | 'error') => void
  onNavigate?: (tab: 'visualizer' | 'player' | 'devices' | 'dsp' | 'extensions' | 'settings') => void
}

export default function ExtensionsPanel({ addToast, onNavigate }: ExtensionsPanelProps) {
  const { t } = useI18n()

  // ── Custom CSS ──
  const [customCss, setCustomCss] = useState(() => localStorage.getItem('phonon-custom-css') || '')
  const [cssEnabled, setCssEnabled] = useState(() => localStorage.getItem('phonon-css-enabled') !== 'false')
  const cssFileInputRef = useRef<HTMLInputElement>(null)

  const injectCustomCss = (css: string) => {
    let el = document.getElementById('phonon-custom-css') as HTMLStyleElement | null
    if (!el) {
      el = document.createElement('style')
      el.id = 'phonon-custom-css'
      document.head.appendChild(el)
    }
    el.textContent = css
    if (el !== document.head.lastElementChild) {
      document.head.appendChild(el)
    }
  }

  const handleCustomCss = (css: string) => {
    setCustomCss(css)
    localStorage.setItem('phonon-custom-css', css)
    if (cssEnabled) injectCustomCss(css)
  }

  const toggleCssEnabled = () => {
    const next = !cssEnabled
    setCssEnabled(next)
    localStorage.setItem('phonon-css-enabled', String(next))
    if (next) {
      injectCustomCss(customCss)
    } else {
      const el = document.getElementById('phonon-custom-css')
      if (el) el.remove()
    }
  }

  const handleCustomCssFile = () => cssFileInputRef.current?.click()

  const handleCssFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (!file) return
    const reader = new FileReader()
    reader.onload = () => handleCustomCss(reader.result as string)
    reader.readAsText(file)
    e.target.value = ''
  }

  const clearCustomCss = () => {
    setCustomCss('')
    localStorage.removeItem('phonon-custom-css')
    const el = document.getElementById('phonon-custom-css')
    if (el) el.remove()
  }

  // ── Background Image ──
  const [bgImageUrl, setBgImageUrl] = useState(() => localStorage.getItem('phonon-bg-image-url') || '')
  const [bgImageDataUrl, setBgImageDataUrl] = useState(() => localStorage.getItem('phonon-bg-image-data') || '')
  const [cropVisible, setCropVisible] = useState(false)
  const [cropSource, setCropSource] = useState('')

  const applyBgImage = (url: string, dataUrl: string) => {
    const el = document.querySelector('.app') as HTMLElement | null
    if (!el) return
    if (dataUrl) {
      el.style.backgroundImage = `url("${dataUrl}")`
    } else if (url) {
      el.style.backgroundImage = `url("${url}")`
    } else {
      el.style.backgroundImage = ''
    }
  }

  const handleBgImageUrl = (url: string) => {
    setBgImageUrl(url)
    localStorage.setItem('phonon-bg-image-url', url)
    applyBgImage(url, bgImageDataUrl)
  }

  const handleBgImageUpload = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (!file) return
    const reader = new FileReader()
    reader.onload = () => {
      setCropSource(reader.result as string)
      setCropVisible(true)
    }
    reader.readAsDataURL(file)
    e.target.value = ''
  }

  const handleCropConfirm = (croppedDataUrl: string) => {
    setCropVisible(false)
    setCropSource('')
    const img = new Image()
    img.onload = () => {
      const MAX_DIM = 1920
      let w = img.width, h = img.height
      if (w > MAX_DIM || h > MAX_DIM) {
        const ratio = Math.min(MAX_DIM / w, MAX_DIM / h)
        w = Math.round(w * ratio)
        h = Math.round(h * ratio)
      }
      const canvas = document.createElement('canvas')
      canvas.width = w
      canvas.height = h
      const ctx = canvas.getContext('2d')
      if (ctx) {
        ctx.drawImage(img, 0, 0, w, h)
        const dataUrl = canvas.toDataURL('image/jpeg', 0.7)
        setBgImageDataUrl(dataUrl)
        try { localStorage.setItem('phonon-bg-image-data', dataUrl) }
        catch { /* quota exceeded */ }
        setBgImageUrl('')
        localStorage.removeItem('phonon-bg-image-url')
        applyBgImage('', dataUrl)
      }
    }
    img.src = croppedDataUrl
  }

  const handleCropCancel = () => {
    setCropVisible(false)
    setCropSource('')
  }

  const clearBgImage = () => {
    setBgImageUrl('')
    setBgImageDataUrl('')
    localStorage.removeItem('phonon-bg-image-url')
    localStorage.removeItem('phonon-bg-image-data')
    applyBgImage('', '')
  }

  // ── WASM Plugin Limits ──
  const [pluginMaxMemory, setPluginMaxMemory] = useState(() => Number(localStorage.getItem('phonon-plugin-max-memory') || '128'))
  const [pluginCallTimeout, setPluginCallTimeout] = useState(() => Number(localStorage.getItem('phonon-plugin-call-timeout') || '5000'))

  // Sync plugin limits when Settings panel changes them
  useEffect(() => {
    const onLimitsChanged = () => {
      safeInvoke<{ max_memory_mb: number; call_timeout_ms: number }>('get_plugin_limits')
        .then((v) => {
          if (!v) return // vite-only fallback: skip update, keep localStorage default
          setPluginMaxMemory(v.max_memory_mb)
          setPluginCallTimeout(v.call_timeout_ms)
        })
        .catch(() => {})
    }
    window.addEventListener('plugin-limits-changed', onLimitsChanged)
    return () => window.removeEventListener('plugin-limits-changed', onLimitsChanged)
  }, [])

  const showToast = (msg: string) => {
    addToast(msg, 'info')
  }

  const handlePluginLimits = async () => {
    localStorage.setItem('phonon-plugin-max-memory', String(pluginMaxMemory))
    localStorage.setItem('phonon-plugin-call-timeout', String(pluginCallTimeout))
    try {
      await safeInvoke('set_plugin_limits', { maxMemoryMb: pluginMaxMemory, callTimeoutMs: pluginCallTimeout })
      showToast(t('toast.pluginLimitsUpdated'))
      // Notify Settings panel to sync
      window.dispatchEvent(new CustomEvent('plugin-limits-changed'))
    } catch {
      showToast(t('toast.pluginLimitsFailed'))
    }
  }

  return (
    <div>
      {/* Legal Disclaimer */}
      <div className="card" style={{
        borderLeft: '3px solid var(--accent)',
        background: 'var(--bg-hover)',
      }}>
        <p style={{ margin: 0, fontSize: 'var(--text-sm)', color: 'var(--text-dim)', lineHeight: 1.6 }}>
          {t('extensions.disclaimer', 'Phonon 不内置任何第三方音源或受版权保护的解码器。扩展功能由用户自行安装与管理。')}
        </p>
      </div>

      {/* Built-in Extensions */}
      <div className="card">
        <h3>{t('settings.extensions.builtin')}</h3>
        <p className="setting-desc">{t('settings.extensions.builtin.desc')}</p>

        {/* Background Image */}
        <div className="setting-section" style={{ marginTop: 12 }}>
          <div className="setting-row">
            <span className="setting-label">{t('settings.appearance.bgImage')}</span>
          </div>
          <div className="setting-row" style={{ marginTop: 8, gap: 8 }}>
            <input
              type="text"
              className="setting-input"
              placeholder={t('settings.appearance.bgImageUrl')}
              value={bgImageUrl}
              onChange={e => handleBgImageUrl(e.target.value)}
              style={{ flex: 1 }}
            />
            <label className="btn-ghost" style={{ cursor: 'pointer', whiteSpace: 'nowrap' }}>
              {t('settings.appearance.bgImageUpload')}
              <input
                type="file"
                accept="image/*"
                style={{ display: 'none' }}
                onChange={handleBgImageUpload}
              />
            </label>
          </div>
          {(bgImageUrl || bgImageDataUrl) && (
            <button className="btn-ghost" onClick={clearBgImage} style={{ marginTop: 8 }}>
              {t('settings.appearance.bgImageClear')}
            </button>
          )}
        </div>

        {/* Custom CSS */}
        <div className="setting-section" style={{ marginTop: 16 }}>
          <div className="setting-row" style={{ marginBottom: 6 }}>
            <span className="setting-label">{t('settings.appearance.customCss')}</span>
            <button
              className={`toggle-btn ${cssEnabled ? 'on' : ''}`}
              onClick={toggleCssEnabled}
              style={{
                marginLeft: 8, padding: '2px 10px', borderRadius: 12,
                fontSize: 'var(--text-xs)', fontWeight: 600, border: 'none', cursor: 'pointer',
                background: cssEnabled ? 'var(--accent)' : 'var(--bg-hover)',
                color: cssEnabled ? '#fff' : 'var(--text-dim)',
              }}
            >
              {cssEnabled ? t('common.on') : t('common.off')}
            </button>
          </div>
          <p className="setting-hint" style={{ marginTop: 4 }}>
            {t('settings.appearance.customCss.desc')}
          </p>
          <div>
            <textarea
              className="custom-css-editor"
              value={customCss}
              onChange={e => handleCustomCss(e.target.value)}
              placeholder={t('settings.appearance.customCss.placeholder')}
              spellCheck={false}
              style={{ maxHeight: 200, overflowY: 'auto' }}
            />
          </div>
          <div className="setting-row" style={{ marginTop: 8, gap: 8 }}>
            <button className="btn-ghost" onClick={handleCustomCssFile}>
              {t('settings.appearance.customCss.load')}
            </button>
            <input
              ref={cssFileInputRef}
              type="file"
              accept=".css,.txt"
              style={{ display: 'none' }}
              onChange={handleCssFileChange}
            />
            {customCss && (
              <button className="btn-ghost" onClick={clearCustomCss}>
                {t('settings.appearance.customCss.clear')}
              </button>
            )}
          </div>
        </div>
      </div>

      {/* WASM Plugin Limits */}
      <div className="card">
        <h3>{t('settings.extensions.wasm')}</h3>
        <p className="setting-desc">{t('settings.extensions.wasm.desc')}</p>
        <div className="setting-sliders" style={{ marginTop: 12 }}>
          <div className="slider-row">
            <span className="slider-label">{t('settings.wasmPlugins.memory')}: {pluginMaxMemory} MB</span>
            <input
              type="range"
              min="16"
              max="512"
              step="16"
              value={pluginMaxMemory}
              onChange={(e) => setPluginMaxMemory(Number(e.target.value))}
              onMouseUp={handlePluginLimits}
              className="eq-gain-slider"
              style={{ width: 140 }}
            />
          </div>
          <div className="slider-row">
            <span className="slider-label">{t('settings.wasmPlugins.timeout')}: {pluginCallTimeout} ms</span>
            <input
              type="range"
              min="1000"
              max="30000"
              step="1000"
              value={pluginCallTimeout}
              onChange={(e) => setPluginCallTimeout(Number(e.target.value))}
              onMouseUp={handlePluginLimits}
              className="eq-gain-slider"
              style={{ width: 140 }}
            />
          </div>
        </div>
        <p className="setting-hint">{t('settings.wasmPlugins.hint')}</p>
      </div>

      {/* Plugin Manager */}
      <PluginManager addToast={addToast} onNavigate={onNavigate} />

      {/* Crop Modal */}
      {cropVisible && (
        <CropEditor source={cropSource} onConfirm={handleCropConfirm} onCancel={handleCropCancel} />
      )}
    </div>
  )
}