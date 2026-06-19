<template>
  <div class="tasks-page">
    <div v-if="!projectId" class="no-project-state">
      <p>Select a project to manage its tasks.</p>
    </div>

    <template v-else>
      <div class="tasks-header">
        <h2>Tasks</h2>
        <button class="p-button--positive create-task-btn" data-testid="create-task" type="button" @click="openCreate">+ Create task</button>
      </div>

      <div class="tasks-layout">
        <div ref="graphEl" class="tasks-graph" />
      </div>

      <div
        class="task-detail-panel"
        :class="{ 'is-open': !!selectedTask, 'is-expanded': panelExpanded }"
      >
        <div class="tdp-header">
          <div class="tdp-header__toolbar">
            <button
              type="button"
              class="p-button--base has-icon u-no-margin source-panel-expand"
              :aria-label="panelExpanded ? 'Shrink panel' : 'Expand panel'"
              @click="toggleExpand"
            ><i :class="panelExpanded ? 'p-icon--chevron-right' : 'p-icon--chevron-left'"></i></button>
            <div class="tdp-actions">
              <button
                class="p-button--positive p-button--small"
                type="button"
                :disabled="running || !selectedTask"
                @click="selectedTask && runTask(selectedTask.id)"
              >
                <svg viewBox="0 0 10 10" width="8" height="8" fill="currentColor" aria-hidden="true" style="flex-shrink:0"><polygon points="2,1 9,5 2,9"/></svg>
                Run
              </button>
              <button class="p-button p-button--small" type="button" :disabled="!selectedTask" @click="selectedTask && openEdit(selectedTask)">Edit</button>
              <button class="p-button--negative p-button--small" type="button" :disabled="!selectedTask" @click="selectedTask && confirmDeleteTask(selectedTask.id)">Delete</button>
            </div>
            <button type="button" class="p-button--base has-icon u-no-margin tdp-close-btn" aria-label="Close panel" @click="closePanel">
              <i class="p-icon--close"></i>
            </button>
          </div>
          <div class="tdp-identity">
            <h3 class="tdp-task-name">{{ selectedTask?.name ?? '—' }}</h3>
            <span v-if="selectedTask" class="task-status" :class="`task-status--${currentStatus ?? selectedTask.status}`">
              {{ currentStatus ?? selectedTask.status }}
            </span>
          </div>
        </div>

        <div class="task-detail-panel__body">
          <template v-if="selectedTask">
            <details v-if="selectedTask.prompt" class="tdp-prompt">
              <summary class="tdp-prompt__summary">
                <svg class="tdp-prompt__chevron" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="2,3 5,7 8,3"/></svg>
                <span>Prompt</span>
              </summary>
              <pre class="tdp-prompt__text">{{ selectedTask.prompt }}</pre>
            </details>

            <template v-if="currentChain.length">
              <div class="tasks-detail-tool-calls">
                <div class="tc-chain">
                  <template v-for="(item, i) in currentChain" :key="i">
                    <details v-if="item.type === 'thinking'" class="thinking-group" open>
                      <summary class="thinking-group__summary">
                        <svg class="tc-group__summary-chevron" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="2,3 5,7 8,3"/></svg>
                        <span>Thinking</span>
                      </summary>
                      <div class="thinking-group__body"><p>{{ item.text }}</p></div>
                    </details>
                    <ToolCallStep v-else :step="item" />
                  </template>
                </div>
              </div>
            </template>

            <template v-else-if="storedOutput?.toolCalls?.length">
              <div class="tasks-detail-tool-calls">
                <div class="tc-chain">
                  <ToolCallStep v-for="(tc, i) in storedOutput.toolCalls" :key="i" :step="tc" />
                </div>
              </div>
            </template>

            <div v-if="currentStatus === 'error' && currentAnswer" class="tdp-error-block">
              <i class="p-icon--error"></i>
              <span>{{ currentAnswer }}</span>
            </div>
            <div v-else-if="currentAnswer" class="tasks-detail-answer" v-html="renderedAnswer" />
            <div v-else-if="!currentChain.length && !storedOutput?.toolCalls?.length" class="tdp-empty">
              <i class="p-icon--status-waiting-small tdp-empty__icon"></i>
              <span>No output yet.</span>
            </div>
          </template>
        </div>
      </div>
    </template>

    <div v-if="showCreate" class="modal" @click.self="showCreate = false">
      <div class="task-edit-modal">
        <div class="task-edit-modal__header">
          <h3 class="task-edit-modal__title">Create task</h3>
          <button type="button" class="p-button--base has-icon u-no-margin" aria-label="Close" @click="showCreate = false">
            <i class="p-icon--close"></i>
          </button>
        </div>

        <div class="task-edit-modal__body">
          <div class="task-edit-form">
            <div class="task-edit-field">
              <label class="task-edit-field__label" for="task-name-input">
                Name <span class="task-edit-field__required" aria-hidden="true">*</span>
              </label>
              <input
                id="task-name-input"
                v-model="newName"
                type="text"
                name="task-name"
                placeholder="Task name"
                autocomplete="off"
              />
            </div>

            <div class="task-edit-field">
              <label class="task-edit-field__label" for="create-task-prompt">
                Prompt <span class="task-edit-field__required" aria-hidden="true">*</span>
              </label>
              <textarea
                id="create-task-prompt"
                v-model="newPrompt"
                rows="8"
                class="task-prompt-input"
                placeholder="Describe what this task should do…"
              />
            </div>

            <div class="task-edit-field">
              <label class="task-edit-field__label">Dependencies</label>
              <div v-if="newDeps.length" class="task-edit-deps__chips">
                <span v-for="id in newDeps" :key="id" class="p-chip">
                  <span class="p-chip__value">{{ tasks.find(t => t.id === id)?.name ?? id }}</span>
                  <button
                    type="button"
                    class="p-chip__dismiss"
                    :aria-label="`Remove ${tasks.find(t => t.id === id)?.name ?? id}`"
                    @click="newDeps = newDeps.filter(d => d !== id)"
                  ><i class="p-icon--close"></i></button>
                </span>
              </div>
              <select class="task-edit-deps__select" @change="addDep($event)">
                <option value="">No dependency</option>
                <option v-for="t in availableForDep" :key="t.id" :value="t.id">{{ t.name }}</option>
              </select>
            </div>

            <div v-if="createError" class="p-notification--negative">
              <div class="p-notification__content">
                <p class="p-notification__message">{{ createError }}</p>
              </div>
            </div>
          </div>
        </div>

        <div class="task-edit-modal__footer">
          <button class="p-button--base" type="button" @click="showCreate = false">Cancel</button>
          <button
            class="p-button--positive"
            type="button"
            :disabled="!newName || !newPrompt || submitting"
            @click="submitCreate"
          >{{ submitting ? 'Creating…' : 'Create task' }}</button>
        </div>
      </div>
    </div>

    <div v-if="showEdit" class="modal" @click.self="showEdit = false">
      <div class="task-edit-modal">
        <div class="task-edit-modal__header">
          <h3 class="task-edit-modal__title">Edit task</h3>
          <button type="button" class="p-button--base has-icon u-no-margin" aria-label="Close" @click="showEdit = false">
            <i class="p-icon--close"></i>
          </button>
        </div>

        <div class="task-edit-modal__body">
          <div class="task-edit-form">
            <div class="task-edit-field">
              <label class="task-edit-field__label" for="edit-task-name">
                Name <span class="task-edit-field__required" aria-hidden="true">*</span>
              </label>
              <input
                id="edit-task-name"
                v-model="editName"
                type="text"
                placeholder="Task name"
                autocomplete="off"
              />
            </div>

            <div class="task-edit-field">
              <label class="task-edit-field__label" for="edit-task-prompt">
                Prompt <span class="task-edit-field__required" aria-hidden="true">*</span>
              </label>
              <textarea
                id="edit-task-prompt"
                v-model="editPrompt"
                rows="8"
                class="task-prompt-input"
                placeholder="Describe what this task should do…"
              />
            </div>

            <div class="task-edit-field">
              <label class="task-edit-field__label">Dependencies</label>
              <div v-if="editDeps.length" class="task-edit-deps__chips">
                <span v-for="id in editDeps" :key="id" class="p-chip">
                  <span class="p-chip__value">{{ tasks.find(t => t.id === id)?.name ?? id }}</span>
                  <button
                    type="button"
                    class="p-chip__dismiss"
                    :aria-label="`Remove ${tasks.find(t => t.id === id)?.name ?? id}`"
                    @click="editDeps = editDeps.filter(d => d !== id)"
                  ><i class="p-icon--close"></i></button>
                </span>
              </div>
              <select class="task-edit-deps__select" @change="addEditDep($event)">
                <option value="" disabled selected>Add a dependency…</option>
                <option v-for="t in availableForEditDep" :key="t.id" :value="t.id">{{ t.name }}</option>
              </select>
            </div>

            <div v-if="editError" class="p-notification--negative">
              <div class="p-notification__content">
                <p class="p-notification__message">{{ editError }}</p>
              </div>
            </div>
          </div>
        </div>

        <div class="task-edit-modal__footer">
          <button class="p-button--base" type="button" @click="showEdit = false">Cancel</button>
          <button
            class="p-button--positive"
            type="button"
            :disabled="!editName || !editPrompt || submitting"
            @click="submitEdit"
          >{{ submitting ? 'Saving…' : 'Save changes' }}</button>
        </div>
      </div>
    </div>

    <div v-if="deletingTaskId" class="modal" @click.self="deletingTaskId = null">
      <div class="modal-content">
        <button class="modal-close" type="button" @click="deletingTaskId = null">✕</button>
        <h3>Delete task</h3>
        <p>Delete <strong>{{ tasks.find(t => t.id === deletingTaskId)?.name }}</strong>? This cannot be undone.</p>
        <div class="modal-actions">
          <button class="p-button--base" type="button" @click="deletingTaskId = null">Cancel</button>
          <button class="p-button--negative" type="button" :disabled="submitting" @click="submitDeleteTask">Delete</button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, reactive, computed, watch, onUnmounted, nextTick } from 'vue';
