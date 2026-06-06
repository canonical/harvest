import { describe, it, expect, vi, beforeEach } from 'vitest';
import {
  renderOverview,
  formatLastUpdated,
  initOverviewPage,
} from '../src/overview.js';

// ── DOM setup ─────────────────────────────────────────────────────────────────

function setupDom() {
  document.body.innerHTML = `
    <section id="page-overview">
      <div id="overview-no-project" hidden></div>
      <div id="overview-empty" hidden></div>
      <div id="overview-content" hidden>
        <div id="overview-body"></div>
        <div id="overview-footer">
          <span id="overview-last-updated"></span>
          <button id="overview-regenerate-btn">Regenerate</button>
        </div>
      </div>
      <div id="overview-loading" hidden></div>
    </section>
  `;
}

// ── renderOverview tests ──────────────────────────────────────────────────────

describe('renderOverview', () => {
  beforeEach(setupDom);

  it('shows empty state when current_status is null and has_conversations is false', () => {
    renderOverview({ current_status: null, current_status_updated_at: null, has_conversations: false });
    expect(document.getElementById('overview-empty').hidden).toBe(false);
    expect(document.getElementById('overview-content').hidden).toBe(true);
  });

  it('shows empty state with start-chatting message when no conversations', () => {
    renderOverview({ current_status: null, current_status_updated_at: null, has_conversations: false });
    const empty = document.getElementById('overview-empty');
    expect(empty.textContent).toContain('chat');
  });

  it('shows empty state with generate hint when conversations exist but no overview yet', () => {
    renderOverview({ current_status: null, current_status_updated_at: null, has_conversations: true });
    const empty = document.getElementById('overview-empty');
    expect(empty.hidden).toBe(false);
    expect(document.getElementById('overview-content').hidden).toBe(true);
  });

  it('shows content when current_status is present', () => {
    renderOverview({
      current_status: '# Status\n- nginx: running',
      current_status_updated_at: '2026-06-05T10:00:00Z',
      has_conversations: true,
    });
    expect(document.getElementById('overview-content').hidden).toBe(false);
    expect(document.getElementById('overview-empty').hidden).toBe(true);
  });

  it('renders markdown in overview body', () => {
    renderOverview({
      current_status: '# My Status\n- item one',
      current_status_updated_at: '2026-06-05T10:00:00Z',
      has_conversations: true,
    });
    const body = document.getElementById('overview-body').innerHTML;
    expect(body).toContain('<h1');
    expect(body).toContain('My Status');
  });

  it('displays last updated timestamp when content is present', () => {
    renderOverview({
      current_status: '# Status',
      current_status_updated_at: '2026-06-05T10:00:00Z',
      has_conversations: true,
    });
    const el = document.getElementById('overview-last-updated');
    expect(el.textContent).not.toBe('');
  });

  it('hides content and shows empty state when status becomes null', () => {
    // First render with content
    renderOverview({
      current_status: '# Status',
      current_status_updated_at: '2026-06-05T10:00:00Z',
      has_conversations: true,
    });
    // Then render without content
    renderOverview({ current_status: null, current_status_updated_at: null, has_conversations: false });
    expect(document.getElementById('overview-content').hidden).toBe(true);
    expect(document.getElementById('overview-empty').hidden).toBe(false);
  });

  it('escapes XSS in markdown rendering context', () => {
    renderOverview({
      current_status: '# Status\n<script>alert(1)</script>',
      current_status_updated_at: '2026-06-05T10:00:00Z',
      has_conversations: true,
    });
    const body = document.getElementById('overview-body').innerHTML;
    expect(body).not.toContain('<script>alert(1)</script>');
  });
});

// ── formatLastUpdated tests ───────────────────────────────────────────────────

describe('formatLastUpdated', () => {
  it('returns empty string for null', () => {
    expect(formatLastUpdated(null)).toBe('');
  });

  it('returns a non-empty string for a valid ISO date', () => {
    const result = formatLastUpdated('2026-06-05T10:00:00Z');
    expect(result).toBeTruthy();
    expect(typeof result).toBe('string');
  });

  it('includes "Last updated" prefix', () => {
    const result = formatLastUpdated('2026-06-05T10:00:00Z');
    expect(result.toLowerCase()).toContain('last updated');
  });
});

// ── initOverviewPage tests ────────────────────────────────────────────────────

describe('initOverviewPage', () => {
  beforeEach(setupDom);

  it('sets up regenerate button without throwing', () => {
    expect(() => initOverviewPage()).not.toThrow();
  });

  it('regenerate button exists in DOM after init', () => {
    initOverviewPage();
    expect(document.getElementById('overview-regenerate-btn')).not.toBeNull();
  });
});

// ── no-project state ──────────────────────────────────────────────────────────

describe('showOverviewNoProject', () => {
  beforeEach(setupDom);

  it('can import showOverviewNoProject without error', async () => {
    const mod = await import('../src/overview.js');
    expect(typeof mod.showOverviewNoProject).toBe('function');
  });

  it('shows no-project element when called', async () => {
    const { showOverviewNoProject } = await import('../src/overview.js');
    showOverviewNoProject();
    expect(document.getElementById('overview-no-project').hidden).toBe(false);
    expect(document.getElementById('overview-content').hidden).toBe(true);
    expect(document.getElementById('overview-empty').hidden).toBe(true);
  });
});
