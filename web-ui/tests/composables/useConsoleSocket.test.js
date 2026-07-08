import { describe, it, expect, beforeEach } from 'vitest';
import { useConsoleSocket } from '../../src/composables/useConsoleSocket.js';

class FakeWebSocket {
  static OPEN = 1;
  static CLOSED = 3;
  static instances = [];

  constructor(url) {
    this.url = url;
    this.readyState = FakeWebSocket.OPEN;
    this.binaryType = null;
    this.sent = [];
    this.onmessage = null;
    this.onclose = null;
    FakeWebSocket.instances.push(this);
  }

  send(data) {
    this.sent.push(data);
  }

  close() {
    this.readyState = FakeWebSocket.CLOSED;
    this.onclose?.();
  }
}

beforeEach(() => {
  FakeWebSocket.instances = [];
  global.WebSocket = FakeWebSocket;
});

describe('useConsoleSocket', () => {
  it('connects and sets binaryType to arraybuffer', () => {
    const { connect } = useConsoleSocket();
    connect('ws://example/console');
    expect(FakeWebSocket.instances).toHaveLength(1);
    expect(FakeWebSocket.instances[0].binaryType).toBe('arraybuffer');
  });

  it('send encodes text as a binary frame', () => {
    const { connect, send } = useConsoleSocket();
    connect('ws://example/console');
    send('echo hi\n');
    const sent = FakeWebSocket.instances[0].sent[0];
    expect(new TextDecoder().decode(sent)).toBe('echo hi\n');
  });

  it('resize sends a JSON control text frame', () => {
    const { connect, resize } = useConsoleSocket();
    connect('ws://example/console');
    resize(80, 24);
    expect(FakeWebSocket.instances[0].sent[0]).toBe('{"type":"resize","cols":80,"rows":24}');
  });

  it('send is a no-op before connecting', () => {
    const { send } = useConsoleSocket();
    expect(() => send('x')).not.toThrow();
  });

  it('onData receives incoming binary frames as Uint8Array', () => {
    const received = [];
    const { connect } = useConsoleSocket();
    connect('ws://example/console', { onData: (d) => received.push(d) });

    const bytes = new TextEncoder().encode('hello');
    FakeWebSocket.instances[0].onmessage({ data: bytes.buffer });

    expect(received).toHaveLength(1);
    expect(received[0]).toBeInstanceOf(Uint8Array);
    expect(new TextDecoder().decode(received[0])).toBe('hello');
  });

  it('onControl receives parsed JSON for text frames', () => {
    const received = [];
    const { connect } = useConsoleSocket();
    connect('ws://example/console', { onControl: (c) => received.push(c) });

    FakeWebSocket.instances[0].onmessage({ data: '{"type":"exited","code":0}' });

    expect(received).toEqual([{ type: 'exited', code: 0 }]);
  });

  it('ignores malformed text frames instead of throwing', () => {
    const received = [];
    const { connect } = useConsoleSocket();
    connect('ws://example/console', { onControl: (c) => received.push(c) });

    expect(() => FakeWebSocket.instances[0].onmessage({ data: 'not-json' })).not.toThrow();
    expect(received).toHaveLength(0);
  });

  it('close() closes the socket and fires onClose', () => {
    let closed = false;
    const { connect, close } = useConsoleSocket();
    connect('ws://example/console', { onClose: () => { closed = true; } });

    close();

    expect(closed).toBe(true);
  });
});
