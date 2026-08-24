const ALLOWED_TRANSITIONS = {
  open:      ['in_review', 'discarded'],
  in_review: ['applied', 'discarded'],
  applied:   [],
  discarded: [],
};

export const CHANGE_REQUEST_STATUSES = ['open', 'in_review', 'applied', 'discarded'];

export function nextStatusOptions(status) {
  return ALLOWED_TRANSITIONS[status] ?? [];
}

export function isValidTransition(from, to) {
  return nextStatusOptions(from).includes(to);
}
