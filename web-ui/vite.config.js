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
      '/query': 'http://localhost:8080',
      '/repositories': 'http://localhost:8080',
      '/graph': 'http://localhost:8080',
      '/health': 'http://localhost:8080',
      '/tool-description': 'http://localhost:8080',
    },
  },
});
