import { shallowRef } from 'vue';

export const pendingConfirmation = shallowRef(null);

export function confirm({ title, message, confirmLabel = 'Confirm', danger = false }) {
  pendingConfirmation.value?.resolve(false);
  return new Promise((resolve) => {
    pendingConfirmation.value = { title, message, confirmLabel, danger, resolve };
  });
}

export function settleConfirmation(answer) {
  const pending = pendingConfirmation.value;
  pendingConfirmation.value = null;
  pending?.resolve(answer);
}
