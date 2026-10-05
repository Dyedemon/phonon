import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { resolve } from 'path'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  // Vite HMR 保持一个永久 WebSocket 连接，使 Playwright 的
  // action + 某些 page.evaluate 调用等待 network idle 而永远不返回
  // （直到 test timeout）。在 E2E 测试（PLAYWRIGHT_E2E=1）中禁用 HMR，
  // 让 Playwright 的 stability check 能达到 idle 状态。
  server: {
    // 端口被占直接报错退出：今天"重启了还是旧代码"的根因——5173 被旧
    // vite 占据时它会静默挪到 5174，而 tauri 窗口永远加载 5173 的旧服务
    strictPort: true,
    // HMR：注释本意是"仅 E2E 时禁用"，实现却写成了无条件禁用（热更新
    // 整个失效，文件改了窗口永远不更新）——按注释意图改为环境变量门控
    hmr: process.env.PLAYWRIGHT_E2E === '1' ? false : true,
    watch: { ignored: ['**/playwright-report/**', '**/test-results/**', '**/node_modules/**'] },
  },
  // Prevent Vite from pre-bundling Tauri APIs — they need
  // window.__TAURI_INTERNALS__ which is only available in the Tauri webview.
  optimizeDeps: {
    exclude: ['@tauri-apps/api', '@tauri-apps/plugin-dialog'],
  },
  build: {
    rollupOptions: {
      // Multi-page setup: the desktop lyrics floating window has its own
      // HTML entry so it is completely isolated from the main window.
      // Both dev and build modes need this so Vite resolves the module
      // graph for each entry point.
      input: {
        main: resolve(__dirname, 'index.html'),
        'desktop-lyrics': resolve(__dirname, 'desktop-lyrics.html'),
        'tray-popup': resolve(__dirname, 'tray-popup.html'),
      },
    },
  },
})
