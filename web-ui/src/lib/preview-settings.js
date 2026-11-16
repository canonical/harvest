export const PREVIEW_KINDS = Object.freeze([
  { id: 'web', label: 'Web app', description: 'A site or dev server, shown inside Harvest.' },
  { id: 'api', label: 'HTTP API', description: 'Send requests to it and read the responses.' },
  { id: 'cli', label: 'Command-line tool', description: 'Run commands in a copy of the workspace.' },
]);

export function needsPort(kind) {
  return kind !== 'cli';
}

export function portProblem(port, kind, previews = []) {
  if (!needsPort(kind)) return '';
  if (!Number.isInteger(port) || port < 1 || port > 65535) return 'Enter a port between 1 and 65535.';
  const taken = previews.find(p => p.port === port);
  return taken ? `Port ${port} is already used by another preview of this session.` : '';
}

export function previewSettings({ kind, port, startCommand }) {
  if (!needsPort(kind)) return { kind, port: null, start_command: null };
  return { kind, port, start_command: startCommand.trim() || null };
}
