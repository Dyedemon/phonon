import { defineConfig, devices } from '@playwright/test';

/**
 * Playwright E2E 配置 — Phonon Tauri 前端
 *
 * 策略：
 *   - 默认对 `vite dev` server 跑前端 E2E（验证 UI 行为 + 0 console.error）
 *   - 完整 Tauri 集成测试（含 Rust 后端 IPC）需要 `tauri dev`，
 *     通过 `playwright.config.tauri.ts` 单独配置（未来扩展）
 *
 * 关键：
 *   - webServer.command 显式设置 PLAYWRIGHT_E2E=1：vite.config.ts 会在
 *     该变量存在时关闭 HMR，避免「HMR WebSocket 持久连接」使 Playwright
 *     的 stability check 永远达不到 idle（从而所有 action/evaluate hang
 *     到 test timeout）。
 *   - workers=1 串行跑，避免单 chromium context 数量过多触发
 *     CDP 命令队列竞争。
 *
 * 对应 tasks.md §15 第五条：
 *   "Tauri E2E Playwright：主窗口打开 → Play/Pause → 音量滑 → EQ 拖动
 *    → state 正确 + 0 console.error"
 */
export default defineConfig({
  testDir: './e2e',
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: 1,
  reporter: 'html',
  use: {
    baseURL: 'http://localhost:5174',
    trace: 'on-first-retry',
    viewport: { width: 1440, height: 1024 },
    actionTimeout: 15_000,
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
  webServer: {
    // ⚠ PLAYWRIGHT_E2E=1 必须传进去；vite.config.ts server.hmr / server.watch
    // 读取该变量，没有这一段 vite 会保持 HMR WebSocket，evaluate 必 hang。
    // 使用 5174 端口 + reuseExistingServer: false，避免复用未设 PLAYWRIGHT_E2E=1
    // 的残留 dev server（5173 端口）导致 HMR WebSocket 干扰。
    command:
      process.platform === 'win32'
        ? 'set PLAYWRIGHT_E2E=1&& npx vite --port 5174 --strictPort'
        : 'PLAYWRIGHT_E2E=1 npx vite --port 5174 --strictPort',
    url: 'http://localhost:5174',
    reuseExistingServer: false,
    timeout: 90_000,
  },
});
