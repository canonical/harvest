export function useConsoleSocket() {
  let socket = null;
  const encoder = new TextEncoder();

  function connect(url, { onData, onControl, onClose } = {}) {
    socket = new WebSocket(url);
    socket.binaryType = 'arraybuffer';

    socket.onmessage = (event) => {
      if (typeof event.data === 'string') {
        let parsed;
        try {
          parsed = JSON.parse(event.data);
        } catch {
          return;
        }
        onControl?.(parsed);
        return;
      }
      onData?.(new Uint8Array(event.data));
    };

    socket.onclose = () => onClose?.();
  }

  function send(text) {
    if (!socket || socket.readyState !== WebSocket.OPEN) return;
    socket.send(encoder.encode(text));
  }

  function resize(cols, rows) {
    if (!socket || socket.readyState !== WebSocket.OPEN) return;
    socket.send(JSON.stringify({ type: 'resize', cols, rows }));
  }

  function close() {
    socket?.close();
    socket = null;
  }

  return { connect, send, resize, close };
}
