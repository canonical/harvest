export const PROVISION_PHASES = [
  { id: 'ensure_network', label: 'Ensuring network exists' },
  { id: 'install_token', label: 'Preparing install token' },
  { id: 'create_container', label: 'Creating container' },
  { id: 'start_container', label: 'Starting container' },
  { id: 'wait_running', label: 'Waiting for container to boot' },
  { id: 'install_agent', label: 'Installing Harvest agent' },
];

export function initialProvisionSteps() {
  return PROVISION_PHASES.map(p => ({ id: p.id, label: p.label, status: 'pending', detail: '' }));
}

export function applyProvisionEvent(steps, event) {
  const next = steps.map(s => ({ ...s }));

  if (event.type === 'phase_start') {
    const idx = next.findIndex(s => s.id === event.phase);
    next.forEach((s, i) => {
      if (i < idx && s.status !== 'error') s.status = 'done';
    });
    if (idx >= 0) {
      next[idx].status = 'active';
      next[idx].detail = '';
    }
    return next;
  }

  if (event.type === 'install_retry') {
    const step = next.find(s => s.id === 'install_agent');
    if (step) step.detail = `attempt ${event.attempt} of ${event.attempts}`;
    return next;
  }

  if (event.type === 'done') {
    next.forEach(s => {
      if (s.status !== 'error') s.status = 'done';
    });
    return next;
  }

  if (event.type === 'error') {
    const step = next.find(s => s.id === event.phase);
    if (step) {
      step.status = 'error';
      step.detail = event.message;
    }
    return next;
  }

  return next;
}

export function isProvisionDone(event) {
  return event?.type === 'done';
}

export function isProvisionError(event) {
  return event?.type === 'error';
}
