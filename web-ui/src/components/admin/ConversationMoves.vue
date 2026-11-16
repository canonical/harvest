<template>
  <div class="admin-panel" role="tabpanel">
    <p class="u-text--muted admin-panel-hint">
      These deploy project chats only searched code. Moving one makes it a personal chat of its author in Chat mode.
    </p>
    <div v-if="loading" class="admin-empty"><p>Loading…</p></div>
    <div v-else-if="!moves.length" class="admin-empty"><p>No chats to move.</p></div>
    <template v-else>
      <div class="admin-table-scroll">
        <table class="p-table">
          <thead>
            <tr>
              <th><input type="checkbox" :checked="allSelected" aria-label="Select all" @change="toggleAll($event.target.checked)" /></th>
              <th>Chat</th>
              <th>Project</th>
              <th>Author</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="move in moves" :key="move.id">
              <td><input v-model="selected" type="checkbox" :value="move.id" :aria-label="`Select ${move.title}`" /></td>
              <td>{{ move.title || 'Untitled chat' }}</td>
              <td>{{ move.project_name }}</td>
              <td>{{ move.author_email }}</td>
            </tr>
          </tbody>
        </table>
      </div>
      <div class="modal-actions">
        <button class="p-button--positive is-dense" type="button" :disabled="!selected.length || moving" @click="moveSelected">
          Move {{ selected.length }} chat{{ selected.length === 1 ? '' : 's' }} to Chat mode
        </button>
      </div>
    </template>
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue';
import { listConversationMoves, moveConversations } from '../../lib/api.js';

const emit = defineEmits(['done', 'failed']);

const moves = ref([]);
const selected = ref([]);
const loading = ref(true);
const moving = ref(false);

const allSelected = computed(() => moves.value.length > 0 && selected.value.length === moves.value.length);

function toggleAll(checked) {
  selected.value = checked ? moves.value.map(m => m.id) : [];
}

async function load() {
  loading.value = true;
  try {
    moves.value = await listConversationMoves();
    selected.value = moves.value.map(m => m.id);
  } catch (e) {
    emit('failed', e.message);
  } finally {
    loading.value = false;
  }
}

async function moveSelected() {
  moving.value = true;
  try {
    const { moved } = await moveConversations(selected.value);
    emit('done', `Moved ${moved} chat${moved === 1 ? '' : 's'} to Chat mode.`);
    await load();
  } catch (e) {
    emit('failed', e.message);
  } finally {
    moving.value = false;
  }
}

onMounted(load);
</script>
