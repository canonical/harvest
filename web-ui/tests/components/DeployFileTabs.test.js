import { describe, it, expect } from 'vitest';
import { mount } from '@vue/test-utils';
import DeployFileTabs from '../../src/components/deployment/DeployFileTabs.vue';

function file(overrides = {}) {
  return {
    id: 'f1', artifactId: null, title: 'deploy.sh', kind: 'bash', status: 'saving',
    subfiles: [{ path: 'deploy.sh', text: 'echo hi' }],
    ...overrides,
  };
}

describe('DeployFileTabs', () => {
  it('renders no tabs and no content pane when there are no files yet', () => {
    const w = mount(DeployFileTabs, { props: { files: [], activeFileId: null } });
    expect(w.findAll('[data-testid="file-tab"]')).toHaveLength(0);
    expect(w.find('[data-testid="file-tab-content"]').exists()).toBe(false);
  });

  it('renders one tab per file and auto-selects the active one', () => {
    const files = [file({ id: 'f1', subfiles: [{ path: 'deploy.sh', text: 'echo hi' }] })];
    const w = mount(DeployFileTabs, { props: { files, activeFileId: 'f1' } });
    const tabs = w.findAll('[data-testid="file-tab"]');
    expect(tabs).toHaveLength(1);
    expect(tabs[0].text()).toContain('deploy.sh');
    expect(w.find('[data-testid="file-tab-content"]').text()).toContain('echo hi');
  });

  it('expands a terraform bundle into one tab per subfile', () => {
    const files = [file({
      id: 'f1', kind: 'terraform', title: 'Infra',
      subfiles: [{ path: 'main.tf', text: 'resource "x" {}' }, { path: 'variables.tf', text: 'variable "x" {}' }],
    })];
    const w = mount(DeployFileTabs, { props: { files, activeFileId: 'f1' } });
    const tabs = w.findAll('[data-testid="file-tab"]');
    expect(tabs.map(t => t.text().trim())).toEqual(expect.arrayContaining([expect.stringContaining('main.tf'), expect.stringContaining('variables.tf')]));
  });

  it('shows a saving indicator for an in-flight file and a saved indicator once saved', () => {
    const saving = file({ id: 'f1', status: 'saving' });
    const w1 = mount(DeployFileTabs, { props: { files: [saving], activeFileId: 'f1' } });
    expect(w1.find('[data-testid="file-tab-saving"]').exists()).toBe(true);

    const saved = file({ id: 'f1', status: 'saved' });
    const w2 = mount(DeployFileTabs, { props: { files: [saved], activeFileId: 'f1' } });
    expect(w2.find('[data-testid="file-tab-saved"]').exists()).toBe(true);
  });

  it('auto-follows the active file as new files arrive', async () => {
    const files1 = [file({ id: 'f1' })];
    const w = mount(DeployFileTabs, { props: { files: files1, activeFileId: 'f1' } });
    expect(w.find('[data-testid="file-tab-content"]').text()).toContain('echo hi');

    const files2 = [...files1, file({ id: 'f2', title: 'destroy.sh', subfiles: [{ path: 'destroy.sh', text: 'echo bye' }] })];
    await w.setProps({ files: files2, activeFileId: 'f2' });
    expect(w.find('[data-testid="file-tab-content"]').text()).toContain('echo bye');
    expect(w.find('[data-testid="file-tabs-resume"]').exists()).toBe(false);
  });

  it('clicking an earlier tab stops auto-follow and shows a Back to live control', async () => {
    const files = [
      file({ id: 'f1' }),
      file({ id: 'f2', title: 'destroy.sh', subfiles: [{ path: 'destroy.sh', text: 'echo bye' }] }),
    ];
    const w = mount(DeployFileTabs, { props: { files, activeFileId: 'f2' } });
    const tabs = w.findAll('[data-testid="file-tab"]');
    await tabs[0].trigger('click');
    expect(w.find('[data-testid="file-tab-content"]').text()).toContain('echo hi');
    expect(w.find('[data-testid="file-tabs-resume"]').exists()).toBe(true);
  });

  it('does not steal the view away from a manually-selected tab when a new file arrives', async () => {
    const files1 = [
      file({ id: 'f1' }),
      file({ id: 'f2', title: 'destroy.sh', subfiles: [{ path: 'destroy.sh', text: 'echo bye' }] }),
    ];
    const w = mount(DeployFileTabs, { props: { files: files1, activeFileId: 'f2' } });
    await w.findAll('[data-testid="file-tab"]')[0].trigger('click');

    const files2 = [...files1, file({ id: 'f3', title: 'main.tf', subfiles: [{ path: 'main.tf', text: 'resource {}' }] })];
    await w.setProps({ files: files2, activeFileId: 'f3' });
    expect(w.find('[data-testid="file-tab-content"]').text()).toContain('echo hi');
  });

  it('Back to live jumps to the current active file and resumes auto-follow', async () => {
    const files1 = [
      file({ id: 'f1' }),
      file({ id: 'f2', title: 'destroy.sh', subfiles: [{ path: 'destroy.sh', text: 'echo bye' }] }),
    ];
    const w = mount(DeployFileTabs, { props: { files: files1, activeFileId: 'f2' } });
    await w.findAll('[data-testid="file-tab"]')[0].trigger('click');
    await w.find('[data-testid="file-tabs-resume"]').trigger('click');
    expect(w.find('[data-testid="file-tab-content"]').text()).toContain('echo bye');
    expect(w.find('[data-testid="file-tabs-resume"]').exists()).toBe(false);
  });

  it('syntax-highlights recognized file extensions', () => {
    const files = [file({ id: 'f1', title: 'main.js', subfiles: [{ path: 'main.js', text: 'const x = 1;' }] })];
    const w = mount(DeployFileTabs, { props: { files, activeFileId: 'f1' } });
    expect(w.find('[data-testid="file-tab-content"]').html()).toContain('hljs-keyword');
  });

  it('falls back to plain escaped text for unrecognized extensions without crashing', () => {
    const files = [file({ id: 'f1', title: 'main.tf', kind: 'terraform', subfiles: [{ path: 'main.tf', text: 'resource "<x>" {}' }] })];
    const w = mount(DeployFileTabs, { props: { files, activeFileId: 'f1' } });
    expect(w.find('[data-testid="file-tab-content"]').text()).toContain('resource "<x>" {}');
  });

  it('shows a caret after the content of the live, still-saving file while running', () => {
    const files = [file({ id: 'f1', status: 'saving' })];
    const w = mount(DeployFileTabs, { props: { files, activeFileId: 'f1', running: true } });
    expect(w.findComponent({ name: 'BlinkingCaret' }).exists()).toBe(true);
  });

  it('hides the caret once the live file is saved', () => {
    const files = [file({ id: 'f1', status: 'saved' })];
    const w = mount(DeployFileTabs, { props: { files, activeFileId: 'f1', running: true } });
    expect(w.findComponent({ name: 'BlinkingCaret' }).exists()).toBe(false);
  });

  it('hides the caret once generation as a whole has finished', () => {
    const files = [file({ id: 'f1', status: 'saving' })];
    const w = mount(DeployFileTabs, { props: { files, activeFileId: 'f1', running: false } });
    expect(w.findComponent({ name: 'BlinkingCaret' }).exists()).toBe(false);
  });

  it('hides the caret while manually viewing a tab that is not the live one', async () => {
    const files = [
      file({ id: 'f1', status: 'saved' }),
      file({ id: 'f2', title: 'destroy.sh', status: 'saving', subfiles: [{ path: 'destroy.sh', text: 'echo bye' }] }),
    ];
    const w = mount(DeployFileTabs, { props: { files, activeFileId: 'f2', running: true } });
    expect(w.findComponent({ name: 'BlinkingCaret' }).exists()).toBe(true);
    await w.findAll('[data-testid="file-tab"]')[0].trigger('click');
    expect(w.findComponent({ name: 'BlinkingCaret' }).exists()).toBe(false);
  });

  it('auto-scrolls the content pane to the bottom as the live file grows', async () => {
    const files1 = [file({ id: 'f1', status: 'saving', subfiles: [{ path: 'deploy.sh', text: 'echo hi' }] })];
    const w = mount(DeployFileTabs, { props: { files: files1, activeFileId: 'f1', running: true } });
    const el = w.find('[data-testid="file-tab-content"]').element;
    Object.defineProperty(el, 'scrollHeight', { configurable: true, get: () => 800 });
    Object.defineProperty(el, 'clientHeight', { configurable: true, get: () => 200 });

    const files2 = [file({ id: 'f1', status: 'saving', artifactId: 'art-1', subfiles: [{ path: 'deploy.sh', text: 'echo hi\necho more' }] })];
    await w.setProps({ files: files2 });
    expect(el.scrollTop).toBe(800);
  });

  it('shows a jump-to-latest control after scrolling away from a live, still-saving file', async () => {
    const files = [file({ id: 'f1', status: 'saving' })];
    const w = mount(DeployFileTabs, { props: { files, activeFileId: 'f1', running: true } });
    const el = w.find('[data-testid="file-tab-content"]').element;
    Object.defineProperty(el, 'scrollHeight', { configurable: true, get: () => 800 });
    Object.defineProperty(el, 'clientHeight', { configurable: true, get: () => 200 });
    Object.defineProperty(el, 'scrollTop', { configurable: true, writable: true, value: 0 });
    await w.find('[data-testid="file-tab-content"]').trigger('scroll');
    expect(w.find('[data-testid="file-tabs-scroll-latest"]').exists()).toBe(true);
    await w.find('[data-testid="file-tabs-scroll-latest"]').trigger('click');
    expect(el.scrollTop).toBe(800);
    expect(w.find('[data-testid="file-tabs-scroll-latest"]').exists()).toBe(false);
  });

  it('resets scroll-follow when a new file becomes the live one', async () => {
    const files1 = [file({ id: 'f1', status: 'saving' })];
    const w = mount(DeployFileTabs, { props: { files: files1, activeFileId: 'f1', running: true } });
    const el1 = w.find('[data-testid="file-tab-content"]').element;
    Object.defineProperty(el1, 'scrollHeight', { configurable: true, get: () => 800 });
    Object.defineProperty(el1, 'clientHeight', { configurable: true, get: () => 200 });
    Object.defineProperty(el1, 'scrollTop', { configurable: true, writable: true, value: 0 });
    await w.find('[data-testid="file-tab-content"]').trigger('scroll');
    expect(w.find('[data-testid="file-tabs-scroll-latest"]').exists()).toBe(true);

    const files2 = [
      { ...files1[0], status: 'saved' },
      file({ id: 'f2', title: 'destroy.sh', status: 'saving', subfiles: [{ path: 'destroy.sh', text: 'echo bye' }] }),
    ];
    await w.setProps({ files: files2, activeFileId: 'f2' });
    expect(w.find('[data-testid="file-tabs-scroll-latest"]').exists()).toBe(false);
  });
});
