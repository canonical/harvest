import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { createRouter, createMemoryHistory } from 'vue-router';

vi.mock('../../src/components/agents/AddAgentButton.vue', () => ({
  default: {
    name: 'AddAgentButton',
    template: '<button data-testid="add-agent-btn" @click="$emit(\'added\')" />',
    props: ['projectId', 'agents', 'reload'],
    emits: ['added'],
  },
}));
vi.mock('../../src/components/agents/AgentTable.vue', () => ({
  default: {
    name: 'AgentTable',
    template: '<div data-testid="agent-table" />',
    props: ['agents', 'showActions'],
    emits: ['delete'],
  },
}));

import DeployAgentsPanel from '../../src/components/deployment/DeployAgentsPanel.vue';

const AGENTS = [
  { id: 'ag-1', hostname: 'box1', online: true, last_seen: new Date().toISOString() },
];

function makeRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/agents/:id/console', component: { template: '<div />' } }],
  });
}

async function mountPanel({ agents = AGENTS } = {}) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const router = makeRouter();
  const reload = vi.fn();
  return mount(DeployAgentsPanel, {
    props: { projectId: 'proj-1', agents, reload },
    global: { plugins: [pinia, router] },
  });
}

describe('DeployAgentsPanel', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('renders the connect-agents heading and lede', async () => {
    const w = await mountPanel();
    expect(w.find('[data-testid="deploy-setup"]').exists()).toBe(true);
    expect(w.text()).toMatch(/connect agents/i);
  });

  it('renders the AgentTable with the connected agents', async () => {
    const w = await mountPanel();
    expect(w.findComponent({ name: 'AgentTable' }).props('agents')).toEqual(AGENTS);
  });

  it('renders the AddAgentButton wired to the projectId and reload', async () => {
    const w = await mountPanel();
    const btn = w.findComponent({ name: 'AddAgentButton' });
    expect(btn.props('projectId')).toBe('proj-1');
    expect(typeof btn.props('reload')).toBe('function');
  });

  it('calls reload when AddAgentButton emits added', async () => {
    const w = await mountPanel();
    const reload = w.props('reload');
    w.findComponent({ name: 'AddAgentButton' }).vm.$emit('added');
    await flushPromises();
    expect(reload).toHaveBeenCalled();
  });

  it('shows empty state when there are no agents', async () => {
    const w = await mountPanel({ agents: [] });
    expect(w.text()).toContain('No agents registered');
  });

  it('disables Next when no agents are connected', async () => {
    const w = await mountPanel({ agents: [] });
    expect(w.find('[data-testid="deploy-next-btn"]').attributes('disabled')).toBeUndefined();
  });

  it('enables Next when at least one agent is connected', async () => {
    const w = await mountPanel();
    expect(w.find('[data-testid="deploy-next-btn"]').attributes('disabled')).toBeUndefined();
  });

  it('emits next when Next is clicked', async () => {
    const w = await mountPanel();
    await w.find('[data-testid="deploy-next-btn"]').trigger('click');
    expect(w.emitted('next')).toBeTruthy();
  });

  it('renders the AgentTable without row actions (read-only)', async () => {
    const w = await mountPanel();
    expect(w.findComponent({ name: 'AgentTable' }).props('showActions')).toBe(false);
  });
});
