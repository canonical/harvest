import { describe, it, expect, vi, afterEach } from 'vitest';
import { parseHeaderLines, shareUrl, sharePreview, AUDIENCES } from '../../src/lib/preview-api.js';

afterEach(() => vi.restoreAllMocks());

describe('preview api', () => {
  it('reads one header per line and keeps colons inside values', () => {
    expect(parseHeaderLines('Content-Type: application/json\n\nX-Time: 12:30\nbroken')).toEqual({
      'Content-Type': 'application/json',
      'X-Time': '12:30',
    });
  });

  it('builds a share link to the public preview page', () => {
    expect(shareUrl('abc', 'https://harvest.example')).toBe('https://harvest.example/#/preview/abc');
  });

  it('offers the three audiences in order of reach', () => {
    expect(AUDIENCES.map((a) => a.id)).toEqual(['owner', 'repository', 'link']);
  });

  it('shares a session preview with an audience', async () => {
    const fetch = vi.spyOn(globalThis, 'fetch').mockResolvedValue({ ok: true, status: 204, json: () => Promise.resolve(null) });
    await sharePreview('s1', 'link');
    const [url, options] = fetch.mock.calls[0];
    expect(url).toBe('/code/sessions/s1/preview/share');
    expect(options.method).toBe('PUT');
    expect(JSON.parse(options.body)).toEqual({ audience: 'link' });
  });
});
