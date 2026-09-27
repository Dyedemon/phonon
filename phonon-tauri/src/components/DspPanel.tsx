import { useState, useEffect, useCallback, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { useI18n } from '../i18n'
import EqPanel from './EqPanel'
import type { TimeStretchMode } from '../api/settings'
import { setTimeStretchMode, getTimeStretchMode } from '../api/dsp'

interface DspInfo {
  id: string
  name: string
  enabled: boolean
  latency_ms: number
}

interface DspPanelProps {
  onNavigate?: (tab: string) => void
  colorScheme?: string
  customColors?: { bottom: string; top: string; peak: string; grid: string }
  addToast: (message: string, kind?: 'info' | 'warn' | 'error') => void
}

export default function DspPanel({ onNavigate, colorScheme, customColors, addToast }: DspPanelProps) {
  const { t } = useI18n()
  const [processors, setProcessors] = useState<DspInfo[]>([])
  const [latency, setLatency] = useState(0)
  const [replayGainMode, setReplayGainMode] = useState<string>('Off')
  const [dragIndex, setDragIndex] = useState<number | null>(null)
  const reorderPickRef = useRef<number | null>(null)
  const [scanning, setScanning] = useState(false)
  // Smart Effect — built-in plugin, managed here in the DSP panel.
  const [smartEffectMode, setSmartEffectMode] = useState<string>('off')
  const [smartEffectAuto, setSmartEffectAuto] = useState(() =>
    localStorage.getItem('phonon-smart-effect-auto') === 'true'
  )
  // Surround Sound — built-in immersive spatial audio plugin.
  const [surroundMode, setSurroundMode] = useState<string>('off')
  // TimeStretch 引擎 — 内置 DSP，本面板独立卡片展示
  const [timeStretchMode, setTimeStretchModeState] = useState<TimeStretchMode>('auto')

  const showToast = (msg: string) => {
    addToast(msg, 'info')
  }

  const refresh = useCallback(async () => {
    try {
      const list = await invoke<DspInfo[]>('list_dsp_processors')
      // 过滤掉 Builtin DSP 处理器（它们在本面板有独立卡片展示）
      const builtinIds = new Set(['replaygain', 'downmix', 'equalizer', 'smart_effect', 'surround_sound', 'peq'])
      setProcessors(list.filter(p => !builtinIds.has(p.id)))
      const lat = await invoke<number>('get_dsp_latency')
      setLatency(lat)
      const mode = await invoke<string>('get_replay_gain')
      setReplayGainMode(mode)
      const seMode = await invoke<string>('get_smart_effect_mode').catch(() => 'off')
      setSmartEffectMode(seMode)
      const ssMode = await invoke<string>('get_surround_sound_mode').catch(() => 'off')
      setSurroundMode(ssMode)
      const tsMode = await getTimeStretchMode().catch(() => 'auto' as TimeStretchMode)
      setTimeStretchModeState(tsMode)
    } catch (_) { /* ignore */ }
  }, [])

  // Initial refresh + very light periodic safety-net polling.
  // Most state changes are event-driven (see listeners below).
  // The 10s polling only catches rare cases where events are missed
  // (e.g. plugin internal state changes that don't emit events).
  // Polling is paused when the document is hidden to save CPU.
  useEffect(() => {
    let timer: number | null = null
    const startPolling = () => {
      if (timer !== null) return
      timer = window.setInterval(() => {
        void refresh()
      }, 10000)
    }
    const stopPolling = () => {
      if (timer !== null) {
        window.clearInterval(timer)
        timer = null
      }
    }
    const onVisibility = () => {
      if (document.hidden) {
        stopPolling()
      } else {
        startPolling()
        void refresh()
      }
    }
    refresh()
    if (!document.hidden) startPolling()
    document.addEventListener('visibilitychange', onVisibility)
    return () => {
      stopPolling()
      document.removeEventListener('visibilitychange', onVisibility)
    }
  }, [refresh])

  // Listen for DSP chain changes (add/remove/reorder/toggle) from
  // PluginManager or other components → refresh immediately.
  useEffect(() => {
    const handler = () => { refresh() }
    window.addEventListener('dsp-chain-changed', handler)
    return () => window.removeEventListener('dsp-chain-changed', handler)
  }, [refresh])

  // Listen for surround sound mode changes from external sources.
  useEffect(() => {
    const handler = (e: Event) => {
      const mode = (e as CustomEvent).detail as string
      setSurroundMode(mode)
    }
    window.addEventListener('surround-sound-mode-change', handler)
    return () => window.removeEventListener('surround-sound-mode-change', handler)
  }, [])

  // Listen for auto-mode preset switches from App.tsx so the UI stays
  // in sync when auto mode changes the backend preset.
  useEffect(() => {
    const handler = (e: Event) => {
      const mode = (e as CustomEvent).detail as string
      setSmartEffectMode(mode)
    }
    window.addEventListener('smart-effect-mode-change', handler)
    return () => window.removeEventListener('smart-effect-mode-change', handler)
  }, [])

  const handleSmartEffectMode = async (mode: string) => {
    if (smartEffectAuto) {
      setSmartEffectAuto(false)
      localStorage.setItem('phonon-smart-effect-auto', 'false')
      window.dispatchEvent(new CustomEvent('smart-effect-auto-change', { detail: false }))
    }
    setSmartEffectMode(mode)
    await invoke('set_smart_effect_mode', { mode }).catch(() => {})
    window.dispatchEvent(new CustomEvent('smart-effect-mode-change', { detail: mode }))
    // Mode change alters the processor's reported latency → refresh the readout now.
    refresh()
  }

  const handleSmartEffectAuto = (enabled: boolean) => {
    setSmartEffectAuto(enabled)
    localStorage.setItem('phonon-smart-effect-auto', String(enabled))
    window.dispatchEvent(new CustomEvent('smart-effect-auto-change', { detail: enabled }))
  }

  const handleSurroundMode = async (mode: string) => {
    setSurroundMode(mode)
    await invoke('set_surround_sound_mode', { mode }).catch(() => {})
    window.dispatchEvent(new CustomEvent('surround-sound-mode-change', { detail: mode }))
    // Mode change alters the processor's reported latency → refresh the readout now.
    refresh()
  }

  const handleTimeStretchMode = async (mode: TimeStretchMode) => {
    setTimeStretchModeState(mode)
    await setTimeStretchMode(mode).catch((e) => console.error('Failed to set TimeStretch mode:', e))
  }

  const toggle = async (id: string, enabled: boolean) => {
    try {
      if (enabled) {
        await invoke('disable_dsp', { id })
      } else {
        await invoke('enable_dsp', { id })
      }
      refresh()
      window.dispatchEvent(new CustomEvent('dsp-chain-changed'))
    } catch (e) {
      console.error('Failed to toggle DSP:', e)
    }
  }

  const handleReplayGain = async (mode: string) => {
    try {
      await invoke('set_replay_gain', { mode })
      setReplayGainMode(mode)
      refresh()
    } catch (e) {
      console.error('Failed to set ReplayGain:', e)
    }
  }

  const handleReorderMouseDown = (e: React.MouseEvent, idx: number) => {
    if (e.button !== 0) return
    reorderPickRef.current = idx
    setDragIndex(idx)
  }

  const handleReorderMouseUp = async (_e: React.MouseEvent, to: number) => {
    const from = reorderPickRef.current
    reorderPickRef.current = null
    setDragIndex(null)
    if (from === null || from === to) return
    try {
      await invoke('reorder_dsp', { from, to })
      refresh()
    } catch (err) {
      console.error('Failed to reorder DSP:', err)
    }
  }

  const importAndAddDsp = async () => {
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: 'WASM Plugin', extensions: ['wasm'] }],
      })
      if (!selected) return
      const path = typeof selected === 'string' ? selected : (selected as { path: string }).path
      setScanning(true)
      // Import copies to plugins dir and scans
      const all = await invoke<{manifest: {plugin_type: string; name: string}}[]>('import_plugin', { sourcePath: path })
      const dspPlugins = all.filter((p) => p.manifest.plugin_type === 'DspProcessor')
      let added = 0
      for (const p of dspPlugins) {
        try {
          await invoke('add_wasm_dsp', {
            name: p.manifest.name,
            id: `wasm_${p.manifest.name}`,
            latency: 0.0,
          })
          added++
        } catch (e) {
          if (!String(e).includes('already exists')) {
            console.error('Failed to add DSP:', e)
          }
        }
      }
      if (added > 0) {
        showToast(t('toast.dspImported').replace('{n}', String(added)))
        refresh()
      }
    } catch (e) {
      console.error('Import failed:', e)
      showToast(t('toast.dspImportFailed'))
    } finally {
      setScanning(false)
    }
  }

  return (
    <div className="dsp-panel">
      {/* ReplayGain Settings */}
      <div className="card">
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <h3 style={{ margin: 0 }}>{t('settings.replayGain')}</h3>
          <div style={{ display: 'flex', gap: 6, flex: 1 }}>
            {(['Off', 'Track', 'Album'] as const).map((mode) => {
                const modeLabels: Record<string, string> = { Off: t('settings.replayGain.off'), Track: t('settings.replayGain.track'), Album: t('settings.replayGain.album') }
                return (
              <button
                key={mode}
                className={`btn ${replayGainMode === mode ? 'btn-primary' : 'btn-outline'} btn-sm`}
                onClick={() => handleReplayGain(mode)}
              >
                {modeLabels[mode]}
              </button>
            )})}
          </div>
          <button className="btn btn-outline btn-sm" onClick={importAndAddDsp} disabled={scanning}>
            {scanning ? t('common.scanning') : t('dsp.import')}
          </button>
          {onNavigate && (
            <button className="btn btn-outline btn-sm" onClick={() => onNavigate('extensions')}>
              {t('tab.extensions')}
            </button>
          )}
        </div>
      </div>

      <div className="card">
        <div className="toolbar">
          <h3 style={{ flex: 1, margin: 0 }}>{t('dsp.title', 'DSP Processors')}</h3>
          <span style={{ fontSize: 'var(--text-sm)', color: 'var(--text-muted)' }}>
            {t('dsp.latency')}: {latency.toFixed(1)}ms
          </span>
        </div>

        {/* ── Smart Effect (built-in) ── */}
        <div style={{
          border: '1px solid var(--border-glow)',
          borderRadius: 'var(--radius)',
          padding: 'var(--dsp-card-padding, 12px)',
          marginBottom: 'var(--dsp-card-mb, 12px)',
          background: 'var(--bg-active)',
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 6 }}>
            <span style={{ fontSize: 'var(--text-base)', fontWeight: 600, color: 'var(--text-bright)' }}>
              {t('dsp.smartEffect', '智能音效')}
            </span>
            <span style={{
              fontSize: 'var(--text-xs)', padding: '1px 6px', borderRadius: 3,
              background: 'var(--accent)', color: '#fff', fontWeight: 500,
            }}>{t('plugins.builtin', '内置')}</span>
            <div style={{ flex: 1 }} />
            <label
              title="自动模式：根据歌曲流派自动切换最佳音效"
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                gap: 6,
                cursor: 'pointer',
                fontSize: 'var(--text-sm)',
                color: 'var(--text-dim)',
                userSelect: 'none',
              }}
            >
              <span>{t('dsp.smartEffect.auto', '自动')}:</span>
              <input
                type="checkbox"
                role="switch"
                checked={smartEffectAuto}
                onChange={(e) => handleSmartEffectAuto(e.target.checked)}
                style={{ transform: 'scale(1.15)', accentColor: 'var(--accent)' }}
              />
            </label>
          </div>
          <p style={{ fontSize: 'var(--text-sm)', color: 'var(--text-dim)', margin: '0 0 8px' }}>
            {t('dsp.smartEffect.desc', '根据歌曲流派自动均衡参数、动态音量调节、人声增强和声场扩展，让歌曲更好听更耐听')}
          </p>
          <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
            {(() => {
              const presetList = [
                ['off', t('dsp.smartEffect.off', '关闭')],
                ['pop', t('dsp.smartEffect.pop', '流行')],
                ['rock', t('dsp.smartEffect.rock', '摇滚')],
                ['jazz', t('dsp.smartEffect.jazz', '爵士')],
                ['classical', t('dsp.smartEffect.classical', '古典')],
                ['electronic', t('dsp.smartEffect.electronic', '电子')],
                ['vocal', t('dsp.smartEffect.vocal', '人声')],
                ['bass_boost', t('dsp.smartEffect.bassBoost', '低音增强')],
              ] as const
              return presetList.map(([mode, label]) => {
                const active = smartEffectMode === mode && !(smartEffectAuto && mode === 'off')
                const disabled = smartEffectAuto && mode !== 'off'
                return (
                  <button
                    key={mode}
                    className={`btn btn-sm ${active ? 'btn-primary' : 'btn-outline'}`}
                    onClick={() => handleSmartEffectMode(mode)}
                    disabled={disabled}
                    style={disabled ? { opacity: 0.5, cursor: 'not-allowed' } : {}}
                  >
                    {label}
                  </button>
                )
              })
            })()}
          </div>
          {smartEffectAuto && (
            <p style={{ fontSize: 'var(--text-xs)', color: 'var(--accent)', margin: '6px 0 0' }}>
              {t('dsp.smartEffect.autoActive', '✓ 自动模式已启用，将根据歌曲流派自动切换')}
            </p>
          )}
        </div>

        {/* ── Surround Sound (built-in) ── */}
        <div style={{
          border: '1px solid var(--border-glow)',
          borderRadius: 'var(--radius)',
          padding: 'var(--dsp-card-padding, 12px)',
          marginBottom: 'var(--dsp-card-mb, 12px)',
          background: 'var(--bg-active)',
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 6 }}>
            <span style={{ fontSize: 'var(--text-base)', fontWeight: 600, color: 'var(--text-bright)' }}>
              {t('dsp.surroundSound', '沉浸式环绕音效')}
            </span>
            <span style={{
              fontSize: 'var(--text-xs)', padding: '1px 6px', borderRadius: 3,
              background: 'var(--accent)', color: '#fff', fontWeight: 500,
            }}>{t('plugins.builtin', '内置')}</span>
          </div>
          <p style={{ fontSize: 'var(--text-sm)', color: 'var(--text-dim)', margin: '0 0 8px' }}>
            {t('dsp.surroundSound.desc', '通过 M/S 立体声扩展、交叉馈送、早期反射模拟与 Haas 优先效应，营造沉浸式空间听觉体验')}
          </p>
          <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
            {(() => {
              const surroundModes = [
                ['off', t('dsp.surroundSound.off', '关闭')],
                ['concert', t('dsp.surroundSound.concert', '音乐会')],
                ['theater', t('dsp.surroundSound.theater', '剧院')],
                ['studio', t('dsp.surroundSound.studio', '录音室')],
                ['spacious', t('dsp.surroundSound.spacious', '宽敞')],
                ['immersive', t('dsp.surroundSound.immersive', '沉浸式')],
              ] as const
              return surroundModes.map(([mode, label]) => {
                const active = surroundMode === mode
                return (
                  <button
                    key={mode}
                    className={`btn btn-sm ${active ? 'btn-primary' : 'btn-outline'}`}
                    onClick={() => handleSurroundMode(mode)}
                    style={{ minWidth: 48 }}
                  >
                    {label}
                  </button>
                )
              })
            })()}
          </div>
          {surroundMode !== 'off' && (
            <p style={{ fontSize: 'var(--text-xs)', color: 'var(--accent)', margin: '6px 0 0' }}>
              {t('dsp.surroundSound.active', '✓ 沉浸式环绕音效已启用')}
            </p>
          )}
        </div>

        {/* ── TimeStretch 引擎 (built-in) ── */}
        <div style={{
          border: '1px solid var(--border-glow)',
          borderRadius: 'var(--radius)',
          padding: 'var(--dsp-card-padding, 12px)',
          marginBottom: 'var(--dsp-card-mb, 12px)',
          background: 'var(--bg-active)',
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 6 }}>
            <span style={{ fontSize: 'var(--text-base)', fontWeight: 600, color: 'var(--text-bright)' }}>
              {t('dsp.timeStretch', '变速引擎')}
            </span>
            <span style={{
              fontSize: 'var(--text-xs)', padding: '1px 6px', borderRadius: 3,
              background: 'var(--accent)', color: '#fff', fontWeight: 500,
            }}>{t('plugins.builtin', '内置')}</span>
          </div>
          <p style={{ fontSize: 'var(--text-sm)', color: 'var(--text-dim)', margin: '0 0 8px' }}>
            {t('dsp.timeStretch.desc', '变速不变调算法：自动模式按倍率切换 WSOLA / 相位声码器，极端变速更稳定')}
          </p>
          <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
            {(() => {
              const modes = [
                ['auto', t('dsp.timeStretch.auto', '自动'), '0.8x – 1.25x 瞬态优先，其他倍率抗涂抹'],
                ['wsola', 'WSOLA', '全速率时域，打击乐/人声最锐利'],
                ['phase_vocoder', t('dsp.timeStretch.phaseVocoder', '相位声码器'), '极端变速 0.25x / 4x 更稳定'],
              ] as const
              return modes.map(([val, label, hint]) => {
                const active = timeStretchMode === val
                return (
                  <button
                    key={val}
                    className={`btn btn-sm ${active ? 'btn-primary' : 'btn-outline'}`}
                    onClick={() => handleTimeStretchMode(val as TimeStretchMode)}
                    title={hint}
                    style={{ minWidth: 48 }}
                  >
                    {label}
                  </button>
                )
              })
            })()}
          </div>
          <p style={{ fontSize: 'var(--text-xs)', color: 'var(--text-dim)', margin: '6px 0 0' }}>
            {timeStretchMode === 'auto' && t('dsp.timeStretch.hintAuto', '自动：常用档 WSOLA，极端档回退相位声码器')}
            {timeStretchMode === 'wsola' && t('dsp.timeStretch.hintWsola', 'WSOLA：所有速率强制时域，瞬态最锐利')}
            {timeStretchMode === 'phase_vocoder' && t('dsp.timeStretch.hintPv', '相位声码器：所有速率强制频域，极端变速最稳定')}
          </p>
        </div>

        {processors.map((proc, idx) => (
          <div
            key={proc.id}
            className="list-item"
            style={{
              cursor: 'grab',
              opacity: dragIndex === idx ? 0.4 : 1,
              transition: 'opacity 0.15s',
            }}
            onMouseDown={(e) => handleReorderMouseDown(e, idx)}
            onMouseUp={(e) => handleReorderMouseUp(e, idx)}
          >
            <div className="info">
              <div className="title">{proc.name}</div>
              <div className="subtitle">{t('dsp.processorInfo').replace('{id}', proc.id).replace('{latency}', proc.latency_ms.toFixed(1))}</div>
            </div>
            <button
              title={proc.enabled ? t('dsp.on', 'DSP:开') : t('dsp.off', 'DSP:关')}
              className={`btn btn-sm ${proc.enabled ? 'btn-primary' : 'btn-outline'}`}
              onClick={() => toggle(proc.id, proc.enabled)}
              style={{ minWidth: 80 }}
            >
              {proc.enabled ? t('dsp.on', 'DSP:开') : t('dsp.off', 'DSP:关')}
            </button>
          </div>
        ))}
      </div>

      <EqPanel colorScheme={colorScheme} customColors={customColors} addToast={addToast} />
    </div>
  )
}