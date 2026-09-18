<template>
  <div class="design-gen" data-testid="design-generation">
    <div v-if="title" class="design-gen__header">
      <p v-if="eyebrow" class="design-gen__eyebrow" data-testid="design-eyebrow">{{ eyebrow }}</p>
      <div class="design-gen__title-row">
        <h2 class="design-gen__title">{{ title }}</h2>
        <span v-if="badge" class="p-chip design-gen__badge">{{ badge }}</span>
      </div>
      <p v-if="subtitle" class="design-gen__subtitle">{{ subtitle }}</p>
    </div>

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

    <DesignDocumentHero :text="heroText" :thinking="heroIsThinking" :running="!finished" />
  </div>
</template>

<script setup>
import { computed, onMounted, onUnmounted } from 'vue';
import { generateDesignStream } from '../../lib/api.js';
import { useAgentStream } from '../../lib/agent-stream.js';
import DesignDocumentHero from './DesignDocumentHero.vue';
import LoadingSpinner from './LoadingSpinner.vue';

const props = defineProps({
  projectId:     { type: String, required: true },
  deploymentId:  { type: String, required: true },
  body:          { type: Object, default: () => ({}) },
  streamFn:      { type: Function, default: generateDesignStream },
  preparingText: { type: String, default: 'Preparing your design document…' },
  readyText:     { type: String, default: 'Design document ready' },
  failedText:    { type: String, default: 'Generation failed' },
  writingText:   { type: String, default: 'Writing the design document…' },
  eyebrow:       { type: String, default: '' },
  title:         { type: String, default: '' },
  subtitle:      { type: String, default: '' },
  badge:         { type: String, default: '' },
});
const emit = defineEmits(['done', 'cancel']);

const {
  finished, error, streamText, thinkingText,
  statusText, elapsedLabel,
  handleEvent, reset, stopTimer,
} = useAgentStream({
  preparingText: props.preparingText,
  readyText:     props.readyText,
  failedText:    props.failedText,
  writingText:   props.writingText,
});

const heroText       = computed(() => streamText.value || thinkingText.value);
const heroIsThinking = computed(() => !streamText.value && !!thinkingText.value);

async function runGeneration() {
  reset();
  try {
    await props.streamFn(props.projectId, props.deploymentId, props.body, (event) => {
      handleEvent(event);
      if (event?.type === 'done') emit('done', { answer: event.answer, text: streamText.value });
    });
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
