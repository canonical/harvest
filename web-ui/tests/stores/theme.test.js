import { describe, it, expect, beforeEach, vi } from 'vitest';
import { useThemeStore } from '../../src/stores/theme.js';

describe('useThemeStore', () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.removeAttribute('data-theme');
  });

  it('defaults to auto', () => {
    const store = useThemeStore();
    expect(store.theme).toBe('auto');
  });

  it('setTheme persists to localStorage', () => {
    const store = useThemeStore();
    store.setTheme('dark');
    expect(localStorage.getItem('theme')).toBe('dark');
    expect(store.theme).toBe('dark');
  });

  it('setTheme updates data-theme on documentElement for light/dark', () => {
    const store = useThemeStore();
    store.setTheme('dark');
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark');
    store.setTheme('light');
    expect(document.documentElement.getAttribute('data-theme')).toBe('light');
  });

  it('setTheme removes data-theme for auto', () => {
    const store = useThemeStore();
    store.setTheme('dark');
    store.setTheme('auto');
    expect(document.documentElement.hasAttribute('data-theme')).toBe(false);
    expect(localStorage.getItem('theme')).toBeNull();
  });

  it('nextTheme cycles auto → light → dark → auto', () => {
    const store = useThemeStore();
    expect(store.theme).toBe('auto');
    store.nextTheme();
    expect(store.theme).toBe('light');
    store.nextTheme();
    expect(store.theme).toBe('dark');
    store.nextTheme();
    expect(store.theme).toBe('auto');
  });

  it('loads stored theme on init', () => {
    localStorage.setItem('theme', 'dark');
    const store = useThemeStore();
    store.loadStored();
    expect(store.theme).toBe('dark');
  });

  it('icon and label vary by theme', () => {
    const store = useThemeStore();
    store.setTheme('light');
    const lightIcon = store.icon;
    store.setTheme('dark');
    const darkIcon = store.icon;
    expect(lightIcon).toBeTruthy();
    expect(darkIcon).toBeTruthy();
    expect(lightIcon).not.toBe(darkIcon);
  });
});
