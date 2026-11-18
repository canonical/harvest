import { ref } from 'vue';
import { useConsoleSocket } from './useConsoleSocket.js';
import { connecting, nextConnection, reconnectDelay, shouldConnect } from '../lib/terminal-connection.js';

const CONTROL_EVENTS = { exit: 'exit', reconnect: 'reconnect' };

export function useTerminalSocket({ urlFor, onData }) {
  const state = ref(connecting());
  const socket = useConsoleSocket();
  let retry = null;
  let disposed = false;

  function dispatch(event) {
    if (disposed) return;
    state.value = nextConnection(state.value, event);
    if (event.type === 'close' && shouldConnect(state.value)) {
      clearTimeout(retry);
      retry = setTimeout(open, reconnectDelay(state.value));
    }
  }

  function onControl(message) {
    const type = CONTROL_EVENTS[message.type];
    if (type) dispatch({ type, code: message.code });
  }

  function open() {
    if (disposed) return;
    socket.connect(urlFor(), {
      onOpen: () => dispatch({ type: 'open' }),
      onData,
      onControl,
      onClose: () => dispatch({ type: 'close' }),
    });
  }

  function restart() {
    socket.close();
    dispatch({ type: 'restart' });
    open();
  }

  function dispose() {
    disposed = true;
    clearTimeout(retry);
    socket.close();
  }

  return { state, open, restart, dispose, send: socket.send, resize: socket.resize };
}
