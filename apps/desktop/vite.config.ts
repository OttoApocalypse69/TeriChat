import react from '@vitejs/plugin-react';
import { defineConfig } from 'vitest/config';

// Tauri dev server must run on a fixed port (see src-tauri/tauri.conf.json devUrl).
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    // Tailwind CSS 4 output needs native cascade layers, @property and
    // color-mix(): Safari/WKWebView 16.4+ (macOS 11+ with current Safari),
    // WebView2/Chromium 111+, WebKitGTK with matching support.
    target: 'safari16.4',
  },
  test: {
    environment: 'node',
    include: ['src/**/*.test.{ts,tsx}'],
  },
});
