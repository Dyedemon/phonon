import { defineConfig, devices } from '@playwright/test';
export default defineConfig({
  testDir: './e2e',
  testMatch: '_tmp-flicker.spec.ts',
  workers: 1,
  reporter: 'list',
  timeout: 180_000,
  use: { baseURL: 'http://localhost:5174', viewport: { width: 960, height: 640 }, channel: 'msedge' },
  projects: [{ name: 'edge', use: { ...devices['Desktop Chrome'], channel: 'msedge' } }],
  webServer: {
    command: process.platform === 'win32'
      ? 'set PLAYWRIGHT_E2E=1&& npx vite --port 5174 --strictPort'
      : 'PLAYWRIGHT_E2E=1 npx vite --port 5174 --strictPort',
    url: 'http://localhost:5174',
    reuseExistingServer: false,
    timeout: 90_000,
  },
});
