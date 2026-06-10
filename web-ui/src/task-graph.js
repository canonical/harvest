import { escapeHtml as esc } from './utils.js';

export function computeGraphLayout(tasks, _targetId) {
  const byId = new Map(tasks.map(t => [t.id, t]));
  const levels = new Map();

  const inDeg = new Map(tasks.map(t => [t.id, 0]));
  for (const t of tasks) {
    for (const dep of (t.depends_on ?? [])) {
      if (byId.has(dep)) inDeg.set(t.id, (inDeg.get(t.id) ?? 0) + 1);
    }
  }

  const queue = [];
  for (const [id, deg] of inDeg) {
    if (deg === 0) { queue.push(id); levels.set(id, 0); }
  }

  while (queue.length > 0) {
    const id = queue.shift();
    for (const dep of tasks.filter(x => (x.depends_on ?? []).includes(id))) {
      const newLevel = (levels.get(id) ?? 0) + 1;
      if (!levels.has(dep.id) || levels.get(dep.id) < newLevel) {
        levels.set(dep.id, newLevel);
      }
      inDeg.set(dep.id, inDeg.get(dep.id) - 1);
      if (inDeg.get(dep.id) === 0) queue.push(dep.id);
    }
  }

  const byLevel = new Map();
  for (const [id, level] of levels) {
    if (!byLevel.has(level)) byLevel.set(level, []);
    byLevel.get(level).push(id);
  }

  const maxLevel = levels.size > 0 ? Math.max(...levels.values()) : 0;
  return { levels, byLevel, maxLevel };
}

const STATUS_LABEL = {
  waiting: 'waiting',
  running: 'running',
  done:    'done',
  error:   'error',
  skipped: 'skipped',
};

export function renderGraph(container, tasks, targetId, runState = new Map(), selectedId = null) {
  container.innerHTML = '';
  if (!tasks.length) return;

  const { byLevel, maxLevel } = computeGraphLayout(tasks, targetId);
  const byId = new Map(tasks.map(t => [t.id, t]));

  const wrap = document.createElement('div');
  wrap.className = 'tg-wrap';

  const cols = [];
  for (let lvl = 0; lvl <= maxLevel; lvl++) {
    const col = document.createElement('div');
    col.className = 'tg-col';
    col.dataset.level = lvl;
    for (const id of (byLevel.get(lvl) ?? [])) {
      const t = byId.get(id);
      if (!t) continue;
      const status     = runState.get(id) ?? 'waiting';
      const isSelected = id === selectedId;
      const node       = document.createElement('div');
      node.className   = `tg-node tg-node--${status}${isSelected ? ' tg-node--selected' : ''}`;
      node.dataset.taskId = id;
      node.tabIndex       = 0;
      node.setAttribute('role', 'button');
      node.setAttribute('aria-label', `${esc(t.name)} (${status})`);
      node.innerHTML = `
        <span class="tg-node__name">${esc(t.name)}</span>
        <span class="tg-node__status">${esc(STATUS_LABEL[status] ?? status)}</span>
        <div class="tg-node__actions" aria-hidden="true">
          <button class="tg-node__action-btn tg-node__action-btn--run"
            data-action="run" data-task-id="${esc(id)}" tabindex="-1"
            title="Run this task and all its dependencies">
            <svg viewBox="0 0 10 10" width="7" height="7" fill="currentColor" aria-hidden="true">
              <polygon points="2,1 9,5 2,9"/>
            </svg>
            Run
          </button>
          <button class="tg-node__action-btn tg-node__action-btn--edit"
            data-action="edit" data-task-id="${esc(id)}" tabindex="-1"
            title="Edit this task">
            <svg viewBox="0 0 10 10" width="7" height="7" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M6.5,1.5 L8.5,3.5 L3,9 L1,9 L1,7 Z"/>
              <line x1="5.5" y1="2.5" x2="7.5" y2="4.5"/>
            </svg>
            Edit
          </button>
        </div>
      `;
      col.appendChild(node);
    }
    wrap.appendChild(col);
    cols.push(col);
  }

  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('class', 'tg-edges');
  svg.setAttribute('aria-hidden', 'true');
  wrap.appendChild(svg);

  container.appendChild(wrap);

  requestAnimationFrame(() => drawEdges(svg, wrap, tasks));
}

function drawEdges(svg, wrap, tasks) {
  const wrapRect = wrap.getBoundingClientRect();
  svg.setAttribute('width', wrapRect.width);
  svg.setAttribute('height', wrapRect.height);
  svg.innerHTML = '';

  const nodeEls = wrap.querySelectorAll('.tg-node[data-task-id]');
  const rects = new Map();
  for (const el of nodeEls) {
    const r = el.getBoundingClientRect();
    rects.set(el.dataset.taskId, {
      left:   r.left - wrapRect.left,
      right:  r.right - wrapRect.left,
      top:    r.top - wrapRect.top,
      bottom: r.bottom - wrapRect.top,
      cy:     r.top - wrapRect.top + r.height / 2,
    });
  }

  for (const task of tasks) {
    const to = rects.get(task.id);
    if (!to) continue;
    for (const depId of (task.depends_on ?? [])) {
      const from = rects.get(depId);
      if (!from) continue;
      const x1 = from.right, y1 = from.cy;
      const x2 = to.left,   y2 = to.cy;
      const cx = (x1 + x2) / 2;
      const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
      path.setAttribute('d', `M${x1},${y1} C${cx},${y1} ${cx},${y2} ${x2},${y2}`);
      path.setAttribute('class', 'tg-edge');
      svg.appendChild(path);
    }
  }
}

export function updateNodeStatus(container, taskId, status) {
  const node = container.querySelector(`.tg-node[data-task-id="${CSS.escape(taskId)}"]`);
  if (!node) return;
  node.className = node.className.replace(/tg-node--(?!selected)\S+/, `tg-node--${status}`);
  const label = node.querySelector('.tg-node__status');
  if (label) label.textContent = STATUS_LABEL[status] ?? status;
  node.setAttribute('aria-label', `${esc(node.querySelector('.tg-node__name')?.textContent ?? '')} (${status})`);
}

export function attachGraphHandlers(container, { onSelect, onRun, onEdit } = {}) {
  container.addEventListener('click', e => {
    const actionBtn = e.target.closest('.tg-node__action-btn[data-action]');
    if (actionBtn) {
      e.stopPropagation();
      const id = actionBtn.dataset.taskId;
      if (actionBtn.dataset.action === 'run')  onRun?.(id);
      if (actionBtn.dataset.action === 'edit') onEdit?.(id);
      return;
    }
    const node = e.target.closest('.tg-node[data-task-id]');
    if (!node) return;
    onSelect?.(node.dataset.taskId);
  });
  container.addEventListener('keydown', e => {
    if (e.key !== 'Enter' && e.key !== ' ') return;
    const node = e.target.closest('.tg-node[data-task-id]');
    if (!node) return;
    e.preventDefault();
    onSelect?.(node.dataset.taskId);
  });
}
