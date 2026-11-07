import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';

// В dev API и WebSocket живут на отдельном порту (Rust-сервер 8081),
// поэтому проксируем их с Vite-дев-сервера. В проде всё отдаётся с одного
// origin, и клиент ходит на относительные URL.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    port: 5173,
    strictPort: true,
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:8081',
        changeOrigin: true,
      },
      '/ws': {
        target: 'ws://127.0.0.1:8081',
        changeOrigin: true,
        ws: true,
      },
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    sourcemap: false,
  },
  preview: {
    port: 4173,
  },
});
