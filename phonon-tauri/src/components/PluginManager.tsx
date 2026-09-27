import { useState, useEffect, useCallback, useMemo, Fragment } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-dialog'
import { useI18n } from '../i18n'
import { listDspProcessors } from '../api/dsp'
import { openPluginDir } from '../api/plugins'

interface PluginManifest {
  name: string
  version: string
  author: string
  description: string
  plugin_type: string
  permissions: string[]
  min_framework_version: string
}

interface PluginInfo {
  manifest: PluginManifest
  state: string
  file_path: string
  error_message: string | null
  origin: 'builtin' | 'wasmExample' | 'wasmExternal'
}

// JS visualization script entry (mirrors Rust scan_vis_sources return type)
interface VisSourceEntry {
  name: string
  file_name: string
}

// localStorage key shared with VisualizerPage
const ENABLED_VIS_KEY = 'phonon-enabled-vis-scripts'

function readEnabledVis(): string[] {
  try {
    const raw = localStorage.getItem(ENABLED_VIS_KEY)
    if (!raw) return []
    const arr = JSON.parse(raw)
    return Array.isArray(arr) ? arr.filter(x => typeof x === 'string') : []
  } catch { return [] }
}

function writeEnabledVis(list: string[]) {
  localStorage.setItem(ENABLED_VIS_KEY, JSON.stringify(list))
  window.dispatchEvent(new CustomEvent('vis-scripts-enabled-changed'))
}

function originLabel(origin: PluginInfo['origin'], t: (key: string, fb?: string) => string): { text: string; style: React.CSSProperties } | null {
  switch (origin) {
    case 'builtin':
      return {
        text: t('plugins.tabs.builtin'),
        style: {
          padding: '1px 8px',
          borderRadius: 4,
          fontSize: 'var(--text-xs)',
          fontWeight: 600,
          background: 'rgba(255,255,255,0.06)',
          color: 'var(--text-dim)',
          border: '1px solid rgba(255,255,255,0.1)',
          letterSpacing: 0.5,
        },
      }
    case 'wasmExample':
      return {
        text: t('plugins.tabs.example'),
        style: {
          padding: '1px 8px',
          borderRadius: 4,
          fontSize: 'var(--text-xs)',
          fontWeight: 600,
          background: 'rgba(59,130,246,0.15)',
          color: '#60a5fa',
          border: '1px solid rgba(59,130,246,0.3)',
          letterSpacing: 0.5,
        },
      }
    case 'wasmExternal':
      return null
  }
}

function stateLabel(state: string, t: (key: string, fb?: string) => string): string {
  switch (state) {
    case 'Discovered': return t('plugins.state.discovered')
    case 'Loaded': return t('plugins.state.active')
    case 'Running': return t('plugins.state.running')
    case 'Error': return t('plugins.state.error')
    case 'Disabled': return t('plugins.state.disabled')
    default: return state
  }
}

function stateColor(state: string): string {
  switch (state) {
    case 'Loaded':
    case 'Running': return 'var(--accent)'
    case 'Error': return 'var(--danger)'
    case 'Disabled': return 'var(--text-dim)'
    default: return 'var(--text)'
  }
}

interface PluginManagerProps {
  addToast: (message: string, kind?: 'info' | 'warn' | 'error') => void
  onNavigate?: (page: 'visualizer' | 'player' | 'devices' | 'dsp' | 'extensions' | 'settings') => void
}

// Unified item type for the plugin list — either a WASM plugin or a JS vis script
type UnifiedItem =
  | { kind: 'wasm'; plugin: PluginInfo }
  | { kind: 'vis'; script: VisSourceEntry; enabled: boolean }

