import { describe, it, expect } from 'vitest';
import { tokenizeDocument, renderBlock } from '../../src/lib/incremental-markdown.js';
import { buildCitationIndex } from '../../src/lib/markdown.js';

describe('tokenizeDocument', () => {
  it('returns an empty array for an empty document', () => {
    expect(tokenizeDocument('')).toEqual([]);
  });

  it('splits a document into one token per top-level block', () => {
    const tokens = tokenizeDocument('# Title\n\nFirst paragraph.\n\nSecond paragraph.');
    expect(tokens.map(t => t.type)).toEqual(['heading', 'paragraph', 'paragraph']);
  });

  it('keeps an unclosed code fence as a single trailing token', () => {
    const tokens = tokenizeDocument('# Title\n\n```js\nconst x = 1;');
    expect(tokens.at(-1).type).toBe('code');
  });

  it('applies citation substitution before tokenizing', () => {
    const sources = [{ repo: 'acme/repo', version: 'main', file: 'src/lib.rs', line: 42 }];
    const citationIndex = buildCitationIndex(sources);
    const tokens = tokenizeDocument(
      'See [acme/repo:main:src/lib.rs:42]',
      { 'acme/repo': 'https://github.com/acme/repo' },
      citationIndex,
    );
    expect(tokens[0].raw).toContain('<a href="https://github.com/acme/repo/blob/main/src/lib.rs#L42"');
  });

  it('growing the same prefix keeps earlier tokens raw-identical', () => {
    const first = tokenizeDocument('# Title\n\nFirst paragraph.');
    const second = tokenizeDocument('# Title\n\nFirst paragraph is now longer.');
    expect(second[0].raw).toBe(first[0].raw);
  });
});

describe('renderBlock', () => {
  it('renders a heading token to an h1', () => {
    const [heading] = tokenizeDocument('# Title');
    expect(renderBlock(heading)).toContain('<h1');
    expect(renderBlock(heading)).toContain('Title');
  });

  it('renders a paragraph token to a p tag', () => {
    const [para] = tokenizeDocument('Just some text.');
    expect(renderBlock(para)).toContain('<p>Just some text.</p>');
  });

  it('renders a code token with syntax highlighting classes, matching renderMarkdown', async () => {
    const { renderMarkdown } = await import('../../src/lib/markdown.js');
    const [code] = tokenizeDocument('```js\nconst x = 1;\n```');
    expect(renderBlock(code).trim()).toBe(renderMarkdown('```js\nconst x = 1;\n```').trim());
  });
});
