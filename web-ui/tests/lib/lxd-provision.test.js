import { describe, it, expect } from 'vitest';
import {
  PROVISION_PHASES,
  initialProvisionSteps,
  applyProvisionEvent,
  isProvisionDone,
  isProvisionError,
} from '../../src/lib/lxd-provision.js';

describe('initialProvisionSteps', () => {
  it('returns one pending step per phase in order', () => {
    const steps = initialProvisionSteps();
    expect(steps.map(s => s.id)).toEqual(PROVISION_PHASES.map(p => p.id));
    expect(steps.every(s => s.status === 'pending')).toBe(true);
  });
});

describe('applyProvisionEvent', () => {
  it('marks the phase as active and earlier phases as done', () => {
    let steps = initialProvisionSteps();
    steps = applyProvisionEvent(steps, { type: 'phase_start', phase: 'create_container' });

    expect(steps.find(s => s.id === 'ensure_network').status).toBe('done');
    expect(steps.find(s => s.id === 'install_token').status).toBe('done');
    expect(steps.find(s => s.id === 'create_container').status).toBe('active');
    expect(steps.find(s => s.id === 'start_container').status).toBe('pending');
  });

  it('does not resurrect an errored earlier step when a later phase starts', () => {
    let steps = initialProvisionSteps();
    steps = applyProvisionEvent(steps, { type: 'error', phase: 'ensure_network', message: 'boom' });
    steps = applyProvisionEvent(steps, { type: 'phase_start', phase: 'install_token' });

    expect(steps.find(s => s.id === 'ensure_network').status).toBe('error');
  });

  it('records retry attempt detail on the install_agent step', () => {
    let steps = initialProvisionSteps();
    steps = applyProvisionEvent(steps, { type: 'phase_start', phase: 'install_agent' });
    steps = applyProvisionEvent(steps, { type: 'install_retry', attempt: 2, attempts: 4 });

    expect(steps.find(s => s.id === 'install_agent').detail).toBe('attempt 2 of 4');
  });

  it('marks every non-errored step done on the done event', () => {
    let steps = initialProvisionSteps();
    steps = applyProvisionEvent(steps, { type: 'phase_start', phase: 'wait_running' });
    steps = applyProvisionEvent(steps, { type: 'done', hostname: 'agent-abcd' });

    expect(steps.every(s => s.status === 'done')).toBe(true);
  });

  it('marks the failing step as error with the message as detail', () => {
    let steps = initialProvisionSteps();
    steps = applyProvisionEvent(steps, { type: 'phase_start', phase: 'install_agent' });
    steps = applyProvisionEvent(steps, { type: 'error', phase: 'install_agent', message: 'exit code 6' });

    const step = steps.find(s => s.id === 'install_agent');
    expect(step.status).toBe('error');
    expect(step.detail).toBe('exit code 6');
  });

  it('does not mutate the input array', () => {
    const steps = initialProvisionSteps();
    const copy = steps.map(s => ({ ...s }));
    applyProvisionEvent(steps, { type: 'phase_start', phase: 'install_token' });
    expect(steps).toEqual(copy);
  });
});

describe('isProvisionDone / isProvisionError', () => {
  it('identifies done events', () => {
    expect(isProvisionDone({ type: 'done', hostname: 'x' })).toBe(true);
    expect(isProvisionDone({ type: 'error' })).toBe(false);
  });

  it('identifies error events', () => {
    expect(isProvisionError({ type: 'error', phase: 'ensure_network', message: 'x' })).toBe(true);
    expect(isProvisionError({ type: 'done' })).toBe(false);
  });
});