import { renderMarkdown } from '../lib/markdown.js';
import { renderGraph, attachGraphHandlers, updateNodeStatus } from '../lib/task-graph.js';
import { describeToolCall } from '../lib/tool-render.js';
import ToolCallStep from '../components/chat/ToolCallStep.vue';
import {
  listProjectTasks,
  createProjectTask,
  updateProjectTask,
  deleteProjectTask,
  getProjectTaskLogs,
  runProjectTaskStream,
} from '../lib/api.js';

const props = defineProps({
  projectId: { type: String, default: null },
});

const tasks        = ref([]);
const selectedTask = ref(null);
const graphEl      = ref(null);
const running      = ref(false);
const runState     = reactive({});
const taskLogs     = reactive({});
const panelExpanded = ref(false);

const showCreate  = ref(false);
const newName     = ref('');
const newPrompt   = ref('');
const newDeps     = ref([]);
const createError = ref('');

const showEdit      = ref(false);
const editingId     = ref(null);
const editName      = ref('');
const editPrompt    = ref('');
const editDeps      = ref([]);
const editError     = ref('');
const submitting    = ref(false);
const deletingTaskId = ref(null);

let pollTimer = null;

const activeRunOutput = computed(() => {
  const id = selectedTask.value?.id;
  return id ? runState[id] ?? null : null;
});

