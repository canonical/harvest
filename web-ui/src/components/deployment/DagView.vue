<template>
  <div class="dag-view">
    <div class="dag-view__toolbar">
      <div class="dag-view__tabs">
        <button
          type="button"
          class="dag-view__tab"
          :class="{ 'dag-view__tab--active': phase === 'deploy' }"
          @click="phase = 'deploy'"
        >Deploy</button>
        <button
          type="button"
          class="dag-view__tab"
          :class="{ 'dag-view__tab--active': phase === 'destroy' }"
          @click="phase = 'destroy'"
        >Destroy</button>
      </div>
      <button
        class="p-button--positive is-dense"
        type="button"
        data-testid="run-all-btn"
        :disabled="!plan.deploy_steps.length"
        @click="$emit('run-all')"
      >Run all</button>
    </div>

    <div class="dag-view__body">
      <div ref="canvasRef" class="dag-view__canvas" data-testid="dag-canvas" />

      <div v-if="selectedStep" class="dag-view__detail" data-testid="node-detail">
        <div class="dag-view__detail-header">
          <button class="dag-view__detail-close" type="button" @click="selectedStep = null">✕</button>
          <span class="dag-view__detail-label">{{ selectedStep.label }}</span>
        </div>
        <span class="dag-view__detail-action">{{ selectedStep.action }}</span>
        <span v-if="stepStatus[selectedStep.id]" class="dag-view__detail-status">{{ stepStatus[selectedStep.id] }}</span>
        <div class="dag-view__detail-actions">
          <button
            class="p-button--base is-dense"
            type="button"
            data-testid="run-node-btn"
            @click="$emit('run-node', selectedStep.id)"
          >Run this step</button>
          <button
            v-if="isTerraform(selectedStep)"
            class="p-button--base is-dense"
            type="button"
            data-testid="plan-preview-btn"
            @click="$emit('plan-preview', selectedStep.id)"
          >Plan preview</button>
        </div>
        <div v-if="isTerraform(selectedStep) && stepFiles[selectedStep.id]" class="dag-view__files">
          <div
            v-for="(content, path) in stepFiles[selectedStep.id]"
            :key="path"
            class="dag-view__file"
          >
            <div class="dag-view__file-path">{{ path }}</div>
            <pre>{{ content }}</pre>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted, watch, nextTick, computed } from 'vue';
import cytoscape from 'cytoscape';

const props = defineProps({
  plan:       { type: Object, required: true },
  stepFiles:  { type: Object, default: () => ({}) },
  stepStatus: { type: Object, default: () => ({}) },
});

const emit = defineEmits(['run-all', 'run-node', 'plan-preview', 'select-artifact']);

const canvasRef     = ref(null);
const selectedStep   = ref(null);
const phase          = ref('deploy');

let cy = null;

const currentSteps = computed(() =>
  phase.value === 'deploy' ? (props.plan.deploy_steps ?? []) : (props.plan.destroy_steps ?? [])
);

const KIND_COLORS = {
  bash:        '#7b42c9',
  terraform:   '#5c4ec4',
  terragrunt:  '#6b3fa0',
  markdown:    '#666',
  pdf:         '#666',
};

function buildElements(steps) {
  const nodes = steps.map(s => ({
    group: 'nodes',
    data: {
      id: s.id,
      label: s.label,
      action: s.action,
      kind: s.artifact?.kind ?? 'unknown',
      stepId: s.id,
      artifactId: s.artifact?.id ?? null,
      color: KIND_COLORS[s.artifact?.kind] ?? '#666',
    },
  }));
  const edges = steps.flatMap(s =>
    (s.depends_on ?? []).map(dep => ({
      group: 'edges',
      data: { id: `e-${s.id}-${dep}`, source: dep, target: s.id },
    }))
  );
  return [...nodes, ...edges];
}

function mountGraph() {
  if (!canvasRef.value) return;
  if (cy) { cy.destroy(); cy = null; }
  const steps = currentSteps.value;
  if (!steps.length) return;
  cy = cytoscape({
    container: canvasRef.value,
    elements: buildElements(steps),
    style: [
      {
        selector: 'node',
        style: {
          'background-color': 'data(color)',
          'label': 'data(label)',
          'color': '#fff',
          'text-wrap': 'wrap',
          'text-valign': 'center',
          'text-halign': 'center',
          'width': 120,
          'height': 50,
          'font-size': '11px',
          'shape': 'round-rectangle',
          'border-width': 2,
          'border-color': '#444',
        },
      },
      {
        selector: 'edge',
        style: {
          'width': 2,
          'line-color': '#888',
          'target-arrow-color': '#888',
          'target-arrow-shape': 'triangle',
          'curve-style': 'bezier',
        },
      },
    ],
    layout: {
      name: 'breadthfirst',
      directed: true,
      padding: 10,
      spacingFactor: 1.2,
    },
    minZoom: 0.3,
    maxZoom: 3,
  });
  cy.on('tap', 'node', (evt) => {
    const data = evt.target.data();
    selectNode(data.stepId);
    if (data.artifactId) emit('select-artifact', data.artifactId);
  });
}

function isTerraform(step) {
  const kind = step?.artifact?.kind;
  return kind === 'terraform' || kind === 'terragrunt';
}

function selectNode(stepId) {
  const allSteps = [...(props.plan.deploy_steps ?? []), ...(props.plan.destroy_steps ?? [])];
  selectedStep.value = allSteps.find(s => s.id === stepId) ?? null;
}

onMounted(() => {
  nextTick(() => mountGraph());
});

watch([currentSteps, () => props.plan], async () => {
  await nextTick();
  mountGraph();
}, { deep: true });

defineExpose({ selectNode });
</script>
