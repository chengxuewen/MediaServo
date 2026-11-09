import { defineConfig } from 'vite';

const proxy = {
  '/api': { target: 'http://127.0.0.1:9800', changeOrigin: true },
  '/ws': { target: 'ws://127.0.0.1:9800', ws: true },
};

export default defineConfig({
  server: { port: 5174, strictPort: true, proxy },
  preview: { port: 5174, strictPort: true, proxy },
});
