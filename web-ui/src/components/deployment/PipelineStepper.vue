<template>
  <div class="pipeline-stepper" data-testid="pipeline-stepper">
    <div v-if="sortedSteps.length === 0" class="pipeline-stepper__empty">
      No steps in the execution plan.
    </div>
    <ul class="pipeline-stepper__list" v-else>
      <li
        v-for="(step, idx) in sortedSteps"
        :key="step.id"
        class="pipeline-stepper__item"
        :class="{
          'pipeline-stepper__item--active': isSelected(step),
          'pipeline-stepper__item--orphaned': isOrphaned(step),
        }"
        :data-testid="`pipeline-step-${step.artifact?.id ?? step.id}`"
        role="button"
        tabindex="0"
        @click="selectStep(step)"
        @keydown.enter="selectStep(step)"
        @keydown.space.prevent="selectStep(step)"
      >
        <div class="pipeline-stepper__connector" aria-hidden="true" v-if="idx > 0 && !isOrphaned(step)">
          <span class="pipeline-stepper__arrow">↓</span>
        </div>
        <div class="pipeline-stepper__connector pipeline-stepper__connector--dashed" aria-hidden="true" v-if="idx > 0 && isOrphaned(step)">
        </div>

        <div class="pipeline-stepper__card">
          <div class="pipeline-stepper__card-header">
            <span class="pipeline-stepper__number">{{ stepNumber(sortedSteps, step.id) }}</span>
            <span class="artifact-kind-badge" :class="kindBadgeClass(step.artifact?.kind)">{{ kindLabel(step.artifact?.kind) }}</span>
            <span class="pipeline-stepper__action">{{ actionLabel(step.action) }}</span>
            <span
              v-if="statusFor(step.id)"
              class="pipeline-stepper__status"
              :class="statusClass(statusFor(step.id))"
            >{{ statusIcon(statusFor(step.id)) }}</span>
          </div>
          <div class="pipeline-stepper__card-title">{{ step.artifact?.title ?? step.label }}</div>
          <div class="pipeline-stepper__deps" v-if="(step.depends_on ?? []).length > 0">
            depends on:
            <span
              v-for="dep in step.depends_on"
              :key="dep"
              class="pipeline-stepper__dep-ref"
            >{{ stepNumber(sortedSteps, dep) }}</span>
          </div>
          <div class="pipeline-stepper__deps pipeline-stepper__deps--orphaned" v-else-if="isOrphaned(step)">
            no dependencies
          </div>
        </div>
      </li>
    </ul>
  </div>
</template>

<script setup>
import { computed } from 'vue';
import { topologicalSort, stepNumber } from '../../lib/dag.js';

const props = defineProps({
  steps:         { type: Array, required: true },
  stepStatus:    { type: Object, default: () => ({}) },
  selectedStepId: { type: String, default: null },
});

const emit = defineEmits(['select']);

const sortedSteps = computed(() => topologicalSort(props.steps));

function isSelected(step) {
  if (!props.selectedStepId) return false;
  const id = step.artifact?.id ?? step.id;
  return id === props.selectedStepId;
}

function isOrphaned(step) {
  return (step.depends_on ?? []).length === 0 && !isReferencedByAnyOther(step);
}

function isReferencedByAnyOther(step) {
  return props.steps.some(s =>
    s.id !== step.id && (s.depends_on ?? []).includes(step.id)
  );
}

function selectStep(step) {
  if (step.artifact?.id) emit('select', step.artifact.id);
}

function statusFor(stepId) {
  return props.stepStatus?.[stepId] ?? null;
}

function statusClass(status) {
  if (status === 'success' || status === 'done') return 'pipeline-stepper__status--success';
  if (status === 'failed' || status === 'error') return 'pipeline-stepper__status--failed';
  if (status === 'running') return 'pipeline-stepper__status--running';
  return '';
}

function statusIcon(status) {
  if (status === 'success' || status === 'done') return '✓';
  if (status === 'failed' || status === 'error') return '✗';
  if (status === 'running') return '…';
  return '';
}

function kindLabel(kind) {
  if (kind === 'pdf') return 'PDF';
  if (kind === 'terraform') return 'Terraform';
  if (kind === 'terragrunt') return 'Terragrunt';
  if (kind === 'bash') return 'Bash';
  if (kind === 'markdown') return 'Markdown';
  return kind ?? 'Unknown';
}

function kindBadgeClass(kind) {
  if (kind === 'pdf') return 'artifact-kind-badge--pdf';
  if (kind === 'terraform' || kind === 'terragrunt') return 'artifact-kind-badge--terraform';
  if (kind === 'bash') return 'artifact-kind-badge--bash';
  if (kind === 'markdown') return 'artifact-kind-badge--markdown';
  return '';
}

function actionLabel(action) {
  if (action === 'apply') return 'apply';
  if (action === 'run') return 'run';
  if (action === 'destroy') return 'destroy';
  if (action === 'plan') return 'plan';
  return action ?? '';
}
</script>
