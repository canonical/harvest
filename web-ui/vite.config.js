import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';

export default defineConfig({
  plugins: [vue()],
  server: {
    proxy: {
      '/auth':             'http://localhost:8080',
      '/query':            'http://localhost:8080',
      '/projects':         'http://localhost:8080',
      '/admin':            'http://localhost:8080',
      '/graph':            'http://localhost:8080',
      '/docs':             'http://localhost:8080',
      '/repositories':     'http://localhost:8080',
      '/machines':         'http://localhost:8080',
      '/health':           'http://localhost:8080',
      '/conversations':    'http://localhost:8080',
      '/groups':           'http://localhost:8080',
      '/skills':           'http://localhost:8080',
      '/agents':           'http://localhost:8080',
      '/agent':            'http://localhost:8080',
      '/tool-description': 'http://localhost:8080',
    },
    allowedHosts: [
      "harvest-development.thinking-dragon.net",
      "harvest-development-vue.thinking-dragon.net",
    ],
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./tests/setup.js'],
  },
});
