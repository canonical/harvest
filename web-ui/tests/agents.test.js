import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderAgents } from '../src/agents.js';

// ── renderAgents unit tests ───────────────────────────────────────────────────

function setupDom() {
  document.body.innerHTML = `
    <table><tbody id="agents-tbody"></tbody></table>
    <p id="agents-empty" hidden></p>
  `;
}

describe('renderAgents', () => {
  beforeEach(setupDom);

  it('shows empty message when no agents', () => {
    renderAgents([]);
    expect(document.getElementById('agents-tbody').innerHTML).toBe('');
    expect(document.getElementById('agents-empty').hidden).toBe(false);
  });

  it('renders a row for each agent', () => {
    renderAgents([
      { id: 'a1', hostname: 'web-01', online: true,  last_seen: new Date().toISOString(), connected_at: new Date().toISOString() },
      { id: 'a2', hostname: 'db-01',  online: false, last_seen: new Date(Date.now() - 600_000).toISOString(), connected_at: null },
    ]);
    const rows = document.querySelectorAll('#agents-tbody tr');
    expect(rows.length).toBe(2);
  });

  it('shows Online status for online agent', () => {
    renderAgents([
      { id: 'a1', hostname: 'web-01', online: true, last_seen: new Date().toISOString(), connected_at: new Date().toISOString() },
    ]);
    expect(document.getElementById('agents-tbody').innerHTML).toContain('online');
    expect(document.getElementById('agents-tbody').innerHTML).toContain('Online');
  });

  it('shows Offline status for offline agent', () => {
    renderAgents([
      { id: 'a2', hostname: 'db-01', online: false, last_seen: new Date(Date.now() - 3_600_000).toISOString(), connected_at: null },
    ]);
    expect(document.getElementById('agents-tbody').innerHTML).toContain('offline');
    expect(document.getElementById('agents-tbody').innerHTML).toContain('Offline');
  });

  it('escapes hostname to prevent XSS', () => {
    renderAgents([
      { id: 'a1', hostname: '<script>alert(1)</script>', online: true, last_seen: new Date().toISOString(), connected_at: new Date().toISOString() },
    ]);
    expect(document.getElementById('agents-tbody').innerHTML).not.toContain('<script>');
  });

  it('hides empty message when agents present', () => {
    document.getElementById('agents-empty').hidden = false;
    renderAgents([
      { id: 'a1', hostname: 'host', online: true, last_seen: new Date().toISOString(), connected_at: new Date().toISOString() },
    ]);
    expect(document.getElementById('agents-empty').hidden).toBe(true);
  });
});

// ── Install command format tests ──────────────────────────────────────────────

describe('install command format', () => {
  it('curl command contains server url and project id', () => {
    const serverUrl = 'https://harvest.example.com';
    const projectId = 'proj-abc-123';
    const cmd = `curl -fsSL ${serverUrl}/agents/${projectId}/install.sh | bash`;
    expect(cmd).toContain(serverUrl);
    expect(cmd).toContain(projectId);
    expect(cmd).toContain('install.sh');
    expect(cmd).toContain('curl -fsSL');
  });

  it('install command pipes to bash', () => {
    const cmd = `curl -fsSL https://example.com/agents/proj-1/install.sh | bash`;
    expect(cmd).toContain('| bash');
  });
});
