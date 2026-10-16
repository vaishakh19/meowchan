import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';

const resolve = (p: string) => fileURLToPath(new URL(p, import.meta.url));

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': resolve('./src'),
    },
  },
  // Must stay in sync with `src-tauri/tauri.conf.json` -> build.devUrl
  server: {
    port: 1420,
    strictPort: true,
    host: true,
    // The dev server is only ever reached through `npm run dev`. Remote preview hosts are
    // proxied through an arena hostname that Vite would otherwise reject with a 403, so the
    // dev/preview servers accept them. The packaged app serves its own files and is unaffected.
    allowedHosts: true,
    watch: {
      // Avoid rescanning the (potentially huge) Rust target dir.
      ignored: ['**/src-tauri/target/**', '**/node_modules/**'],
    },
  },
  preview: {
    port: 1420,
    strictPort: true,
    host: true,
    // Same reasoning as `server.allowedHosts`: this is the dev-time `vite preview` path only.
    allowedHosts: true,
  },
  build: {
    target: 'es2022',
    sourcemap: false,
    chunkSizeWarningLimit: 1400,
    rollupOptions: {
      output: {
        // Keep the vendor chunks stable so a code change does not invalidate React and D3.
        manualChunks(id: string) {
          if (!id.includes('node_modules')) return undefined;
          if (id.includes('/d3/') || id.includes('d3-') || id.includes('/internmap/')) return 'd3';
          if (
            id.includes('/react/') ||
            id.includes('/react-dom/') ||
            id.includes('/scheduler/')
          ) {
            return 'react';
          }
          return undefined;
        },
      },
    },
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test/setup.ts'],
    include: ['src/**/*.{test,spec}.{ts,tsx}', 'tests/**/*.{test,spec}.{ts,tsx}'],
    css: false,
    restoreMocks: true,
  },
});
