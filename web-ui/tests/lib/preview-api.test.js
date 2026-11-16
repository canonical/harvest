import { describe, it, expect, vi, afterEach } from 'vitest';
import { parseHeaderLines, shareUrl, sharePreview, enterPreview, AUDIENCES } from '../../src/lib/preview-api.js';

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

  it('shares one preview with an audience', async () => {
    const fetch = vi.spyOn(globalThis, 'fetch').mockResolvedValue({ ok: true, status: 204, json: () => Promise.resolve(null) });
    await sharePreview('p1', 'link');
    const [url, options] = fetch.mock.calls[0];
    expect(url).toBe('/code/previews/p1/share');
    expect(options.method).toBe('PUT');
    expect(JSON.parse(options.body)).toEqual({ audience: 'link' });
  });

  it('asks Harvest for the address that signs the viewer into a web preview', async () => {
    const fetch = vi.spyOn(globalThis, 'fetch').mockResolvedValue({
      ok: true,
      status: 200,
      json: () => Promise.resolve({ url: 'https://p1.preview.example/__harvest/enter?ticket=t' }),
    });
    const { url } = await enterPreview('p1');
    expect(url).toBe('https://p1.preview.example/__harvest/enter?ticket=t');
    expect(fetch).toHaveBeenCalledWith('/previews/p1/enter', expect.objectContaining({ method: 'POST', credentials: 'same-origin' }));
  });

  it('can ask to land on a path of the previewed app', async () => {
    const fetch = vi.spyOn(globalThis, 'fetch').mockResolvedValue({ ok: true, status: 200, json: () => Promise.resolve({ url: 'u' }) });
    await enterPreview('p1', '/admin');
    expect(JSON.parse(fetch.mock.calls[0][1].body)).toEqual({ to: '/admin' });
  });
});
