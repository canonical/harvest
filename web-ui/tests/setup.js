import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, vi } from 'vitest';

beforeEach(() => {
  setActivePinia(createPinia());
});

// jsdom doesn't implement EventSource — provide a minimal stub
if (typeof globalThis.EventSource === 'undefined') {
  globalThis.EventSource = class EventSource {
    constructor() { this.onmessage = null; this.onerror = null; }
    close() {}
  };
}
