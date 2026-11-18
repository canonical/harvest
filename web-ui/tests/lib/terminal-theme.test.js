import { describe, it, expect } from 'vitest';
import { terminalTheme } from '../../src/lib/terminal-theme.js';

const styles = (tokens) => ({ getPropertyValue: (name) => tokens[name] ?? '' });

describe('terminalTheme', () => {
  it('uses the app code colours with the Ubuntu palette and an orange cursor', () => {
    const theme = terminalTheme(styles({ '--bg-code': ' #1a1a1a', '--text-code': '#f0f0f0' }));
    expect(theme).toMatchObject({ background: '#1a1a1a', foreground: '#f0f0f0', cursor: '#e95420', red: '#c01c28', brightGreen: '#33d17a' });
  });

  it('falls back to dark defaults', () => {
    expect(terminalTheme(styles({}))).toMatchObject({ background: '#0d0d0d', foreground: '#f0f0f0' });
  });
});
