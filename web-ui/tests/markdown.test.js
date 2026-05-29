import { describe, it, expect } from 'vitest';
import { renderMarkdown, buildFileUrl, formatCitation, parseCitations } from '../src/markdown.js';

// ── renderMarkdown ────────────────────────────────────────────────────────────

describe('renderMarkdown', () => {
  it('renders bold text', () => {
    const html = renderMarkdown('**hello**');
    expect(html).toContain('<strong>hello</strong>');
  });

  it('renders italic text', () => {
    const html = renderMarkdown('_hi_');
    expect(html).toContain('<em>hi</em>');
  });

  it('renders a fenced code block with language class', () => {
    const html = renderMarkdown('```rust\nfn main() {}\n```');
    expect(html).toContain('class="language-rust"');
    // highlight.js wraps keywords in spans; verify the function name survives
    expect(html).toContain('main');
  });

  it('renders inline code', () => {
    const html = renderMarkdown('use `cargo build`');
    expect(html).toContain('<code>cargo build</code>');
  });

  it('renders an unordered list', () => {
    const html = renderMarkdown('- alpha\n- beta');
    expect(html).toContain('<ul>');
    expect(html).toContain('<li>alpha</li>');
  });

  it('renders headings', () => {
    const html = renderMarkdown('## Section');
    expect(html).toContain('<h2>Section</h2>');
  });

  it('escapes HTML in plain text to prevent XSS', () => {
    const html = renderMarkdown('<script>alert("xss")</script>');
    expect(html).not.toContain('<script>');
  });

  it('converts citation brackets to spans when no URL is available', () => {
    const html = renderMarkdown('See [myrepo:v1.0:src/lib.rs:42] for details.');
    expect(html).toContain('data-citation');
    expect(html).toContain('myrepo');
    expect(html).not.toContain('<a ');
  });

  it('converts citation brackets to links when repoUrlMap provides a URL', () => {
    const html = renderMarkdown(
      'See [myrepo:v1.0:src/lib.rs:42] for details.',
      { myrepo: 'https://github.com/owner/myrepo.git' },
    );
    expect(html).toContain('<a ');
    expect(html).toContain('href=');
    expect(html).toContain('github.com/owner/myrepo/blob/v1.0/src/lib.rs#L42');
    expect(html).toContain('class="citation"');
    expect(html).toContain('target="_blank"');
  });

  it('returns a string for empty input', () => {
    expect(typeof renderMarkdown('')).toBe('string');
  });
});

// ── buildFileUrl ──────────────────────────────────────────────────────────────

describe('buildFileUrl', () => {
  it('builds a GitHub blob URL', () => {
    const url = buildFileUrl('https://github.com/owner/repo.git', 'v1.0', 'src/lib.rs', 42);
    expect(url).toBe('https://github.com/owner/repo/blob/v1.0/src/lib.rs#L42');
  });

  it('strips .git suffix', () => {
    const url = buildFileUrl('https://github.com/owner/repo.git', 'main', 'README.md', 1);
    expect(url).not.toContain('.git');
  });

  it('converts SSH clone URL to HTTPS', () => {
    const url = buildFileUrl('git@github.com:owner/repo.git', 'v2.0', 'src/main.rs', 10);
    expect(url).toBe('https://github.com/owner/repo/blob/v2.0/src/main.rs#L10');
  });

  it('builds a GitLab URL with /-/blob/ path', () => {
    const url = buildFileUrl('https://gitlab.com/owner/repo.git', 'v1.0', 'lib/foo.py', 5);
    expect(url).toBe('https://gitlab.com/owner/repo/-/blob/v1.0/lib/foo.py#L5');
  });

  it('builds a Bitbucket URL', () => {
    const url = buildFileUrl('https://bitbucket.org/owner/repo.git', 'v1.0', 'src/foo.py', 7);
    expect(url).toBe('https://bitbucket.org/owner/repo/src/v1.0/src/foo.py#lines-7');
  });

  it('returns null for a null URL', () => {
    expect(buildFileUrl(null, 'v1', 'file.rs', 1)).toBeNull();
  });

  it('returns null for an empty URL', () => {
    expect(buildFileUrl('', 'v1', 'file.rs', 1)).toBeNull();
  });
});

// ── formatCitation ────────────────────────────────────────────────────────────

describe('formatCitation', () => {
  it('formats a source into a short readable label', () => {
    const label = formatCitation({ repo: 'harvest', version: 'v1.0', file: 'src/lib.rs', line: 42 });
    expect(label).toContain('harvest');
    expect(label).toContain('v1.0');
    expect(label).toContain('lib.rs');
    expect(label).toContain('42');
  });
});

// ── parseCitations ────────────────────────────────────────────────────────────

describe('parseCitations', () => {
  it('extracts a single citation', () => {
    const sources = parseCitations('see [repo:v1:src/lib.rs:10]');
    expect(sources).toHaveLength(1);
    expect(sources[0]).toMatchObject({ repo: 'repo', version: 'v1', file: 'src/lib.rs', line: 10 });
  });

  it('extracts multiple citations', () => {
    const sources = parseCitations('[a:v1:f.rs:1] and [b:v2:g.rs:5]');
    expect(sources).toHaveLength(2);
  });

  it('deduplicates identical citations', () => {
    const sources = parseCitations('[r:v1:f.rs:1] [r:v1:f.rs:1]');
    expect(sources).toHaveLength(1);
  });

  it('returns empty array when there are no citations', () => {
    const sources = parseCitations('plain text');
    expect(sources).toEqual([]);
  });

  it('ignores malformed brackets missing parts', () => {
    const sources = parseCitations('[only:two:fields]');
    expect(sources).toEqual([]);
  });
});
