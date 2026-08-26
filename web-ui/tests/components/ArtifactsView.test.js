import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { createRouter, createMemoryHistory } from 'vue-router';

const ARTIFACTS = [
  { id: 'a1', title: 'Deploy report', kind: 'markdown', created_by: 'alice', created_at: new Date().toISOString(), updated_at: new Date().toISOString() },
  { id: 'a2', title: 'Incident PDF',  kind: 'pdf',      created_by: 'assistant', created_at: new Date().toISOString(), updated_at: new Date().toISOString() },
  { id: 'a3', title: 'Web app infra', kind: 'terraform', created_by: 'assistant', created_at: new Date().toISOString(), updated_at: new Date().toISOString() },
];

const AGENTS = [{ id: 'agent-1', hostname: 'host-1', online: true }];

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    listProjectArtifacts: vi.fn(),
    getArtifact:          vi.fn(),
    deleteArtifact:       vi.fn(() => Promise.resolve()),
    listProjectAgents:    vi.fn(),
    runTerraformArtifact: vi.fn(),
    createProjectArtifact: vi.fn(() => Promise.resolve({ id: 'a9', title: 'New', kind: 'markdown', content: '# Hi', created_by: 'alice', created_at: new Date().toISOString() })),
  };
});

import ArtifactsView from '../../src/views/ArtifactsView.vue';
import * as api from '../../src/lib/api.js';

function makeRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/artifacts',     component: ArtifactsView },
      { path: '/artifacts/:id', component: ArtifactsView },
    ],
  });
}

async function mountView({ path = '/artifacts', projectId = 'proj-1' } = {}) {
  const router = makeRouter();
  router.push(path);
  await router.isReady();
  const w = mount(ArtifactsView, {
    props: { projectId },
    global: { plugins: [createPinia(), router] },
  });
  await flushPromises();
  return { w, router };
}

