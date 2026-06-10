import {
  createProjectTask,
  deleteProjectTask,
  getProjectTaskLogs,
  listProjectTasks,
  runProjectTaskStream,
  updateProjectTask,
} from './api.js';
import { renderMarkdown } from './markdown.js';
import { describeToolCall, renderToolCall, attachTcStepHandlers } from './tool-render.js';
import { escapeHtml as esc } from './utils.js';
import { renderGraph, updateNodeStatus, attachGraphHandlers } from './task-graph.js';

let _projectId  = null;
let _tasks      = [];
let _selectedId = null;
let _running    = false;
let _pollTimer  = null;
let _runState   = new Map();
let _taskLogs   = new Map();

export function initTasksPage(_pageEl) {
  document.getElementById('create-task-btn')?.addEventListener('click', openCreateModal);
  document.getElementById('create-task-modal-close')?.addEventListener('click', closeCreateModal);
  document.getElementById('create-task-cancel')?.addEventListener('click', closeCreateModal);
  document.getElementById('create-task-submit')?.addEventListener('click', handleCreateTask);
  document.getElementById('create-task-modal')?.addEventListener('click', e => {
    if (e.target === document.getElementById('create-task-modal')) closeCreateModal();
  });

  document.getElementById('edit-task-modal-close')?.addEventListener('click', closeEditModal);
  document.getElementById('edit-task-cancel')?.addEventListener('click', closeEditModal);
  document.getElementById('edit-task-submit')?.addEventListener('click', handleEditTask);
  document.getElementById('edit-task-modal')?.addEventListener('click', e => {
    if (e.target === document.getElementById('edit-task-modal')) closeEditModal();
  });

  document.getElementById('task-panel-close')?.addEventListener('click', closeSidePanel);
  document.getElementById('task-panel-expand')?.addEventListener('click', toggleExpandPanel);
  document.getElementById('task-panel-delete-btn')?.addEventListener('click', () => {
    if (_selectedId) handleDelete(_selectedId);
  });

  document.getElementById('task-deps-container')?.addEventListener('click', () => openDepsPanel('create'));
  document.getElementById('task-deps-input')?.addEventListener('input', e => renderDepOptions('create', e.target.value));
  document.getElementById('edit-task-deps-container')?.addEventListener('click', () => openDepsPanel('edit'));
  document.getElementById('edit-task-deps-input')?.addEventListener('input', e => renderDepOptions('edit', e.target.value));

  document.addEventListener('click', e => {
    if (_depsState.create.open && !e.target.closest('#task-deps-widget') && !e.target.closest('#task-deps-panel'))
      closeDepsPanel('create');
    if (_depsState.edit.open && !e.target.closest('#edit-task-deps-widget') && !e.target.closest('#edit-task-deps-panel'))
      closeDepsPanel('edit');
  });
  document.addEventListener('keydown', e => {
    if (e.key === 'Escape') {
      if (!document.getElementById('create-task-modal')?.hidden) closeCreateModal();
      else if (!document.getElementById('edit-task-modal')?.hidden) closeEditModal();
      else if (document.getElementById('task-detail-panel')?.classList.contains('is-open')) closeSidePanel();
      else closeDepsPanel();
    }
  });
}

export function onTasksPageShow(projectId) {
  _projectId = projectId;

  const createBtn = document.getElementById('create-task-btn');
  const noProject = document.getElementById('tasks-no-project');
  const content   = document.getElementById('tasks-content');

  if (!projectId) {
    if (createBtn) createBtn.hidden = true;
    if (noProject) noProject.hidden = false;
    if (content)   content.hidden   = true;
    stopPolling();
    return;
  }

  if (createBtn) createBtn.hidden = false;
  if (noProject) noProject.hidden = true;
  if (content)   content.hidden   = false;

  loadAndRender();
  startPolling();
}

export function onTasksPageHide() {
  stopPolling();
}

async function loadAndRender() {
  if (!_projectId) return;
  try {
    _tasks = await listProjectTasks(_projectId);
    if (_selectedId && !_tasks.find(t => t.id === _selectedId)) {
      _selectedId = null;
      _runState   = new Map();
      _taskLogs   = new Map();
    }
    if (!_running) renderPage();
  } catch {}
}

async function loadTaskLogs(taskId) {
  if (!_projectId || !taskId) return;
  try {
    const data      = await getProjectTaskLogs(_projectId, taskId);
    const toolCalls = parseStoredToolCalls(data.tool_calls);
    _taskLogs.set(taskId, {
      status:    data.status ?? 'idle',
      toolCalls,
      answer:    data.output ?? null,
    });
    if (_selectedId === taskId) renderPanelOutput(taskId);
  } catch {}
}

