<template>
  <div class="design-gen" data-testid="design-generation">
    <div class="design-gen__header">
      <p class="p-text--small-caps u-text--muted">Design</p>
      <h2 class="p-heading--3">Generating design document</h2>
      <p class="u-text--muted">{{ deploymentName }}</p>
    </div>

    <div v-if="error" class="design-gen__error" data-testid="design-gen-error">
      <div class="p-notification--negative">
        <div class="p-notification__content">
          <p class="p-notification__message">{{ error }}</p>
        </div>
      </div>
    </div>

    <div v-if="!started && !error" class="design-gen__spinner" data-testid="design-gen-spinner">
      <LoadingSpinner text="Starting…" />
    </div>

    <div v-if="intent || phase" class="design-gen__status-bar">
      <span v-if="intent" class="intent-badge" :class="`intent-badge--${intent}`" data-testid="design-gen-intent">
        {{ intentLabel }}
      </span>
      <span v-if="phase" class="design-gen__phase" data-testid="design-gen-phase">{{ phase }}</span>
    </div>

    <div
      ref="activityRef"
      class="design-gen__activity"
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
import { ref, computed, onMounted, watch } from 'vue';
import { generateDesignStream } from '../../lib/api.js';
import { renderMarkdown } from '../../lib/markdown.js';
import { describeToolCall } from '../../lib/tool-render.js';
import ThinkingBlock from '../chat/ThinkingBlock.vue';
import ToolCallStep from '../chat/ToolCallStep.vue';
import LoadingSpinner from './LoadingSpinner.vue';

const props = defineProps({
  projectId:    { type: String, required: true },
  deploymentId: { type: String, required: true },
  body:         { type: Object, default: () => ({}) },
  deploymentName: { type: String, default: '' },
});
const emit = defineEmits(['done']);

const activityRef     = ref(null);
const started      = ref(false);
const finished     = ref(false);
const error        = ref(null);
const intent       = ref(null);
const phase        = ref('');
const thinkingText = ref('');
const thinkingStreaming = ref(false);
const streamText   = ref('');
const chain        = ref([]);

const renderedStream = computed(() => streamText.value ? renderMarkdown(streamText.value, {}, {}) : '');

function scrollToBottom() {
  const el = activityRef.value;
  if (!el) return;
  el.scrollTop = el.scrollHeight;
}

watch([chain, thinkingText], scrollToBottom, { deep: true, flush: 'post' });

const intentLabel = computed(() => {
  switch (intent.value) {
    case 'conversational': return 'Answering';
    case 'research':      return 'Researching';
    case 'action':        return 'Executing';
    case 'hybrid':        return 'Researching → Executing';
    default:              return '';
  }
});

function completeToolCall(name, preview) {
  const idx = chain.value.findIndex(s => s.name === name && s.status === 'running');
  if (idx !== -1) {
    chain.value[idx] = { ...chain.value[idx], status: 'done', preview };
  }
}

function handleEvent(event) {
  if (!event) return;
  started.value = true;
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
      break;
    case 'thinking_delta':
      thinkingText.value += event.text || '';
      thinkingStreaming.value = true;
      break;
    case 'text_delta':
      thinkingText.value = '';
      thinkingStreaming.value = false;
      streamText.value += event.text || '';
      break;
    case 'tool_call':
      thinkingText.value = '';
      thinkingStreaming.value = false;
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
      emit('done');
      break;
    case 'error':
      error.value = event.message || 'Generation failed';
      finished.value = true;
      emit('done');
      break;
  }
}

onMounted(async () => {
  try {
    await generateDesignStream(props.projectId, props.deploymentId, props.body, handleEvent);
    if (!finished.value && !error.value) {
      finished.value = true;
      emit('done');
    }
  } catch (e) {
    error.value = e.message || 'Generation failed';
    finished.value = true;
    emit('done');
  }
});
</script>
