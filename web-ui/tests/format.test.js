import { describe, it, expect } from 'vitest';
import { renderJsonToHtml, renderPreviewToHtml } from '../src/format.js';

// ── renderJsonToHtml ──────────────────────────────────────────────────────────

describe('renderJsonToHtml — primitives', () => {
  it('renders null as an em dash placeholder', () => {
    expect(renderJsonToHtml(null)).toContain('—');
  });

  it('renders undefined as an em dash placeholder', () => {
    expect(renderJsonToHtml(undefined)).toContain('—');
  });

  it('renders a string as escaped text', () => {
    expect(renderJsonToHtml('hello world')).toContain('hello world');
  });

  it('escapes HTML in strings', () => {
    const html = renderJsonToHtml('<script>alert(1)</script>');
    expect(html).not.toContain('<script>');
    expect(html).toContain('&lt;script&gt;');
  });

  it('renders a number wrapped in code', () => {
    expect(renderJsonToHtml(42)).toContain('42');
  });

  it('renders true and false wrapped in code', () => {
    expect(renderJsonToHtml(true)).toContain('true');
    expect(renderJsonToHtml(false)).toContain('false');
  });
});

describe('renderJsonToHtml — plain objects', () => {
  it('returns a table element', () => {
    const html = renderJsonToHtml({ query: 'foo' });
    expect(html).toContain('<table');
  });

  it('renders each key as a row header', () => {
    const html = renderJsonToHtml({ query: 'foo', repo: 'bar' });
    expect(html).toContain('query');
    expect(html).toContain('repo');
  });

  it('renders the values in table cells', () => {
    const html = renderJsonToHtml({ query: 'foo' });
    expect(html).toContain('foo');
  });

  it('escapes keys and values', () => {
    const html = renderJsonToHtml({ '<key>': '<val>' });
    expect(html).not.toContain('<key>');
    expect(html).not.toContain('<val>');
    expect(html).toContain('&lt;key&gt;');
  });

  it('renders an empty object as a placeholder', () => {
    const html = renderJsonToHtml({});
    expect(html).toContain('—');
  });
});

describe('renderJsonToHtml — arrays of objects', () => {
  it('returns a table with thead for arrays of objects', () => {
    const html = renderJsonToHtml([
      { name: 'alpha', file: 'a.rs', score: 0.9 },
      { name: 'beta',  file: 'b.rs', score: 0.7 },
    ]);
    expect(html).toContain('<thead');
    expect(html).toContain('<tbody');
  });

  it('uses the object keys as column headers', () => {
    const html = renderJsonToHtml([{ name: 'foo', file: 'bar.rs' }]);
    expect(html).toContain('name');
    expect(html).toContain('file');
  });

  it('renders all rows', () => {
    const html = renderJsonToHtml([{ name: 'alpha' }, { name: 'beta' }]);
    expect(html).toContain('alpha');
    expect(html).toContain('beta');
  });

  it('handles missing keys in some rows with a placeholder', () => {
    const html = renderJsonToHtml([
      { name: 'alpha', score: 0.9 },
      { name: 'beta' },
    ]);
    expect(html).toContain('alpha');
    expect(html).toContain('beta');
    expect(html).toContain('—');
  });

  it('returns a placeholder for an empty array', () => {
    expect(renderJsonToHtml([])).toContain('—');
  });
});

describe('renderJsonToHtml — arrays of primitives', () => {
  it('renders primitive arrays as a list', () => {
    const html = renderJsonToHtml(['v1.0', 'v1.1', 'v2.0']);
    expect(html).toContain('v1.0');
    expect(html).toContain('v1.1');
    expect(html).toContain('v2.0');
  });

  it('escapes values in primitive arrays', () => {
    const html = renderJsonToHtml(['<b>bold</b>']);
    expect(html).not.toContain('<b>');
  });
});

// ── renderPreviewToHtml ───────────────────────────────────────────────────────

