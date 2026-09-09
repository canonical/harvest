<template>
  <div class="design-gen deploy-gen" data-testid="deploy-generation">
    <div class="deploy-gen__header">
      <p class="deploy-gen__eyebrow">Step 2 of 2</p>
      <h2 class="deploy-gen__title">Generating deployment artifacts</h2>
      <p class="deploy-gen__subtitle">{{ deploymentName }}</p>
    </div>

    <div class="design-gen__status deploy-gen__status">
      <LoadingSpinner v-if="!finished" data-testid="deploy-gen-spinner" />
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
      <span class="design-gen__status-text">{{ statusText }}</span>
      <span v-if="!finished" class="design-gen__elapsed">{{ elapsedLabel }}</span>
      <span
        v-if="intent"
        class="intent-badge deploy-gen__intent"
        :class="`intent-badge--${intent}`"
        data-testid="deploy-gen-intent"
      >{{ intentLabel }}</span>
      <span v-if="phase" class="deploy-gen__phase" data-testid="deploy-gen-phase">{{ phase }}</span>
    </div>

    <div v-if="error" class="design-gen__error deploy-gen__error" data-testid="deploy-gen-error">
      <div class="p-notification--negative">
        <div class="p-notification__content">
          <p class="p-notification__message">{{ error }}</p>
        </div>
      </div>
      <div class="deploy-gen__error-actions">
        <button type="button" class="p-button--base is-dense" data-testid="deploy-gen-back" @click="$emit('cancel')">Back</button>
        <button type="button" class="p-button--positive is-dense" data-testid="deploy-gen-retry" @click="retry">Try again</button>
      </div>
    </div>

    <div
      v-if="hasDetails"
      ref="activityRef"
      class="design-gen__activity"
      :class="{ 'design-gen__activity--bounded': streamText }"
    >
      <div v-if="thinkingText" class="design-gen__thinking" data-testid="deploy-gen-thinking">
        <ThinkingBlock :text="thinkingText" :streaming="thinkingStreaming" />
      </div>

      <div v-if="chain.length" class="design-gen__timeline tc-chain" :class="{ 'tc-chain--running': !finished }" data-testid="deploy-gen-timeline">
        <ToolCallStep
          v-for="(step, i) in chain"
          :key="i"
          :step="step"
        />
      </div>
    </div>

    <div v-if="streamText" class="design-gen__preview-wrapper">
      <div class="deploy-gen__preview-label">Live preview</div>
      <div class="design-gen__preview doc-body" v-html="renderedStream"></div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, watch } from 'vue';
import { generateProvisionStream } from '../../lib/api.js';
import { renderMarkdown } from '../../lib/markdown.js';
import { describeToolCall } from '../../lib/tool-render.js';
import ThinkingBlock from '../chat/ThinkingBlock.vue';
import ToolCallStep from '../chat/ToolCallStep.vue';
import LoadingSpinner from './LoadingSpinner.vue';

const props = defineProps({
  projectId:       { type: String, required: true },
  deploymentId:    { type: String, required: true },
  deploymentName:  { type: String, default: '' },
});
const emit = defineEmits(['done', 'cancel']);

const activityRef         = ref(null);
const finished            = ref(false);
const error               = ref(null);
const intent              = ref(null);
const phase               = ref('');
const thinkingText        = ref('');
const thinkingStreaming   = ref(false);
const streamText          = ref('');
const chain               = ref([]);
const hasDetails          = ref(false);
const elapsedSeconds      = ref(0);

let timerId = null;
let startedAt = 0;

const renderedStream = computed(() => streamText.value ? renderMarkdown(streamText.value, {}, {}) : '');

const runningStep = computed(() => {
  for (let i = chain.value.length - 1; i >= 0; i--) {
    if (chain.value[i].status === 'running') return chain.value[i];
  }
  return null;
});

const statusText = computed(() => {
  if (error.value)        return 'Generation failed';
  if (finished.value)     return 'Deployment artifacts ready';
  if (streamText.value)   return 'Writing deployment artifacts…';
  if (runningStep.value)  return `${runningStep.value.description}…`;
  if (thinkingText.value)  return 'Thinking…';
  return 'Preparing deployment artifacts…';
});

const elapsedLabel = computed(() => {
  const s = elapsedSeconds.value;
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, '0')}s`;
});

const intentLabel = computed(() => {
  switch (intent.value) {
    case 'conversational': return 'Answering';
    case 'research':       return 'Researching';
    case 'action':         return 'Executing';
    case 'hybrid':         return 'Researching → Executing';
    default:               return '';
  }
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
    case 'intent':
      intent.value = event.mode;
      break;
    case 'phase':
      phase.value = event.label;
      break;
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
      emit('done');
      break;
    case 'error':
      error.value = event.message || 'Generation failed';
      finished.value = true;
      stopTimer();
      emit('done');
      break;
  }
}

async function runGeneration() {
  finished.value           = false;
  error.value              = null;
  intent.value             = null;
  phase.value              = '';
  thinkingText.value       = '';
  thinkingStreaming.value  = false;
  streamText.value         = '';
  chain.value              = [];
  hasDetails.value         = false;
  startTimer();
  try {
    await generateProvisionStream(props.projectId, props.deploymentId, handleEvent);
    if (!finished.value && !error.value) {
      finished.value = true;
      emit('done');
    }
  } catch (e) {
    error.value = e.message || 'Generation failed';
    finished.value = true;
    emit('done');
  } finally {
    stopTimer();
  }
}

function retry() {
  runGeneration();
}

onMounted(runGeneration);
onUnmounted(stopTimer);
</script>
