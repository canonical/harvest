import { describe, it, expect, vi } from 'vitest';
import { mount } from '@vue/test-utils';
import CodeSearchExplorer from '../../src/components/chat/CodeSearchExplorer.vue';
import ToolCallStep from '../../src/components/chat/ToolCallStep.vue';

const location = (name, file, line) => ({ name, file, line });

const view = () => ({
  kind: 'code_search',
  repo: 'amafu',
  version: 'main',
  query: 'How does Amafu get installed through MAAS?',
  focus: [],
  tier: 'tier1',
  total_ms: 11.6,
  degraded: null,
  items: [
    {
      kind: 'method', name: 'MaasClient.deploy', file: 'crates/installer/src/maas.rs', start_line: 178, end_line: 180,
      code: 'pub async fn deploy(&self) {\n    self.find().await\n}', truncated: false,
      owner: location('MaasClient', 'crates/installer/src/maas.rs', 86),
      calls: [location('MaasClient.find', 'crates/installer/src/maas.rs', 160), location('MaasClient.state', 'crates/installer/src/maas.rs', 174)],
    },
    {
      kind: 'function', name: 'run', file: 'crates/command-line/src/install.rs', start_line: 33, end_line: 34,
      code: 'pub async fn run() {}', truncated: true, owner: null, calls: [],
    },
    {
      kind: 'method', name: 'MaasClient.find', file: 'crates/installer/src/maas.rs', start_line: 160, end_line: 161,
      code: 'fn find() {}', truncated: false, owner: null, calls: [],
    },
  ],
  documentation: [
    {
      name: 'Install with MAAS (planned)', file: 'README.md', start_line: 52, end_line: 56,
      excerpt: 'Run `amafu cluster create`.', points_to: [location('run', 'crates/command-line/src/install.rs', 33)],
    },
  ],
  gaps: ['MaasClient.state is not in the evidence'],
});

function mountExplorer(overrides = {}) {
  return mount(CodeSearchExplorer, { props: { view: { ...view(), ...overrides } }, attachTo: document.body });
}

