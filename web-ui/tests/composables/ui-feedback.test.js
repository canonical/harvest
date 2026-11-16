import { describe, it, expect, vi, afterEach } from 'vitest';
import { confirm, settleConfirmation, pendingConfirmation } from '../../src/composables/useConfirm.js';
import { showToast, toasts, runToastAction, dismissToast } from '../../src/composables/useToasts.js';

afterEach(() => {
  vi.useRealTimers();
  for (const toast of [...toasts.value]) dismissToast(toast.id);
});

describe('confirmations', () => {
  it('resolve with the answer the person gives', async () => {
    const answer = confirm({ title: 'Delete?', message: 'Sure?' });
    expect(pendingConfirmation.value.title).toBe('Delete?');
    settleConfirmation(true);
    expect(await answer).toBe(true);
    expect(pendingConfirmation.value).toBeNull();
  });

  it('a newer question cancels the older one', async () => {
    const first = confirm({ title: 'One', message: '' });
    const second = confirm({ title: 'Two', message: '' });
    expect(await first).toBe(false);
    settleConfirmation(true);
    expect(await second).toBe(true);
  });
});

describe('toasts', () => {
  it('expire after their timeout and then run their expiry', () => {
    vi.useFakeTimers();
    const onExpire = vi.fn();
    showToast({ message: 'Deleted', timeout: 1000, onExpire });
    expect(toasts.value).toHaveLength(1);
    vi.advanceTimersByTime(1000);
    expect(toasts.value).toHaveLength(0);
    expect(onExpire).toHaveBeenCalledOnce();
  });

  it('an undo action skips the expiry', () => {
    vi.useFakeTimers();
    const onExpire = vi.fn();
    const run = vi.fn();
    const id = showToast({ message: 'Deleted', action: { label: 'Undo', run }, onExpire });
    runToastAction(id);
    vi.advanceTimersByTime(10000);
    expect(run).toHaveBeenCalledOnce();
    expect(onExpire).not.toHaveBeenCalled();
  });
});
