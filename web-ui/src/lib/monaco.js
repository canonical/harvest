import { isDarkTheme } from './theme.js';
import { nameOf } from './repository-browser.js';

const PLAIN_TEXT = 'plaintext';

let monacoModule = null;

export function loadMonaco() {
  monacoModule ??= import('monaco-editor');
  return monacoModule;
}

export function monacoTheme() {
  return isDarkTheme() ? 'vs-dark' : 'vs';
}

export function languageForPath(languages, path) {
  const name = nameOf(path).toLowerCase();
  const byFileName = languages.find((language) => (language.filenames ?? []).some((candidate) => candidate.toLowerCase() === name));
  if (byFileName) return byFileName.id;
  let best = { id: PLAIN_TEXT, length: 0 };
  for (const language of languages) {
    for (const extension of language.extensions ?? []) {
      const matches = name.endsWith(extension.toLowerCase());
      if (matches && extension.length > best.length) best = { id: language.id, length: extension.length };
    }
  }
  return best.id;
}
