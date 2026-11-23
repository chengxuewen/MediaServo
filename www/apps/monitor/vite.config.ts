import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Branding: VITE_APP_TITLE injects the monitor title (default "MediaServo Monitor").
const APP_TITLE = process.env.VITE_APP_TITLE || 'MediaServo Monitor';

// Proxy mirrors the admin/voice posture (PIT-106 lesson: ws:true on both /api and
// /ws). Dev AND preview need it — login (/api) plus signaling WS (/ws) are same-origin.
const proxy = {
  '/api': { target: 'http://127.0.0.1:9800', changeOrigin: true, ws: true },
  '/ws': { target: 'ws://127.0.0.1:9800', ws: true, changeOrigin: true },
};

export default defineConfig({
  plugins: [react()],
  define: { __APP_TITLE__: JSON.stringify(APP_TITLE) },
  server: {
    host: '0.0.0.0',
    port: 5176,
    proxy,
  },
  preview: {
    port: 5176,
    proxy,
  },
  build: {
    outDir: 'dist',
    sourcemap: true,
  },
});