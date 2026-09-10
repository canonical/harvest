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
  const mockModifiedEditor = {
    getValue: vi.fn(() => ''),
    dispose: vi.fn(),
  };
  const mockDiffEditor = {
    dispose: vi.fn(),
    setModel: vi.fn(),
    getModifiedEditor: vi.fn(() => mockModifiedEditor),
  };
  const createdModels = [];
  const editorApi = {
    create: vi.fn(() => mockEditor),
    createDiffEditor: vi.fn(() => mockDiffEditor),
    setModelLanguage: vi.fn(),
    createModel: vi.fn((value) => {
      const m = { dispose: vi.fn(), getValue: vi.fn(() => value || '') };
      createdModels.push(m);
      return m;
    }),
  };
  return {
    default: { editor: editorApi },
    editor: editorApi,
    __mockEditor: mockEditor,
    __mockDiffEditor: mockDiffEditor,
    __mockModifiedEditor: mockModifiedEditor,
    __createdModels: createdModels,
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
    proposeProvisionChangeStream:  vi.fn(),
    applyProvisionChange:          vi.fn(),
  };
});

vi.mock('../../src/lib/markdown.js', () => ({
  renderMarkdown: vi.fn(() => '<p>explanation</p>'),
}));

vi.mock('../../src/components/deployment/DesignGenerationPanel.vue', () => ({
  default: {
    name: 'DesignGenerationPanel',
    template: '<div data-testid="design-gen-panel" />',
    props: ['projectId', 'deploymentId', 'streamFn', 'body', 'preparingText', 'readyText', 'failedText'],
    emits: ['done', 'cancel'],
  },
}));

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

const PROPOSAL_ANSWER = 'I changed the resource type\n\n```json\n{"main.tf": "resource \\"z\\" {}", "variables.tf": "variable \\"y\\" {}"}\n```';

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

async function startProposal(w, prompt = 'Change something') {
  await w.find('[data-testid="propose-artifact-btn"]').trigger('click');
  await flushPromises();
  await w.find('[data-testid="artifact-propose-prompt"]').setValue(prompt);
  await w.find('[data-testid="submit-propose-artifact-btn"]').trigger('click');
  await flushPromises();
}

async function emitProposalDone(w, answer = PROPOSAL_ANSWER) {
  w.findComponent({ name: 'DesignGenerationPanel' }).vm.$emit('done', { answer, text: answer });
  await flushPromises();
}

