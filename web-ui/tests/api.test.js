import { describe, it, expect, vi, beforeEach } from 'vitest';
import { queryStream, queryOnce, fetchRepositories, fetchToolDescription } from '../src/api.js';

// ── helpers ────────────────────────────────────────────────────────────────────

function makeReadableStream(chunks) {
  const encoder = new TextEncoder();
  return new ReadableStream({
    start(controller) {
      for (const chunk of chunks) {
        controller.enqueue(encoder.encode(chunk));
      }
      controller.close();
    },
  });
}

function sseLines(...events) {
  return events.map(e => `data: ${JSON.stringify(e)}\n\n`).join('');
}

function mockFetch(status, body, contentType = 'application/json') {
  return vi.fn().mockResolvedValue({
    ok: status >= 200 && status < 300,
    status,
    headers: { get: (h) => (h === 'content-type' ? contentType : null) },
    body: makeReadableStream([body]),
    json: () => Promise.resolve(JSON.parse(body)),
    text: () => Promise.resolve(body),
  });
}

// ── queryStream ───────────────────────────────────────────────────────────────

describe('queryStream', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('POSTs to /query/stream with the query body', async () => {
    const events = sseLines(
      { type: 'done', answer: 'hi', sources: [], tool_calls_made: 0 },
    );
    global.fetch = mockFetch(200, events, 'text/event-stream');

    const received = [];
    await queryStream('hello', (e) => received.push(e));

    expect(fetch).toHaveBeenCalledOnce();
    const [url, init] = fetch.mock.calls[0];
    expect(url).toBe('/query/stream');
    expect(init.method).toBe('POST');
    expect(JSON.parse(init.body).query).toBe('hello');
  });

  it('delivers tool_call events to the callback', async () => {
    const events = sseLines(
      { type: 'tool_call', name: 'search_symbols', input: { query: 'foo' } },
      { type: 'done', answer: 'ok', sources: [], tool_calls_made: 1 },
    );
    global.fetch = mockFetch(200, events, 'text/event-stream');

    const received = [];
    await queryStream('q', (e) => received.push(e));

    const toolCall = received.find(e => e.type === 'tool_call');
    expect(toolCall).toBeDefined();
    expect(toolCall.name).toBe('search_symbols');
    expect(toolCall.input.query).toBe('foo');
  });

  it('delivers tool_result events to the callback', async () => {
    const events = sseLines(
      { type: 'tool_result', name: 'list_repositories', preview: 'repo-a' },
      { type: 'done', answer: 'ok', sources: [], tool_calls_made: 1 },
    );
    global.fetch = mockFetch(200, events, 'text/event-stream');

    const received = [];
    await queryStream('q', (e) => received.push(e));

    const result = received.find(e => e.type === 'tool_result');
    expect(result).toBeDefined();
    expect(result.name).toBe('list_repositories');
    expect(result.preview).toBe('repo-a');
  });

  it('delivers done event with answer and sources', async () => {
    const doneEvent = {
      type: 'done',
      answer: 'The answer is 42',
      sources: [{ repo: 'r', version: 'v1', file: 'f.rs', line: 10 }],
      tool_calls_made: 2,
    };
    global.fetch = mockFetch(200, sseLines(doneEvent), 'text/event-stream');

    const received = [];
    await queryStream('q', (e) => received.push(e));

    const done = received.find(e => e.type === 'done');
    expect(done.answer).toBe('The answer is 42');
    expect(done.sources).toHaveLength(1);
    expect(done.tool_calls_made).toBe(2);
  });

  it('delivers error events to the callback', async () => {
    const events = sseLines({ type: 'error', message: 'llm failed' });
    global.fetch = mockFetch(200, events, 'text/event-stream');

    const received = [];
    await queryStream('q', (e) => received.push(e));

    expect(received.find(e => e.type === 'error')).toBeDefined();
  });

  it('skips malformed SSE lines without throwing', async () => {
    const body = 'data: not-json\n\ndata: {"type":"done","answer":"ok","sources":[],"tool_calls_made":0}\n\n';
    global.fetch = mockFetch(200, body, 'text/event-stream');

    const received = [];
    await expect(queryStream('q', (e) => received.push(e))).resolves.not.toThrow();
    expect(received.some(e => e.type === 'done')).toBe(true);
  });

  it('throws when the server responds with a non-OK HTTP status', async () => {
    global.fetch = mockFetch(500, 'internal error', 'text/plain');

    await expect(queryStream('q', () => {})).rejects.toThrow(/500/);
  });
});

// ── queryOnce (non-streaming fallback) ────────────────────────────────────────

describe('queryOnce', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('POSTs to /query and returns parsed JSON', async () => {
    const body = JSON.stringify({ answer: 'hello', sources: [], tool_calls_made: 0 });
    global.fetch = mockFetch(200, body);

    const result = await queryOnce('what is main?');
    expect(result.answer).toBe('hello');
    expect(result.sources).toEqual([]);
  });

  it('throws on non-OK response', async () => {
    global.fetch = mockFetch(500, 'error');
    await expect(queryOnce('q')).rejects.toThrow(/500/);
  });
});

// ── fetchToolDescription ──────────────────────────────────────────────────────

describe('fetchToolDescription', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('POSTs to /tool-description with name and input', async () => {
    global.fetch = mockFetch(200, JSON.stringify({ description: 'Searching for foo' }));
    await fetchToolDescription('search_symbols', { query: 'foo' });
    const [url, init] = fetch.mock.calls[0];
    expect(url).toBe('/tool-description');
    expect(init.method).toBe('POST');
    const body = JSON.parse(init.body);
    expect(body.name).toBe('search_symbols');
    expect(body.input).toEqual({ query: 'foo' });
  });

  it('returns the description string from the response', async () => {
    global.fetch = mockFetch(200, JSON.stringify({ description: 'Listing all repositories' }));
    const result = await fetchToolDescription('list_repositories', {});
    expect(result).toBe('Listing all repositories');
  });

  it('returns null on non-OK response instead of throwing', async () => {
    global.fetch = mockFetch(500, 'error');
    const result = await fetchToolDescription('search_symbols', {});
    expect(result).toBeNull();
  });

  it('returns null on network error instead of throwing', async () => {
    global.fetch = vi.fn().mockRejectedValue(new Error('network failure'));
    const result = await fetchToolDescription('search_symbols', {});
    expect(result).toBeNull();
  });
});

// ── fetchRepositories ─────────────────────────────────────────────────────────

describe('fetchRepositories', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('GETs /repositories and returns parsed array', async () => {
    const repos = [{ name: 'repo-a', versions: ['v1.0'] }];
    global.fetch = mockFetch(200, JSON.stringify(repos));

    const result = await fetchRepositories();
    expect(result).toHaveLength(1);
    expect(result[0].name).toBe('repo-a');
  });

  it('returns empty array on non-OK response', async () => {
    global.fetch = mockFetch(503, 'unavailable');
    const result = await fetchRepositories();
    expect(result).toEqual([]);
  });
});
