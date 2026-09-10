import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';

vi.mock('monaco-editor', () => {
  let changeCb = null;
  const mockModel = { id: 'mock-model', dispose: vi.fn() };
  const mockEditor = {
    getValue: vi.fn(() => ''),
    setValue: vi.fn(),
    onDidChangeModelContent: vi.fn((cb) => { changeCb = cb; }),
    dispose: vi.fn(),
    updateOptions: vi.fn(),
    focus: vi.fn(),
    getModel: vi.fn(() => mockModel),
  };
  const editorApi = {
    create: vi.fn(() => mockEditor),
    setModelLanguage: vi.fn(),
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
    getArtifact:                   vi.fn(),
    updateArtifact:                vi.fn(),
  };
});

import ArtifactEditor from '../../src/components/deployment/ArtifactEditor.vue';
import * as api from '../../src/lib/api.js';
import * as monaco from 'monaco-editor';

const ARTIFACT = {
  id: 'a1', title: 'Infra', kind: 'terraform', content: JSON.stringify({
    'main.tf': 'resource "x" {}',
    'variables.tf': 'variable "y" {}',
  }),
};

const BASH_ARTIFACT = {
  id: 'a2', title: 'Prep', kind: 'bash', content: '#!/bin/bash\necho hello',
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
    api.updateArtifact.mockResolvedValue({ id: 'a1', title: 'Infra', kind: 'terraform', updated_at: 'now' });
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

  it('mounts the editor in read-write mode', async () => {
    await mountEditor();
    await flushPromises();
    expect(monaco.editor.create).toHaveBeenCalledWith(expect.anything(), expect.objectContaining({
      readOnly: false,
    }));
  });

  it('shows the Save button', async () => {
    const w = await mountEditor();
    await flushPromises();
    expect(w.find('[data-testid="save-artifact-btn"]').exists()).toBe(true);
  });

  it('does not show a Propose a change button', async () => {
    const w = await mountEditor();
    await flushPromises();
    expect(w.find('[data-testid="propose-artifact-btn"]').exists()).toBe(false);
  });

  it('disables the Save button when content is unchanged', async () => {
    const w = await mountEditor();
    await flushPromises();
    expect(w.find('[data-testid="save-artifact-btn"]').attributes('disabled')).toBeDefined();
  });

  it('shows file tabs for terraform bundle artifacts', async () => {
    const w = await mountEditor();
    await flushPromises();
    expect(w.find('[data-testid="artifact-editor-tabs"]').exists()).toBe(true);
    expect(w.find('[data-testid="artifact-tab-main.tf"]').exists()).toBe(true);
    expect(w.find('[data-testid="artifact-tab-variables.tf"]').exists()).toBe(true);
  });

  it('does not show file tabs for non-bundle artifacts', async () => {
    api.getArtifact.mockResolvedValue(structuredClone(BASH_ARTIFACT));
    const w = await mountEditor({ artifactId: 'a2' });
    await flushPromises();
    expect(w.find('[data-testid="artifact-editor-tabs"]').exists()).toBe(false);
  });

  it('switching tabs updates the editor content', async () => {
    const e = getMockEditor();
    e.getValue.mockReturnValue('resource "x" {}');
    const w = await mountEditor();
    await flushPromises();
    e.setValue.mockClear();
    await w.find('[data-testid="artifact-tab-variables.tf"]').trigger('click');
    await flushPromises();
    expect(e.setValue).toHaveBeenCalled();
  });

  it('enables the Save button when editor content changes', async () => {
    const e = getMockEditor();
    e.getValue.mockReturnValue('changed content');
    const w = await mountEditor();
    await flushPromises();
    getChangeCb()();
    await flushPromises();
    expect(w.find('[data-testid="save-artifact-btn"]').attributes('disabled')).toBeUndefined();
  });

  it('shows diff badge with +N -M when content is modified', async () => {
    const e = getMockEditor();
    e.getValue.mockReturnValue('changed line\nnew line');
    const w = await mountEditor();
    await flushPromises();
    getChangeCb()();
    await flushPromises();
    expect(w.find('[data-testid="diff-badge"]').exists()).toBe(true);
    expect(w.find('.artifact-editor__diff-added').exists()).toBe(true);
    expect(w.find('.artifact-editor__diff-removed').exists()).toBe(true);
  });

  it('does not show diff badge when content is unchanged', async () => {
    const w = await mountEditor();
    await flushPromises();
    expect(w.find('[data-testid="diff-badge"]').exists()).toBe(false);
  });

  it('clicking Save persists the change via updateArtifact', async () => {
    const e = getMockEditor();
    e.getValue.mockReturnValue('resource "y" {}');
    const w = await mountEditor();
    await flushPromises();
    getChangeCb()();
    await flushPromises();
    await w.find('[data-testid="save-artifact-btn"]').trigger('click');
    await flushPromises();
    expect(api.updateArtifact).toHaveBeenCalledWith('a1', expect.objectContaining({
      title: ARTIFACT.title,
      kind: ARTIFACT.kind,
    }));
  });

  it('serializes bundle files back to JSON on save', async () => {
    const e = getMockEditor();
    e.getValue.mockReturnValue('resource "z" {}');
    const w = await mountEditor();
    await flushPromises();
    getChangeCb()();
    await flushPromises();
    await w.find('[data-testid="save-artifact-btn"]').trigger('click');
    await flushPromises();
    const callArgs = api.updateArtifact.mock.calls[0][1];
    const saved = JSON.parse(callArgs.content);
    expect(saved['main.tf']).toBe('resource "z" {}');
    expect(saved['variables.tf']).toBe('variable "y" {}');
  });

  it('Ctrl+S triggers Save', async () => {
    const e = getMockEditor();
    e.getValue.mockReturnValue('changed');
    const w = await mountEditor();
    await flushPromises();
    getChangeCb()();
    await flushPromises();
    w.vm.handleKeydown({ ctrlKey: true, key: 's', preventDefault: vi.fn() });
    await flushPromises();
    expect(api.updateArtifact).toHaveBeenCalled();
  });

  it('does not save via Ctrl+S when content is unchanged', async () => {
    const e = getMockEditor();
    e.getValue.mockReturnValue('');
    const w = await mountEditor();
    await flushPromises();
    w.vm.handleKeydown({ ctrlKey: true, key: 's', preventDefault: vi.fn() });
    await flushPromises();
    expect(api.updateArtifact).not.toHaveBeenCalled();
  });

  it('disables Save again after a successful save', async () => {
    const e = getMockEditor();
    e.getValue.mockReturnValue('changed content');
    const w = await mountEditor();
    await flushPromises();
    getChangeCb()();
    await flushPromises();
    await w.find('[data-testid="save-artifact-btn"]').trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="save-artifact-btn"]').attributes('disabled')).toBeDefined();
  });

  it('shows an error message when the save fails', async () => {
    api.updateArtifact.mockRejectedValue(new Error('boom'));
    const e = getMockEditor();
    e.getValue.mockReturnValue('changed');
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
    e.getValue.mockReturnValue('changed');
    const w = await mountEditor();
    await flushPromises();
    getChangeCb()();
    await flushPromises();
    await w.find('[data-testid="save-artifact-btn"]').trigger('click');
    await flushPromises();
    expect(w.emitted('saved')).toBeTruthy();
  });
});