const storedOutput = computed(() => {
  const id = selectedTask.value?.id;
  return id ? taskLogs[id] ?? null : null;
});

const currentChain = computed(() => activeRunOutput.value?.chain ?? []);
const currentStatus = computed(() => activeRunOutput.value?.status ?? storedOutput.value?.status ?? null);
const currentAnswer = computed(() => activeRunOutput.value?.answer ?? storedOutput.value?.answer ?? null);

const renderedAnswer = computed(() =>
  currentAnswer.value ? renderMarkdown(currentAnswer.value) : ''
);

const availableForDep = computed(() =>
  tasks.value.filter(t => !newDeps.value.includes(t.id))
);

const availableForEditDep = computed(() =>
  tasks.value.filter(t => t.id !== editingId.value && !editDeps.value.includes(t.id))
);

async function loadTasks() {
  if (!props.projectId) return;
  try {
    tasks.value = await listProjectTasks(props.projectId);
    nextTick(renderTaskGraph);
  } catch {}
}

async function loadTaskLogs(taskId) {
  if (!props.projectId || !taskId) return;
  try {
    const data = await getProjectTaskLogs(props.projectId, taskId);
    const toolCalls = parseStoredToolCalls(data.tool_calls);
    taskLogs[taskId] = { status: data.status ?? 'idle', toolCalls, answer: data.output ?? null };
  } catch {}
}

