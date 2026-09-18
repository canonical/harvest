import { describe, it, expect } from 'vitest';
import { mount } from '@vue/test-utils';
import DesignDocumentHero from '../../src/components/deployment/DesignDocumentHero.vue';

function mountHero(props = {}) {
  return mount(DesignDocumentHero, { props: { text: '', running: true, ...props } });
}

describe('DesignDocumentHero', () => {
  it('shows only the caret when there is no text yet and it is running', () => {
    const w = mountHero({ text: '' });
    expect(w.find('[data-testid="document-hero-block"]').exists()).toBe(false);
    expect(w.findComponent({ name: 'BlinkingCaret' }).exists()).toBe(true);
  });

  it('hides the caret once not running', () => {
    const w = mountHero({ text: '# Title', running: false });
    expect(w.findComponent({ name: 'BlinkingCaret' }).exists()).toBe(false);
  });

  it('renders one block per top-level markdown block', () => {
    const w = mountHero({ text: '# Title\n\nFirst paragraph.\n\nSecond paragraph.' });
    const blocks = w.findAll('[data-testid="document-hero-block"]');
    expect(blocks).toHaveLength(3);
    expect(blocks[0].html()).toContain('<h1');
    expect(blocks[2].text()).toBe('Second paragraph.');
  });

  it('places the caret after the last (live) block', async () => {
    const w = mountHero({ text: '# Title\n\nGrowing paragraph' });
    const blocks = w.findAll('[data-testid="document-hero-block"]');
    const caret = w.findComponent({ name: 'BlinkingCaret' });
    const blockIndex = [...w.element.querySelectorAll('*')].indexOf(blocks.at(-1).element);
    const caretIndex = [...w.element.querySelectorAll('*')].indexOf(caret.element);
    expect(caretIndex).toBeGreaterThan(blockIndex);
  });

  it('keeps a completed block byte-identical when only the trailing block grows', async () => {
    const w = mountHero({ text: '# Title\n\nFirst paragraph.\n\nGrowing' });
    const firstBlockHtmlBefore = w.findAll('[data-testid="document-hero-block"]')[1].html();
    await w.setProps({ text: '# Title\n\nFirst paragraph.\n\nGrowing more now' });
    const firstBlockHtmlAfter = w.findAll('[data-testid="document-hero-block"]')[1].html();
    expect(firstBlockHtmlAfter).toBe(firstBlockHtmlBefore);
  });

  it('re-renders the trailing block as it grows', async () => {
    const w = mountHero({ text: '# Title\n\nGrowing' });
    await w.setProps({ text: '# Title\n\nGrowing more now' });
    const lastBlock = w.findAll('[data-testid="document-hero-block"]').at(-1);
    expect(lastBlock.text()).toBe('Growing more now');
  });

  it('reuses the same live element while its own block keeps growing', async () => {
    const w = mountHero({ text: '# Title\n\nGrowing' });
    const before = w.findAll('[data-testid="document-hero-block"]').at(-1).element;
    await w.setProps({ text: '# Title\n\nGrowing more now' });
    const after = w.findAll('[data-testid="document-hero-block"]').at(-1).element;
    expect(after).toBe(before);
  });

  it('mounts a fresh live element once a new block starts, so its enter animation plays', async () => {
    const w = mountHero({ text: '# Title\n\nFirst paragraph' });
    const before = w.findAll('[data-testid="document-hero-block"]').at(-1).element;
    await w.setProps({ text: '# Title\n\nFirst paragraph\n\nSecond paragraph' });
    const after = w.findAll('[data-testid="document-hero-block"]').at(-1).element;
    expect(after).not.toBe(before);
  });

  it('auto-scrolls to the bottom as text grows', async () => {
    const w = mountHero({ text: '# Title' });
    const el = w.find('[data-testid="document-hero"]').element;
    Object.defineProperty(el, 'scrollHeight', { configurable: true, get: () => 800 });
    Object.defineProperty(el, 'clientHeight', { configurable: true, get: () => 200 });
    await w.setProps({ text: '# Title\n\nMore content now.' });
    expect(el.scrollTop).toBe(800);
  });

  it('stops auto-scrolling once the user scrolls away, and shows a jump-to-latest control', async () => {
    const w = mountHero({ text: '# Title' });
    const el = w.find('[data-testid="document-hero"]').element;
    Object.defineProperty(el, 'scrollHeight', { configurable: true, get: () => 800 });
    Object.defineProperty(el, 'clientHeight', { configurable: true, get: () => 200 });
    Object.defineProperty(el, 'scrollTop', { configurable: true, writable: true, value: 0 });
    await w.find('[data-testid="document-hero"]').trigger('scroll');
    expect(w.find('[data-testid="document-hero-jump-latest"]').exists()).toBe(true);
    el.scrollTop = 0;
    await w.setProps({ text: '# Title\n\nMore content now.' });
    expect(el.scrollTop).toBe(0);
  });

  it('jump-to-latest scrolls to the bottom and hides itself again', async () => {
    const w = mountHero({ text: '# Title' });
    const el = w.find('[data-testid="document-hero"]').element;
    Object.defineProperty(el, 'scrollHeight', { configurable: true, get: () => 800 });
    Object.defineProperty(el, 'clientHeight', { configurable: true, get: () => 200 });
    Object.defineProperty(el, 'scrollTop', { configurable: true, writable: true, value: 0 });
    await w.find('[data-testid="document-hero"]').trigger('scroll');
    await w.find('[data-testid="document-hero-jump-latest"]').trigger('click');
    expect(el.scrollTop).toBe(800);
    expect(w.find('[data-testid="document-hero-jump-latest"]').exists()).toBe(false);
  });

  it('applies a thinking style when in thinking mode', () => {
    const w = mountHero({ text: 'Considering the options.', thinking: true });
    expect(w.find('[data-testid="document-hero"]').classes()).toContain('doc-hero--thinking');
  });

  it('does not apply the thinking style by default', () => {
    const w = mountHero({ text: '# Design' });
    expect(w.find('[data-testid="document-hero"]').classes()).not.toContain('doc-hero--thinking');
  });

  it('links citations using the provided repoUrlMap and citationIndex', () => {
    const w = mountHero({
      text: 'See [acme/repo:main:src/lib.rs:42]',
      repoUrlMap: { 'acme/repo': 'https://github.com/acme/repo' },
      citationIndex: { 'acme/repo:main:src/lib.rs:42': 1 },
    });
    expect(w.html()).toContain('href="https://github.com/acme/repo/blob/main/src/lib.rs#L42"');
  });
});
