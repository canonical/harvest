import { describe, it, expect } from 'vitest';

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

  it('does not re-process already mounted blocks', async () => {
    const container = makeContainer(`
      <pre><code class="language-mermaid mermaid-mounted">already processed</code></pre>
    `);

    await mountMermaidDiagrams(container);
    const code = container.querySelector('code');
    expect(code.classList.contains('mermaid-mounted')).toBe(true);
  });
});
