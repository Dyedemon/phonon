import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { invoke } from '@tauri-apps/api/core'
import './index.css'
import App from './App.tsx'

// 启动层（HTML 内联样式的半透明深色 + 亚克力模糊）此刻已完成首次绘制，
// 让后端显示窗口——避免 WebView2 的默认白色底在 show 瞬间露出。
// 失败时后端的 3 秒兜底定时器仍会显示窗口。
invoke('show_main_window').catch(() => {})

// Main window entry point only.
// The desktop lyrics floating window has its own HTML file
// (desktop-lyrics.html) and entry point (src/desktop-lyrics-main.tsx),
// loaded via WebviewUrl::App("desktop-lyrics.html").

// ── HMR dispose hook (Vite dev only) ──
// When a module is hot-replaced, `beforeunload` does NOT fire.
// We must notify Rust to stop playback and suppress event emission
// into the dying React tree. The "Couldn't find callback id" warn is
// already suppressed by the js_init_script in the phonon-callback-guard
// plugin (runs before any page JS), so here we only need the Rust side.
if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    try { invoke('prepare_for_reload').catch(() => {}) } catch {}
  })
}

// F5 / Ctrl+R full refresh — tell Rust to stop emitting.
// JS-side suppression is handled by the plugin's initialization script.
window.addEventListener('beforeunload', () => {
  try { invoke('prepare_for_reload').catch(() => {}) } catch {}
})

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