describe('renderPreviewToHtml', () => {
  it('returns empty string for null or empty input', () => {
    expect(renderPreviewToHtml(null)).toBe('');
    expect(renderPreviewToHtml('')).toBe('');
  });

  it('parses valid JSON array and returns a table', () => {
    const json = JSON.stringify([{ name: 'foo', score: 1 }]);
    const html = renderPreviewToHtml(json);
    expect(html).toContain('<table');
    expect(html).toContain('foo');
  });

  it('parses valid JSON object and returns a table', () => {
    const json = JSON.stringify({ repo: 'harvest', tag: 'v1' });
    const html = renderPreviewToHtml(json);
    expect(html).toContain('<table');
    expect(html).toContain('harvest');
  });

  it('shows a no-results message for an empty array', () => {
    const html = renderPreviewToHtml('[]');
    expect(html).not.toContain('<table');
    expect(html.toLowerCase()).toContain('no result');
  });

  it('recovers complete rows from a truncated JSON array', () => {
    const full = JSON.stringify([
      { name: 'alpha', score: 0.9 },
      { name: 'beta',  score: 0.7 },
      { name: 'gamma', score: 0.5 },
    ]);
    const truncated = full.slice(0, full.lastIndexOf('{') - 1); // drop last object
    const html = renderPreviewToHtml(truncated);
    expect(html).toContain('<table');
    expect(html).toContain('alpha');
  });

  it('includes a truncation notice when recovering a partial array', () => {
    const full = JSON.stringify([{ name: 'alpha' }, { name: 'beta' }, { name: 'gamma' }]);
    const truncated = full.slice(0, full.lastIndexOf('{') - 1);
    const html = renderPreviewToHtml(truncated);
    expect(html.toLowerCase()).toContain('truncat');
  });

  it('falls back to plain text for truncated non-array JSON', () => {
    const html = renderPreviewToHtml('{"incomplete":');
    expect(html).not.toContain('<table');
    expect(html).toContain('incomplete');
  });

  it('escapes plain text fallback content', () => {
    const html = renderPreviewToHtml('<not json>');
    expect(html).not.toContain('<not json>');
    expect(html).toContain('&lt;not json&gt;');
  });
});

// ── source code preview ───────────────────────────────────────────────────────

describe('renderPreviewToHtml — source code', () => {
  const sourceResult = JSON.stringify([{
    name: 'my_func',
    start_line: 10,
    end_line: 20,
    signature: 'fn my_func()',
    source: 'fn my_func() {\n    42\n}',
  }]);

  it('renders a code block instead of a table when all items have a source field', () => {
    const html = renderPreviewToHtml(sourceResult);
    expect(html).toContain('<pre');
    expect(html).not.toContain('<table');
  });

  it('includes the source text in the output', () => {
    const html = renderPreviewToHtml(sourceResult);
    expect(html).toContain('my_func');
  });

  it('shows name and line range in the meta header', () => {
    const html = renderPreviewToHtml(sourceResult);
    expect(html).toContain('my_func');
    expect(html).toContain('L10');
    expect(html).toContain('20');
  });

  it('sets the language class from the filePath extension', () => {
    const html = renderPreviewToHtml(sourceResult, 'src/lib.rs');
    expect(html).toContain('language-rust');
  });

  it('falls back to plaintext when extension is unknown', () => {
    const html = renderPreviewToHtml(sourceResult, 'src/file.xyz');
    expect(html).toContain('language-plaintext');
  });

  it('renders multiple items as separate blocks (e.g. diff view)', () => {
    const diff = JSON.stringify([
      { version: 'v1.0', start_line: 1, end_line: 3, source: 'old code' },
      { version: 'v2.0', start_line: 1, end_line: 4, source: 'new code' },
    ]);
    const html = renderPreviewToHtml(diff);
    expect(html).toContain('v1.0');
    expect(html).toContain('v2.0');
    expect(html).toContain('old code');
    expect(html).toContain('new code');
  });

  it('falls back to table rendering when not all items have a source field', () => {
    const mixed = JSON.stringify([{ name: 'foo', score: 0.9 }]);
    const html = renderPreviewToHtml(mixed);
    expect(html).toContain('<table');
    expect(html).not.toContain('<pre');
  });
});

// ── path shortening in table cells ───────────────────────────────────────────

describe('path shortening', () => {
  it('shortens long absolute paths in table cells', () => {
    const full = '/tmp/harvest-repos/my-repo/subdir/module/submodule/file.py';
    const html = renderJsonToHtml([{ path: full }]);
    const div = document.createElement('div');
    div.innerHTML = html;
    const span = div.querySelector('.tool-data__path');
    expect(span).not.toBeNull();
    // visible text is the shortened tail — no /tmp prefix
    expect(span.textContent).not.toContain('/tmp/harvest-repos');
    expect(span.textContent).toContain('file.py');
  });

  it('adds the full path as a title tooltip', () => {
    const full = '/tmp/harvest-repos/my-repo/subdir/module/submodule/file.py';
    const html = renderJsonToHtml([{ path: full }]);
    const div = document.createElement('div');
    div.innerHTML = html;
    const span = div.querySelector('.tool-data__path');
    expect(span.title).toBe(full);
  });

  it('leaves short absolute paths unchanged', () => {
    const html = renderJsonToHtml({ path: '/etc/hosts' });
    expect(html).toContain('/etc/hosts');
  });

  it('leaves relative paths unchanged', () => {
    const html = renderJsonToHtml([{ file: 'src/lib/module.py' }]);
    expect(html).toContain('src/lib/module.py');
  });
});
