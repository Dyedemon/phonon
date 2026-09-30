// Host for the visual-mode plugin (视觉升华 / plugins/vis/vis_mode.js).
//
// The plugin is self-contained and owns ALL of its effects via this contract:
//   styles       — CSS injected into a <style> tag while the plugin is enabled
//                  (vis button/panel UI + the whole quality-mode scheme + the
//                  ambient blob layer)
//   activate()   — called on enable: creates the ambient layer, hooks audio events
//   deactivate() — called on disable: tears those down
//   frame(now)   — rAF-driven while a visual mode is active; writes --vis-*
//                  CSS vars that drive the audio-reactive blobs
//   init/render/resize/destroy — legacy 2d-script interface kept for the
//                  VisualizerPage script list
//
// The host is intentionally dumb: inject/remove styles and run/stop the loop.
// Mode gating stays in App.tsx via the data-vis-* attributes, which the
// plugin's CSS keys on.

import { invoke } from '@tauri-apps/api/core'

export interface VisModePlugin {
  name?: string
  type?: string
  styles?: string
  activate?: () => void
  deactivate?: () => void
  frame?: (now: number) => void
  init?: (canvas: HTMLCanvasElement, w: number, h: number) => void
  render?: (canvas: HTMLCanvasElement, features: unknown, w: number, h: number, time: number) => void
  resize?: (canvas: HTMLCanvasElement, w: number, h: number) => void
  destroy?: () => void
}

const STYLE_ID = 'phonon-vis-plugin-styles'
const SCRIPT_FILE = 'vis_mode.js'

let plugin: VisModePlugin | null = null
let loadPromise: Promise<VisModePlugin | null> | null = null
let rafId = 0
// 最后一次请求的启用状态:文件读取是异步的,期间开关可能翻转,
// await 之后的注入/激活必须以最新意图为准
let stylesWanted = false
// 帧循环是否被模式激活所需要(防止 loadPromise 补启出僵尸循环)
let loopWanted = false

function loadPlugin(): Promise<VisModePlugin | null> {
  if (plugin) return Promise.resolve(plugin)
  if (!loadPromise) {
    loadPromise = (async () => {
      try {
        const content = await invoke<string>('read_vis_script', { fileName: SCRIPT_FILE })
        const fn = new Function(content)
        const script = (await fn()) as VisModePlugin
        if (script && typeof script === 'object') plugin = script
      } catch (e) {
        console.warn('[visModeHost] 加载视觉插件失败:', e)
      }
      return plugin
    })()
  }
  return loadPromise
}

/**
 * Inject/remove the plugin's styles. Call whenever the plugin enable-state
 * changes. Resolves to false when the plugin could not be loaded (missing or
 * broken file) so the caller can self-heal to a disabled state.
 */
export async function syncVisPluginStyles(enabled: boolean): Promise<boolean> {
  stylesWanted = enabled
  if (!enabled) {
    document.getElementById(STYLE_ID)?.remove()
    try { plugin?.deactivate?.() } catch { /* 插件内部错误不阻断宿主 */ }
    stopVisLoop()
    // 丢弃缓存:下次启用时重新读文件,和 PluginManager 的重载语义一致
    plugin = null
    loadPromise = null
    return false
  }
  const p = await loadPlugin()
  if (!stylesWanted) {
    // 加载期间被禁用:丢弃刚加载的实例,不注入任何东西
    try { p?.deactivate?.() } catch { /* ignore */ }
    plugin = null
    loadPromise = null
    return false
  }
  if (!p) return false
  let styleEl = document.getElementById(STYLE_ID) as HTMLStyleElement | null
  if (!styleEl) {
    styleEl = document.createElement('style')
    styleEl.id = STYLE_ID
    document.head.appendChild(styleEl)
  }
  styleEl.textContent = p.styles ?? ''
  try { p.activate?.() } catch (e) { console.warn('[visModeHost] 插件 activate 失败:', e) }
  return true
}

/** rAF loop while a visual mode is active — drives the plugin's audio reactivity. */
export function startVisLoop(): void {
  loopWanted = true
  if (rafId) return
  if (!plugin) {
    // 插件还没加载完(刚启用就立刻开模式的小窗口期):加载完成后补启
    void loadPlugin().then((p) => {
      if (p && loopWanted && !rafId) startVisLoop()
    })
    return
  }
  const loop = (now: number) => {
    try { plugin?.frame?.(now) } catch { /* 插件错误不终止循环 */ }
    rafId = requestAnimationFrame(loop)
  }
  rafId = requestAnimationFrame(loop)
}

export function stopVisLoop(): void {
  loopWanted = false
  if (rafId) {
    cancelAnimationFrame(rafId)
    rafId = 0
  }
}
