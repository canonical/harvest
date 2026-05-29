import { describe, it, expect, beforeEach, vi } from 'vitest';
import { THEMES, getTheme, setTheme, nextTheme, getThemeIcon, getThemeLabel } from '../src/theme.js';

// ── setup: provide a minimal document.documentElement stub ───────────────────

beforeEach(() => {
  localStorage.clear();
  document.documentElement.removeAttribute('data-theme');
  vi.restoreAllMocks();
});

// ── THEMES constant ───────────────────────────────────────────────────────────

describe('THEMES', () => {
  it('contains exactly auto, light, dark in that order', () => {
    expect(THEMES).toEqual(['auto', 'light', 'dark']);
  });
});

// ── getTheme ──────────────────────────────────────────────────────────────────

describe('getTheme', () => {
  it('returns "auto" by default when nothing is stored', () => {
    expect(getTheme()).toBe('auto');
  });

  it('returns the value previously stored in localStorage', () => {
    localStorage.setItem('theme', 'dark');
    expect(getTheme()).toBe('dark');
  });

  it('returns "auto" when an unrecognised value is stored', () => {
    localStorage.setItem('theme', 'rainbow');
    expect(getTheme()).toBe('auto');
  });
});

// ── setTheme ──────────────────────────────────────────────────────────────────

describe('setTheme', () => {
  it('stores the theme in localStorage', () => {
    setTheme('dark');
    expect(localStorage.getItem('theme')).toBe('dark');
  });

  it('sets data-theme attribute on documentElement for light', () => {
    setTheme('light');
    expect(document.documentElement.getAttribute('data-theme')).toBe('light');
  });

  it('sets data-theme attribute on documentElement for dark', () => {
    setTheme('dark');
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark');
  });

  it('removes data-theme attribute when set to auto', () => {
    document.documentElement.setAttribute('data-theme', 'dark');
    setTheme('auto');
    expect(document.documentElement.hasAttribute('data-theme')).toBe(false);
  });

  it('stores "auto" in localStorage when set to auto', () => {
    setTheme('auto');
    expect(localStorage.getItem('theme')).toBe('auto');
  });
});

// ── nextTheme ─────────────────────────────────────────────────────────────────

describe('nextTheme', () => {
  it('cycles auto → light', () => {
    setTheme('auto');
    expect(nextTheme()).toBe('light');
  });

  it('cycles light → dark', () => {
    setTheme('light');
    expect(nextTheme()).toBe('dark');
  });

  it('cycles dark → auto', () => {
    setTheme('dark');
    expect(nextTheme()).toBe('auto');
  });

  it('applies the new theme as a side effect', () => {
    setTheme('light');
    nextTheme(); // → dark
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark');
  });

  it('returns the newly applied theme', () => {
    setTheme('auto');
    const applied = nextTheme();
    expect(applied).toBe('light');
    expect(getTheme()).toBe('light');
  });
});

// ── getThemeIcon ──────────────────────────────────────────────────────────────

describe('getThemeIcon', () => {
  it('returns a non-empty string for each theme', () => {
    for (const t of THEMES) {
      expect(typeof getThemeIcon(t)).toBe('string');
      expect(getThemeIcon(t).length).toBeGreaterThan(0);
    }
  });

  it('returns distinct icons for each theme', () => {
    const icons = THEMES.map(getThemeIcon);
    expect(new Set(icons).size).toBe(THEMES.length);
  });
});

// ── getThemeLabel ─────────────────────────────────────────────────────────────

describe('getThemeLabel', () => {
  it('returns a non-empty label for each theme', () => {
    for (const t of THEMES) {
      expect(getThemeLabel(t).length).toBeGreaterThan(0);
    }
  });

  it('includes the word "auto" in the auto label', () => {
    expect(getThemeLabel('auto').toLowerCase()).toContain('auto');
  });

  it('includes "light" in the light label', () => {
    expect(getThemeLabel('light').toLowerCase()).toContain('light');
  });

  it('includes "dark" in the dark label', () => {
    expect(getThemeLabel('dark').toLowerCase()).toContain('dark');
  });
});
