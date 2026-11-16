import { ref } from 'vue';

const DEFAULT_TIMEOUT_MS = 5000;

export const toasts = ref([]);
let nextId = 1;

export function dismissToast(id) {
  const toast = toasts.value.find(t => t.id === id);
  if (!toast) return;
  clearTimeout(toast.timer);
  toasts.value = toasts.value.filter(t => t.id !== id);
  toast.onExpire?.();
}

export function showToast({ message, kind = 'positive', action = null, timeout = DEFAULT_TIMEOUT_MS, onExpire = null }) {
  const id = nextId++;
  const toast = { id, message, kind, action, onExpire };
  toast.timer = setTimeout(() => dismissToast(id), timeout);
  toasts.value = [...toasts.value, toast];
  return id;
}

export function runToastAction(id) {
  const toast = toasts.value.find(t => t.id === id);
  if (!toast) return;
  clearTimeout(toast.timer);
  toasts.value = toasts.value.filter(t => t.id !== id);
  toast.action?.run();
}

export function flushToasts() {
  for (const toast of [...toasts.value]) dismissToast(toast.id);
}
