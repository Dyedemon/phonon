/// <reference types="vitest" />
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { resolve } from 'path';

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  // Prevent Vite from pre-bundling Tauri APIs — they need
  // window.__TAURI_INTERNALS__ which is only available in the Tauri webview.
  optimizeDeps: {
    exclude: ['@tauri-apps/api', '@tauri-apps/plugin-dialog'],
  },
  build: {
    rollupOptions: {
      external: ['@tauri-apps/api', '@tauri-apps/plugin-dialog'],
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
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test/setup.ts'],
    include: ['src/**/*.{test,spec}.{ts,tsx}'],
  },
});
