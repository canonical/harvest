import { describe, it, expect } from 'vitest';
import { runningVerb, describeToolCall } from '../../src/lib/tool-verbs.js';

describe('runningVerb', () => {
  it('returns Creating… for create_lxd_agent', () => {
    expect(runningVerb('create_lxd_agent')).toBe('Creating…');
  });

  it('returns Deleting… for delete_agent', () => {
    expect(runningVerb('delete_agent')).toBe('Deleting…');
  });

  it('returns Creating… for create_port_forward', () => {
    expect(runningVerb('create_port_forward')).toBe('Creating…');
  });

  it('returns Updating… for update_port_forward', () => {
    expect(runningVerb('update_port_forward')).toBe('Updating…');
  });

  it('returns Deleting… for delete_port_forward', () => {
    expect(runningVerb('delete_port_forward')).toBe('Deleting…');
  });

  it('returns a generic fallback for an unrecognised tool name', () => {
    expect(runningVerb('some_future_tool')).toBe('Working…');
  });
});

describe('describeToolCall', () => {
  it('names the repository, short version and query of a code search', () => {
    expect(describeToolCall('search_code', { repo: 'cinder', version: 'unmaintained/2024.1', query: 'volume drivers' }))
      .toBe('Searching cinder 2024.1 for “volume drivers”');
  });

  it('describes a focused code search as reading the named targets', () => {
    expect(describeToolCall('search_code', { repo: 'cinder', version: 'v1', focus: ['Fence::commit', 'src/jobs/fence.rs'] }))
      .toBe('Reading Fence::commit, fence.rs');
  });

  it('truncates long queries', () => {
    const label = describeToolCall('search_code', { repo: 'r', query: 'x'.repeat(200) });
    expect(label.length).toBeLessThan(90);
    expect(label).toContain('…');
  });

  it('lists file base names and summarises long lists when reading sources', () => {
    expect(describeToolCall('read_sources', { files: ['doc/source/reference/support-matrix.ini'] }))
      .toBe('Reading support-matrix.ini');
    expect(describeToolCall('read_sources', { files: ['a.py', 'b.py', 'c.py', 'd.py', 'e.py'] }))
      .toBe('Reading a.py, b.py, c.py and 2 more');
  });

  it('describes graph tools by their target', () => {
    expect(describeToolCall('find_subclasses', { class: 'BaseVD' })).toBe('Listing subclasses of BaseVD');
    expect(describeToolCall('find_callers', { function_name: 'attach_volume' })).toBe('Finding callers of attach_volume');
    expect(describeToolCall('list_repositories', {})).toBe('Listing indexed repositories');
  });

  it('falls back to the tool name in words for unknown tools', () => {
    expect(describeToolCall('collocate_exec', {})).toBe('Running collocate exec');
    expect(describeToolCall('some_tool')).toBe('Running some tool');
  });

  it('tolerates a missing input', () => {
    expect(describeToolCall('read_sources', null)).toBe('Reading source code');
  });
});