function parseStoredToolCalls(raw) {
  if (!raw) return [];
  try {
    const arr = typeof raw === 'string' ? JSON.parse(raw) : raw;
    if (!Array.isArray(arr)) return [];
    return arr.map(tc => ({
      name:        tc.name,
      input:       tc.input ?? null,
      status:      'done',
      description: describeToolCall(tc.name, tc.input ?? {}),
      preview:     tc.preview ?? null,
    }));
  } catch { return []; }
}

function startPolling() {
  stopPolling();
  _pollTimer = setInterval(pollStatuses, 5000);
}

function stopPolling() {
  if (_pollTimer) { clearInterval(_pollTimer); _pollTimer = null; }
}

async function pollStatuses() {
  if (!_projectId || _running) return;
  try {
    const fresh = await listProjectTasks(_projectId);

    for (const updated of fresh) {
      const existing = _tasks.find(t => t.id === updated.id);
      if (!existing) continue;
      if (existing.status !== updated.status) {
        existing.status = updated.status;

        const graphEl = document.getElementById('tasks-graph');
        if (graphEl) updateNodeStatus(graphEl, updated.id, updated.status);

        if (updated.id === _selectedId) {
          updatePanelStatusBadge(updated.status);
          if (updated.status === 'done' || updated.status === 'error') {
            loadTaskLogs(updated.id);
          }
        }
      }
    }
  } catch {}
}

function renderPage() {
  renderFullGraph();
  renderSidePanel();
}

export function renderTasks(tasks) {
  _tasks = tasks;
  renderFullGraph();
}

function renderFullGraph() {
  const graphEl = document.getElementById('tasks-graph');
  if (!graphEl) return;

  if (!_tasks.length) {
    graphEl.innerHTML = '<div class="tasks-graph-empty">No tasks yet.</div>';
    return;
  }

  const runState = new Map();
  for (const t of _tasks) {
    const live   = _runState.get(t.id);
    const stored = _taskLogs.get(t.id);
    runState.set(t.id, live?.status ?? stored?.status ?? t.status ?? 'idle');
  }

  renderGraph(graphEl, _tasks, null, runState, _selectedId);
  attachGraphHandlers(graphEl, {
    onSelect: selectTask,
    onRun:    id => handleRun(id),
    onEdit:   id => openEditModal(id),
  });
}

function renderSidePanel() {
  const panelEl = document.getElementById('task-detail-panel');
  if (!panelEl) return;

  if (!_selectedId) {
    panelEl.classList.remove('is-open');
    return;
  }

  const task = _tasks.find(t => t.id === _selectedId);
  if (!task) {
    panelEl.classList.remove('is-open');
    return;
  }

  const nameEl   = document.getElementById('task-panel-name');
  const statusEl = document.getElementById('task-panel-status');
  if (nameEl)   nameEl.textContent = task.name;
  if (statusEl) { statusEl.textContent = task.status; statusEl.className = `task-status task-status--${task.status}`; }

  renderPanelOutput(task.id);
  panelEl.classList.add('is-open');
}

function renderPanelOutput(taskId) {
  const bodyEl = document.getElementById('task-panel-body');
  if (!bodyEl) return;

  const source = _runState.get(taskId) ?? _taskLogs.get(taskId) ?? null;

  if (!source || (!source.answer && !(source.toolCalls ?? []).length)) {
    bodyEl.innerHTML = '<div class="task-panel-empty">No output yet.</div>';
    return;
  }

  const tcsHtml = buildToolCallsHtml(source.toolCalls ?? []);

  if (source.status === 'error') {
    bodyEl.innerHTML = tcsHtml + `<div class="tasks-detail-error" style="padding:1rem 1.5rem">Error: ${esc(source.answer ?? 'Unknown error')}</div>`;
  } else if (source.answer) {
    bodyEl.innerHTML = tcsHtml + `<div class="tasks-detail-answer">${renderMarkdown(source.answer)}</div>`;
  } else {
    bodyEl.innerHTML = tcsHtml;
  }

  attachTcStepHandlers(bodyEl);
}

function updatePanelStatusBadge(status) {
  const statusEl = document.getElementById('task-panel-status');
  if (!statusEl) return;
  statusEl.textContent = status;
  statusEl.className = `task-status task-status--${status}`;
}