function parseStoredToolCalls(raw) {
  if (!raw) return [];
  try {
    const arr = typeof raw === 'string' ? JSON.parse(raw) : raw;
    if (!Array.isArray(arr)) return [];
    return arr.map(tc => ({ name: tc.name, input: tc.input ?? null, status: 'done', description: describeToolCall(tc.name, tc.input ?? {}), preview: tc.preview ?? null }));
  } catch { return []; }
}

function renderTaskGraph() {
  if (!graphEl.value) return;
  if (!tasks.value.length) {
    graphEl.value.innerHTML = '<div class="tasks-graph-empty">No tasks yet.</div>';
    return;
  }
  const rs = new Map();
  for (const t of tasks.value) {
    rs.set(t.id, runState[t.id]?.status ?? taskLogs[t.id]?.status ?? t.status ?? 'idle');
  }
  renderGraph(graphEl.value, tasks.value, null, rs, selectedTask.value?.id ?? null);
  attachGraphHandlers(graphEl.value, {
    onSelect: selectTask,
    onRun:    (id) => runTask(id),
    onView:   (id) => selectTask(id),
    onEdit:   (id) => openEdit(tasks.value.find(t => t.id === id)),
  });
}

function selectTask(id) {
  const task = tasks.value.find(t => t.id === id);
  if (!task) return;
  selectedTask.value = task;
  loadTaskLogs(id);
  nextTick(renderTaskGraph);
}

function toggleExpand() { panelExpanded.value = !panelExpanded.value; }

function closePanel() {
  selectedTask.value = null;
  panelExpanded.value = false;
  nextTick(renderTaskGraph);
}

function collectRunIds(targetId) {
  const byId    = new Map(tasks.value.map(t => [t.id, t]));
  const visited = new Set();
  const stack   = [targetId];
  while (stack.length) {
    const id = stack.pop();
    if (visited.has(id)) continue;
    visited.add(id);
    const t = byId.get(id);
    if (t) for (const dep of (t.depends_on ?? [])) stack.push(dep);
  }
  return visited;
}

async function runTask(taskId) {
  if (!props.projectId || running.value) return;
  running.value = true;

  for (const id of collectRunIds(taskId)) {
    runState[id] = { status: 'waiting', chain: [], toolCalls: [], answer: null };
  }
  nextTick(renderTaskGraph);

  try {
    await runProjectTaskStream(props.projectId, taskId, (event) => {
      handleRunEvent(event);
      if (event.task_id && graphEl.value) {
        const s = runState[event.task_id]?.status;
        if (s) updateNodeStatus(graphEl.value, event.task_id, s);
      }
    });
  } catch {}
  finally {
    running.value = false;
    await loadTasks();
    if (selectedTask.value) loadTaskLogs(selectedTask.value.id);
  }
}

function handleRunEvent(event) {
  const { type, task_id } = event;
  if (type === 'run_done') return;
  if (!task_id) return;

  if (!runState[task_id]) {
    runState[task_id] = { status: 'running', chain: [], toolCalls: [], answer: null };
  }
  const state = runState[task_id];

  if (type === 'task_start')   { state.status = 'running'; return; }
  if (type === 'task_skipped') { state.status = 'skipped'; return; }

  if (type === 'thinking') {
    state.chain.push({ type: 'thinking', text: event.text });
  } else if (type === 'tool_call') {
    const tc = { type: 'tool_call', name: event.name, input: event.input ?? null, status: 'running', description: describeToolCall(event.name, event.input ?? {}), preview: null };
    state.chain.push(tc);
    state.toolCalls.push(tc);
  } else if (type === 'tool_result') {
    const tc = state.toolCalls.findLast(t => t.name === event.name && t.status === 'running')
            ?? state.toolCalls.findLast(t => t.name === event.name);
    if (tc) { tc.status = 'done'; tc.preview = event.preview ?? null; }
  } else if (type === 'done') {
    state.answer = event.answer;
    state.status = 'done';
    state.toolCalls.forEach(tc => { if (tc.status === 'running') tc.status = 'done'; });
  } else if (type === 'error') {
    state.answer = event.message;
    state.status = 'error';
  }
}

