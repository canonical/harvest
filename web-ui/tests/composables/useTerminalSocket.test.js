import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { useTerminalSocket } from '../../src/composables/useTerminalSocket.js';

class FakeWebSocket {
  static OPEN = 1;
  static instances = [];

  constructor(url) {
    this.url = url;
    this.readyState = FakeWebSocket.OPEN;
    this.sent = [];
    FakeWebSocket.instances.push(this);
  }

  send(data) {
    this.sent.push(data);
  }

  close() {
    this.readyState = 3;
  }

  serverSays(message) {
    this.onmessage?.({ data: JSON.stringify(message) });
  }
}

const latest = () => FakeWebSocket.instances.at(-1);

beforeEach(() => {
  FakeWebSocket.instances = [];
  global.WebSocket = FakeWebSocket;
  vi.useFakeTimers();
});

afterEach(() => vi.useRealTimers());

function connection() {
  let n = 0;
  const socket = useTerminalSocket({ urlFor: () => `ws://t/${++n}`, onData: () => {} });
  socket.open();
  latest().onopen();
  return socket;
}

describe('useTerminalSocket', () => {
  it('reconnects with backoff after the connection drops', () => {
    const socket = connection();
    latest().onclose();
    expect(socket.state.value).toMatchObject({ status: 'reconnecting', attempt: 1 });
    vi.advanceTimersByTime(499);
    expect(FakeWebSocket.instances).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(latest().url).toBe('ws://t/2');
  });

  it('reconnects at once when the server moves the terminal', () => {
    connection();
    latest().serverSays({ type: 'reconnect' });
    latest().onclose();
    vi.advanceTimersByTime(0);
    expect(FakeWebSocket.instances).toHaveLength(2);
  });

  it('stops at an exit until restarted, then opens exactly one new socket', () => {
    const socket = connection();
    latest().serverSays({ type: 'exit', code: 0 });
    latest().onclose();
    vi.runAllTimers();
    expect(FakeWebSocket.instances).toHaveLength(1);
    expect(socket.state.value).toMatchObject({ status: 'exited', exitCode: 0 });

    socket.restart();
    vi.runAllTimers();
    expect(FakeWebSocket.instances).toHaveLength(2);
    expect(socket.state.value.status).toBe('connecting');
  });

  it('never reconnects once disposed', () => {
    const socket = connection();
    socket.dispose();
    vi.runAllTimers();
    expect(FakeWebSocket.instances).toHaveLength(1);
  });
});
