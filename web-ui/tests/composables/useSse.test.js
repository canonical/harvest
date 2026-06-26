import { describe, it, expect, vi, beforeEach } from 'vitest';
import { useSse } from '../../src/composables/useSse.js';

function sseBody(lines) {
  const text = lines.join('\n') + '\n';
  return new ReadableStream({
    start(controller) {
      controller.enqueue(new TextEncoder().encode(text));
      controller.close();
    },
  });
}

function mockFetch(body, ok = true) {
  global.fetch = vi.fn().mockResolvedValue({
    ok,
    status: ok ? 200 : 400,
    body,
  });
}

describe('useSse', () => {
  beforeEach(() => vi.restoreAllMocks());

  it('parses data lines and calls onEvent for each', async () => {
    const events = [];
    mockFetch(sseBody([
      'data: {"type":"tool_call","name":"search"}',
      '',
      'data: {"type":"done","answer":"hi"}',
      '',
    ]));

    const { stream } = useSse();
    await stream('/query/stream', { method: 'POST' }, e => events.push(e));

    expect(events).toHaveLength(2);
    expect(events[0]).toEqual({ type: 'tool_call', name: 'search' });
    expect(events[1]).toEqual({ type: 'done', answer: 'hi' });
  });

  it('skips non-data lines (comments, empty)', async () => {
    const events = [];
    mockFetch(sseBody([
      ': keep-alive',
      '',
      'data: {"type":"done","answer":"ok"}',
      '',
    ]));

    const { stream } = useSse();
    await stream('/q', {}, e => events.push(e));

    expect(events).toHaveLength(1);
    expect(events[0].type).toBe('done');
  });

  it('ignores lines with invalid JSON', async () => {
    const events = [];
    mockFetch(sseBody([
      'data: not-json',
      '',
      'data: {"type":"done","answer":"ok"}',
      '',
    ]));

    const { stream } = useSse();
    await stream('/q', {}, e => events.push(e));

    expect(events).toHaveLength(1);
  });

  it('throws on non-ok fetch response', async () => {
    mockFetch(sseBody([]), false);
    const { stream } = useSse();
    await expect(stream('/q', {}, () => {})).rejects.toThrow();
  });

  it('exposes abort() to cancel stream', async () => {
    const events = [];
    let controllerRef;
    global.fetch = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      body: new ReadableStream({
        start(c) { controllerRef = c; },
      }),
    });

    const { stream, abort } = useSse();
    const p = stream('/q', {}, e => events.push(e));
    abort();
    await p;
    expect(events).toHaveLength(0);
  });
});
