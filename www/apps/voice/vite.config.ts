import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Voice app (P3-3): /dispatch/:room + /field/:room pages over one WS surface.
// Dev proxy mirrors the admin posture (PIT-106 lesson: ws:true on both).
export default defineConfig({
  plugins: [react()],
  server: {
    host: '0.0.0.0',
    port: 5175,
    proxy: {
      '/api': { target: 'http://127.0.0.1:9800', changeOrigin: true, ws: true },
      '/ws': { target: 'ws://127.0.0.1:9800', ws: true, changeOrigin: true },
    },
  },
  build: { outDir: 'dist', emptyOutDir: true },
});
