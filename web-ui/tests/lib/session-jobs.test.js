import { describe, expect, it } from 'vitest';
import { canStop, checksBadge, duration, fixRequest, needsFix, statusLabel } from '../../src/lib/session-jobs.js';

const job = (fields) => ({ id: 'j', kind: 'checks', status: 'running', output: '', created_at: '2026-11-17T10:00:00Z', ...fields });

describe('session jobs', () => {
  it('labels statuses, and a finished review as done', () => {
    expect(statusLabel(job({ status: 'succeeded' }))).toBe('Passed');
    expect(statusLabel(job({ status: 'cancelled' }))).toBe('Stopped');
    expect(statusLabel(job({ kind: 'review', status: 'succeeded' }))).toBe('Done');
  });

  it('lets running container jobs stop, but not reviews', () => {
    expect(canStop(job({}))).toBe(true);
    expect(canStop(job({ kind: 'review' }))).toBe(false);
    expect(canStop(job({ status: 'failed' }))).toBe(false);
  });

  it('offers a fix only for failed checks', () => {
    expect(needsFix(job({ status: 'failed' }))).toBe(true);
    expect(needsFix(job({ kind: 'command', status: 'failed' }))).toBe(false);
  });

  it('summarises the latest checks for the header', () => {
    expect(checksBadge([])).toBe(null);
    expect(checksBadge([job({}), job({ status: 'failed' })]).kind).toBe('running');
    expect(checksBadge([job({ kind: 'review' }), job({ status: 'failed' })]).label).toBe('Checks failed');
    expect(checksBadge([job({ status: 'succeeded' })]).kind).toBe('passed');
  });

  it('measures how long a job ran', () => {
    expect(duration(job({ finished_at: '2026-11-17T10:00:42Z' }))).toBe('42s');
    expect(duration(job({}), Date.parse('2026-11-17T10:02:05Z'))).toBe('2m 5s');
  });

  it('asks the agent to fix failures with the end of the output', () => {
    const output = Array.from({ length: 100 }, (_, i) => `line ${i}`).join('\n');
    const request = fixRequest(job({ status: 'failed', output }));
    expect(request).toContain('line 99');
    expect(request).not.toContain('line 39\n');
  });
});
