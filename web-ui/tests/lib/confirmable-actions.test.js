import { describe, it, expect, vi, beforeEach } from 'vitest';
import { runConfirmableAction } from '../../src/lib/confirmable-actions.js';
import {
  provisionLxdAgent, deleteAgent, createPortForward, updatePortForward, deletePortForward, runTerraformArtifact,
} from '../../src/lib/api.js';

vi.mock('../../src/lib/api.js', () => ({
  provisionLxdAgent:    vi.fn(),
  deleteAgent:          vi.fn(),
  createPortForward:    vi.fn(),
  updatePortForward:    vi.fn(),
  deletePortForward:    vi.fn(),
  runTerraformArtifact: vi.fn(),
}));

function makeThreadStore(action) {
  const messages = [{
    role: 'assistant',
    chain: [{ type: 'confirm_action', id: action.id, name: action.name, input: action.input, status: 'pending', steps: [], resultText: '' }],
  }];
  const store = {
    messages,
    updateConfirmActionItem: vi.fn((id, patch) => {
      const item = messages.at(-1)?.chain?.find(c => c.type === 'confirm_action' && c.id === id);
      if (item) Object.assign(item, patch);
    }),
  };
  store.get = (id) => messages.at(-1)?.chain?.find(c => c.type === 'confirm_action' && c.id === id);
  return store;
}

describe('runConfirmableAction', () => {
  beforeEach(() => { vi.clearAllMocks(); });

  it('confirming create_lxd_agent calls provisionLxdAgent and marks done', async () => {
    provisionLxdAgent.mockImplementation((projectId, body, onEvent) => {
      onEvent({ type: 'done' });
      return Promise.resolve();
    });
    const action = { id: 'a1', name: 'create_lxd_agent', input: { name: 'runner', flavor: 'small' } };
    const store = makeThreadStore(action);
    await runConfirmableAction('proj1', action, store);

    expect(provisionLxdAgent).toHaveBeenCalledWith('proj1', { name: 'runner', description: '', flavor: 'small' }, expect.any(Function));
    expect(store.get('a1').status).toBe('done');
  });

  it('failed create_lxd_agent marks the action as error', async () => {
    provisionLxdAgent.mockImplementation((projectId, body, onEvent) => {
      onEvent({ type: 'error', message: 'boom' });
      return Promise.resolve();
    });
    const action = { id: 'a1', name: 'create_lxd_agent', input: { name: 'runner', flavor: 'small' } };
    const store = makeThreadStore(action);
    await runConfirmableAction('proj1', action, store);

    expect(store.get('a1').status).toBe('error');
    expect(store.get('a1').resultText).toBe('boom');
  });

  it('confirming delete_agent calls deleteAgent and marks done', async () => {
    deleteAgent.mockResolvedValue();
    const action = { id: 'a1', name: 'delete_agent', input: { agent_id: 'abc' } };
    const store = makeThreadStore(action);
    await runConfirmableAction('proj1', action, store);

    expect(deleteAgent).toHaveBeenCalledWith('proj1', 'abc');
    expect(store.get('a1').status).toBe('done');
  });

  it('failed delete_agent marks the action as error', async () => {
    deleteAgent.mockRejectedValue(new Error('nope'));
    const action = { id: 'a1', name: 'delete_agent', input: { agent_id: 'abc' } };
    const store = makeThreadStore(action);
    await runConfirmableAction('proj1', action, store);

    expect(store.get('a1').status).toBe('error');
    expect(store.get('a1').resultText).toBe('nope');
  });

  it('confirming create_port_forward calls createPortForward and marks done', async () => {
    createPortForward.mockResolvedValue();
    const action = { id: 'a1', name: 'create_port_forward', input: { agent_id: 'ag1', port: 8080, route_name: 'app' } };
    const store = makeThreadStore(action);
    await runConfirmableAction('proj1', action, store);

    expect(createPortForward).toHaveBeenCalledWith('proj1', 'ag1', { port: 8080, routeName: 'app' });
    expect(store.get('a1').status).toBe('done');
  });

  it('confirming update_port_forward calls updatePortForward and marks done', async () => {
    updatePortForward.mockResolvedValue();
    const action = { id: 'a1', name: 'update_port_forward', input: { agent_id: 'ag1', forward_id: 'f1', port: 9090, route_name: 'app2' } };
    const store = makeThreadStore(action);
    await runConfirmableAction('proj1', action, store);

    expect(updatePortForward).toHaveBeenCalledWith('proj1', 'ag1', 'f1', { port: 9090, routeName: 'app2' });
    expect(store.get('a1').status).toBe('done');
  });

  it('confirming delete_port_forward calls deletePortForward and marks done', async () => {
    deletePortForward.mockResolvedValue();
    const action = { id: 'a1', name: 'delete_port_forward', input: { agent_id: 'ag1', forward_id: 'f1' } };
    const store = makeThreadStore(action);
    await runConfirmableAction('proj1', action, store);

    expect(deletePortForward).toHaveBeenCalledWith('proj1', 'ag1', 'f1');
    expect(store.get('a1').status).toBe('done');
  });

  it('confirming run_terraform_apply calls runTerraformArtifact and marks done on exit 0', async () => {
    runTerraformArtifact.mockResolvedValue({ stdout: 'applied', stderr: '', exit_code: 0 });
    const action = { id: 'a1', name: 'run_terraform_apply', input: { agent_id: 'ag1', artifact_id: 'art1', timeout_secs: 600 } };
    const store = makeThreadStore(action);
    await runConfirmableAction('proj1', action, store);

    expect(runTerraformArtifact).toHaveBeenCalledWith('proj1', 'ag1', 'art1', 'apply', 600);
    expect(store.get('a1').status).toBe('done');
    expect(store.get('a1').resultText).toContain('succeeded');
  });

  it('confirming run_terraform_destroy calls runTerraformArtifact with destroy and marks error on nonzero exit', async () => {
    runTerraformArtifact.mockResolvedValue({ stdout: '', stderr: 'no such resource', exit_code: 1 });
    const action = { id: 'a1', name: 'run_terraform_destroy', input: { agent_id: 'ag1', artifact_id: 'art1' } };
    const store = makeThreadStore(action);
    await runConfirmableAction('proj1', action, store);

    expect(runTerraformArtifact).toHaveBeenCalledWith('proj1', 'ag1', 'art1', 'destroy', undefined);
    expect(store.get('a1').status).toBe('error');
    expect(store.get('a1').resultText).toContain('no such resource');
  });

  it('a thrown error while running terraform marks the action as error', async () => {
    runTerraformArtifact.mockRejectedValue(new Error('agent disconnected'));
    const action = { id: 'a1', name: 'run_terraform_apply', input: { agent_id: 'ag1', artifact_id: 'art1' } };
    const store = makeThreadStore(action);
    await runConfirmableAction('proj1', action, store);

    expect(store.get('a1').status).toBe('error');
    expect(store.get('a1').resultText).toBe('agent disconnected');
  });

  it('an unknown action name does nothing', async () => {
    const action = { id: 'a1', name: 'some_unhandled_tool', input: {} };
    const store = makeThreadStore(action);
    await runConfirmableAction('proj1', action, store);

    expect(store.updateConfirmActionItem).not.toHaveBeenCalled();
  });
});
