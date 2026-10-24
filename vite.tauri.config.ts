import { resolve } from 'node:path'
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import svgr from 'vite-plugin-svgr'

export default defineConfig({
  root: 'src/renderer',
  server: {
    host: '127.0.0.1',
    port: 5173,
    strictPort: true,
    // Monaco and the large renderer tree can exhaust inotify/file handles on
    // constrained Linux desktop sessions when Tauri starts Cargo alongside
    // Vite. Polling keeps the dev server reliable in those environments.
    watch: {
      usePolling: true,
      interval: 500
    }
  },
  plugins: [react(), tailwindcss(), svgr()],
  resolve: {
    alias: {
      '@': resolve('src/renderer/src'),
      '@renderer': resolve('src/renderer/src'),
      '@shared': resolve('src/shared')
    }
  },
  build: {
    outDir: resolve('dist/renderer'),
    emptyOutDir: false,
    // Monaco and the existing workbench are intentionally kept in one
    // renderer bundle for parity with the Electron frontend. This threshold
    // prevents a non-fatal size warning from being mistaken for a build error.
    chunkSizeWarningLimit: 6000
  }
})