function openCreate() {
  newName.value    = '';
  newPrompt.value  = '';
  newDeps.value    = [];
  createError.value = '';
  showCreate.value = true;
}

function addDep(e) {
  const id = e.target.value;
  if (id && !newDeps.value.includes(id)) newDeps.value = [...newDeps.value, id];
  e.target.value = '';
}

async function submitCreate() {
  if (!newName.value || !newPrompt.value) return;
  submitting.value = true;
  createError.value = '';
  try {
    const task = await createProjectTask(props.projectId, {
      name: newName.value, prompt: newPrompt.value,
      depends_on: newDeps.value.length ? newDeps.value : undefined,
    });
    showCreate.value = false;
    await loadTasks();
    selectedTask.value = tasks.value.find(t => t.id === task.id) ?? null;
  } catch {
    createError.value = 'Failed to create task.';
  } finally {
    submitting.value = false;
  }
}

function openEdit(task) {
  if (!task) return;
  editingId.value  = task.id;
  editName.value   = task.name ?? '';
  editPrompt.value = task.prompt ?? '';
  editDeps.value   = [...(task.depends_on ?? [])];
  editError.value  = '';
  showEdit.value   = true;
}

function addEditDep(e) {
  const id = e.target.value;
  if (id && !editDeps.value.includes(id)) editDeps.value = [...editDeps.value, id];
  e.target.value = '';
}

async function submitEdit() {
  if (!editName.value || !editPrompt.value || !editingId.value) return;
  submitting.value = true;
  editError.value  = '';
  try {
    await updateProjectTask(props.projectId, editingId.value, {
      name: editName.value, prompt: editPrompt.value, depends_on: editDeps.value,
    });
    showEdit.value = false;
    await loadTasks();
  } catch {
    editError.value = 'Failed to save task.';
  } finally {
    submitting.value = false;
  }
}

function confirmDeleteTask(id) {
  deletingTaskId.value = id;
}

async function submitDeleteTask() {
  const id = deletingTaskId.value;
  if (!id) return;
  submitting.value = true;
  try {
    await deleteProjectTask(props.projectId, id);
    deletingTaskId.value = null;
    if (selectedTask.value?.id === id) closePanel();
    await loadTasks();
  } catch {
  } finally {
    submitting.value = false;
  }
}

function startPolling() {
  stopPolling();
  pollTimer = setInterval(async () => {
    if (running.value || !props.projectId) return;
    try {
      const fresh = await listProjectTasks(props.projectId);
      for (const updated of fresh) {
        const existing = tasks.value.find(t => t.id === updated.id);
        if (!existing || existing.status === updated.status) continue;
        existing.status = updated.status;
        if (graphEl.value) updateNodeStatus(graphEl.value, updated.id, updated.status);
        if (updated.id === selectedTask.value?.id &&
            (updated.status === 'done' || updated.status === 'error')) {
          loadTaskLogs(updated.id);
        }
      }
    } catch {}
  }, 5000);
}

function stopPolling() {
  if (pollTimer) { clearInterval(pollTimer); pollTimer = null; }
}

watch(() => props.projectId, async (id) => {
  tasks.value        = [];
  selectedTask.value = null;
  Object.keys(runState).forEach(k => delete runState[k]);
  Object.keys(taskLogs).forEach(k => delete taskLogs[k]);
  stopPolling();
  if (id) { await loadTasks(); startPolling(); }
}, { immediate: true });

onUnmounted(() => { stopPolling(); });
</script>
