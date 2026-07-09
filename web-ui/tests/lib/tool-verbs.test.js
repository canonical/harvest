import { describe, it, expect } from 'vitest';
import { runningVerb } from '../../src/lib/tool-verbs.js';

describe('runningVerb', () => {
  it('returns Creating… for create_lxd_agent', () => {
    expect(runningVerb('create_lxd_agent')).toBe('Creating…');
  });

  it('returns Deleting… for delete_agent', () => {
    expect(runningVerb('delete_agent')).toBe('Deleting…');
  });

  it('returns Creating… for create_port_forward', () => {
    expect(runningVerb('create_port_forward')).toBe('Creating…');
  });

  it('returns Updating… for update_port_forward', () => {
    expect(runningVerb('update_port_forward')).toBe('Updating…');
  });

  it('returns Deleting… for delete_port_forward', () => {
    expect(runningVerb('delete_port_forward')).toBe('Deleting…');
  });

  it('returns a generic fallback for an unrecognised tool name', () => {
    expect(runningVerb('some_future_tool')).toBe('Working…');
  });
});
