export function useConsoleSocket() {
  let socket = null;
  const encoder = new TextEncoder();

  function connect(url, { onOpen, onData, onControl, onClose } = {}) {
    close();
    socket = new WebSocket(url);
    socket.binaryType = 'arraybuffer';
    socket.onopen = () => onOpen?.();

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
    if (socket) {
      socket.onopen = null;
      socket.onmessage = null;
      socket.onclose = null;
      socket.close();
    }
    socket = null;
  }

  return { connect, send, resize, close };
}
