import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// A (shell-stays): the monitor realtime face is shared source consumed by the admin
// SPA route (single shell) AND the standalone monitor app (desktop/deep-link).
const here = path.dirname(fileURLToPath(import.meta.url));

// Branding: the VITE_APP_TITLE env injects the admin title (default "MediaServo Admin" unchanged).
const APP_TITLE = process.env.VITE_APP_TITLE || 'MediaServo Admin';

export default defineConfig({
  plugins: [react()],
  define: { __APP_TITLE__: JSON.stringify(APP_TITLE) },
  resolve: { alias: { '@monitor': path.resolve(here, '../monitor/src') } },
  server: {
    host: '0.0.0.0',  // allow external access
    port: 5173,
    proxy: {
      // PIT-106 (I2 review): the events WS (/api/admin/events) previously lacked ws:true → under 5173 it
      // stayed stuck at CONNECTING forever and piled up half-open sockets that dragged down the proxy (/ws video was affected too)
      '/api': { target: 'http://127.0.0.1:9800', changeOrigin: true, ws: true },
      '/ws': { target: 'ws://127.0.0.1:9800', ws: true, changeOrigin: true },
    },
  },
  build: {
    outDir: 'dist',
    sourcemap: true,
  },
});
