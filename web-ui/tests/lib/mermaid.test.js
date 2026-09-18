import { describe, it, expect, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const renderMock = vi.fn(async (id, source) => {
  if (source.includes('bad syntax')) {
    throw new Error('Parse error');
  }
  return { svg: '<svg data-testid="rendered"></svg>' };
});

vi.mock('mermaid', () => ({
  default: {
    initialize: vi.fn(),
    render: renderMock,
  },
}));

const { mountMermaidDiagrams } = await import('../../src/lib/mermaid.js');

function makeContainer(html) {
  const container = document.createElement('div');
  container.innerHTML = html;
  return container;
}

describe('mountMermaidDiagrams', () => {
  it('does not touch non-mermaid code blocks', async () => {
    const container = makeContainer(`
      <pre><code class="language-rust">fn main() {}</code></pre>
    `);

    await mountMermaidDiagrams(container);
    expect(container.querySelector('svg')).toBeNull();
    expect(container.querySelector('code.language-rust')).toBeTruthy();
  });

  it('handles empty containers gracefully', async () => {
    const container = makeContainer('');
    await mountMermaidDiagrams(container);
  });

  it('handles containers with no code blocks', async () => {
    const container = makeContainer('<p>just text</p>');
    await mountMermaidDiagrams(container);
    expect(container.querySelector('p')).toBeTruthy();
  });

  it('preserves original code block when render fails', async () => {
    const container = makeContainer(`
      <pre><code class="language-mermaid">bad syntax</code></pre>
    `);

    await mountMermaidDiagrams(container);

    const pre = container.querySelector('pre');
    expect(pre).toBeTruthy();
    expect(pre.textContent).toContain('bad syntax');
  });

  it('shows a visible failure note instead of silently leaving raw code', async () => {
    const container = makeContainer(`
      <pre><code class="language-mermaid">bad syntax</code></pre>
    `);

    await mountMermaidDiagrams(container);

    const note = container.querySelector('.mermaid-error-note');
    expect(note).toBeTruthy();
    expect(note.textContent).toMatch(/failed to render/i);
  });

  it('does not stack duplicate failure notes on repeated mount attempts', async () => {
    const container = makeContainer(`
      <pre><code class="language-mermaid">bad syntax</code></pre>
    `);

    await mountMermaidDiagrams(container);
    await mountMermaidDiagrams(container);

    expect(container.querySelectorAll('.mermaid-error-note').length).toBe(1);
  });

  it('does not leave any element behind in document.body when render fails', async () => {
    const bodyChildrenBefore = document.body.children.length;
    const container = makeContainer(`
      <pre><code class="language-mermaid">bad syntax</code></pre>
    `);

    await mountMermaidDiagrams(container);

    expect(document.body.children.length).toBe(bodyChildrenBefore);
  });

  it('attaches the sandbox to document.body during render (so getBBox works), then removes it', async () => {
    renderMock.mockClear();
    let sandboxWasConnectedDuringRender = null;
    renderMock.mockImplementationOnce(async (id, source, sandbox) => {
      sandboxWasConnectedDuringRender = sandbox.isConnected;
      return { svg: '<svg data-testid="rendered"></svg>' };
    });
    const container = makeContainer(`
      <pre><code class="language-mermaid">graph TD; A-->B;</code></pre>
    `);

    await mountMermaidDiagrams(container);

    expect(renderMock).toHaveBeenCalled();
    const sandboxArg = renderMock.mock.calls[0][2];
    expect(sandboxArg).toBeInstanceOf(HTMLElement);
    expect(sandboxWasConnectedDuringRender).toBe(true);
    expect(sandboxArg.isConnected).toBe(false);
  });

  it('does not re-process already mounted blocks', async () => {
    const container = makeContainer(`
      <pre><code class="language-mermaid mermaid-mounted">already processed</code></pre>
    `);

    await mountMermaidDiagrams(container);
    const code = container.querySelector('code');
    expect(code.classList.contains('mermaid-mounted')).toBe(true);
  });
});

describe('mermaid module loading', () => {
  it('never imports mermaid from a third-party URL at runtime', () => {
    const sourcePath = resolve(process.cwd(), 'src/lib/mermaid.js');
    const source = readFileSync(sourcePath, 'utf8');
    expect(source).not.toMatch(/https?:\/\//);
  });
});