export default function PluginManager({ addToast, onNavigate }: PluginManagerProps) {
  const { t } = useI18n()
  const [plugins, setPlugins] = useState<PluginInfo[]>([])
  const [scanning, setScanning] = useState(false)
  const [expandedConfig, setExpandedConfig] = useState<string | null>(null)
  const [configText, setConfigText] = useState('')
  const [configSaving, setConfigSaving] = useState(false)

  // JS visualization scripts
  const [visScripts, setVisScripts] = useState<VisSourceEntry[]>([])
  const [enabledVis, setEnabledVis] = useState<string[]>(readEnabledVis())

  const showToast = (msg: string) => {
    addToast(msg, 'info')
  }

  const showErrorToast = (msg: string) => {
    addToast(msg, 'error')
  }

  const refresh = useCallback(async () => {
    try {
      const list = await invoke<PluginInfo[]>('list_plugins')
      setPlugins(list)
    } catch (e) {
      console.error('[PluginManager] list_plugins failed:', e)
    }
    // 同步 DSP 链状态到 dspEnabled，确保 Builtin 与 Wasm 插件的开/关状态正确显示
    try {
      const procs = await listDspProcessors()
      const enabledMap: Record<string, boolean> = {}
      // DSP processor name() → plugin manifest name 映射
      const dspNameToPlugin: Record<string, string> = {
        'Equalizer': 'phonon_equalizer',
        'Smart Effect': 'phonon_smart_effect',
        'Surround Sound': 'phonon_surround_sound',
        'Downmix': 'phonon_downmix',
        'ReplayGain': 'phonon_replaygain',
      }
      for (const p of procs) {
        const pluginName = dspNameToPlugin[p.name] ?? p.name
        enabledMap[pluginName] = p.enabled
      }
      setDspEnabled(enabledMap)
    } catch (e) {
      console.error('[PluginManager] listDspProcessors failed:', e)
    }
  }, [])

  const refreshVisScripts = useCallback(async () => {
    try {
      const list = await invoke<VisSourceEntry[]>('scan_vis_sources')
      setVisScripts(list)
      // 清理 localStorage 中已不存在的脚本（例如手动删除了文件）
      const existing = list.map(s => s.file_name)
      const saved = readEnabledVis()
      const cleaned = saved.filter(f => existing.includes(f))
      if (cleaned.length !== saved.length) {
        writeEnabledVis(cleaned)
      }
      setEnabledVis(cleaned)
    } catch (e) {
      console.error('[PluginManager] scan_vis_sources failed:', e)
    }
  }, [])

  const toggleVisScript = useCallback((fileName: string) => {
    const current = readEnabledVis()
    const idx = current.indexOf(fileName)
    const next = idx >= 0
      ? current.filter(x => x !== fileName)
      : [...current, fileName]
    writeEnabledVis(next)
    setEnabledVis(next)
  }, [])

  useEffect(() => {
    refresh()
    refreshVisScripts()
  }, [refresh, refreshVisScripts])

  useEffect(() => {
    const onLimitsChanged = () => { refresh() }
    const onDspChainChanged = () => { refresh() }
    window.addEventListener('plugin-limits-changed', onLimitsChanged)
    window.addEventListener('dsp-chain-changed', onDspChainChanged)
    return () => {
      window.removeEventListener('plugin-limits-changed', onLimitsChanged)
      window.removeEventListener('dsp-chain-changed', onDspChainChanged)
    }
  }, [refresh])

  useEffect(() => {
    const unlisten = Promise.all([
      listen('plugins-changed', () => { refresh() }),
      listen('plugin-reloaded', () => { refresh() }),
    ])
    return () => { unlisten.then(([u1, u2]) => { u1(); u2() }) }
  }, [refresh])

  useEffect(() => {
    const enabled = plugins
      .filter((p) => (p.state === 'Loaded' || p.state === 'Running') && p.origin !== 'builtin')
      .map((p) => ({ file_path: p.file_path, plugin_type: p.manifest.plugin_type }))
    localStorage.setItem('phonon-enabled-plugins', JSON.stringify(enabled))
  }, [plugins])

  // Build unified list: WASM plugins first, then JS vis scripts
  const unifiedItems: UnifiedItem[] = useMemo(() => {
    const wasm: UnifiedItem[] = plugins.map(p => ({ kind: 'wasm' as const, plugin: p }))
    const vis: UnifiedItem[] = visScripts.map(s => ({
      kind: 'vis' as const,
      script: s,
      enabled: enabledVis.includes(s.file_name),
    }))
    return [...wasm, ...vis]
  }, [plugins, visScripts, enabledVis])

  const scan = async () => {
    setScanning(true)
    try {
      const [wasmList] = await Promise.all([
        invoke<PluginInfo[]>('scan_plugins'),
        refreshVisScripts(),
      ])
      setPlugins(wasmList)
    } catch (e) {
      console.error('Failed to scan plugins:', e)
    } finally {
      setScanning(false)
    }
  }

  const importPlugin = async () => {
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: 'WASM Plugin', extensions: ['wasm'] }],
      })
      if (!selected) return
      const path = typeof selected === 'string' ? selected : (selected as { path: string }).path
      const list = await invoke<PluginInfo[]>('import_plugin', { sourcePath: path })
      setPlugins(list)
      showToast(t('plugins.imported'))
    } catch (e) {
      console.error('Failed to import plugin:', e)
      showToast(t('plugins.importFailed'))
    }
  }

  const openFolder = async () => {
    try {
      await openPluginDir()
    } catch (e) {
      console.error('Failed to open plugin folder:', e)
      showToast(t('plugins.openFolderFailed', 'Failed to open plugin folder'))
    }
  }

  const remove = async (plugin: PluginInfo) => {
    const ok = window.confirm(
      t('plugins.removeConfirm', `确定要删除插件「{name}」吗？\n将直接删除对应的插件文件，此操作不可恢复。`).replace('{name}', plugin.manifest.name)
    )
    if (!ok) return
    try {
      const list = await invoke<PluginInfo[]>('remove_plugin', { filePath: plugin.file_path })
      setPlugins(list)
      // 同步清理 dspEnabled 中已删除插件的记录
      setDspEnabled((prev) => {
        const next = { ...prev }
        delete next[plugin.manifest.name]
        return next
      })
      showToast(t('plugins.removed'))
    } catch (e) {
      console.error('Failed to remove plugin:', e)
      showToast(t('plugins.removeFailed'))
    }
  }

  const reload = async (plugin: PluginInfo) => {
    try {
      const list = await invoke<PluginInfo[]>('reload_plugin', { filePath: plugin.file_path })
      setPlugins(list)
      showToast(t('plugins.reloaded'))
    } catch (e) {
      console.error('Failed to reload plugin:', e)
      showToast(t('plugins.reloadFailed'))
    }
  }

  const isDspPlugin = (plugin: PluginInfo) => {
    const ty = plugin.manifest.plugin_type
    if (ty !== 'DspProcessor' && ty !== 'processor' && ty !== 'Visualizer' && ty !== 'VisualizerAnalyzer') {
      return false
    }
    // Calibration plugin has its own panel in the DSP page — no need for
    // the "add to DSP" button or DSP toggle here.
    const name = plugin.manifest.name.toLowerCase()
    if (name === 'calibration' || name === 'acoustic calibration') {
      return false
    }
    return true
  }

  // The acoustic calibration plugin ships with the app (plugins/calibration.wasm,
  // gitignored build artifact). Its 删除 button is hidden: removing the file
  // silently kills the whole feature and there is no in-app way to get it back.
  const isCalibrationPlugin = (plugin: PluginInfo) => {
    const name = plugin.manifest.name.toLowerCase()
    return name === 'calibration' || name === 'acoustic calibration'
  }

  const isTimeStretchPlugin = (plugin: PluginInfo) => {
    const ty = plugin.manifest.plugin_type
    return ty === 'TimeStretch' || ty === 'timestretch'
  }

  const enable = async (plugin: PluginInfo) => {
    try {
      if (isTimeStretchPlugin(plugin)) {
        await invoke('load_time_stretch_plugin', { filePath: plugin.file_path })
      } else {
        await invoke('enable_plugin', { id: plugin.file_path })
      }
      refresh()
    } catch (e) {
      console.error('Failed to enable plugin:', e)
    }
  }

  const disable = async (plugin: PluginInfo) => {
    try {
      if (isTimeStretchPlugin(plugin)) {
        await invoke('unload_time_stretch_plugin', { filePath: plugin.file_path })
      } else {
        await invoke('disable_plugin', { id: plugin.file_path })
        if (isDspPlugin(plugin)) {
          try {
            await invoke('remove_wasm_dsp', { id: `wasm_${plugin.manifest.name}` })
          } catch (_) { /* may not be in chain */ }
          // 从 dspEnabled 中移除该插件
          setDspEnabled((prev) => {
            const next = { ...prev }
            delete next[plugin.manifest.name]
            return next
          })
        }
      }
      refresh()
    } catch (e) {
      console.error('Failed to disable plugin:', e)
    }
  }

  const addToDsp = async (plugin: PluginInfo) => {
    try {
      await invoke('add_wasm_dsp', {
        name: plugin.manifest.name,
        id: `wasm_${plugin.manifest.name}`,
        latency: 0.0,
      })
      // 加入成功后同步 dspEnabled，让 UI 显示 DSP 开/关按钮而非 +DSP 按钮
      setDspEnabled((prev) => ({ ...prev, [plugin.manifest.name]: false }))
      const visualizerKinds = ['Visualizer', 'VisualizerAnalyzer']
      const msg = visualizerKinds.includes(plugin.manifest.plugin_type)
        ? t('plugins.dsp.added')
        : t('plugins.addedToDsp')
      showToast(msg)
    } catch (e) {
      const msg = String(e)
      if (msg.includes('already exists')) {
        showErrorToast(t('plugins.alreadyInDsp'))
      } else {
        showErrorToast(t('plugins.failed'))
      }
    }
  }

  // 真实 enable_dsp 开关状态：首次点击时按用户选择传给后端，之后本地镜像保持同步。
  // 启动时我们不主动拉 DSP 链 enabled 快照（避免阻塞 list_plugins），需要时可调用
  //   api/dsp.ts 中的 listDspProcessors()。
  const [dspEnabled, setDspEnabled] = useState<Record<string, boolean>>({})

  const togglePluginInDsp = async (plugin: PluginInfo, next: boolean) => {
    try {
      await invoke('toggle_plugin_in_dsp', {
        name: plugin.manifest.name,
        enabled: next,
      })
      setDspEnabled((prev) => ({ ...prev, [plugin.manifest.name]: next }))
      showToast(next ? t('plugins.dsp.enabled') : t('plugins.dsp.disabled'))
      window.dispatchEvent(new CustomEvent('dsp-chain-changed'))
    } catch (e) {
      console.error('[togglePluginInDsp] failed:', e)
      addToast(String(e), 'error')
    }
  }

  const openConfig = async (plugin: PluginInfo) => {
    try {
      const cfg = await invoke<string | null>('get_plugin_config', { name: plugin.manifest.name })
      setConfigText(cfg || '{}')
      setExpandedConfig(plugin.file_path)
    } catch (_) {
      setConfigText('{}')
      setExpandedConfig(plugin.file_path)
    }
  }

  const saveConfig = async (plugin: PluginInfo) => {
    setConfigSaving(true)
    try {
      await invoke('set_plugin_config', { name: plugin.manifest.name, config: configText })
      showToast(t('plugins.configSaved'))
      setExpandedConfig(null)
    } catch (e) {
      showToast(String(e))
    } finally {
      setConfigSaving(false)
    }
  }

  const hasAny = unifiedItems.length > 0

  return (
    <div className="card">
      <div className="toolbar">
        <h3 style={{ flex: 1, margin: 0 }}>{t('plugins.title', 'Plugins')}</h3>
        <button className="btn btn-outline btn-sm" onClick={importPlugin}>
          {t('plugins.import', 'Import')}
        </button>
        <button className="btn btn-outline btn-sm" onClick={scan} disabled={scanning}>
          {scanning ? t('common.scanning') : t('plugins.scan')}
        </button>
        <button className="btn btn-outline btn-sm" onClick={openFolder}>
          {t('plugins.openFolder', 'Open Folder')}
        </button>
      </div>

      {!hasAny ? (
        <div className="empty-state">
          <div className="icon">{'\uD83E\uDDE9'}</div>
          <p>{t('plugins.noPlugins', 'No plugins found')}</p>
          <p style={{ fontSize: 'var(--text-sm)', color: 'var(--text)' }}>
            {t('plugins.hintText', '将 .wasm 文件放入')}
            <code style={{ margin: '0 4px', padding: '1px 6px', background: 'var(--bg-hover)', borderRadius: 4, fontSize: '0.9em' }}>./plugins</code>
            {t('plugins.hintSuffix', '目录后点击扫描，或使用导入功能')}
          </p>
        </div>
      ) : (
        <div>
          {unifiedItems.map((item) => {
            if (item.kind === 'wasm') {
              const plugin = item.plugin
              const isBuiltin = plugin.origin === 'builtin'
              const badge = originLabel(plugin.origin, t)
              return (
                <Fragment key={plugin.file_path}>
                  <div className="list-item" style={{ alignItems: 'flex-start' }}>
                    <div className="info">
                      <div className="title" style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                        {plugin.manifest.name}
                        {badge && <span style={badge.style}>{badge.text}</span>}
                      </div>
                      <div className="subtitle">
                        v{plugin.manifest.version} by {plugin.manifest.author || t('plugins.unknownAuthor')}
                      </div>
                      <div className="meta">
                        <span>{plugin.manifest.plugin_type}</span>
                        {plugin.manifest.permissions.length > 0 && (
                          <>
                            <span className="meta-sep">|</span>
                            <span>{plugin.manifest.permissions.join(', ')}</span>
                          </>
                        )}
                      </div>
                      {(plugin.manifest.description || isCalibrationPlugin(plugin)) && (
                        <div className="meta" style={{ marginTop: 2 }}>
                          {isCalibrationPlugin(plugin)
                            ? t('plugins.calibration.desc')
                            : plugin.manifest.description}
                        </div>
                      )}
                      {plugin.error_message && (
                        <div className="meta" style={{ color: 'var(--danger)', marginTop: 2 }}>{plugin.error_message}</div>
                      )}
                    </div>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexShrink: 0 }}>
                      {/* 状态 badge：非 DSP 插件保留（TimeStretch/SourceProvider 等）；
                          DSP 插件：Builtin 显示运行状态文字，Wasm 显示 DSP 开关按钮 */}
                      {!isBuiltin && !isDspPlugin(plugin) && (
                        <span style={{ fontSize: 'var(--text-sm)', color: stateColor(plugin.state), fontWeight: 500 }}>
                          {stateLabel(plugin.state, t)}
                        </span>
                      )}

                      {/* Builtin：显示运行状态文字（不在 DSP 面板外提供开关） */}
                      {isBuiltin && (() => {
                        const enabled = dspEnabled[plugin.manifest.name] ?? false
                        return (
                          <span style={{
                            fontSize: 'var(--text-sm)',
                            fontWeight: 500,
                            color: enabled ? 'var(--success, #22c55e)' : 'var(--text-dim)',
                          }}>
                            {enabled ? t('plugins.status.running') : t('plugins.status.notRunning')}
                          </span>
                        )
                      })()}

                      {/* Wasm DSP：三态智能按钮 */}
                      {!isBuiltin && isDspPlugin(plugin) && (() => {
                        const inChain = dspEnabled[plugin.manifest.name] !== undefined
                        const enabled = dspEnabled[plugin.manifest.name] ?? false
                        if (!inChain) {
                          return (
                            <button
                              title="加入 DSP 链"
                              className="btn btn-sm btn-primary"
                              onClick={() => addToDsp(plugin)}
                              style={{ minWidth: 90 }}
                            >
                              {['Visualizer', 'VisualizerAnalyzer'].includes(plugin.manifest.plugin_type)
                                ? '加入分析链路'
                                : t('plugins.addToDsp')}
                            </button>
                          )
                        }
                        return (
                          <button
                            title={enabled ? t('dsp.on', 'DSP:开') : t('dsp.off', 'DSP:关')}
                            className={`btn btn-sm ${enabled ? 'btn-primary' : 'btn-outline'}`}
                            onClick={() => togglePluginInDsp(plugin, !enabled)}
                            style={{ minWidth: 80 }}
                          >
                            {enabled
                              ? t('dsp.on', 'DSP:开')
                              : t('dsp.off', 'DSP:关')}
                          </button>
                        )
                      })()}

                      {!isBuiltin && (plugin.state === 'Discovered' || plugin.state === 'Disabled') && (
                        <>
                          <button className="btn btn-primary btn-sm" onClick={() => enable(plugin)}>
                            {t('common.enable')}
                          </button>
                          {!isCalibrationPlugin(plugin) && (
                            <button className="btn btn-danger btn-sm" onClick={() => remove(plugin)}>
                              {t('common.remove')}
                            </button>
                          )}
                        </>
                      )}
                      {!isBuiltin && (plugin.state === 'Loaded' || plugin.state === 'Running') && (
                        <>
                          <button className="btn btn-outline btn-sm" onClick={() => reload(plugin)} title={t('plugins.reload')}>
                            &#x21bb;
                          </button>
                          <button className="btn btn-outline btn-sm" onClick={() => openConfig(plugin)} title={t('plugins.config')}>
                            &#x2699;
                          </button>
                          <button className="btn btn-danger btn-sm" onClick={() => disable(plugin)}>
                            {t('common.disable')}
                          </button>
                        </>
                      )}
                      {/* Calibration: the workflow lives in the DSP page panel —
                          one-click entry here; 删除 is intentionally not offered. */}
                      {!isBuiltin && isCalibrationPlugin(plugin) && (
                        <button
                          className="btn btn-primary btn-sm"
                          onClick={() => onNavigate?.('dsp')}
                          title={t('plugins.calibration.openDsp')}
                        >
                          {t('plugins.calibration.openDsp')}
                        </button>
                      )}
                      {/* Builtin（static）→ 只提供 Config 按钮，样式与外部插件完全一致（icon-only） */}
                      {isBuiltin && (
                        <button className="btn btn-outline btn-sm" onClick={() => openConfig(plugin)} title={t('plugins.config')}>
                          &#x2699;
                        </button>
                      )}
                    </div>
                  </div>
                  {/* 行内展开：只在当前点击行下面渲染配置编辑器，避免跳到列表最底部 */}
                  {expandedConfig === plugin.file_path && (() => {
                    return (
                      <div style={{
                        margin: '4px 0 12px 0', padding: 12,
                        background: 'var(--bg-hover)', borderRadius: 6,
                        border: '1px solid var(--border)',
                      }}>
                        <div style={{ display: 'flex', alignItems: 'center', marginBottom: 8, gap: 8 }}>
                          <span style={{ fontSize: 'var(--text-sm)', fontWeight: 600, color: 'var(--accent)' }}>
                            {t('plugins.configFor', 'Config')}: {plugin.manifest.name}
                          </span>
                          <div style={{ flex: 1 }} />
                          <button
                            className="btn btn-primary btn-sm"
                            onClick={() => saveConfig(plugin)}
                            disabled={configSaving}
                          >
                            {configSaving ? t('common.saving') : t('common.save')}
                          </button>
                          <button className="btn btn-outline btn-sm" onClick={() => setExpandedConfig(null)}>
                            {t('common.cancel')}
                          </button>
                        </div>
                        <textarea
                          value={configText}
                          onChange={(e) => setConfigText(e.target.value)}
                          rows={6}
                          style={{
                            width: '100%', background: 'var(--bg)', color: 'var(--text)',
                            border: '1px solid var(--border)', borderRadius: 4, padding: 8,
                            fontFamily: 'monospace', fontSize: 'var(--text-sm)', resize: 'vertical',
                          }}
                          spellCheck={false}
                        />
                        <p style={{ fontSize: 'var(--text-xs)', color: 'var(--text-dim)', marginTop: 4 }}>
                          {t('plugins.configHint')}
                        </p>
                      </div>
                    )
                  })()}
                </Fragment>
              )
            } else {
              const s = item.script
              const enabled = item.enabled
              return (
                <div
                  key={'vis-' + s.file_name}
                  className="list-item"
                  style={{
                    alignItems: 'flex-start',
                    borderLeft: enabled ? '3px solid var(--accent)' : '3px solid var(--text-dim)',
                  }}
                >
                  <div className="info">
                    <div className="title">✨ {s.name}</div>
                    <div className="subtitle" style={{ fontFamily: 'monospace' }}>
                      {s.file_name}
                    </div>
                    <div className="meta">
                      <span>SublimationScript</span>
                      <span className="meta-sep">|</span>
                      <span>JS 视觉升华效果插件</span>
                    </div>
                    <div className="meta" style={{ marginTop: 2, color: 'var(--text-dim)' }}>
                      放入 <code>plugins/vis/</code> 目录的视觉升华脚本，启用后可在导航栏按钮中切换模式
                    </div>
                  </div>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexShrink: 0 }}>
                    <span style={{
                      fontSize: 'var(--text-sm)',
                      color: enabled ? 'var(--accent)' : 'var(--text-dim)',
                      fontWeight: 500,
                    }}>
                      {enabled ? '已启用' : '未启用'}
                    </span>
                    {enabled ? (
                      <button
                        className="btn btn-danger btn-sm"
                        onClick={() => toggleVisScript(s.file_name)}
                      >
                        禁用
                      </button>
                    ) : (
                      <button
                        className="btn btn-primary btn-sm"
                        onClick={() => toggleVisScript(s.file_name)}
                      >
                        启用
                      </button>
                    )}
                  </div>
                </div>
              )
            }
          })}
        </div>
      )}
    </div>
  )
}
