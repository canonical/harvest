import { describe, it, expect } from 'vitest';
import {
  MAX_RECONNECTS, connecting, connectionNotice, nextConnection, reconnectDelay, shouldConnect,
} from '../../src/lib/terminal-connection.js';

const after = (...types) => types.reduce((state, type) => nextConnection(state, { type }), connecting());

describe('terminal connection', () => {
  it('opens and retries with growing delays after a drop', () => {
    const dropped = after('open', 'close');
    expect(dropped).toMatchObject({ status: 'reconnecting', attempt: 1 });
    expect(shouldConnect(dropped)).toBe(true);
    expect(reconnectDelay(dropped)).toBe(500);
    expect(reconnectDelay(after('open', 'close', 'close', 'close'))).toBe(2000);
    expect(after('open', 'close', 'open')).toMatchObject({ status: 'open', attempt: 0 });
  });

  it('reconnects at once when the server moves the terminal to another node', () => {
    const moved = after('open', 'reconnect', 'close');
    expect(moved).toMatchObject({ status: 'reconnecting', attempt: 0 });
    expect(reconnectDelay(moved)).toBe(0);
  });

  it('gives up after too many attempts', () => {
    const types = ['open', ...Array(MAX_RECONNECTS + 1).fill('close')];
    const lost = after(...types);
    expect(lost.status).toBe('lost');
    expect(shouldConnect(lost)).toBe(false);
    expect(connectionNotice(lost)).toMatchObject({ tone: 'negative', action: 'Reconnect' });
  });

  it('stays exited with the exit code until restarted', () => {
    const exited = nextConnection(after('open'), { type: 'exit', code: 130 });
    const closedAfterExit = nextConnection(exited, { type: 'close' });
    expect(closedAfterExit).toEqual(exited);
    expect(connectionNotice(exited)).toEqual({ tone: 'information', message: 'The shell exited with code 130.', action: 'Restart' });
    expect(nextConnection(exited, { type: 'restart' })).toEqual(connecting());
  });

  it('says nothing while connected or connecting for the first time', () => {
    expect(connectionNotice(connecting())).toBeNull();
    expect(connectionNotice(after('open'))).toBeNull();
    expect(connectionNotice(after('open', 'close'))).toMatchObject({ tone: 'caution', action: null });
  });
});
