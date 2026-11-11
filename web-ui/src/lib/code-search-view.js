import hljs from 'highlight.js';
import { escapeHtml } from './utils.js';

export const VIEW_KIND = 'code_search';

export const TAB = Object.freeze({
  CODE: 'code',
  DOCS: 'docs',
  GAPS: 'gaps',
});

const LANGUAGE_BY_EXTENSION = {
  rs: 'rust', py: 'python', js: 'javascript', mjs: 'javascript', cjs: 'javascript', jsx: 'javascript',
  ts: 'typescript', tsx: 'typescript', go: 'go', java: 'java', kt: 'kotlin', kts: 'kotlin',
  scala: 'scala', c: 'c', h: 'c', cc: 'cpp', cpp: 'cpp', cxx: 'cpp', hpp: 'cpp', cs: 'csharp',
  rb: 'ruby', php: 'php', swift: 'swift', lua: 'lua', sh: 'bash', bash: 'bash', ex: 'elixir',
  exs: 'elixir', hs: 'haskell', ml: 'ocaml', r: 'r', pl: 'perl', pm: 'perl', sql: 'sql',
  yaml: 'yaml', yml: 'yaml', toml: 'ini', ini: 'ini', cfg: 'ini', json: 'json', md: 'markdown',
  dart: 'dart', erl: 'erlang', fs: 'fsharp', vue: 'xml', html: 'xml', xml: 'xml', proto: 'protobuf',
};

export function isCodeSearchView(view) {
  return view?.kind === VIEW_KIND;
}

export function languageOf(file) {
  const extension = String(file ?? '').split('.').pop().toLowerCase();
  const language = LANGUAGE_BY_EXTENSION[extension];
  return language && hljs.getLanguage(language) ? language : 'plaintext';
}

export function highlightedLines(code, file, startLine = 1) {
  const html = hljs.highlight(code ?? '', { language: languageOf(file), ignoreIllegals: true }).value;
  return splitHighlighted(html).map((line, i) => ({ number: startLine + i, html: line }));
}

function splitHighlighted(html) {
  const lines = [];
  let open = [];
  for (const raw of html.split('\n')) {
    const line = open.join('') + raw;
    for (const tag of raw.match(/<span[^>]*>|<\/span>/g) ?? []) {
      if (tag === '</span>') open.pop();
      else open.push(tag);
    }
    lines.push(line + '</span>'.repeat(open.length));
  }
  return lines;
}

export function plainLines(text) {
  return String(text ?? '').split('\n').map(escapeHtml);
}

export function groupByFile(items) {
  const groups = [];
  items.forEach((item, index) => {
    let group = groups.find(g => g.file === item.file);
    if (!group) {
      group = { file: item.file, entries: [] };
      groups.push(group);
    }
    group.entries.push({ item, index });
  });
  return groups;
}

export function summarize(view) {
  const files = new Set(view.items.map(i => i.file).filter(Boolean)).size;
  return {
    items: view.items.length,
    files,
    documentation: view.documentation.length,
    gaps: view.gaps.length,
    tier: view.tier ?? null,
    milliseconds: view.total_ms == null ? null : Math.round(view.total_ms),
  };
}

export function itemIndexAt(view, location) {
  return view.items.findIndex(item =>
    item.name === location.name && item.file === location.file
  );
}

export function locationLabel(entry) {
  if (!entry.file) return '';
  const end = entry.end_line && entry.end_line !== entry.start_line ? `-${entry.end_line}` : '';
  return `${entry.file}:${entry.start_line}${end}`;
}

export function citation(view, entry) {
  return `[${view.repo}:${view.version}:${locationLabel(entry)}]`;
}

export function kindLetter(kind) {
  return (kind ?? '?').charAt(0).toUpperCase();
}

export function firstTab(view) {
  if (view.items.length) return TAB.CODE;
  if (view.documentation.length) return TAB.DOCS;
  return TAB.GAPS;
}
