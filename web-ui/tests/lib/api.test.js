import { describe, it, expect, vi, beforeEach } from 'vitest';
import { queryStream, fetchLlmProviders, projectQueryStart, runTerraformArtifact } from '../../src/lib/api.js';

function mockStreamResponse() {
  return {
    ok: true,
    status: 200,
    body: { getReader: () => ({ read: () => Promise.resolve({ done: true }), releaseLock: () => {} }) },
  };
}

function mockJsonResponse(body, ok = true, status = 200) {
  return { ok, status, json: () => Promise.resolve(body) };
}

describe('queryStream', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('omits provider_id and model when no selection is given', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockStreamResponse()));
    await queryStream('hi', 'conv1', [], () => {});

    const [, options] = global.fetch.mock.calls[0];
    const body = JSON.parse(options.body);
    expect(body).not.toHaveProperty('provider_id');
    expect(body).not.toHaveProperty('model');
  });

  it('includes provider_id and model when a selection is given', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockStreamResponse()));
    await queryStream('hi', 'conv1', [], () => {}, { providerId: 'anthropic-main', model: 'claude-sonnet-5' });

    const [, options] = global.fetch.mock.calls[0];
    const body = JSON.parse(options.body);
    expect(body.provider_id).toBe('anthropic-main');
    expect(body.model).toBe('claude-sonnet-5');
  });

  it('includes provider_id without model when model is unset', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockStreamResponse()));
    await queryStream('hi', 'conv1', [], () => {}, { providerId: 'anthropic-main' });

    const [, options] = global.fetch.mock.calls[0];
    const body = JSON.parse(options.body);
    expect(body.provider_id).toBe('anthropic-main');
    expect(body).not.toHaveProperty('model');
  });
});

describe('projectQueryStart', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('omits provider_id and model when no selection is given', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ ok: true })));
    await projectQueryStart('proj1', 'hi', 'conv1', []);

    const [, options] = global.fetch.mock.calls[0];
    const body = JSON.parse(options.body);
    expect(body).not.toHaveProperty('provider_id');
    expect(body).not.toHaveProperty('model');
  });

  it('includes provider_id and model when a selection is given', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ ok: true })));
    await projectQueryStart('proj1', 'hi', 'conv1', [], { providerId: 'gemini-1', model: 'gemini-2.5-flash' });

    const [, options] = global.fetch.mock.calls[0];
    const body = JSON.parse(options.body);
    expect(body.provider_id).toBe('gemini-1');
    expect(body.model).toBe('gemini-2.5-flash');
  });
});

describe('runTerraformArtifact', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('posts artifact_id, action, and timeout_secs to the terraform route', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ stdout: 'ok', stderr: '', exit_code: 0 })));
    await runTerraformArtifact('proj1', 'agent1', 'art1', 'plan', 600);

    const [url, options] = global.fetch.mock.calls[0];
    expect(url).toBe('/projects/proj1/agents/agent1/terraform');
    expect(options.method).toBe('POST');
    const body = JSON.parse(options.body);
    expect(body).toEqual({ artifact_id: 'art1', action: 'plan', timeout_secs: 600 });
  });

  it('defaults timeout_secs to 300 when omitted', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ stdout: '', stderr: '', exit_code: 0 })));
    await runTerraformArtifact('proj1', 'agent1', 'art1', 'apply');

    const [, options] = global.fetch.mock.calls[0];
    const body = JSON.parse(options.body);
    expect(body.timeout_secs).toBe(300);
  });

  it('throws on a non-ok response', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ error: 'nope' }, false, 500)));
    await expect(runTerraformArtifact('proj1', 'agent1', 'art1', 'destroy')).rejects.toThrow();
  });
});

describe('fetchLlmProviders', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('fetches the providers list', async () => {
    const payload = { providers: [{ id: 'a', kind: 'anthropic', default_model: 'x', models: [] }] };
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse(payload)));

    const result = await fetchLlmProviders();
    expect(global.fetch).toHaveBeenCalledWith('/llm/providers');
    expect(result).toEqual(payload);
  });

  it('returns an empty providers list on failure', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({}, false, 500)));
    const result = await fetchLlmProviders();
    expect(result).toEqual({ providers: [] });
  });

  it('returns an empty providers list without throwing when the body is not valid JSON', async () => {
    global.fetch = vi.fn(() => Promise.resolve({
      ok: true,
      status: 200,
      json: () => Promise.reject(new SyntaxError('Unexpected token <')),
    }));
    const result = await fetchLlmProviders();
    expect(result).toEqual({ providers: [] });
  });
});
