<template>
  <div class="design-gen" data-testid="design-generation">
    <div class="design-gen__status" data-testid="design-gen-status">
      <LoadingSpinner v-if="!finished" />
      <svg
        v-else-if="error"
        class="design-gen__status-icon design-gen__status-icon--error"
        viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"
        stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"
      >
        <circle cx="8" cy="8" r="6.5"/>
        <line x1="8" y1="5" x2="8" y2="8.75"/>
        <circle cx="8" cy="11" r="0.6" fill="currentColor" stroke="none"/>
      </svg>
      <svg
        v-else
        class="design-gen__status-icon design-gen__status-icon--done"
        viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"
        stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"
      >
        <circle cx="8" cy="8" r="6.5"/>
        <polyline points="5 8.2 7.1 10.3 11 6.2"/>
      </svg>
      <span class="design-gen__status-text" data-testid="design-gen-status-text">{{ statusText }}</span>
      <span v-if="!finished" class="design-gen__elapsed" data-testid="design-gen-elapsed">{{ elapsedLabel }}</span>
    </div>

    <div v-if="error" class="design-gen__error" data-testid="design-gen-error">
      <div class="p-notification--negative">
        <div class="p-notification__content">
          <p class="p-notification__message">{{ error }}</p>
        </div>
      </div>
      <div class="design-gen__error-actions">
        <button type="button" class="p-button--base is-dense" data-testid="design-gen-back" @click="cancel">Back</button>
        <button type="button" class="p-button--positive is-dense" data-testid="design-gen-retry" @click="retry">Try again</button>
      </div>
    </div>

    <div
      v-if="hasDetails"
      ref="activityRef"
      class="design-gen__activity"
      :class="{ 'design-gen__activity--bounded': streamText }"
      data-testid="design-gen-activity"
    >
      <div v-if="thinkingText" class="design-gen__thinking" data-testid="design-gen-thinking">
        <ThinkingBlock :text="thinkingText" :streaming="thinkingStreaming" />
      </div>

      <div v-if="chain.length" class="design-gen__timeline tc-chain" :class="{ 'tc-chain--running': !finished }" data-testid="design-gen-timeline">
        <ToolCallStep
          v-for="(step, i) in chain"
          :key="i"
          :step="step"
        />
      </div>
    </div>

    <div v-if="streamText" class="design-gen__preview-wrapper">
      <div class="p-text--small-caps u-text--muted">Live preview</div>
      <div class="design-gen__preview doc-body" data-testid="design-gen-preview" v-html="renderedStream"></div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, watch } from 'vue';
import { generateDesignStream } from '../../lib/api.js';
import { renderMarkdown } from '../../lib/markdown.js';
import { describeToolCall } from '../../lib/tool-render.js';
import ThinkingBlock from '../chat/ThinkingBlock.vue';
import ToolCallStep from '../chat/ToolCallStep.vue';
import LoadingSpinner from './LoadingSpinner.vue';

const props = defineProps({
  projectId:     { type: String, required: true },
  deploymentId:  { type: String, required: true },
  body:          { type: Object, default: () => ({}) },
  streamFn:      { type: Function, default: generateDesignStream },
  preparingText: { type: String, default: 'Preparing your design document…' },
  readyText:     { type: String, default: 'Design document ready' },
  failedText:    { type: String, default: 'Generation failed' },
});
const emit = defineEmits(['done', 'cancel']);

const activityRef       = ref(null);
const finished          = ref(false);
const error             = ref(null);
const thinkingText      = ref('');
const thinkingStreaming = ref(false);
const streamText        = ref('');
const chain             = ref([]);
const hasDetails        = ref(false);
const elapsedSeconds    = ref(0);

let timerId    = null;
let startedAt  = 0;

const renderedStream = computed(() => streamText.value ? renderMarkdown(streamText.value, {}, {}) : '');

// The most specific thing we know is happening right now: the tool currently
// running, in its own words (e.g. "Generating artifact design.md"). This is
// far more informative than the coarse backend-derived phase bucket, which
// collapses most tool calls into a generic "Working".
const runningStep = computed(() => {
  for (let i = chain.value.length - 1; i >= 0; i--) {
    if (chain.value[i].status === 'running') return chain.value[i];
  }
  return null;
});

const statusText = computed(() => {
  if (error.value)         return props.failedText;
  if (finished.value)      return props.readyText;
  if (streamText.value)    return 'Writing the design document…';
  if (runningStep.value)   return `${runningStep.value.description}…`;
  if (thinkingText.value)  return 'Thinking…';
  return props.preparingText;
});

const elapsedLabel = computed(() => {
  const s = elapsedSeconds.value;
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, '0')}s`;
});

function scrollToBottom() {
  const el = activityRef.value;
  if (!el) return;
  el.scrollTop = el.scrollHeight;
}

watch([chain, thinkingText], scrollToBottom, { deep: true, flush: 'post' });

function startTimer() {
  stopTimer();
  startedAt = Date.now();
  elapsedSeconds.value = 0;
  timerId = setInterval(() => {
    elapsedSeconds.value = Math.floor((Date.now() - startedAt) / 1000);
  }, 1000);
}

function stopTimer() {
  if (timerId) {
    clearInterval(timerId);
    timerId = null;
  }
}

function completeToolCall(name, preview) {
  const idx = chain.value.findIndex(s => s.name === name && s.status === 'running');
  if (idx !== -1) {
    chain.value[idx] = { ...chain.value[idx], status: 'done', preview };
  }
}

function handleEvent(event) {
  if (!event) return;
  switch (event.type) {
    case 'thinking':
      thinkingText.value = event.text || '';
      thinkingStreaming.value = false;
      hasDetails.value = true;
      break;
    case 'thinking_delta':
      thinkingText.value += event.text || '';
      thinkingStreaming.value = true;
      hasDetails.value = true;
      break;
    case 'text_delta':
      thinkingText.value = '';
      thinkingStreaming.value = false;
      streamText.value += event.text || '';
      break;
    case 'tool_call':
      thinkingText.value = '';
      thinkingStreaming.value = false;
      hasDetails.value = true;
      chain.value = [...chain.value, {
        type: 'tool_call',
        name: event.name,
        input: event.input,
        status: 'running',
        description: describeToolCall(event.name, event.input ?? {}),
      }];
      break;
    case 'tool_result':
      completeToolCall(event.name, event.preview);
      break;
    case 'done':
      finished.value = true;
      stopTimer();
      emit('done', { answer: event.answer, text: streamText.value });
      break;
    case 'error':
      error.value = event.message || props.failedText;
      finished.value = true;
      stopTimer();
      break;
  }
}

async function runGeneration() {
  finished.value           = false;
  error.value              = null;
  thinkingText.value       = '';
  thinkingStreaming.value  = false;
  streamText.value         = '';
  chain.value              = [];
  hasDetails.value         = false;
  startTimer();
  try {
    await props.streamFn(props.projectId, props.deploymentId, props.body, handleEvent);
    if (!finished.value && !error.value) {
      finished.value = true;
      emit('done', { text: streamText.value });
    }
  } catch (e) {
    error.value = e.message || props.failedText;
    finished.value = true;
  } finally {
    stopTimer();
  }
}

function retry() {
  runGeneration();
}

function cancel() {
  emit('cancel');
}

onMounted(runGeneration);
onUnmounted(stopTimer);
</script>
