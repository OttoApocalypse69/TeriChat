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
    target: 'safari13',
  },
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
  },
});
