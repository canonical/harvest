export const MAX_RECONNECTS = 8;
const BACKOFF_MS = [500, 1000, 2000, 4000, 8000];

export function connecting() {
  return { status: 'connecting', attempt: 0, exitCode: null };
}

export function nextConnection(state, event) {
  switch (event.type) {
    case 'open':
      return { status: 'open', attempt: 0, exitCode: null };
    case 'exit':
      return { status: 'exited', attempt: 0, exitCode: event.code ?? null };
    case 'reconnect':
      return { status: 'moving', attempt: 0, exitCode: null };
    case 'close':
      return closed(state);
    case 'restart':
      return connecting();
    default:
      return state;
  }
}

function closed(state) {
  if (state.status === 'exited' || state.status === 'lost') return state;
  if (state.status === 'moving') return { status: 'reconnecting', attempt: 0, exitCode: null };
  if (state.attempt >= MAX_RECONNECTS) return { status: 'lost', attempt: state.attempt, exitCode: null };
  return { status: 'reconnecting', attempt: state.attempt + 1, exitCode: null };
}

export function shouldConnect(state) {
  return state.status === 'connecting' || state.status === 'reconnecting';
}

export function reconnectDelay(state) {
  if (state.attempt === 0) return 0;
  return BACKOFF_MS[Math.min(state.attempt - 1, BACKOFF_MS.length - 1)];
}

export function connectionNotice(state) {
  if (state.status === 'exited') {
    const code = state.exitCode === null ? '' : ` with code ${state.exitCode}`;
    return { tone: 'information', message: `The shell exited${code}.`, action: 'Restart' };
  }
  if (state.status === 'lost') {
    return { tone: 'negative', message: 'The connection to this terminal was lost.', action: 'Reconnect' };
  }
  if (state.status === 'reconnecting' && state.attempt > 0) {
    return { tone: 'caution', message: 'Reconnecting to the terminal…', action: null };
  }
  return null;
}
