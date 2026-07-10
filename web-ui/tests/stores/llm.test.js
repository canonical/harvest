import { describe, it, expect, beforeEach, vi } from 'vitest';
import { useLlmStore } from '../../src/stores/llm.js';
import { useAuthStore } from '../../src/stores/auth.js';
import * as api from '../../src/lib/api.js';

describe('useLlmStore', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it('defaults to no selection (auto)', () => {
    const store = useLlmStore();
    expect(store.selection).toBeNull();
    expect(store.providers).toEqual([]);
  });

  it('load() fetches and stores the providers list', async () => {
    const payload = { providers: [{ id: 'a', kind: 'anthropic', default_model: 'm1', models: [{ id: 'm1' }] }] };
    vi.spyOn(api, 'fetchLlmProviders').mockResolvedValue(payload);

    const store = useLlmStore();
    await store.load();

    expect(store.providers).toEqual(payload.providers);
  });

  it('setSelection updates the selection and persists it to the user profile', () => {
    const updateMeSpy = vi.spyOn(api, 'updateMe').mockResolvedValue({ ok: true });
    const store = useLlmStore();
    store.setSelection('anthropic-main', 'claude-sonnet-5');

    expect(store.selection).toEqual({ providerId: 'anthropic-main', model: 'claude-sonnet-5' });
    expect(updateMeSpy).toHaveBeenCalledWith({
      last_llm_provider_id: 'anthropic-main', last_llm_model: 'claude-sonnet-5',
    });
  });

  it('setSelection does not throw when the profile update fails', async () => {
    vi.spyOn(api, 'updateMe').mockRejectedValue(new Error('network error'));
    const store = useLlmStore();
    expect(() => store.setSelection('anthropic-main', 'claude-sonnet-5')).not.toThrow();
    await Promise.resolve();
  });

  it('setSelection(null) clears the selection without calling updateMe', () => {
    const updateMeSpy = vi.spyOn(api, 'updateMe').mockResolvedValue({ ok: true });
    const store = useLlmStore();
    store.setSelection('anthropic-main', 'claude-sonnet-5');
    store.setSelection(null);

    expect(store.selection).toBeNull();
    expect(updateMeSpy).toHaveBeenCalledTimes(1);
  });

  it('loadFromProfile() restores the selection from the logged-in user', () => {
    const auth = useAuthStore();
    auth.user = { id: 'u1', last_llm_provider_id: 'gemini-1', last_llm_model: 'gemini-flash' };

    const store = useLlmStore();
    store.loadFromProfile();

    expect(store.selection).toEqual({ providerId: 'gemini-1', model: 'gemini-flash' });
  });

  it('loadFromProfile() leaves selection null when the user has no saved provider', () => {
    const auth = useAuthStore();
    auth.user = { id: 'u1', last_llm_provider_id: null, last_llm_model: null };

    const store = useLlmStore();
    store.loadFromProfile();

    expect(store.selection).toBeNull();
  });

  it('loadFromProfile() leaves selection null when there is no logged-in user', () => {
    const store = useLlmStore();
    store.loadFromProfile();
    expect(store.selection).toBeNull();
  });
});
