import { describe, it, expect, beforeEach } from 'vitest';
import { renderTasks } from '../src/tasks.js';

function setupDom() {
  document.body.innerHTML = `<div id="tasks-graph"></div>`;
}

describe('renderTasks', () => {
  beforeEach(setupDom);

  it('shows empty message when no tasks', () => {
    renderTasks([]);
    expect(document.getElementById('tasks-graph').innerHTML).toContain('tasks-graph-empty');
  });

  it('renders a graph node for each task', () => {
    renderTasks([
      { id: 't1', name: 'Health check', status: 'idle' },
      { id: 't2', name: 'Deploy check', status: 'done' },
    ]);
    const nodes = document.querySelectorAll('#tasks-graph .tg-node');
    expect(nodes.length).toBe(2);
  });

  it('does not show empty message when tasks are present', () => {
    renderTasks([
      { id: 't1', name: 'Task', status: 'idle' },
    ]);
    expect(document.getElementById('tasks-graph').innerHTML).not.toContain('tasks-graph-empty');
  });

  it('shows task name in each node', () => {
    renderTasks([
      { id: 't1', name: 'My task', status: 'idle' },
    ]);
    expect(document.getElementById('tasks-graph').innerHTML).toContain('My task');
  });

  it('renders node with the correct status class', () => {
    renderTasks([
      { id: 't1', name: 'Task A', status: 'running' },
    ]);
    const node = document.querySelector('#tasks-graph .tg-node');
    expect(node.className).toContain('tg-node--running');
  });

  it('renders done status class', () => {
    renderTasks([
      { id: 't1', name: 'Task A', status: 'done' },
    ]);
    const node = document.querySelector('#tasks-graph .tg-node');
    expect(node.className).toContain('tg-node--done');
  });

  it('escapes task name to prevent XSS', () => {
    renderTasks([
      { id: 't1', name: '<script>alert(1)</script>', status: 'idle' },
    ]);
    expect(document.getElementById('tasks-graph').innerHTML).not.toContain('<script>');
  });

  it('attaches task id to each node', () => {
    renderTasks([
      { id: 'task-abc', name: 'My task', status: 'idle' },
    ]);
    const node = document.querySelector('#tasks-graph .tg-node');
    expect(node.dataset.taskId).toBe('task-abc');
  });
});