describe('ArtifactsView', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    api.listProjectArtifacts.mockResolvedValue(ARTIFACTS);
    api.getArtifact.mockImplementation((id) => {
      if (id === 'a3') {
        return Promise.resolve({
          ...ARTIFACTS.find(a => a.id === id),
          content: JSON.stringify({ 'main.tf': 'resource "local_file" "x" {}', 'variables.tf': 'variable "x" {}' }),
        });
      }
      return Promise.resolve({ ...ARTIFACTS.find(a => a.id === id), content: '# Hello\n\nBody text.' });
    });
    api.listProjectAgents.mockResolvedValue(AGENTS);
    api.runTerraformArtifact.mockResolvedValue({ stdout: 'plan output', stderr: '', exit_code: 0 });
  });

  it('shows no-project state when projectId is null and no artifact is targeted', async () => {
    const { w } = await mountView({ path: '/artifacts', projectId: null });
    expect(w.text()).toContain('project');
  });

  it('renders artifact list after load', async () => {
    const { w } = await mountView();
    expect(w.text()).toContain('Deploy report');
    expect(w.text()).toContain('Incident PDF');
  });

  it('shows empty state when no artifacts', async () => {
    api.listProjectArtifacts.mockResolvedValue([]);
    const { w } = await mountView();
    expect(w.text()).toMatch(/no artifacts/i);
  });

  it('selecting an artifact loads and renders its content', async () => {
    const { w } = await mountView();
    const items = w.findAll('.artifacts-list-item');
    expect(items.length).toBe(3);
    await items[0].trigger('click');
    await flushPromises();
    expect(api.getArtifact).toHaveBeenCalledWith('a1');
    expect(w.text()).toContain('Hello');
    expect(w.text()).toContain('Body text');
  });

  it('shows a working download link for the selected artifact', async () => {
    const { w } = await mountView();
    const items = w.findAll('.artifacts-list-item');
    await items[0].trigger('click');
    await flushPromises();
    const link = w.find('a.artifact-download-btn');
    expect(link.exists()).toBe(true);
    expect(link.attributes('href')).toBe('/artifacts/a1/download');
  });

  it('loads the artifact directly when navigated to /artifacts/:id, independent of projectId', async () => {
    const { w } = await mountView({ path: '/artifacts/a2', projectId: null });
    expect(api.getArtifact).toHaveBeenCalledWith('a2');
    expect(w.text()).toContain('Incident PDF');
    expect(w.text()).toContain('Hello');
  });

  it('deletes an artifact after confirmation', async () => {
    const { w } = await mountView();
    const items = w.findAll('.artifacts-list-item');
    await items[0].trigger('click');
    await flushPromises();
    await w.find('.delete-artifact-btn').trigger('click');
    await w.find('.modal .p-button--negative').trigger('click');
    await flushPromises();
    expect(api.deleteArtifact).toHaveBeenCalledWith('a1');
  });

  it('shows a Terraform badge and renders each bundle file as its own section', async () => {
    const { w } = await mountView();
    const items = w.findAll('.artifacts-list-item');
    await items[2].trigger('click');
    await flushPromises();
    expect(w.text()).toContain('Terraform');
    expect(w.text()).toContain('main.tf');
    expect(w.text()).toContain('variables.tf');
    expect(w.text()).toContain('resource "local_file" "x" {}');
  });

  it('falls back to a raw code block when bundle content is not valid JSON', async () => {
    api.getArtifact.mockResolvedValueOnce({ ...ARTIFACTS[2], content: 'not json' });
    const { w } = await mountView({ path: '/artifacts/a3' });
    expect(w.text()).toContain('not json');
  });

  it('does not show a Run on agent button for non-terraform artifacts', async () => {
    const { w } = await mountView();
    const items = w.findAll('.artifacts-list-item');
    await items[0].trigger('click');
    await flushPromises();
    expect(w.find('.run-on-agent-btn').exists()).toBe(false);
  });

  it('opens the run modal and loads agents for a terraform artifact', async () => {
    const { w } = await mountView();
    const items = w.findAll('.artifacts-list-item');
    await items[2].trigger('click');
    await flushPromises();
    await w.find('.run-on-agent-btn').trigger('click');
    await flushPromises();
    expect(api.listProjectAgents).toHaveBeenCalledWith('proj-1');
    expect(w.find('.modal').text()).toContain('host-1');
  });

  it('disables Run until an agent is selected', async () => {
    const { w } = await mountView();
    const items = w.findAll('.artifacts-list-item');
    await items[2].trigger('click');
    await flushPromises();
    await w.find('.run-on-agent-btn').trigger('click');
    await flushPromises();
    const runButton = w.find('.modal .p-button--positive');
    expect(runButton.attributes('disabled')).toBeDefined();
  });

  it('requires the danger checkbox before apply can run', async () => {
    const { w } = await mountView();
    const items = w.findAll('.artifacts-list-item');
    await items[2].trigger('click');
    await flushPromises();
    await w.find('.run-on-agent-btn').trigger('click');
    await flushPromises();

    await w.find('#run-agent-select').setValue('agent-1');
    await w.find('#run-action-select').setValue('apply');

    const runButton = w.find('.modal .p-button--negative');
    expect(runButton.attributes('disabled')).toBeDefined();

    await w.find('.run-modal-confirm input[type="checkbox"]').setValue(true);
    expect(runButton.attributes('disabled')).toBeUndefined();
  });

  it('runs plan on the selected agent and shows the result', async () => {
    const { w } = await mountView();
    const items = w.findAll('.artifacts-list-item');
    await items[2].trigger('click');
    await flushPromises();
    await w.find('.run-on-agent-btn').trigger('click');
    await flushPromises();

    await w.find('#run-agent-select').setValue('agent-1');
    await w.find('.modal .p-button--positive').trigger('click');
    await flushPromises();

    expect(api.runTerraformArtifact).toHaveBeenCalledWith('proj-1', 'agent-1', 'a3', 'plan');
    expect(w.find('.run-modal-result').text()).toContain('plan output');
  });

  it('shows Create and Upload buttons in the page header', async () => {
    const { w } = await mountView();
    expect(w.find('[data-testid="create-artifact-btn"]').exists()).toBe(true);
    expect(w.find('[data-testid="upload-artifact-btn"]').exists()).toBe(true);
  });

  it('does not show Create/Upload buttons when no project is selected', async () => {
    const { w } = await mountView({ path: '/artifacts', projectId: null });
    expect(w.find('[data-testid="create-artifact-btn"]').exists()).toBe(false);
    expect(w.find('[data-testid="upload-artifact-btn"]').exists()).toBe(false);
  });

  it('opens the create modal with a type selector and content textarea', async () => {
    const { w } = await mountView();
    await w.find('[data-testid="create-artifact-btn"]').trigger('click');
    expect(w.find('[data-testid="create-artifact-modal"]').exists()).toBe(true);
    expect(w.find('[data-testid="create-artifact-kind"]').exists()).toBe(true);
    expect(w.find('[data-testid="create-artifact-title"]').exists()).toBe(true);
    expect(w.find('[data-testid="create-artifact-content"]').exists()).toBe(true);
  });

  it('creates an artifact from the modal and refreshes the list', async () => {
    const { w } = await mountView();
    await w.find('[data-testid="create-artifact-btn"]').trigger('click');
    await w.find('[data-testid="create-artifact-title"]').setValue('My notes');
    await w.find('[data-testid="create-artifact-kind"]').setValue('markdown');
    await w.find('[data-testid="create-artifact-content"]').setValue('# Hello world');
    await w.find('[data-testid="submit-create-artifact"]').trigger('click');
    await flushPromises();
    expect(api.createProjectArtifact).toHaveBeenCalledWith('proj-1', {
      title: 'My notes', kind: 'markdown', content: '# Hello world',
    });
  });

  it('disables the create submit button until title and content are provided', async () => {
    const { w } = await mountView();
    await w.find('[data-testid="create-artifact-btn"]').trigger('click');
    const btn = w.find('[data-testid="submit-create-artifact"]');
    expect(btn.attributes('disabled')).toBeDefined();
    await w.find('[data-testid="create-artifact-title"]').setValue('T');
    expect(btn.attributes('disabled')).toBeDefined();
    await w.find('[data-testid="create-artifact-content"]').setValue('C');
    expect(btn.attributes('disabled')).toBeUndefined();
  });

  it('closes the create modal on cancel', async () => {
    const { w } = await mountView();
    await w.find('[data-testid="create-artifact-btn"]').trigger('click');
    await w.find('[data-testid="cancel-create-artifact"]').trigger('click');
    expect(w.find('[data-testid="create-artifact-modal"]').exists()).toBe(false);
  });

  it('shows an error in the create modal when creation fails', async () => {
    api.createProjectArtifact.mockRejectedValueOnce(new Error('kind must be valid'));
    const { w } = await mountView();
    await w.find('[data-testid="create-artifact-btn"]').trigger('click');
    await w.find('[data-testid="create-artifact-title"]').setValue('Bad');
    await w.find('[data-testid="create-artifact-content"]').setValue('x');
    await w.find('[data-testid="submit-create-artifact"]').trigger('click');
    await flushPromises();
    expect(w.find('[data-testid="create-artifact-modal"]').text()).toContain('kind must be valid');
  });

  it('opens the upload modal with a dropzone', async () => {
    const { w } = await mountView();
    await w.find('[data-testid="upload-artifact-btn"]').trigger('click');
    expect(w.find('[data-testid="upload-artifact-modal"]').exists()).toBe(true);
    expect(w.find('[data-testid="upload-dropzone"]').exists()).toBe(true);
  });

  function setFiles(input, files) {
  Object.defineProperty(input, 'files', { value: files, configurable: true, writable: true });
}

