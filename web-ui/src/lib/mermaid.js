const MERMAID_CDN = 'https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.esm.min.mjs';

let mermaidInstance = null;
let loadingPromise = null;

export async function loadMermaid() {
  if (mermaidInstance) return mermaidInstance;
  if (loadingPromise) {
    await loadingPromise;
    return mermaidInstance;
  }
  loadingPromise = (async () => {
    const mod = await import(/* @vite-ignore */ MERMAID_CDN);
    const mermaid = mod.default;
    mermaid.initialize({
      startOnLoad: false,
      theme: 'default',
      securityLevel: 'strict',
      fontFamily: 'Ubuntu, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
    });
    mermaidInstance = mermaid;
  })();
  await loadingPromise;
  return mermaidInstance;
}

export async function mountMermaidDiagrams(containerEl) {
  const blocks = containerEl.querySelectorAll('pre > code.language-mermaid:not(.mermaid-mounted)');

  for (const codeEl of blocks) {
    codeEl.classList.add('mermaid-mounted');
    const preEl = codeEl.parentElement;
    const source = codeEl.textContent;

    try {
      const mermaid = await loadMermaid();
      const id = `mermaid-${Math.random().toString(36).slice(2, 10)}`;
      const { svg } = await mermaid.render(id, source);
      const wrapper = document.createElement('div');
      wrapper.className = 'mermaid-diagram';
      wrapper.innerHTML = svg;
      preEl.replaceWith(wrapper);
    } catch {
      codeEl.classList.remove('mermaid-mounted');
    }
  }
}
