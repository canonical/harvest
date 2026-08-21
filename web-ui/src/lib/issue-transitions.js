const ALLOWED_TRANSITIONS = {
  untriaged:   ['in_progress'],
  in_progress: ['fixed', 'rejected'],
  fixed:       [],
  rejected:    [],
};

export const ISSUE_STATUSES = ['untriaged', 'in_progress', 'fixed', 'rejected'];

export function nextStatusOptions(status) {
  return ALLOWED_TRANSITIONS[status] ?? [];
}

export function isValidTransition(from, to) {
  return nextStatusOptions(from).includes(to);
}
