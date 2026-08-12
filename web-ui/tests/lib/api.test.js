import { describe, it, expect, vi, beforeEach } from 'vitest';
import {
  queryStream, fetchLlmProviders, projectQueryStart, runTerraformArtifact,
  listProjectDeployments, createProjectDeployment, getProjectDeployment, updateProjectDeployment,
  deleteProjectDeployment, deployDeployment, redeployDeployment, destroyDeployment, listDeploymentRuns,
  listGroupTemplates, createGroupTemplate, getGroupTemplate, updateGroupTemplate, deleteGroupTemplate,
  generateEnvironmentQuestions, generateDesign, generateDesignDecisions, reviseDesign,
  generateProvision, proposeProvisionChange, applyProvisionChange,
  diagnoseProvisionFailure, dismissProvisionDiagnosis,
} from '../../src/lib/api.js';

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

describe('deployment API functions', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('listProjectDeployments GETs the project deployments list', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse([{ id: 'd1' }])));
    const result = await listProjectDeployments('proj1');
    expect(global.fetch.mock.calls[0][0]).toBe('/projects/proj1/deployments');
    expect(result).toEqual([{ id: 'd1' }]);
  });

  it('createProjectDeployment POSTs name/environment_description/product_template_id', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ id: 'd1', conversation_id: 'c1' })));
    await createProjectDeployment('proj1', { name: 'Rollout', environment_description: 'env', product_template_id: null });

    const [url, options] = global.fetch.mock.calls[0];
    expect(url).toBe('/projects/proj1/deployments');
    expect(options.method).toBe('POST');
    expect(JSON.parse(options.body)).toEqual({ name: 'Rollout', environment_description: 'env', product_template_id: null });
  });

  it('getProjectDeployment GETs a single deployment', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ id: 'd1' })));
    await getProjectDeployment('proj1', 'd1');
    expect(global.fetch.mock.calls[0][0]).toBe('/projects/proj1/deployments/d1');
  });

  it('updateProjectDeployment PATCHes a deployment', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ ok: true })));
    await updateProjectDeployment('proj1', 'd1', { environment_description: 'v2' });
    const [url, options] = global.fetch.mock.calls[0];
    expect(url).toBe('/projects/proj1/deployments/d1');
    expect(options.method).toBe('PATCH');
  });

  it('deleteProjectDeployment DELETEs a deployment', async () => {
    global.fetch = vi.fn(() => Promise.resolve({ ok: true, status: 204, json: () => Promise.resolve({}) }));
    await deleteProjectDeployment('proj1', 'd1');
    const [url, options] = global.fetch.mock.calls[0];
    expect(url).toBe('/projects/proj1/deployments/d1');
    expect(options.method).toBe('DELETE');
  });

  it('deleteProjectDeployment throws on a non-ok response', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ error: 'nope' }, false, 500)));
    await expect(deleteProjectDeployment('proj1', 'd1')).rejects.toThrow();
  });

  it('deployDeployment POSTs agent_id and timeout_secs to the deploy route', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ runs: [] })));
    await deployDeployment('proj1', 'd1', { agent_id: 'a1', timeout_secs: 600 });
    const [url, options] = global.fetch.mock.calls[0];
    expect(url).toBe('/projects/proj1/deployments/d1/deploy');
    expect(options.method).toBe('POST');
    expect(JSON.parse(options.body)).toEqual({ agent_id: 'a1', timeout_secs: 600 });
  });

  it('redeployDeployment POSTs to the redeploy route', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ runs: [] })));
    await redeployDeployment('proj1', 'd1', { agent_id: 'a1' });
    expect(global.fetch.mock.calls[0][0]).toBe('/projects/proj1/deployments/d1/redeploy');
  });

  it('destroyDeployment POSTs to the destroy route', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ runs: [] })));
    await destroyDeployment('proj1', 'd1', { agent_id: 'a1' });
    expect(global.fetch.mock.calls[0][0]).toBe('/projects/proj1/deployments/d1/destroy');
  });

  it('listDeploymentRuns GETs the run history', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse([])));
    await listDeploymentRuns('proj1', 'd1');
    expect(global.fetch.mock.calls[0][0]).toBe('/projects/proj1/deployments/d1/runs');
  });

  it('listGroupTemplates GETs the group templates list', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse([])));
    await listGroupTemplates('grp1');
    expect(global.fetch.mock.calls[0][0]).toBe('/groups/grp1/templates');
  });

  it('createGroupTemplate POSTs name/description/content', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ id: 't1' })));
    await createGroupTemplate('grp1', { name: 'X', description: 'd', content: 'c' });
    const [url, options] = global.fetch.mock.calls[0];
    expect(url).toBe('/groups/grp1/templates');
    expect(options.method).toBe('POST');
  });

  it('getGroupTemplate GETs a single template', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ id: 't1' })));
    await getGroupTemplate('grp1', 't1');
    expect(global.fetch.mock.calls[0][0]).toBe('/groups/grp1/templates/t1');
  });

  it('updateGroupTemplate PUTs a template', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ ok: true })));
    await updateGroupTemplate('grp1', 't1', { content: 'c2' });
    const [url, options] = global.fetch.mock.calls[0];
    expect(url).toBe('/groups/grp1/templates/t1');
    expect(options.method).toBe('PUT');
  });

  it('deleteGroupTemplate DELETEs a template', async () => {
    global.fetch = vi.fn(() => Promise.resolve({ ok: true, status: 204, json: () => Promise.resolve({}) }));
    await deleteGroupTemplate('grp1', 't1');
    const [url, options] = global.fetch.mock.calls[0];
    expect(url).toBe('/groups/grp1/templates/t1');
    expect(options.method).toBe('DELETE');
  });
});

