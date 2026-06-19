import { defineStore } from 'pinia';
import { ref, computed } from 'vue';

const CYCLE = ['auto', 'light', 'dark'];
const ICONS  = { auto: '☀', light: '☀', dark: '☾' };
const LABELS = { auto: 'Auto', light: 'Light', dark: 'Dark' };

export const useThemeStore = defineStore('theme', () => {
  const theme = ref('auto');

  function setTheme(t) {
    theme.value = t;
    if (t === 'auto') {
      localStorage.removeItem('theme');
      document.documentElement.removeAttribute('data-theme');
    } else {
      localStorage.setItem('theme', t);
      document.documentElement.setAttribute('data-theme', t);
    }
  }

  function nextTheme() {
    const idx = CYCLE.indexOf(theme.value);
    setTheme(CYCLE[(idx + 1) % CYCLE.length]);
  }

  function loadStored() {
    const stored = localStorage.getItem('theme');
    if (stored === 'light' || stored === 'dark') {
      theme.value = stored;
      document.documentElement.setAttribute('data-theme', stored);
    }
  }

  const icon  = computed(() => ICONS[theme.value]  ?? ICONS.auto);
  const label = computed(() => LABELS[theme.value] ?? LABELS.auto);

  return { theme, setTheme, nextTheme, loadStored, icon, label };
});
