import { describe, it, expect, vi, afterEach } from 'vitest';
import { openCodeSession, closeCodeSession, isOpen, suggestBranch, proposeChange, changeRequestName, indexSummary } from '../../src/lib/code-api.js';
import { classifyDiffLines } from '../../src/lib/diff-lines.js';

function respond(status, body) {
  return Promise.resolve({ ok: status < 400, status, json: () => Promise.resolve(body) });
}

afterEach(() => vi.restoreAllMocks());

describe('code api', () => {
  it('opens a session on a branch from a base branch', async () => {
    const fetch = vi.spyOn(globalThis, 'fetch').mockReturnValue(respond(201, { id: 's1' }));
    const session = await openCodeSession('p1', 'feat/x', 'main');
    expect(session.id).toBe('s1');
    const [url, options] = fetch.mock.calls[0];
    expect(url).toBe('/code/projects/p1/sessions');
    expect(JSON.parse(options.body)).toEqual({ branch: 'feat/x', base_branch: 'main' });
  });

  it('surfaces the server error message', async () => {
    vi.spyOn(globalThis, 'fetch').mockReturnValue(respond(503, { error: 'Code mode needs LXD' }));
    await expect(openCodeSession('p1', 'x', 'main')).rejects.toThrow('Code mode needs LXD');
  });

  it('treats an empty response as success', async () => {
    vi.spyOn(globalThis, 'fetch').mockReturnValue(respond(204, null));
    await expect(closeCodeSession('s1')).resolves.toBeNull();
  });

  it('considers starting and active sessions open', () => {
    expect(isOpen({ status: 'starting' })).toBe(true);
    expect(isOpen({ status: 'active' })).toBe(true);
    expect(isOpen({ status: 'closed' })).toBe(false);
    expect(isOpen(null)).toBe(false);
  });

  it('suggests a branch name from a description', () => {
    expect(suggestBranch('Fix the Login page!')).toBe('harvest/fix-the-login-page');
    expect(suggestBranch('  ')).toBe('');
  });
});

describe('change requests', () => {
  it('proposes a change with a title and description', async () => {
    const fetch = vi.spyOn(globalThis, 'fetch').mockReturnValue(respond(200, { url: 'u', state: 'open' }));
    await proposeChange('s1', 'Fix login', 'Details');
    const [url, options] = fetch.mock.calls[0];
    expect(url).toBe('/code/sessions/s1/change-request');
    expect(JSON.parse(options.body)).toEqual({ title: 'Fix login', description: 'Details' });
  });

  it('names change requests the way each platform does', () => {
    expect(changeRequestName('github')).toBe('Pull request');
    expect(changeRequestName('gitlab')).toBe('Merge request');
    expect(changeRequestName('gerrit')).toBe('Gerrit change');
    expect(changeRequestName('launchpad')).toBe('Merge proposal');
    expect(changeRequestName('other')).toBe('Change request');
  });
});

describe('diff lines', () => {
  it('classifies files, hunks, additions and deletions', () => {
    const kinds = classifyDiffLines('diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n same')
      .map((line) => line.kind);
    expect(kinds).toEqual(['diff-file', 'diff-file', 'diff-file', 'diff-hunk', 'diff-del', 'diff-add', 'diff-context']);
  });

  it('summarises the state of a project code graph', () => {
    expect(indexSummary(null)).toBe('not indexed');
    expect(indexSummary({ ingested: false })).toBe('being built');
    expect(indexSummary({ ingested: true, version: 'main', commit: '0123456789abcdef' })).toBe('main at 0123456789ab');
    expect(indexSummary({ ingested: true, version: 'main', commit: null, error: 'denied' }))
      .toBe('main; the last refresh failed: denied');
  });
});