function selectTask(taskId) {
  if (taskId === _selectedId) {
    closeSidePanel();
    return;
  }

  if (_selectedId) {
    document.querySelector(`.tg-node[data-task-id="${CSS.escape(_selectedId)}"]`)
      ?.classList.remove('tg-node--selected');
  }

  _selectedId = taskId;
  document.querySelector(`.tg-node[data-task-id="${CSS.escape(taskId)}"]`)
    ?.classList.add('tg-node--selected');

  renderSidePanel();
  if (!_running) loadTaskLogs(taskId);
}

function toggleExpandPanel() {
  const panel = document.getElementById('task-detail-panel');
  const expanded = panel?.classList.toggle('is-expanded');
  const btn = document.getElementById('task-panel-expand');
  if (!btn) return;
  btn.querySelector('i').className = expanded ? 'p-icon--chevron-right' : 'p-icon--chevron-left';
  btn.setAttribute('aria-label', expanded ? 'Shrink task panel' : 'Expand task panel');
}

function closeSidePanel() {
  if (_selectedId) {
    document.querySelector(`.tg-node[data-task-id="${CSS.escape(_selectedId)}"]`)
      ?.classList.remove('tg-node--selected');
  }
  _selectedId = null;
  const panel = document.getElementById('task-detail-panel');
  panel?.classList.remove('is-open');
  panel?.classList.remove('is-expanded');
  const btn = document.getElementById('task-panel-expand');
  if (btn) {
    btn.querySelector('i').className = 'p-icon--chevron-left';
    btn.setAttribute('aria-label', 'Expand task panel');
  }
}

