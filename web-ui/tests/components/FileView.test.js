import { describe, it, expect } from 'vitest';
import { mount } from '@vue/test-utils';
import FileView from '../../src/components/code/browser/FileView.vue';

const CodeViewer = { name: 'CodeViewer', props: ['path', 'content', 'line'], template: '<pre class="code-viewer-stub">{{ path }}:{{ line }}</pre>' };

function fileView(file, line = null) {
  return mount(FileView, {
    props: { file, rawUrl: '/raw/file', line },
    global: { stubs: { CodeViewer } },
  });
}

describe('FileView', () => {
  it('shows code in the code viewer with the linked line', () => {
    const w = fileView({ path: 'src/main.rs', size: 13, kind: 'text', content: 'fn main() {}' }, 4);
    expect(w.find('.code-viewer-stub').text()).toBe('src/main.rs:4');
    expect(w.find('.file-view__size').text()).toBe('13 bytes');
    expect(w.find('a[download]').attributes()).toMatchObject({ href: '/raw/file', download: 'main.rs' });
  });

  it('previews markdown and switches to its source', async () => {
    const w = fileView({ path: 'README.md', size: 9, kind: 'text', content: '# Sample' });
    expect(w.find('.markdown-viewer h1').text()).toBe('Sample');
    await w.findAll('.p-segmented-control__button')[1].trigger('click');
    expect(w.find('.code-viewer-stub').exists()).toBe(true);
    expect(w.findAll('.p-segmented-control__button')[1].attributes('aria-pressed')).toBe('true');
  });

  it('shows images from the raw file address', () => {
    const w = fileView({ path: 'docs/logo.png', size: 2048, kind: 'binary' });
    expect(w.find('.image-viewer img').attributes()).toMatchObject({ src: '/raw/file', alt: 'logo.png' });
  });

  it('explains why binary and oversized files are not shown', () => {
    expect(fileView({ path: 'app.bin', size: 10, kind: 'binary' }).find('.p-notification__message').text()).toContain('binary file');
    expect(fileView({ path: 'dump.sql', size: 3 * 1024 * 1024, kind: 'too_large' }).find('.p-notification__message').text())
      .toContain('3 MB, too large to show here');
  });

  it('offers the preview again when another markdown file opens', async () => {
    const w = fileView({ path: 'README.md', size: 9, kind: 'text', content: '# One' });
    await w.findAll('.p-segmented-control__button')[1].trigger('click');
    await w.setProps({ file: { path: 'docs/guide.md', size: 9, kind: 'text', content: '# Two' } });
    expect(w.find('.markdown-viewer h1').text()).toBe('Two');
  });
});
