import { describe, it, expect, vi, afterEach } from 'vitest';
import { terminalSocketUrl, listCodeSessions, listRepositoryTree, getRepositoryFile, repositoryRawUrl, listRepositoryVersions, openCodeSession, closeCodeSession, isOpen, suggestBranch, proposeChange, changeRequestName, indexSummary } from '../../src/lib/code-api.js';
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

  it('lists the sessions of the signed-in user', async () => {
    const fetch = vi.spyOn(globalThis, 'fetch').mockReturnValue(respond(200, [{ id: 's1' }]));
    expect(await listCodeSessions()).toEqual([{ id: 's1' }]);
    expect(fetch.mock.calls[0][0]).toBe('/code/sessions');
  });

  it('addresses repository resources by name, version and path', async () => {
    const fetch = vi.spyOn(globalThis, 'fetch').mockReturnValue(respond(200, []));
    await listRepositoryVersions('my repo');
    await listRepositoryTree('flask', 'main', '');
    await getRepositoryFile('flask', '2.0/x', 'src/a b.py');
    expect(fetch.mock.calls.map(([url]) => url)).toEqual([
      '/code/repositories/my%20repo/versions',
      '/code/repositories/flask/tree?version=main',
      '/code/repositories/flask/file?version=2.0%2Fx&path=src%2Fa+b.py',
    ]);
    expect(repositoryRawUrl('flask', 'main', 'logo.png')).toBe('/code/repositories/flask/raw?version=main&path=logo.png');
  });

  it('opens terminal sockets on the same host with the matching scheme', () => {
    expect(terminalSocketUrl('s1', 't1', { cols: 80, rows: 24 }, { protocol: 'http:', host: '10.0.0.1' }))
      .toBe('ws://10.0.0.1/code/sessions/s1/terminals/t1/socket?cols=80&rows=24');
    expect(terminalSocketUrl('s1', 't1', { cols: 120, rows: 40 }, { protocol: 'https:', host: 'harvest.example.com' }))
      .toBe('wss://harvest.example.com/code/sessions/s1/terminals/t1/socket?cols=120&rows=40');
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
