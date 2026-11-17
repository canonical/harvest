<template>
  <div class="session-tab">
    <div class="session-tab__bar">
      <span class="changes-summary">Environment setup, background commands, checks and reviews</span>
      <button class="p-button--base is-dense" type="button" :disabled="starting" @click="runChecks">
        {{ starting ? 'Starting…' : 'Run checks' }}
      </button>
    </div>
    <div class="session-tab__body session-tab__body--scroll">
      <p v-if="error" class="u-text--muted">{{ error }}</p>
      <p v-else-if="!jobs.length" class="u-text--muted">No jobs yet. Harvest prepares the environment when the session starts and runs the checks after each change.</p>
      <ul v-else class="job-list">
        <li v-for="job in jobs" :key="job.id" class="job" :class="`job--${job.status}`">
          <div class="job__row">
            <span class="job__status">{{ statusLabel(job) }}</span>
            <span class="job__kind">{{ JOB_KIND_LABELS[job.kind] ?? job.kind }}</span>
            <span class="job__label">{{ job.label }}</span>
            <span class="job__duration">{{ duration(job, now) }}</span>
            <button
              class="p-button--base is-dense u-no-margin--bottom"
              type="button"
              :aria-expanded="String(expanded.has(job.id))"
              @click="toggle(job.id)"
            >{{ expanded.has(job.id) ? 'Hide output' : 'Show output' }}</button>
            <button v-if="canStop(job)" class="p-button--base is-dense u-no-margin--bottom" type="button" @click="stop(job)">Stop</button>
            <button v-if="needsFix(job)" class="p-button--positive is-dense u-no-margin--bottom" type="button" @click="askToFix(job)">Ask the agent to fix this</button>
          </div>
          <pre v-if="expanded.has(job.id)" class="job__output">{{ job.output || 'No output yet.' }}</pre>
        </li>
      </ul>
    </div>
  </div>
</template>

<script setup>
import { onMounted, onUnmounted, reactive, ref } from 'vue';
import { runSessionChecks, stopJob } from '../../lib/code-api.js';
import { JOB_KIND_LABELS, canStop, duration, fixRequest, needsFix, statusLabel } from '../../lib/session-jobs.js';
import { useLayoutWorkspace } from '../../lib/layout-workspace.js';
import { useSessionJobs } from '../../composables/useSessionJobs.js';
import { showToast } from '../../composables/useToasts.js';

const props = defineProps({
  tab: { type: Object, required: true },
  paneId: { type: String, required: true },
});

const CLOCK_MS = 1000;
const workspace = useLayoutWorkspace();
const { jobs, error, refresh } = useSessionJobs(workspace.sessionId);
const expanded = reactive(new Set());
const starting = ref(false);
const now = ref(Date.now());
let clock = null;

function toggle(id) {
  if (expanded.has(id)) expanded.delete(id);
  else expanded.add(id);
}

async function attempt(action) {
  try {
    await action();
  } catch (e) {
    showToast({ message: e.message, kind: 'negative' });
  }
}

async function runChecks() {
  starting.value = true;
  await attempt(async () => {
    await runSessionChecks(workspace.sessionId);
    await refresh();
  });
  starting.value = false;
}

function stop(job) {
  attempt(async () => {
    await stopJob(job.id);
    await refresh();
    showToast({ message: `Stopped “${job.label}”` });
  });
}

function askToFix(job) {
  attempt(() => workspace.askAgent(props.paneId, fixRequest(job)));
}

onMounted(() => {
  clock = setInterval(() => { now.value = Date.now(); }, CLOCK_MS);
});
onUnmounted(() => clearInterval(clock));
</script>