function collectRunIds(targetId) {
  const byId    = new Map(_tasks.map(t => [t.id, t]));
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

async function handleRun(taskId) {
  if (!_projectId || _running) return;
  _running  = true;
  _runState = new Map();

  for (const id of collectRunIds(taskId)) {
    _runState.set(id, { status: 'waiting', toolCalls: [], answer: null });
    const t = _tasks.find(x => x.id === id);
    if (t) t.status = 'waiting';
  }
  renderPage();
  await new Promise(resolve => setTimeout(resolve, 0));

  try {
    await runProjectTaskStream(_projectId, taskId, event => {
      handleRunEvent(event);

      if (event.task_id) {
        const graphEl = document.getElementById('tasks-graph');
        const s = _runState.get(event.task_id)?.status;
        if (graphEl && s) updateNodeStatus(graphEl, event.task_id, s);
        if (event.task_id === _selectedId) updatePanelStatusBadge(s);
      }

      if (_selectedId && (event.task_id === _selectedId || event.type === 'run_done')) {
        renderPanelOutput(_selectedId);
      }
    });
  } catch {
    renderPage();
  } finally {
    _running = false;
    await loadAndRender();
    if (_selectedId) loadTaskLogs(_selectedId);
  }
}

function handleRunEvent(event) {
  const { type, task_id } = event;

  if (type === 'task_start') {
    _runState.set(task_id, { status: 'running', toolCalls: [], answer: null });
    const t = _tasks.find(x => x.id === task_id);
    if (t) t.status = 'running';
    return;
  }
  if (type === 'task_skipped') {
    _runState.set(task_id, { status: 'skipped', toolCalls: [], answer: null });
    const t = _tasks.find(x => x.id === task_id);
    if (t) t.status = 'skipped';
    return;
  }
  if (type === 'run_done') return;
  if (!task_id) return;

  const state = _runState.get(task_id) ?? { status: 'running', toolCalls: [], answer: null };

  if (type === 'tool_call') {
    state.toolCalls.push({
      name:        event.name,
      input:       event.input ?? null,
      status:      'running',
      description: describeToolCall(event.name, event.input ?? {}),
      preview:     null,
    });
  } else if (type === 'tool_result') {
    const tc = state.toolCalls.findLast(t => t.name === event.name && t.status === 'running')
            ?? state.toolCalls.findLast(t => t.name === event.name);
    if (tc) { tc.status = 'done'; tc.preview = event.preview ?? null; }
  } else if (type === 'done') {
    state.answer = event.answer;
    state.status = 'done';
    state.toolCalls.forEach(tc => { if (tc.status === 'running') tc.status = 'done'; });
    const t = _tasks.find(x => x.id === task_id);
    if (t) t.status = 'done';
  } else if (type === 'error') {
    state.answer = event.message;
    state.status = 'error';
    const t = _tasks.find(x => x.id === task_id);
    if (t) t.status = 'error';
  }

  _runState.set(task_id, state);
}


async function handleDelete(taskId) {
  if (!_projectId) return;
  try {
    await deleteProjectTask(_projectId, taskId);
    if (_selectedId === taskId) {
      closeSidePanel();
      _runState = new Map();
      _taskLogs = new Map();
    }
    await loadAndRender();
  } catch {}
}


function openCreateModal() {
  document.getElementById('task-name-input').value   = '';
  document.getElementById('task-prompt-input').value = '';
  document.getElementById('create-task-error').textContent = '';
  _depsState.create.selected = new Set();
  closeDepsPanel('create');
  renderDepsContainer('create');
  document.getElementById('create-task-modal').hidden = false;
  document.getElementById('task-name-input')?.focus();
}

function closeCreateModal() {
  document.getElementById('create-task-modal').hidden = true;
  closeDepsPanel('create');
}

async function handleCreateTask() {
  if (!_projectId) return;
  const name   = document.getElementById('task-name-input')?.value.trim()   ?? '';
  const prompt = document.getElementById('task-prompt-input')?.value.trim() ?? '';
  const errEl  = document.getElementById('create-task-error');
  if (!name)   { if (errEl) errEl.textContent = 'Name is required.';   return; }
  if (!prompt) { if (errEl) errEl.textContent = 'Prompt is required.'; return; }

  const submitBtn = document.getElementById('create-task-submit');
  if (submitBtn) submitBtn.disabled = true;
  try {
    const sel        = _depsState.create.selected;
    const depends_on = sel.size > 0 ? [...sel] : undefined;
    const task = await createProjectTask(_projectId, { name, prompt, depends_on });
    closeCreateModal();
    _selectedId = task.id;
    _runState   = new Map();
    _taskLogs   = new Map();
    await loadAndRender();
  } catch {
    if (errEl) errEl.textContent = 'Failed to create task.';
  } finally {
    if (submitBtn) submitBtn.disabled = false;
  }
}


let _editingTaskId = null;

function openEditModal(taskId) {
  const task = _tasks.find(t => t.id === taskId);
  if (!task) return;
  _editingTaskId = taskId;
  document.getElementById('edit-task-name-input').value   = task.name   ?? '';
  document.getElementById('edit-task-prompt-input').value = task.prompt ?? '';
  document.getElementById('edit-task-error').textContent  = '';
  _depsState.edit.selected = new Set(task.depends_on ?? []);
  closeDepsPanel('edit');
  renderDepsContainer('edit');
  document.getElementById('edit-task-modal').hidden = false;
  document.getElementById('edit-task-name-input')?.focus();
}

function closeEditModal() {
  document.getElementById('edit-task-modal').hidden = true;
  closeDepsPanel('edit');
  _editingTaskId = null;
}

async function handleEditTask() {
  if (!_projectId || !_editingTaskId) return;
  const name   = document.getElementById('edit-task-name-input')?.value.trim()   ?? '';
  const prompt = document.getElementById('edit-task-prompt-input')?.value.trim() ?? '';
  const errEl  = document.getElementById('edit-task-error');
  if (!name)   { if (errEl) errEl.textContent = 'Name is required.';   return; }
  if (!prompt) { if (errEl) errEl.textContent = 'Prompt is required.'; return; }

  const submitBtn = document.getElementById('edit-task-submit');
  if (submitBtn) submitBtn.disabled = true;
  try {
    const depends_on = [..._depsState.edit.selected];
    await updateProjectTask(_projectId, _editingTaskId, { name, prompt, depends_on });
    closeEditModal();
    await loadAndRender();
  } catch {
    if (errEl) errEl.textContent = 'Failed to save task.';
  } finally {
    if (submitBtn) submitBtn.disabled = false;
  }
}


const _depsState = {
  create: { selected: new Set(), open: false },
  edit:   { selected: new Set(), open: false },
};

function _depsIds(ctx) {
  return {
    container: ctx === 'create' ? 'task-deps-container'     : 'edit-task-deps-container',
    panel:     ctx === 'create' ? 'task-deps-panel'         : 'edit-task-deps-panel',
    input:     ctx === 'create' ? 'task-deps-input'         : 'edit-task-deps-input',
    options:   ctx === 'create' ? 'task-deps-options'       : 'edit-task-deps-options',
  };
}

function openDepsPanel(ctx) {
  const ids       = _depsIds(ctx);
  const container = document.getElementById(ids.container);
  const panel     = document.getElementById(ids.panel);
  const input     = document.getElementById(ids.input);
  if (!container || !panel) return;

  const rect = container.getBoundingClientRect();
  const w    = Math.max(rect.width, 280);
  let   left = rect.left;
  let   top  = rect.bottom;
  if (left + w > window.innerWidth - 8) left = Math.max(8, window.innerWidth - w - 8);

  panel.style.top   = `${Math.round(top)}px`;
  panel.style.left  = `${Math.round(left)}px`;
  panel.style.width = `${Math.round(w)}px`;
  panel.setAttribute('aria-hidden', 'false');
  container.setAttribute('aria-expanded', 'true');
  _depsState[ctx].open = true;

  if (input) input.value = '';
  renderDepOptions(ctx, '');
  requestAnimationFrame(() => input?.focus());
}

function closeDepsPanel(ctx) {
  if (!ctx) { closeDepsPanel('create'); closeDepsPanel('edit'); return; }
  const ids = _depsIds(ctx);
  document.getElementById(ids.panel)?.setAttribute('aria-hidden', 'true');
  document.getElementById(ids.container)?.setAttribute('aria-expanded', 'false');
  _depsState[ctx].open = false;
}

function renderDepOptions(ctx, query) {
  const ids    = _depsIds(ctx);
  const opts   = document.getElementById(ids.options);
  const sel    = _depsState[ctx].selected;
  const editId = ctx === 'edit' ? _editingTaskId : null;
  if (!opts) return;

  const q        = query.trim().toLowerCase();
  const filtered = _tasks
    .filter(t => t.id !== editId)
    .filter(t => !q || t.name.toLowerCase().includes(q));

  if (!filtered.length) {
    opts.innerHTML = `<p class="u-text--muted" style="padding:0.5rem 0.75rem;font-size:0.875rem;">${q ? 'No tasks match.' : 'No other tasks yet.'}</p>`;
    return;
  }

  opts.innerHTML = filtered.map(t => {
    const selected = sel.has(t.id);
    return `<button type="button" class="p-chip${selected ? ' p-chip--positive' : ''}"
      data-dep-id="${esc(t.id)}" aria-pressed="${selected}">
      <span class="p-chip__value">${esc(t.name)}</span>
    </button>`;
  }).join('');

  opts.querySelectorAll('.p-chip[data-dep-id]').forEach(chip => {
    chip.addEventListener('click', () => {
      const id = chip.dataset.depId;
      if (sel.has(id)) sel.delete(id); else sel.add(id);
      renderDepsContainer(ctx);
      renderDepOptions(ctx, document.getElementById(ids.input)?.value ?? '');
    });
  });
}

function renderDepsContainer(ctx) {
  const ids       = _depsIds(ctx);
  const container = document.getElementById(ids.container);
  const input     = document.getElementById(ids.input);
  if (!container) return;

  container.querySelectorAll('.p-chip').forEach(c => c.remove());

  const sel = [..._depsState[ctx].selected];
  if (!sel.length) {
    container.dataset.empty = 'true';
    if (input) input.placeholder = 'Add dependencies…';
    return;
  }

  container.dataset.empty = 'false';
  if (input) input.placeholder = '';

  const form = container.querySelector('.p-search-and-filter__box');
  for (const id of sel) {
    const task = _tasks.find(t => t.id === id);
    if (!task) continue;
    const chip = document.createElement('button');
    chip.type      = 'button';
    chip.className = 'p-chip';
    chip.dataset.depId = id;
    chip.innerHTML = `<span class="p-chip__value">${esc(task.name)}</span>`;
    chip.addEventListener('click', e => {
      e.stopPropagation();
      _depsState[ctx].selected.delete(id);
      renderDepsContainer(ctx);
      if (_depsState[ctx].open) renderDepOptions(ctx, document.getElementById(ids.input)?.value ?? '');
    });
    container.insertBefore(chip, form);
  }
}


function buildToolCallsHtml(tcs) {
  if (!tcs.length) return '';
  const n          = tcs.length;
  const anyRunning = tcs.some(tc => tc.status === 'running');
  return `
    <div class="tasks-detail-tool-calls">
      <details class="tc-group${anyRunning ? ' tc-group--running' : ''}" open>
        <summary class="tc-group__summary">
          <svg class="tc-group__summary-chevron" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="2,3 5,7 8,3"/></svg>
          <span>${n} tool call${n === 1 ? '' : 's'}</span>
        </summary>
        ${tcs.map((tc, i) => renderToolCall(tc, i)).join('')}
      </details>
    </div>
  `;
}
