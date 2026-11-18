export const TERMINAL_FONT = "'Ubuntu Mono', 'DejaVu Sans Mono', monospace";

const UBUNTU_PALETTE = {
  black: '#171421',
  red: '#c01c28',
  green: '#26a269',
  yellow: '#a2734c',
  blue: '#12488b',
  magenta: '#a347ba',
  cyan: '#2aa1b3',
  white: '#d0cfcc',
  brightBlack: '#5e5c64',
  brightRed: '#f66151',
  brightGreen: '#33d17a',
  brightYellow: '#e9ad0c',
  brightBlue: '#2a7bde',
  brightMagenta: '#c061cb',
  brightCyan: '#33c7de',
  brightWhite: '#ffffff',
};

function token(styles, name, fallback) {
  return styles.getPropertyValue(name).trim() || fallback;
}

export function terminalTheme(styles = getComputedStyle(document.documentElement)) {
  return {
    ...UBUNTU_PALETTE,
    background: token(styles, '--bg-code', '#0d0d0d'),
    foreground: token(styles, '--text-code', '#f0f0f0'),
    cursor: '#e95420',
    cursorAccent: '#0d0d0d',
    selectionBackground: 'rgba(233, 84, 32, 0.35)',
  };
}
