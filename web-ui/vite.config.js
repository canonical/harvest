import { defineConfig } from 'vite';

export default defineConfig({
  test: {
    environment: 'jsdom',
    globals: true,
    coverage: {
      provider: 'v8',
      reporter: ['text', 'json'],
    },
  },
  server: {
    port: 5173,
    proxy: {
      '/auth':             'http://localhost:8080',
      '/admin':            'http://localhost:8080',
      '/conversations':    'http://localhost:8080',
      '/groups':           'http://localhost:8080',
      '/projects':         'http://localhost:8080',
      '/query':            'http://localhost:8080',
      '/repositories':     'http://localhost:8080',
      '/graph':            'http://localhost:8080',
      '/health':           'http://localhost:8080',
      '/tool-description': 'http://localhost:8080',
      '/docs':             'http://localhost:8080',
      '/agents':           'http://localhost:8080',
      '/agent':            'http://localhost:8080',
    },
    allowedHosts: ["harvest-development.thinking-dragon.net"]
  },
});
