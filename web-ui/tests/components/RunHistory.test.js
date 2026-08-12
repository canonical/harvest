import { describe, it, expect } from 'vitest';
import { mount } from '@vue/test-utils';

import RunHistory from '../../src/components/deployment/RunHistory.vue';

const RUNS = [
  {
    id: 'r2', action: 'apply', status: 'failed', exit_code: 1,
    stdout_preview: '', stderr_preview: 'Error: connection refused',
    initiated_by: 'user', reasoning: null, created_at: '2026-08-07T12:00:00Z',
  },
  {
    id: 'r1', action: 'apply', status: 'success', exit_code: 0,
    stdout_preview: 'Apply complete!', stderr_preview: '',
    initiated_by: 'user', reasoning: null, created_at: '2026-08-06T12:00:00Z',
  },
];

describe('RunHistory', () => {
  it('shows an empty state with no runs and no live entry', () => {
    const w = mount(RunHistory, { props: { runs: [] } });
    expect(w.text()).toContain('No runs yet.');
    expect(w.find('[data-testid="run-history-item"]').exists()).toBe(false);
  });

  it('lists every run with its action and exit code', () => {
    const w = mount(RunHistory, { props: { runs: RUNS } });
    const items = w.findAll('[data-testid="run-history-item"]');
    expect(items).toHaveLength(2);
    expect(w.text()).toContain('apply');
    expect(w.text()).toContain('exit 1');
    expect(w.text()).toContain('exit 0');
  });

  it('shows the most recent run selected and its output by default', () => {
    const w = mount(RunHistory, { props: { runs: RUNS } });
    expect(w.text()).toContain('Error: connection refused');
    expect(w.text()).not.toContain('Apply complete!');
  });

  it('switches the detail pane when an older run is selected', async () => {
    const w = mount(RunHistory, { props: { runs: RUNS } });
    const rows = w.findAll('.run-history__row');
    await rows[1].trigger('click');

    expect(w.text()).toContain('Apply complete!');
    expect(w.text()).not.toContain('Error: connection refused');
  });

  it('marks failed and successful runs with distinct status dots', () => {
    const w = mount(RunHistory, { props: { runs: RUNS } });
    const dots = w.findAll('.run-history__status-dot');
    expect(dots[0].classes()).toContain('run-history__status-dot--failed');
    expect(dots[1].classes()).toContain('run-history__status-dot--success');
  });

  it('selects the newest run when the list changes', async () => {
    const w = mount(RunHistory, { props: { runs: [RUNS[1]] } });
    expect(w.text()).toContain('Apply complete!');

    await w.setProps({ runs: RUNS });
    expect(w.text()).toContain('Error: connection refused');
  });

  it('shows a synthetic live entry at the top while a run is in flight, with streaming output', () => {
    const w = mount(RunHistory, {
      props: {
        runs: RUNS,
        liveEntry: { action: 'deploy', agentHostname: 'host-1' },
        liveLog: [{ stream: 'stdout', line: 'Initializing...' }, { stream: 'stderr', line: 'warning: x' }],
      },
    });

    const items = w.findAll('[data-testid="run-history-item"]');
    expect(items).toHaveLength(3);
    expect(w.text()).toContain('Deploying on host-1');
    expect(w.text()).toContain('Initializing...');
    expect(w.text()).toContain('warning: x');
    expect(w.find('.run-history__log-line--stderr').exists()).toBe(true);
  });

  it('follows history back to the newest completed run once the live run finishes', async () => {
    const w = mount(RunHistory, {
      props: { runs: RUNS, liveEntry: { action: 'deploy', agentHostname: 'host-1' }, liveLog: [] },
    });
    expect(w.text()).toContain('Deploying on host-1');

    const completed = {
      id: 'r3', action: 'apply', status: 'success', exit_code: 0,
      stdout_preview: 'Deploy finished', stderr_preview: '',
      initiated_by: 'user', reasoning: null, created_at: '2026-08-08T12:00:00Z',
    };
    await w.setProps({ runs: [completed, ...RUNS], liveEntry: null, liveLog: [] });

    expect(w.text()).toContain('Deploy finished');
  });
});
