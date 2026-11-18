import { describe, it, expect, vi } from 'vitest';

const webglShouldFail = { value: false };

vi.mock('@xterm/addon-webgl', () => ({
  WebglAddon: class {
    constructor() {
      if (webglShouldFail.value) throw new Error('no WebGL');
    }
    onContextLoss() {}
  },
}));

const { loadRenderer } = await import('../../src/lib/terminal-addons.js');

describe('loadRenderer', () => {
  it('uses WebGL when the browser offers it', () => {
    webglShouldFail.value = false;
    const terminal = { loadAddon: vi.fn() };
    expect(loadRenderer(terminal)).toBe('webgl');
    expect(terminal.loadAddon).toHaveBeenCalledTimes(1);
  });

  it('falls back to the default renderer without WebGL', () => {
    webglShouldFail.value = true;
    expect(loadRenderer({ loadAddon: vi.fn() })).toBe('dom');
  });
});
