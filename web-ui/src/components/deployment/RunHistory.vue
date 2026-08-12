<template>
  <div class="run-history" data-testid="run-history">
    <p v-if="!combinedRuns.length" class="run-history__empty-state">No runs yet.</p>

    <template v-else>
      <div class="run-history__list">
        <button
          v-for="run in combinedRuns"
          :key="run.id"
          type="button"
          class="run-history__row"
          :class="{ 'run-history__row--selected': run.id === selectedId }"
          data-testid="run-history-item"
          @click="selectedId = run.id"
        >
          <span class="run-history__status-dot" :class="`run-history__status-dot--${run.status}`" aria-hidden="true"></span>
          <span class="run-history__action">{{ run.action }}</span>
          <span v-if="run.status === 'running'" class="run-history__exit">running…</span>
          <span v-else class="run-history__exit">exit {{ run.exit_code }}</span>
          <span class="run-history__time">{{ run.status === 'running' ? '' : formatTime(run.created_at) }}</span>
        </button>
      </div>

      <div v-if="selectedRun" class="run-history__detail">
        <template v-if="selectedRun.status === 'running'">
          <BusyStatus :text="liveStatusText" />
          <pre ref="liveLogEl" class="run-history__log"><span
            v-for="(l, i) in liveLog"
            :key="i"
            class="run-history__log-line"
            :class="{ 'run-history__log-line--stderr': l.stream === 'stderr' }"
          >{{ l.line }}
</span></pre>
        </template>
        <template v-else>
          <p v-if="selectedRun.reasoning" class="run-history__reasoning">{{ selectedRun.reasoning }}</p>
          <pre v-if="selectedRun.stdout_preview || selectedRun.stderr_preview" class="run-history__log">{{ selectedRun.stdout_preview }}{{ selectedRun.stderr_preview }}</pre>
          <p v-else class="run-history__empty">No output captured for this run.</p>
        </template>
      </div>
    </template>
  </div>
</template>

<script setup>
import { ref, computed, watch, nextTick } from 'vue';
import BusyStatus from './BusyStatus.vue';

const props = defineProps({
  runs:      { type: Array, default: () => [] },
  // { action, agentHostname } while a deploy/redeploy/destroy is in flight, else null.
  liveEntry: { type: Object, default: null },
  liveLog:   { type: Array, default: () => [] },
});

const selectedId = ref(null);
const liveLogEl = ref(null);

const combinedRuns = computed(() => {
  if (!props.liveEntry) return props.runs;
  return [{ id: '__live__', status: 'running', exit_code: null, created_at: null, ...props.liveEntry }, ...props.runs];
});

const selectedRun = computed(() => combinedRuns.value.find(r => r.id === selectedId.value) ?? null);

const liveStatusText = computed(() => {
  if (!props.liveEntry) return '';
  const verb = { deploy: 'Deploying', redeploy: 'Redeploying', destroy: 'Destroying' }[props.liveEntry.action] ?? 'Running';
  return `${verb}${props.liveEntry.agentHostname ? ` on ${props.liveEntry.agentHostname}` : ''}… this can take a few minutes.`;
});

function formatTime(iso) {
  if (!iso) return '';
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
}

// Whenever a new run appears at the top (a run starts, or a run just completed), follow it.
watch(() => combinedRuns.value[0]?.id, (id) => {
  if (id) selectedId.value = id;
}, { immediate: true });

watch(() => props.liveLog.length, () => {
  nextTick(() => {
    if (liveLogEl.value) liveLogEl.value.scrollTop = liveLogEl.value.scrollHeight;
  });
});
</script>