describe('ArtifactEditor', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    monaco.__createdModels.length = 0;
    const e = getMockEditor();
    e.getValue.mockReturnValue('');
    api.getArtifact.mockResolvedValue(structuredClone(ARTIFACT));
    api.updateArtifact.mockResolvedValue({ id: 'a1', title: 'Infra', kind: 'terraform', updated_at: 'now' });
    api.applyProvisionChange.mockResolvedValue({});
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

  it('shows Save and Propose a change buttons', async () => {
    const w = await mountEditor();
    await flushPromises();
    expect(w.find('[data-testid="save-artifact-btn"]').exists()).toBe(true);
    expect(w.find('[data-testid="propose-artifact-btn"]').exists()).toBe(true);
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

  it('opens the propose modal when Propose a change is clicked', async () => {
    const w = await mountEditor();
    await flushPromises();
    await w.find('[data-testid="propose-artifact-btn"]').trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="artifact-propose-modal"]').exists()).toBe(true);
  });

  it('disables the Propose button when the prompt is empty', async () => {
    const w = await mountEditor();
    await flushPromises();
    await w.find('[data-testid="propose-artifact-btn"]').trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="submit-propose-artifact-btn"]').attributes('disabled')).toBeDefined();
  });

  it('shows streaming generation panel when proposing', async () => {
    const w = await mountEditor();
    await flushPromises();
    await startProposal(w);
    expect(w.find('[data-testid="design-gen-panel"]').exists()).toBe(true);
    expect(w.find('[data-testid="save-artifact-btn"]').exists()).toBe(false);
    expect(w.find('[data-testid="propose-artifact-btn"]').exists()).toBe(false);
  });

  it('passes the stream fn and body to the generation panel', async () => {
    const w = await mountEditor();
    await flushPromises();
    await startProposal(w, 'Increase instance count');
    const panel = w.findComponent({ name: 'DesignGenerationPanel' });
    expect(typeof panel.props('streamFn')).toBe('function');
    expect(panel.props('body')).toEqual({ instructions: 'Increase instance count', artifact_id: 'a1' });
  });

  it('shows diff review with Apply/Discard/Modify after stream done', async () => {
    const w = await mountEditor();
    await flushPromises();
    await startProposal(w);
    await emitProposalDone(w);
    await flushPromises();
    expect(w.find('[data-testid="apply-proposal-btn"]').exists()).toBe(true);
    expect(w.find('[data-testid="modify-proposal-btn"]').exists()).toBe(true);
    expect(w.find('[data-testid="discard-proposal-btn"]').exists()).toBe(true);
    expect(w.find('[data-testid="save-artifact-btn"]').exists()).toBe(false);
    expect(w.find('[data-testid="propose-artifact-btn"]').exists()).toBe(false);
  });

  it('mounts a diff editor after stream done', async () => {
    const w = await mountEditor();
    await flushPromises();
    monaco.editor.createDiffEditor.mockClear();
    await startProposal(w);
    await emitProposalDone(w);
    await flushPromises();
    expect(monaco.editor.createDiffEditor).toHaveBeenCalled();
  });

  it('Apply calls applyProvisionChange and returns to editor', async () => {
    const w = await mountEditor();
    await flushPromises();
    const modEditor = monaco.__mockModifiedEditor;
    modEditor.getValue.mockReturnValue('resource "z" {}');
    await startProposal(w);
    await emitProposalDone(w);
    await flushPromises();
    await w.find('[data-testid="apply-proposal-btn"]').trigger('click');
    await flushPromises();
    expect(api.applyProvisionChange).toHaveBeenCalledWith('proj-1', 'd1', expect.objectContaining({
      files: expect.objectContaining({
        'main.tf': 'resource "z" {}',
      }),
    }));
    expect(w.find('[data-testid="save-artifact-btn"]').exists()).toBe(true);
    expect(w.find('[data-testid="apply-proposal-btn"]').exists()).toBe(false);
  });

  it('emits saved after applying a proposal', async () => {
    const w = await mountEditor();
    await flushPromises();
    const modEditor = monaco.__mockModifiedEditor;
    modEditor.getValue.mockReturnValue('resource "z" {}');
    await startProposal(w);
    await emitProposalDone(w);
    await flushPromises();
    await w.find('[data-testid="apply-proposal-btn"]').trigger('click');
    await flushPromises();
    expect(w.emitted('saved')).toBeTruthy();
  });

  it('Discard returns to editor without applying', async () => {
    const w = await mountEditor();
    await flushPromises();
    await startProposal(w);
    await emitProposalDone(w);
    await flushPromises();
    api.applyProvisionChange.mockClear();
    await w.find('[data-testid="discard-proposal-btn"]').trigger('click');
    await flushPromises();
    expect(api.applyProvisionChange).not.toHaveBeenCalled();
    expect(w.find('[data-testid="save-artifact-btn"]').exists()).toBe(true);
    expect(w.find('[data-testid="apply-proposal-btn"]').exists()).toBe(false);
  });

  it('Modify reopens the propose modal with the previous prompt', async () => {
    const w = await mountEditor();
    await flushPromises();
    await startProposal(w, 'Change something');
    await emitProposalDone(w);
    await flushPromises();
    await w.find('[data-testid="modify-proposal-btn"]').trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="artifact-propose-modal"]').exists()).toBe(true);
    expect(w.find('[data-testid="artifact-propose-prompt"]').element.value).toBe('Change something');
  });

  it('shows error when stream done has no answer', async () => {
    const w = await mountEditor();
    await flushPromises();
    await startProposal(w);
    await emitProposalDone(w, '');
    await flushPromises();
    expect(w.find('[data-testid="artifact-editor-error"]').text()).toContain('Failed to propose changes');
  });

  it('shows error when stream done answer has no JSON block', async () => {
    const w = await mountEditor();
    await flushPromises();
    await startProposal(w);
    await emitProposalDone(w, 'I cannot help with that');
    await flushPromises();
    expect(w.find('[data-testid="artifact-editor-error"]').text()).toContain('Failed to parse');
  });

  it('shows changed indicator on tabs with proposed changes', async () => {
    const w = await mountEditor();
    await flushPromises();
    await startProposal(w);
    await emitProposalDone(w);
    await flushPromises();
    const mainTab = w.find('[data-testid="artifact-tab-main.tf"]');
    const varsTab = w.find('[data-testid="artifact-tab-variables.tf"]');
    expect(mainTab.find('[data-testid="tab-changed-dot"]').exists()).toBe(true);
    expect(varsTab.find('[data-testid="tab-changed-dot"]').exists()).toBe(false);
  });

  it('cancel from generation panel returns to editor', async () => {
    const w = await mountEditor();
    await flushPromises();
    await startProposal(w);
    w.findComponent({ name: 'DesignGenerationPanel' }).vm.$emit('cancel');
    await flushPromises();
    expect(w.find('[data-testid="design-gen-panel"]').exists()).toBe(false);
    expect(w.find('[data-testid="save-artifact-btn"]').exists()).toBe(true);
  });
});
