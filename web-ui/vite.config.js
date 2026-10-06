import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';
import { fileURLToPath } from 'node:url';

const monacoStub = fileURLToPath(new URL('./tests/stubs/monaco-editor.js', import.meta.url));

// Overridable so `vite preview` (scripts/start-harvest.sh) can follow the port
// set in server.toml and accept the instance's public hostname.
const backend = process.env.HARVEST_BACKEND_URL || 'http://localhost:8080';
const extraAllowedHosts = (process.env.HARVEST_ALLOWED_HOSTS || '')
  .split(',').map((h) => h.trim()).filter(Boolean);

export default defineConfig({
  plugins: [vue()],
  server: {
    proxy: {
      '/auth':             backend,
      '/query':            backend,
      '/projects':         { target: backend, ws: true },
      '/admin':            backend,
      '/graph':            backend,
      '/docs':             backend,
      '/repositories':     backend,
      '/machines':         backend,
      '/health':           backend,
      '/conversations':    backend,
      '/chat-layouts':     backend,
      '/groups':           backend,
      '/templates':        backend,
      '/skills':           backend,
      '/agents':           backend,
      '/agent':            { target: backend, ws: true },
      '/tool-description': backend,
      '/llm':              backend,
      '/artifacts':        backend,
    },
    allowedHosts: [
      "harvest-development.thinking-dragon.net",
      "harvest-development-vue.thinking-dragon.net",
      ...extraAllowedHosts,
    ],
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./tests/setup.js'],
    alias: {
      'monaco-editor': monacoStub,
    },
    server: {
      deps: {
        inline: ['monaco-editor'],
      },
    },
  },
});
