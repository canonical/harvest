import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, vi } from 'vitest';
import { config } from '@vue/test-utils';

beforeEach(() => {
  setActivePinia(createPinia());
});

if (typeof globalThis.EventSource === 'undefined') {
  globalThis.EventSource = class EventSource {
    constructor() { this.onmessage = null; this.onerror = null; }
    close() {}
  };
}

if (typeof globalThis.ResizeObserver === 'undefined') {
  globalThis.ResizeObserver = class ResizeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

config.global.stubs = {
  ...config.global.stubs,
  RouterLink: {
    props: ['to'],
    template: '<a :href="typeof to === \'string\' ? to : to.path"><slot /></a>',
  },
};