describe('deployment phase-action API functions', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('generateEnvironmentQuestions POSTs to the environment/questions route', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ questions: [] })));
    await generateEnvironmentQuestions('proj1', 'd1');
    const [url, options] = global.fetch.mock.calls[0];
    expect(url).toBe('/projects/proj1/deployments/d1/environment/questions');
    expect(options.method).toBe('POST');
  });

  it('generateDesign POSTs to the design/generate route', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ id: 'd1' })));
    await generateDesign('proj1', 'd1');
    expect(global.fetch.mock.calls[0][0]).toBe('/projects/proj1/deployments/d1/design/generate');
  });

  it('generateDesignDecisions POSTs to the design/decisions route', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ decisions: [] })));
    await generateDesignDecisions('proj1', 'd1');
    expect(global.fetch.mock.calls[0][0]).toBe('/projects/proj1/deployments/d1/design/decisions');
  });

  it('reviseDesign POSTs decisions and instructions to the design/revise route', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ id: 'd1' })));
    await reviseDesign('proj1', 'd1', { decisions: [{ question: 'q', answer: 'a' }], instructions: 'note' });
    const [url, options] = global.fetch.mock.calls[0];
    expect(url).toBe('/projects/proj1/deployments/d1/design/revise');
    expect(JSON.parse(options.body)).toEqual({ decisions: [{ question: 'q', answer: 'a' }], instructions: 'note' });
  });

  it('generateProvision POSTs to the provision/generate route', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ id: 'd1' })));
    await generateProvision('proj1', 'd1');
    expect(global.fetch.mock.calls[0][0]).toBe('/projects/proj1/deployments/d1/provision/generate');
  });

  it('proposeProvisionChange POSTs instructions/error_context to the propose-change route', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ proposed_files: {} })));
    await proposeProvisionChange('proj1', 'd1', { instructions: 'add a vm' });
    const [url, options] = global.fetch.mock.calls[0];
    expect(url).toBe('/projects/proj1/deployments/d1/provision/propose-change');
    expect(JSON.parse(options.body)).toEqual({ instructions: 'add a vm' });
  });

  it('applyProvisionChange POSTs files to the apply-change route', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ id: 'd1' })));
    await applyProvisionChange('proj1', 'd1', { files: { 'main.tf': 'x' } });
    const [url, options] = global.fetch.mock.calls[0];
    expect(url).toBe('/projects/proj1/deployments/d1/provision/apply-change');
    expect(JSON.parse(options.body)).toEqual({ files: { 'main.tf': 'x' } });
  });

  it('diagnoseProvisionFailure POSTs to the provision/diagnose route', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ started: true })));
    await diagnoseProvisionFailure('proj1', 'd1');
    const [url, options] = global.fetch.mock.calls[0];
    expect(url).toBe('/projects/proj1/deployments/d1/provision/diagnose');
    expect(options.method).toBe('POST');
  });

  it('dismissProvisionDiagnosis POSTs to the provision/diagnose/dismiss route', async () => {
    global.fetch = vi.fn(() => Promise.resolve(mockJsonResponse({ id: 'd1' })));
    await dismissProvisionDiagnosis('proj1', 'd1');
    const [url, options] = global.fetch.mock.calls[0];
    expect(url).toBe('/projects/proj1/deployments/d1/provision/diagnose/dismiss');
    expect(options.method).toBe('POST');
  });
});
