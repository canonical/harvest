import { describe, it, expect } from 'vitest';
import {
  TAB,
  citation,
  firstTab,
  displayName,
  groupByFile,
  hasSourceLines,
  highlightedLines,
  isCodeSearchView,
  itemIndexAt,
  kindLetter,
  languageOf,
  locationLabel,
  plainLines,
  summarize,
} from '../../src/lib/code-search-view.js';

const item = (name, file, start = 1, end = 5) => ({ kind: 'function', name, file, start_line: start, end_line: end, code: 'fn a() {}', calls: [] });

const view = {
  kind: 'code_search', repo: 'amafu', version: 'main', tier: 'tier1', total_ms: 12.4,
  items: [item('run', 'crates/command-line/src/install.rs', 33, 51), item('MaasClient', 'crates/installer/src/maas.rs', 86, 90), item('MaasClient.find', 'crates/installer/src/maas.rs', 160, 169)],
  documentation: [{ name: 'Decision', file: 'docs/0022.md', start_line: 5, end_line: 23, excerpt: 'x', points_to: [] }],
  gaps: ['MaasClient.state is missing'],
};

describe('code search view helpers', () => {
  it('recognises a code search view', () => {
    expect(isCodeSearchView(view)).toBe(true);
    expect(isCodeSearchView({ kind: 'other' })).toBe(false);
    expect(isCodeSearchView(undefined)).toBe(false);
  });

  it('groups items by file in first-seen order and keeps their indexes', () => {
    const groups = groupByFile(view.items);
    expect(groups.map(g => g.file)).toEqual(['crates/command-line/src/install.rs', 'crates/installer/src/maas.rs']);
    expect(groups[1].entries.map(e => e.index)).toEqual([1, 2]);
  });

  it('names modules by their file and numbers only real source lines', () => {
    const module = { kind: 'module', name: 'crates/domain/src/install.rs', file: 'crates/domain/src/install.rs' };
    const method = { kind: 'method', name: 'MaasClient.find', file: 'crates/installer/src/maas.rs' };
    expect(displayName(module)).toBe('install.rs');
    expect(displayName(method)).toBe('MaasClient.find');
    expect(hasSourceLines(module)).toBe(false);
    expect(hasSourceLines(method)).toBe(true);
    expect(hasSourceLines({ kind: 'table', file: '' })).toBe(false);
  });

  it('puts tables after the code files', () => {
    const table = { kind: 'table', name: 'callers of run', file: '' };
    const groups = groupByFile([table, ...view.items]);
    expect(groups.map(g => g.file)).toEqual(['crates/command-line/src/install.rs', 'crates/installer/src/maas.rs', '']);
  });

  it('summarises counts, tier and rounded time', () => {
    expect(summarize(view)).toEqual({ items: 3, files: 2, documentation: 1, gaps: 1, tier: 'tier1', milliseconds: 12 });
  });

  it('finds the item a call or a documentation pointer refers to', () => {
    expect(itemIndexAt(view, { name: 'MaasClient.find', file: 'crates/installer/src/maas.rs', line: 160 })).toBe(2);
    expect(itemIndexAt(view, { name: 'MaasClient.state', file: 'crates/installer/src/maas.rs', line: 174 })).toBe(-1);
  });

  it('labels locations and citations like the answer does', () => {
    expect(locationLabel(view.items[0])).toBe('crates/command-line/src/install.rs:33-51');
    expect(locationLabel({ file: 'a.rs', start_line: 7, end_line: 7 })).toBe('a.rs:7');
    expect(locationLabel({ file: '', start_line: 0, end_line: 0 })).toBe('');
    expect(citation(view, view.items[1])).toBe('[amafu:main:crates/installer/src/maas.rs:86-90]');
  });

  it('picks a highlighting language from the file extension', () => {
    expect(languageOf('crates/a.rs')).toBe('rust');
    expect(languageOf('cinder/volume/driver.py')).toBe('python');
    expect(languageOf('Makefile')).toBe('plaintext');
  });

  it('numbers highlighted lines from the start line and keeps spans balanced per line', () => {
    const lines = highlightedLines('/* a\nb */\nfn x() {}', 'a.rs', 10);
    expect(lines.map(l => l.number)).toEqual([10, 11, 12]);
    for (const line of lines) {
      expect((line.html.match(/<span/g) ?? []).length).toBe((line.html.match(/<\/span>/g) ?? []).length);
    }
    expect(lines[2].html).toContain('hljs-keyword');
  });

  it('escapes plain text lines', () => {
    expect(plainLines('<b>\nok')).toEqual(['&lt;b&gt;', 'ok']);
  });

  it('opens on the first tab that has something to show', () => {
    expect(firstTab(view)).toBe(TAB.CODE);
    expect(firstTab({ ...view, items: [] })).toBe(TAB.DOCS);
    expect(firstTab({ ...view, items: [], documentation: [] })).toBe(TAB.GAPS);
  });

  it('shows the first letter of a kind', () => {
    expect(kindLetter('method')).toBe('M');
    expect(kindLetter(undefined)).toBe('?');
  });
});
