import { describe, it, expect, vi } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { defineComponent, h, ref } from 'vue';
import PaneSplit from '../../src/components/layout/PaneSplit.vue';
import { createLeaf, createTab } from '../../src/lib/pane-layout.js';
import { createLayoutOperations, operationsOf, provideLayoutWorkspace } from '../../src/lib/layout-workspace.js';
import { startTabDrag, endTabDrag } from '../../src/lib/pane-drag-state.js';

const Content = defineComponent({
  props: { tab: Object, paneId: String },
  setup: (props) => () => h('p', { class: 'content' }, `${props.tab.kind}:${props.tab.title}`),
});

function mountWith(kinds) {
  const tree = ref({ version: 1, root: createLeaf([createTab({ kind: 'chat', title: 'First' })]) });
  const operations = createLayoutOperations({ tree, changed: vi.fn() });
  const createTabFor = vi.fn(async (kind) => createTab({ kind, title: kind }));
  const workspace = {
    id: 'w1',
    tabKinds: kinds,
    content: Content,
    titleOf: (tab) => tab.title,
    createTab: createTabFor,
    ...operationsOf(operations),
  };
  const Host = defineComponent({
    setup() {
      provideLayoutWorkspace(workspace);
      return () => h(PaneSplit, { node: tree.value.root });
    },
  });
  return { tree, createTabFor, wrapper: mount(Host) };
}

const ONE_KIND = [{ kind: 'chat', label: 'Chat', icon: 'chat' }];
const THREE_KINDS = [
  { kind: 'chat', label: 'Chat', icon: 'chat' },
  { kind: 'changes', label: 'Changes', icon: 'changes' },
  { kind: 'preview', label: 'Preview', icon: 'preview' },
];

describe('layout pane', () => {
  it('renders the active tab through the workspace content component', async () => {
    const { wrapper } = mountWith(ONE_KIND);
    expect(wrapper.find('.content').text()).toBe('chat:First');
  });

  it('opens a tab straight away when the workspace has one kind of tab', async () => {
    const { wrapper, tree, createTabFor } = mountWith(ONE_KIND);
    await wrapper.find('[aria-label="New tab"]').trigger('click');
    await flushPromises();
    expect(createTabFor).toHaveBeenCalledWith('chat', expect.objectContaining({ paneId: tree.value.root.id }));
    expect(tree.value.root.tabs).toHaveLength(2);
    expect(wrapper.find('.app-menu').exists()).toBe(false);
  });

  it('asks which kind of tab to open when there are several', async () => {
    const { wrapper, tree } = mountWith(THREE_KINDS);
    await wrapper.find('[aria-label="New tab"]').trigger('click');
    const options = wrapper.findAll('.app-menu__item');
    expect(options.map(o => o.text())).toEqual(['Chat', 'Changes', 'Preview']);
    await options[2].trigger('click');
    await flushPromises();
    expect(tree.value.root.tabs.map(t => t.kind)).toEqual(['chat', 'preview']);
  });

  it('splits with the chosen kind of tab', async () => {
    const { wrapper, tree } = mountWith(THREE_KINDS);
    await wrapper.find('[aria-label="Split right"]').trigger('click');
    await wrapper.findAll('.app-menu__item')[1].trigger('click');
    await flushPromises();
    expect(tree.value.root.type).toBe('split');
    expect(tree.value.root.children[1].tabs[0].kind).toBe('changes');
  });

  it('ignores tabs dragged from another workspace', async () => {
    const { wrapper } = mountWith(ONE_KIND);
    startTabDrag('elsewhere', 'other-pane', 'w2');
    await flushPromises();
    expect(wrapper.find('.pane__drop-overlay').exists()).toBe(false);
    endTabDrag();
  });
});