async function pickUploadFile(w, file) {
  Object.defineProperty(w.find('[data-testid="upload-file-input"]').element, 'files', { value: [file], configurable: true, writable: true });
  await w.find('[data-testid="upload-file-input"]').trigger('change');
  await flushPromises();
}

  it('parses a .md file as a markdown artifact', async () => {
    const { w } = await mountView();
    await w.find('[data-testid="upload-artifact-btn"]').trigger('click');
    await pickUploadFile(w, new File(['# Markdown body'], 'notes.md', { type: 'text/markdown' }));
    await w.find('[data-testid="submit-upload-artifact"]').trigger('click');
    await flushPromises();
    await flushPromises();
    expect(api.createProjectArtifact).toHaveBeenCalledWith('proj-1', expect.objectContaining({
      kind: 'markdown', content: '# Markdown body',
    }));
  });

  it('parses a .sh file as a bash artifact', async () => {
    const { w } = await mountView();
    await w.find('[data-testid="upload-artifact-btn"]').trigger('click');
    await pickUploadFile(w, new File(['echo hi'], 'run.sh', { type: 'text/x-shellscript' }));
    await w.find('[data-testid="submit-upload-artifact"]').trigger('click');
    await flushPromises();
    await flushPromises();
    expect(api.createProjectArtifact).toHaveBeenCalledWith('proj-1', expect.objectContaining({
      kind: 'bash', content: 'echo hi',
    }));
  });

  it('rejects unsupported file extensions in the upload modal', async () => {
    const { w } = await mountView();
    await w.find('[data-testid="upload-artifact-btn"]').trigger('click');
    await pickUploadFile(w, new File(['x'], 'image.png', { type: 'image/png' }));
    expect(w.find('[data-testid="upload-artifact-modal"]').text()).toMatch(/not a supported|unsupported/i);
  });
});
