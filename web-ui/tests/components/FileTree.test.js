import { describe, it, expect, vi, afterEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import FileTree from '../../src/components/code/browser/FileTree.vue';

const entry = (path, kind = 'file') => ({ name: path.split('/').pop(), path, kind });

const LISTINGS = {
  '': [entry('src', 'directory'), entry('README.md')],
  src: [entry('src/lib', 'directory'), entry('src/main.rs')],
  'src/lib': [entry('src/lib/mod.rs')],
};

let wrapper;

async function tree(selectedPath = '') {
  const loadChildren = vi.fn(async (path) => LISTINGS[path]);
  wrapper = mount(FileTree, { props: { label: 'Files', loadChildren, selectedPath }, attachTo: document.body });
  await flushPromises();
  return { w: wrapper, loadChildren };
}

const paths = (w) => w.findAll('[role="treeitem"]').map((item) => item.attributes('data-path'));
const item = (w, path) => w.find(`[data-path="${path}"]`);

afterEach(() => wrapper?.unmount());

describe('FileTree', () => {
  it('shows the top level of the repository', async () => {
    const { w, loadChildren } = await tree();
    expect(paths(w)).toEqual(['src', 'README.md']);
    expect(loadChildren).toHaveBeenCalledWith('');
    expect(item(w, 'src').attributes('aria-expanded')).toBe('false');
    expect(item(w, 'README.md').attributes('aria-expanded')).toBeUndefined();
  });

  it('expands a directory and reports the selection when it is clicked', async () => {
    const { w } = await tree();
    await item(w, 'src').trigger('click');
    await flushPromises();
    expect(paths(w)).toEqual(['src', 'src/lib', 'src/main.rs', 'README.md']);
    expect(item(w, 'src/main.rs').attributes('aria-level')).toBe('2');
    expect(w.emitted('select')[0][0].path).toBe('src');
  });

  it('opens the directories that lead to the selected file', async () => {
    const { w } = await tree('src/lib/mod.rs');
    expect(paths(w)).toContain('src/lib/mod.rs');
    expect(item(w, 'src/lib/mod.rs').attributes('aria-selected')).toBe('true');
    expect(item(w, 'src/lib/mod.rs').attributes('tabindex')).toBe('0');
  });

  it('moves through the tree with the arrow keys', async () => {
    const { w } = await tree('README.md');
    const list = w.find('[role="tree"]');
    await list.trigger('keydown', { key: 'ArrowUp' });
    expect(document.activeElement.dataset.path).toBe('src');
    await list.trigger('keydown', { key: 'ArrowRight' });
    await flushPromises();
    expect(item(w, 'src').attributes('aria-expanded')).toBe('true');
    await list.trigger('keydown', { key: 'ArrowRight' });
    expect(document.activeElement.dataset.path).toBe('src/lib');
    await list.trigger('keydown', { key: 'ArrowLeft' });
    expect(document.activeElement.dataset.path).toBe('src');
  });

  it('opens a file with Enter', async () => {
    const { w } = await tree('README.md');
    await w.find('[role="tree"]').trigger('keydown', { key: 'Enter' });
    expect(w.emitted('select')[0][0].path).toBe('README.md');
  });

  it('shows why the tree could not be loaded', async () => {
    wrapper = mount(FileTree, {
      props: { label: 'Files', loadChildren: async () => { throw new Error('could not fetch the repository'); } },
    });
    await flushPromises();
    expect(wrapper.find('[role="alert"]').text()).toBe('could not fetch the repository');
  });
});
