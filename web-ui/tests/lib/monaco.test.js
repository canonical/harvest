import { describe, it, expect } from 'vitest';
import { languageForPath } from '../../src/lib/monaco.js';

const languages = [
  { id: 'typescript', extensions: ['.ts'] },
  { id: 'typescript-declaration', extensions: ['.d.ts'] },
  { id: 'rust', extensions: ['.rs'] },
  { id: 'dockerfile', extensions: ['.dockerfile'], filenames: ['Dockerfile'] },
];

describe('languageForPath', () => {
  it('matches the extension of the file name', () => {
    expect(languageForPath(languages, 'src/main.rs')).toBe('rust');
    expect(languageForPath(languages, 'web/App.TS')).toBe('typescript');
  });

  it('prefers the longest matching extension', () => {
    expect(languageForPath(languages, 'types/index.d.ts')).toBe('typescript-declaration');
  });

  it('recognises well-known file names', () => {
    expect(languageForPath(languages, 'build/Dockerfile')).toBe('dockerfile');
  });

  it('falls back to plain text', () => {
    expect(languageForPath(languages, 'LICENSE')).toBe('plaintext');
  });
});
