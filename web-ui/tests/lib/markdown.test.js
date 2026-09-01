import { describe, it, expect } from 'vitest';
import { renderMarkdown, buildFileUrl, buildCitationIndex } from '../../src/lib/markdown.js';

describe('buildFileUrl', () => {
  it('builds a single-line GitHub URL', () => {
    expect(buildFileUrl('https://github.com/acme/repo', 'main', 'src/lib.rs', 42))
      .toBe('https://github.com/acme/repo/blob/main/src/lib.rs#L42');
  });

  it('builds a range GitHub URL', () => {
    expect(buildFileUrl('https://github.com/acme/repo', 'main', 'src/lib.rs', 42, 50))
      .toBe('https://github.com/acme/repo/blob/main/src/lib.rs#L42-L50');
  });

  it('builds a range GitLab URL', () => {
    expect(buildFileUrl('https://gitlab.com/acme/repo', 'main', 'src/lib.rs', 42, 50))
      .toBe('https://gitlab.com/acme/repo/-/blob/main/src/lib.rs#L42-50');
  });

  it('builds a range Bitbucket URL', () => {
    expect(buildFileUrl('https://bitbucket.org/acme/repo', 'main', 'src/lib.rs', 42, 50))
      .toBe('https://bitbucket.org/acme/repo/src/main/src/lib.rs#lines-42:50');
  });
});

describe('buildFileUrl with no line number', () => {
  it('links to the bare file with no anchor', () => {
    expect(buildFileUrl('https://github.com/acme/repo', 'main', 'src/lib.rs'))
      .toBe('https://github.com/acme/repo/blob/main/src/lib.rs');
  });

  it('treats line 0 the same as no line', () => {
    expect(buildFileUrl('https://github.com/acme/repo', 'main', 'src/lib.rs', 0))
      .toBe('https://github.com/acme/repo/blob/main/src/lib.rs');
  });
});

describe('renderMarkdown citations', () => {
  const sources = [{ repo: 'acme/repo', version: 'main', file: 'src/lib.rs', line: 42 }];
  const citationIndex = buildCitationIndex(sources);

  it('renders a linked citation with a numbered label when the repo URL is known', () => {
    const html = renderMarkdown('See [acme/repo:main:src/lib.rs:42]', { 'acme/repo': 'https://github.com/acme/repo' }, citationIndex);
    expect(html).toContain('href="https://github.com/acme/repo/blob/main/src/lib.rs#L42"');
    expect(html).toContain('>1</a>');
  });

  it('degrades to an inert span with a full title when the repo URL is unknown', () => {
    const html = renderMarkdown('See [acme/repo:main:src/lib.rs:42]', {}, citationIndex);
    expect(html).not.toContain('<a ');
    expect(html).toContain('title="acme/repo main · src/lib.rs:42"');
  });

  it('builds a range URL and shows the full range in the title for a multi-line citation', () => {
    const html = renderMarkdown('See [acme/repo:main:src/lib.rs:42-50]', { 'acme/repo': 'https://github.com/acme/repo' }, citationIndex);
    expect(html).toContain('href="https://github.com/acme/repo/blob/main/src/lib.rs#L42-L50"');
    expect(html).toContain('title="acme/repo main · src/lib.rs:42-50"');
    expect(html).toContain('>1</a>');
  });

  it('renders a citation with no line number as a link to the bare file, not literal brackets', () => {
    const wholeFileIndex = buildCitationIndex([{ repo: 'acme/repo', version: 'main', file: 'src/lib.rs', line: 0 }]);
    const html = renderMarkdown('See [acme/repo:main:src/lib.rs]', { 'acme/repo': 'https://github.com/acme/repo' }, wholeFileIndex);
    expect(html).not.toContain('[acme/repo:main:src/lib.rs]');
    expect(html).toContain('href="https://github.com/acme/repo/blob/main/src/lib.rs"');
    expect(html).toContain('title="acme/repo main · src/lib.rs"');
    expect(html).toContain('>1</a>');
  });

  it('degrades a line-less citation to an inert span with the file-only title when the repo URL is unknown', () => {
    const html = renderMarkdown('See [acme/repo:main:src/lib.rs]', {}, {});
    expect(html).not.toContain('<a ');
    expect(html).toContain('title="acme/repo main · src/lib.rs"');
  });
});
