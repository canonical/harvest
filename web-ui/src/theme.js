export const THEMES = /** @type {const} */ (['auto', 'light', 'dark']);

const STORAGE_KEY = 'theme';

/** Return the current persisted theme, defaulting to 'auto'. */
export function getTheme() {
  const stored = localStorage.getItem(STORAGE_KEY);
  return THEMES.includes(stored) ? stored : 'auto';
}

/**
 * Apply a theme by setting/removing the data-theme attribute on <html>
 * and persisting the choice to localStorage.
 * @param {'auto'|'light'|'dark'} theme
 */
export function setTheme(theme) {
  localStorage.setItem(STORAGE_KEY, theme);
  if (theme === 'auto') {
    document.documentElement.removeAttribute('data-theme');
  } else {
    document.documentElement.setAttribute('data-theme', theme);
  }
}

/**
 * Advance to the next theme in the cycle (auto → light → dark → auto),
 * apply it, and return the new theme.
 * @returns {'auto'|'light'|'dark'}
 */
export function nextTheme() {
  const current = getTheme();
  const idx = THEMES.indexOf(current);
  const next = THEMES[(idx + 1) % THEMES.length];
  setTheme(next);
  return next;
}

/** Apply the stored theme on page load. */
export function applyStoredTheme() {
  setTheme(getTheme());
}

const ICONS = {
  auto:  '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="5"/><path d="M12 2v2M12 20v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M2 12h2M20 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42"/><path d="M12 2a10 10 0 0 1 0 20" stroke-dasharray="2 2"/></svg>',
  light: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="5"/><path d="M12 2v2M12 20v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M2 12h2M20 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42"/></svg>',
  dark:  '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/></svg>',
};

const LABELS = {
  auto:  'Auto (system)',
  light: 'Light mode',
  dark:  'Dark mode',
};

/** @param {'auto'|'light'|'dark'} theme */
export function getThemeIcon(theme) { return ICONS[theme] ?? ICONS.auto; }

/** @param {'auto'|'light'|'dark'} theme */
export function getThemeLabel(theme) { return LABELS[theme] ?? LABELS.auto; }
