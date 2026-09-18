<template>
  <div class="design-gen" data-testid="deploy-generation">
    <div class="design-gen__header">
      <p class="design-gen__eyebrow">Deploy</p>
      <div class="design-gen__title-row">
        <h2 class="design-gen__title">{{ deploymentName }}</h2>
        <span class="p-chip design-gen__badge">Step 2 of 2</span>
      </div>
      <p class="design-gen__subtitle">Generating deployment artifacts…</p>
    </div>

    <div class="design-gen__status" data-testid="deploy-gen-status">
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
    </div>

    <div v-if="error" class="design-gen__error" data-testid="deploy-gen-error">
      <div class="p-notification--negative">
        <div class="p-notification__content">
          <p class="p-notification__message">{{ error }}</p>
        </div>
      </div>
      <div class="design-gen__error-actions">
        <button type="button" class="p-button--base is-dense" data-testid="deploy-gen-back" @click="$emit('cancel')">Back</button>
        <button type="button" class="p-button--positive is-dense" data-testid="deploy-gen-retry" @click="retry">Try again</button>
      </div>
    </div>

    <DesignDocumentHero v-if="!files.length" :text="thinkingText" thinking :running="!finished" />
    <DeployFileTabs v-else :files="files" :active-file-id="activeFileId" :running="!finished" />
  </div>
</template>

<script setup>
import { onMounted, onUnmounted } from 'vue';
import { generateProvisionStream } from '../../lib/api.js';
import { useAgentStream } from '../../lib/agent-stream.js';
import DeployFileTabs from './DeployFileTabs.vue';
import DesignDocumentHero from './DesignDocumentHero.vue';
import LoadingSpinner from './LoadingSpinner.vue';

const props = defineProps({
  projectId:      { type: String, required: true },
  deploymentId:   { type: String, required: true },
  deploymentName: { type: String, default: '' },
});
const emit = defineEmits(['done', 'cancel']);

const {
  finished, error, files, activeFileId, thinkingText,
  statusText, elapsedLabel,
  handleEvent, reset, stopTimer,
} = useAgentStream({
  preparingText: 'Preparing deployment artifacts…',
  readyText:     'Deployment artifacts ready',
  failedText:    'Generation failed',
  writingText:   'Writing deployment artifacts…',
});

async function runGeneration() {
  reset();
  try {
    await generateProvisionStream(props.projectId, props.deploymentId, (event) => {
      handleEvent(event);
      if (event?.type === 'done') emit('done');
    });
    if (!finished.value && !error.value) {
      finished.value = true;
      emit('done');
    }
  } catch (e) {
    error.value = e.message || 'Generation failed';
    finished.value = true;
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
