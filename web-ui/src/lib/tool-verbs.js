const RUNNING_VERBS = {
  create_lxd_agent:    'Creating…',
  delete_agent:        'Deleting…',
  create_port_forward: 'Creating…',
  update_port_forward: 'Updating…',
  delete_port_forward: 'Deleting…',
  save_setup_script:   'Saving…',
};

export function runningVerb(name) {
  return RUNNING_VERBS[name] ?? 'Working…';
}