describe('CodeSearchExplorer', () => {
  it('summarises the result and counts each tab', () => {
    const w = mountExplorer();
    expect(w.find('.cs-summary').text()).toContain('3 code items · 2 files');
    expect(w.find('.cs-summary').text()).toContain('1 documentation · not verified');
    expect(w.find('.cs-summary').text()).toContain('tier1 · 12 ms');
    expect(w.findAll('.cs-tab').map(t => t.text())).toEqual(['Code 3', 'Docs 1', 'Gaps 1']);
    w.unmount();
  });

  it('lists code grouped by file and shows the first item with numbered, highlighted lines', () => {
    const w = mountExplorer();
    expect(w.findAll('.cs-list__file').map(f => f.text())).toEqual(['crates/installer/src/maas.rs', 'crates/command-line/src/install.rs']);
    expect(w.find('.cs-detail__name').text()).toBe('MaasClient.deploy');
    expect(w.find('.cs-detail__location').text()).toBe('crates/installer/src/maas.rs:178-180');
    expect(w.findAll('.cs-code__line').map(l => l.attributes('data-line'))).toEqual(['178', '179', '180']);
    expect(w.find('.cs-code').html()).toContain('hljs-keyword');
    w.unmount();
  });

  it('follows a call to another item in the results and marks calls outside them', async () => {
    const w = mountExplorer();
    const outside = w.findAll('.cs-chip--outside');
    expect(outside.map(c => c.text())).toEqual(['MaasClient', 'MaasClient.state']);
    expect(outside[0].element.tagName).toBe('SPAN');
    const find = w.findAll('button.cs-chip').find(c => c.text() === 'MaasClient.find');
    await find.trigger('click');
    expect(w.find('.cs-detail__name').text()).toBe('MaasClient.find');
    w.unmount();
  });

  it('moves through the list in file order with the arrow keys', async () => {
    const w = mountExplorer();
    const list = w.find('.cs-list');
    await list.trigger('keydown', { key: 'ArrowDown' });
    expect(w.find('.cs-detail__name').text()).toBe('MaasClient.find');
    await list.trigger('keydown', { key: 'ArrowDown' });
    expect(w.find('.cs-detail__name').text()).toBe('run');
    await list.trigger('keydown', { key: 'ArrowDown' });
    expect(w.find('.cs-detail__name').text()).toBe('run');
    await list.trigger('keydown', { key: 'ArrowUp' });
    expect(w.find('.cs-detail__name').text()).toBe('MaasClient.find');
    w.unmount();
  });

  it('shows documentation as not verified and jumps to the code it points to', async () => {
    const w = mountExplorer();
    await w.findAll('.cs-tab')[1].trigger('click');
    expect(w.find('.cs-detail').text()).toContain('Not verified');
    expect(w.find('.cs-excerpt').text()).toContain('amafu cluster create');
    await w.find('button.cs-chip').trigger('click');
    expect(w.findAll('.cs-tab')[0].attributes('aria-selected')).toBe('true');
    expect(w.find('.cs-detail__name').text()).toBe('run');
    expect(w.find('.cs-note').text()).toContain('first part');
    w.unmount();
  });

  it('lists gaps and disables empty tabs', async () => {
    const w = mountExplorer({ documentation: [] });
    expect(w.findAll('.cs-tab')[1].attributes('disabled')).toBeDefined();
    await w.findAll('.cs-tab')[2].trigger('click');
    expect(w.find('.cs-list__gap').text()).toBe('MaasClient.state is not in the evidence');
    w.unmount();
  });

  it('opens on documentation when no code matched', () => {
    const w = mountExplorer({ items: [] });
    expect(w.findAll('.cs-tab')[1].attributes('aria-selected')).toBe('true');
    expect(w.find('.cs-detail__name').text()).toBe('Install with MAAS (planned)');
    w.unmount();
  });

  it('renders a table as rows that link to the symbols in the results', async () => {
    const table = {
      kind: 'table', name: 'callers of MaasClient.find', file: '', start_line: 0, end_line: 0,
      code: 'MaasClient.deploy (crates/installer/src/maas.rs:178)', truncated: false, owner: null, calls: [],
      rows: [
        { symbol: location('MaasClient.deploy', 'crates/installer/src/maas.rs', 178), detail: '' },
        { symbol: location('Installer.run', 'crates/installer/src/run.rs', 12), detail: 'extends Runner' },
      ],
    };
    const w = mountExplorer({ items: [table, ...view().items] });
    expect(w.find('.cs-detail__name').text()).toBe('MaasClient.deploy');
    expect(w.findAll('.cs-list__file').at(-1).text()).toBe('Tables');
    await w.findAll('.cs-list__entry').at(-1).trigger('click');
    expect(w.find('.cs-detail__location').text()).toBe('2 rows');
    expect(w.find('.cs-code').exists()).toBe(false);
    expect(w.findAll('.cs-table th').map(h => h.text())).toEqual(['Symbol', 'Detail']);
    const rows = w.findAll('.cs-table tbody tr').map(r => [
      r.find('.cs-chip').text(),
      r.find('.cs-table__location').text(),
      r.findAll('td')[1].text(),
    ]);
    expect(rows).toEqual([
      ['MaasClient.deploy', 'crates/installer/src/maas.rs:178', ''],
      ['Installer.run', 'crates/installer/src/run.rs:12', 'extends Runner'],
    ]);
    await w.find('.cs-table button.cs-chip').trigger('click');
    expect(w.find('.cs-detail__name').text()).toBe('MaasClient.deploy');
    w.unmount();
  });

  it('says so when a table found nothing', () => {
    const empty = { kind: 'table', name: 'callers of run', file: '', start_line: 0, end_line: 0, code: '', truncated: false, owner: null, calls: [] };
    const w = mountExplorer({ items: [empty] });
    expect(w.find('.cs-detail__location').text()).toBe('0 rows');
    expect(w.find('.cs-code').exists()).toBe(false);
    expect(w.find('.cs-note').text()).toBe('The search found nothing for this table.');
    w.unmount();
  });

  it('copies the citation of the selected item', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    const w = mountExplorer();
    await w.find('.cs-action').trigger('click');
    await Promise.resolve();
    expect(writeText).toHaveBeenCalledWith('[amafu:main:crates/installer/src/maas.rs:178-180]');
    w.unmount();
  });
});

describe('ToolCallStep with a code search view', () => {
  const step = (extra = {}) => ({
    type: 'tool_call', id: 1, name: 'search_code', status: 'done',
    input: { repo: 'amafu', version: 'main', query: 'How does Amafu get installed through MAAS?' },
    preview: 'Evidence for …',
    ...extra,
  });

  it('opens the explorer instead of the plain preview', async () => {
    const w = mount(ToolCallStep, { props: { step: step({ view: view() }) } });
    expect(w.find('.tc-step__label').text()).toBe('Searching amafu main for “How does Amafu get installed through MAAS?”');
    await w.find('.tc-step__row').trigger('click');
    expect(w.find('.cs').exists()).toBe(true);
    expect(w.find('.tool-data').exists()).toBe(false);
  });

  it('keeps the plain preview for results without a view', async () => {
    const w = mount(ToolCallStep, { props: { step: step() } });
    await w.find('.tc-step__row').trigger('click');
    expect(w.find('.cs').exists()).toBe(false);
    expect(w.find('.tool-data').exists()).toBe(true);
  });
});
