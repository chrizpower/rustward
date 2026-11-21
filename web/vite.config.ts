import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vite';

const host = '127.0.0.1';
const port = 5173;
const previewOrigin = 'http://127.0.0.1:4173';

export default defineConfig({
  plugins: [svelte()],
  server: {
    host,
    port,
    strictPort: true,
    proxy: {
      '/api': {
        target: previewOrigin,
        changeOrigin: true,
        configure(proxy) {
          // Only requests from this development console use the preview origin.
          proxy.on('proxyReq', (outgoing, incoming) => {
            if (incoming.headers.origin === `http://${host}:${port}`) {
              outgoing.setHeader('origin', previewOrigin);
            }
          });
        },
      },
    },
  },
  build: {
    modulePreload: false,
    rollupOptions: {
      output: {
        inlineDynamicImports: true,
        entryFileNames: 'console.js',
        assetFileNames: 'console[extname]',
      },
    },
  },
});
