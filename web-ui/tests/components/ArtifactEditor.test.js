import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';

vi.mock('monaco-editor', () => {
  let changeCb = null;
  const mockEditor = {
    getValue: vi.fn(() => ''),
    setValue: vi.fn(),
    onDidChangeModelContent: vi.fn((cb) => { changeCb = cb; }),
    dispose: vi.fn(),
    updateOptions: vi.fn(),
  };
  const editorApi = {
    create: vi.fn(() => mockEditor),
  };
  return {
    default: { editor: editorApi },
    editor: editorApi,
    __mockEditor: mockEditor,
    __getChangeCb: () => changeCb,
    __resetChangeCb: () => { changeCb = null; },
  };
});

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    getArtifact:           vi.fn(),
    proposeArtifactChange: vi.fn(),
  };
});

import ArtifactEditor from '../../src/components/deployment/ArtifactEditor.vue';
import * as api from '../../src/lib/api.js';
import * as monaco from 'monaco-editor';

const ARTIFACT = {
  id: 'a1', title: 'Infra', kind: 'terraform', content: '{"main.tf":"resource \"x\" {}"}',
};

async function mountEditor({ projectId = 'proj-1', deploymentId = 'd1', artifactId = 'a1' } = {}) {
  return mount(ArtifactEditor, {
    props: { projectId, deploymentId, artifactId },
  });
}

function getMockEditor() {
  return monaco.__mockEditor;
}

function getChangeCb() {
  return monaco.__getChangeCb();
}

describe('ArtifactEditor', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    const e = getMockEditor();
    e.getValue.mockReturnValue('');
    api.getArtifact.mockResolvedValue(structuredClone(ARTIFACT));
    api.proposeArtifactChange.mockResolvedValue({ id: 'p1', status: 'pending' });
  });

  it('fetches the artifact on mount', async () => {
    await mountEditor();
    await flushPromises();
    expect(api.getArtifact).toHaveBeenCalledWith('a1');
  });

  it('shows the artifact title and kind badge', async () => {
    const w = await mountEditor();
    await flushPromises();
    expect(w.text()).toContain('Infra');
    expect(w.find('.artifact-kind-badge').exists()).toBe(true);
  });

  it('shows an empty state when no artifact is selected', async () => {
    const w = await mountEditor({ artifactId: null });
    await flushPromises();
    expect(w.find('[data-testid="artifact-editor-empty"]').exists()).toBe(true);
  });

  it('shows a loading indicator while fetching', async () => {
    let resolveFn;
    api.getArtifact.mockReturnValue(new Promise(r => { resolveFn = r; }));
    const w = await mountEditor();
    await flushPromises();
    expect(w.find('[data-testid="artifact-editor-loading"]').exists()).toBe(true);
    resolveFn(structuredClone(ARTIFACT));
    await flushPromises();
  });

  it('hides the Save button when the content is unchanged', async () => {
    const w = await mountEditor();
    await flushPromises();
    expect(w.find('[data-testid="save-artifact-btn"]').exists()).toBe(false);
  });

  it('shows the Save button when the editor content changes', async () => {
    const e = getMockEditor();
    e.getValue.mockReturnValue('{"main.tf":"resource \"y\" {}"}');
    const w = await mountEditor();
    await flushPromises();
    getChangeCb()();
    await flushPromises();
    expect(w.find('[data-testid="save-artifact-btn"]').exists()).toBe(true);
  });

  it('clicking Save persists the change as a proposal via proposeArtifactChange', async () => {
    const e = getMockEditor();
    const newContent = '{"main.tf":"resource \"y\" {}"}';
    e.getValue.mockReturnValue(newContent);
    const w = await mountEditor();
    await flushPromises();
    getChangeCb()();
    await flushPromises();
    await w.find('[data-testid="save-artifact-btn"]').trigger('click');
    await flushPromises();
    expect(api.proposeArtifactChange).toHaveBeenCalledWith('proj-1', 'd1', expect.objectContaining({
      artifact_id:      'a1',
      current_content:  ARTIFACT.content,
      proposed_content: newContent,
    }));
  });

  it('Ctrl+S triggers Save without clicking the button', async () => {
    const e = getMockEditor();
    const newContent = '{"main.tf":"resource \"y\" {}"}';
    e.getValue.mockReturnValue(newContent);
    const w = await mountEditor();
    await flushPromises();
    getChangeCb()();
    await flushPromises();
    w.vm.handleKeydown({ ctrlKey: true, key: 's', preventDefault: vi.fn() });
    await flushPromises();
    expect(api.proposeArtifactChange).toHaveBeenCalledWith('proj-1', 'd1', expect.objectContaining({
      artifact_id: 'a1',
      proposed_content: newContent,
    }));
  });

  it('does not save when content is unchanged and Ctrl+S is pressed', async () => {
    const e = getMockEditor();
    e.getValue.mockReturnValue(ARTIFACT.content);
    const w = await mountEditor();
    await flushPromises();
    w.vm.handleKeydown({ ctrlKey: true, key: 's', preventDefault: vi.fn() });
    await flushPromises();
    expect(api.proposeArtifactChange).not.toHaveBeenCalled();
  });

  it('clears the dirty state after a successful save', async () => {
    const e = getMockEditor();
    e.getValue.mockReturnValue('{"main.tf":"changed"}');
    const w = await mountEditor();
    await flushPromises();
    getChangeCb()();
    await flushPromises();
    expect(w.find('[data-testid="save-artifact-btn"]').exists()).toBe(true);
    await w.find('[data-testid="save-artifact-btn"]').trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="save-artifact-btn"]').exists()).toBe(false);
  });

  it('shows an error message when the save fails', async () => {
    api.proposeArtifactChange.mockRejectedValue(new Error('boom'));
    const e = getMockEditor();
    e.getValue.mockReturnValue('{"main.tf":"changed"}');
    const w = await mountEditor();
    await flushPromises();
    getChangeCb()();
    await flushPromises();
    await w.find('[data-testid="save-artifact-btn"]').trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="artifact-editor-error"]').text()).toContain('boom');
  });

  it('emits saved after a successful save', async () => {
    const e = getMockEditor();
    e.getValue.mockReturnValue('{"main.tf":"changed"}');
    const w = await mountEditor();
    await flushPromises();
    getChangeCb()();
    await flushPromises();
    await w.find('[data-testid="save-artifact-btn"]').trigger('click');
    await flushPromises();
    expect(w.emitted('saved')).toBeTruthy();
  });
});
