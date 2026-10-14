import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Branding: the VITE_APP_TITLE env injects the admin title (default "MediaServo Admin" unchanged).
const APP_TITLE = process.env.VITE_APP_TITLE || 'MediaServo Admin';

export default defineConfig({
  plugins: [react()],
  define: { __APP_TITLE__: JSON.stringify(APP_TITLE) },
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
