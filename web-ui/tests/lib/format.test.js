import { describe, it, expect } from 'vitest';
import { renderPreviewToHtml } from '../../src/lib/format.js';

function stripTags(html) {
  return html.replace(/<[^>]+>/g, '');
}

describe('renderPreviewToHtml link preview', () => {
  it('renders a __type: link payload as an anchor with the given href and label', () => {
    const payload = JSON.stringify({ __type: 'link', href: '#/artifacts/abc-123', label: 'Open Deploy report' });
    const html = renderPreviewToHtml(payload);
    expect(html).toContain('<a');
    expect(html).toContain('href="#/artifacts/abc-123"');
    expect(html).toContain('Open Deploy report');
  });

  it('escapes label and href to avoid injecting markup', () => {
    const payload = JSON.stringify({ __type: 'link', href: '"><script>x</script>', label: '<b>x</b>' });
    const html = renderPreviewToHtml(payload);
    expect(html).not.toContain('<script>');
    expect(html).not.toContain('<b>x</b>');
  });
});

describe('renderPreviewToHtml source rendering', () => {
  it('renders an array of items with a source field as highlighted code blocks', () => {
    const payload = JSON.stringify([
      { name: 'foo', start_line: 1, end_line: 3, source: 'fn foo() {}' },
    ]);
    const html = renderPreviewToHtml(payload, 'main.rs');
    expect(html).toContain('tool-source__pre');
    expect(stripTags(html)).toContain('fn foo');
    expect(html).not.toContain('tool-data__table');
  });

  it('still renders items with a source field even when a sibling item is missing one', () => {
    const payload = JSON.stringify([
      { name: 'foo', start_line: 1, end_line: 3, source: 'fn foo() {}' },
      { name: 'bar', start_line: 10, end_line: null, source: null },
    ]);
    const html = renderPreviewToHtml(payload);
    expect(html).toContain('tool-source__pre');
    expect(html).toContain('fn foo');
    expect(html).not.toContain('tool-data__table');
  });

  it('recognizes "code" as a source-like field name for ad-hoc query results', () => {
    const payload = JSON.stringify([{ name: 'foo', code: 'def foo():\n    return 1' }]);
    const html = renderPreviewToHtml(payload);
    expect(html).toContain('tool-source__pre');
    expect(html).toContain('def foo');
  });

  it('recognizes "content" as a source-like field name for ad-hoc query results', () => {
    const payload = JSON.stringify([{ name: 'bar', content: 'class Bar {}' }]);
    const html = renderPreviewToHtml(payload);
    expect(html).toContain('tool-source__pre');
    expect(html).toContain('class Bar');
  });

  it('ignores an empty source string on an item', () => {
    const payload = JSON.stringify([
      { name: 'foo', source: 'fn foo() {}' },
      { name: 'bar', source: '' },
    ]);
    const html = renderPreviewToHtml(payload);
    expect(html).toContain('tool-source__pre');
    expect(html).toContain('fn foo');
  });

  it('renders as a normal table when no item has a source-like field', () => {
    const payload = JSON.stringify([{ name: 'foo', start_line: 1, score: 0.9 }]);
    const html = renderPreviewToHtml(payload);
    expect(html).toContain('tool-data__table');
    expect(html).not.toContain('tool-source__pre');
  });

  it('recovers a source block from truncated JSON ending mid-source-string', () => {
    const truncated = '[{"name":"foo","start_line":1,"end_line":10,"source":"fn foo() {\\n    do_thing();';
    const html = renderPreviewToHtml(truncated);
    expect(html).toContain('tool-source__pre');
    expect(html).toContain('do_thing');
    expect(html).toContain('tool-data__truncated');
  });

  it('recovers a source block from truncated JSON when the field is named "code"', () => {
    const truncated = '[{"name":"foo","code":"def foo():\\n    return 1';
    const html = renderPreviewToHtml(truncated);
    expect(html).toContain('tool-source__pre');
    expect(html).toContain('def foo');
  });

  it('recovers a source block from truncated JSON when the field is named "content"', () => {
    const truncated = '[{"name":"foo","content":"class Foo {\\n  int x;';
    const html = renderPreviewToHtml(truncated);
    expect(html).toContain('tool-source__pre');
    expect(html).toContain('class Foo');
  });
});

describe('renderPreviewToHtml fallback rendering', () => {
  it('renders non-JSON text as a highlighted, escaped code block instead of flat text', () => {
    const html = renderPreviewToHtml('<div class="report">hello</div>');
    expect(html).toContain('<pre');
    expect(html).toContain('class="hljs');
    expect(html).not.toContain('<div class="report">');
    expect(html).toContain('hello');
  });

  it('preserves newlines in fallback multi-line text', () => {
    const text = 'line one\nline two\nline three';
    const html = renderPreviewToHtml(text);
    expect(html).toContain('<pre');
    expect(html).toContain('line one');
    expect(html).toContain('line two');
    expect(html).toContain('line three');
  });

  it('still renders plain single-line text without crashing', () => {
    const html = renderPreviewToHtml('Deleted successfully');
    expect(html).toContain('Deleted successfully');
  });
});
