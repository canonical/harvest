<template>
  <div class="review-inbox" data-testid="review-inbox">
    <p v-if="!proposals.length" class="review-inbox__empty">No proposals.</p>
    <div class="review-inbox__scroll">
      <div v-for="p in proposals" :key="p.id" class="review-inbox__item" :data-testid="`proposal-${p.id}`">
        <div class="review-inbox__item-header">
          <span class="review-inbox__source">{{ p.source }}</span>
          <span class="review-inbox__kind">{{ p.target_artifact_kind }}</span>
          <span v-if="p.status !== 'pending'" class="review-inbox__status">{{ p.status }}</span>
        </div>
        <p class="review-inbox__explanation">{{ p.explanation }}</p>
        <button class="p-button--base is-dense review-inbox__expand" type="button" @click="toggleExpand(p.id)">
          {{ expanded[p.id] ? 'Hide diff' : 'Show diff' }}
        </button>
        <DiffView
          v-if="expanded[p.id]"
          :before="parsedCurrent(p)"
          :after="parsedProposed(p)"
        />
        <div v-if="p.status === 'pending'" class="review-inbox__actions">
          <button class="p-button--base is-dense" type="button" :disabled="busy[p.id]" @click="startEdit(p)">Edit</button>
          <button class="p-button--base is-dense" type="button" :disabled="busy[p.id]" @click="discard(p.id)">Discard</button>
          <button class="p-button--positive is-dense" type="button" :disabled="busy[p.id]" @click="approve(p.id)">Approve</button>
        </div>
        <div v-if="editingId === p.id" class="review-inbox__edit">
          <textarea v-model="editedContent" rows="6" data-testid="edit-content" />
          <div class="review-inbox__edit-actions">
            <button class="p-button--base is-dense" type="button" @click="cancelEdit">Cancel</button>
            <button class="p-button--positive is-dense" type="button" @click="approveEdited(p.id)">Apply</button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed } from 'vue';
import DiffView from './DiffView.vue';
import { listDeploymentProposals, approveProposal, discardProposal } from '../../lib/api.js';

const props = defineProps({
  projectId:    { type: String, required: true },
  deploymentId: { type: String, required: true },
});

const proposals   = ref([]);
const expanded     = ref({});
const busy         = ref({});
const editingId    = ref(null);
const editedContent = ref('');

async function load() {
  try {
    proposals.value = await listDeploymentProposals(props.projectId, props.deploymentId);
  } catch {
    proposals.value = [];
  }
}

function toggleExpand(id) {
  expanded.value[id] = !expanded.value[id];
}

function parsedCurrent(p) {
  try { return JSON.parse(p.current_content); } catch { return {}; }
}

function parsedProposed(p) {
  try { return JSON.parse(p.proposed_content); } catch { return {}; }
}

async function approve(id) {
  busy.value[id] = true;
  try {
    await approveProposal(props.projectId, props.deploymentId, id, {});
    await load();
  } finally {
    busy.value[id] = false;
  }
}

async function discard(id) {
  busy.value[id] = true;
  try {
    await discardProposal(props.projectId, props.deploymentId, id);
    await load();
  } finally {
    busy.value[id] = false;
  }
}

function startEdit(p) {
  editingId.value = p.id;
  let content = p.proposed_content;
  try {
    const parsed = JSON.parse(content);
    content = JSON.stringify(parsed, null, 2);
  } catch {}
  editedContent.value = content;
}

function cancelEdit() {
  editingId.value = null;
  editedContent.value = '';
}

async function approveEdited(id) {
  busy.value[id] = true;
  try {
    await approveProposal(props.projectId, props.deploymentId, id, { edited_content: editedContent.value });
    editingId.value = null;
    editedContent.value = '';
    await load();
  } finally {
    busy.value[id] = false;
  }
}

load();
</script>
