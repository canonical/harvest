import { describe, it, expect } from 'vitest';
import { imageNotices, imageStatusLabel } from '../../src/lib/code-images.js';

describe('image status label', () => {
  it('names the state sessions can rely on', () => {
    expect(imageStatusLabel({ status: 'ready' })).toBe('Ready');
    expect(imageStatusLabel({ status: 'building' })).toBe('Being prepared…');
    expect(imageStatusLabel(null)).toBe('Not prepared yet');
  });

  it('mentions a rebuild running behind a ready image', () => {
    expect(imageStatusLabel({ status: 'ready', attempt: { status: 'building' } })).toBe('Ready · rebuilding…');
  });
});

describe('image notices', () => {
  it('has nothing to say about a healthy image', () => {
    expect(imageNotices({ status: 'ready', attempt: null, setup_warning: null })).toEqual([]);
    expect(imageNotices(null)).toEqual([]);
  });

  it('reassures that sessions keep working when only a rebuild failed', () => {
    const [notice] = imageNotices({ status: 'ready', attempt: { status: 'failed', failure: 'lxd_unavailable', log: 'refused' } });
    expect(notice).toMatchObject({ tone: 'caution', title: 'The latest rebuild failed', log: 'refused' });
    expect(notice.message).toContain('could not reach its container service');
    expect(notice.message).toContain('Sessions keep using the previous containers');
  });

  it('is negative when no containers can be prepared at all', () => {
    const [notice] = imageNotices({ status: 'failed', attempt: { status: 'failed', failure: 'infrastructure', log: 'x' } });
    expect(notice).toMatchObject({ tone: 'negative', title: 'Containers could not be prepared' });
  });

  it('shows a partly failed setup with its log', () => {
    const notices = imageNotices({ status: 'ready', setup_warning: "The project's setup script failed with exit code 1.", log: 'pip: error' });
    expect(notices).toEqual([
      { tone: 'caution', title: 'Part of the setup failed', message: "The project's setup script failed with exit code 1.", log: 'pip: error' },
    ]);
  });
});
