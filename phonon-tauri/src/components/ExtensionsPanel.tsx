import { useState } from 'react'
import { invoke as tauriInvoke } from '@tauri-apps/api/core'
import PluginManager from './PluginManager'
import { useI18n } from '../i18n'

interface ExtensionsPanelProps {
  addToast: (message: string, kind?: 'info' | 'warn' | 'error') => void
  onNavigate?: (tab: 'visualizer' | 'player' | 'devices' | 'dsp' | 'extensions' | 'settings') => void
}

export default function ExtensionsPanel({ addToast, onNavigate }: ExtensionsPanelProps) {
  const { t } = useI18n()

  // ── WASM Plugin Limits ──
  const [pluginMaxMemory, setPluginMaxMemory] = useState(() => Number(localStorage.getItem('phonon-plugin-max-memory') || '128'))
  const [pluginCallTimeout, setPluginCallTimeout] = useState(() => Number(localStorage.getItem('phonon-plugin-call-timeout') || '5000'))

  const showToast = (msg: string) => {
    addToast(msg, 'info')
  }

  const handlePluginLimits = async () => {
    localStorage.setItem('phonon-plugin-max-memory', String(pluginMaxMemory))
    localStorage.setItem('phonon-plugin-call-timeout', String(pluginCallTimeout))
    // vite-only/E2E 环境无 Tauri 运行时：只保存 localStorage，不调用后端。
    if (!(globalThis as any).__TAURI_INTERNALS__) return
    try {
      // Tauri v2 默认将 Rust snake_case 形参映射为 camelCase key，
      // 传 max_memory_mb 会被判定缺失参数（与 Settings 页旧实现同源 bug）。
      await tauriInvoke('set_plugin_limits', { maxMemoryMb: pluginMaxMemory, callTimeoutMs: pluginCallTimeout })
      showToast(t('toast.pluginLimitsUpdated'))
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
    </div>
  )
}
