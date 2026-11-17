import { onUnmounted, ref } from 'vue';
import { listSessionJobs } from '../lib/code-api.js';
import { isRunning } from '../lib/session-jobs.js';

const FAST_POLL_MS = 3000;
const SLOW_POLL_MS = 15000;
const feeds = new Map();

function feedFor(sessionId) {
  if (!feeds.has(sessionId)) {
    feeds.set(sessionId, { jobs: ref([]), error: ref(''), users: 0, timer: null });
  }
  return feeds.get(sessionId);
}

async function poll(sessionId) {
  const feed = feeds.get(sessionId);
  if (!feed || feed.users === 0) return;
  clearTimeout(feed.timer);
  if (document.visibilityState === 'visible') {
    try {
      feed.jobs.value = (await listSessionJobs(sessionId)).jobs ?? [];
      feed.error.value = '';
    } catch (e) {
      feed.error.value = e.message;
    }
  }
  if (feed.users === 0) return;
  const delay = feed.jobs.value.some(isRunning) ? FAST_POLL_MS : SLOW_POLL_MS;
  feed.timer = setTimeout(() => poll(sessionId), delay);
}

export function useSessionJobs(sessionId) {
  const feed = feedFor(sessionId);
  feed.users += 1;
  if (feed.users === 1) poll(sessionId);
  onUnmounted(() => {
    feed.users -= 1;
    if (feed.users === 0) {
      clearTimeout(feed.timer);
      feeds.delete(sessionId);
    }
  });
  return {
    jobs: feed.jobs,
    error: feed.error,
    refresh: () => poll(sessionId),
  };
}
