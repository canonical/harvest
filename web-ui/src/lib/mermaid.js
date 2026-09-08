const MERMAID_CDN = 'https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.esm.min.mjs';

let mermaidInstance = null;
let loadingPromise = null;
let initializedTheme = null;
const mountedDiagrams = new Set();

function isDarkTheme() {
  const attr = document.documentElement.getAttribute('data-theme');
  if (attr === 'dark') return true;
  if (attr === 'light') return false;
  return typeof window.matchMedia === 'function' && window.matchMedia('(prefers-color-scheme: dark)').matches;
}

async function loadMermaidModule() {
  if (mermaidInstance) return mermaidInstance;
  if (loadingPromise) {
    await loadingPromise;
    return mermaidInstance;
  }
  loadingPromise = (async () => {
    const mod = await import(/* @vite-ignore */ MERMAID_CDN);
    mermaidInstance = mod.default;
  })();
  await loadingPromise;
  return mermaidInstance;
}

export async function loadMermaid() {
  const mermaid = await loadMermaidModule();
  const theme = isDarkTheme() ? 'dark' : 'default';
  if (theme !== initializedTheme) {
    mermaid.initialize({
      startOnLoad: false,
      theme,
      securityLevel: 'strict',
      fontFamily: 'Ubuntu, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
    });
    initializedTheme = theme;
  }
  return mermaid;
}

async function renderInto(wrapperEl, source) {
  const mermaid = await loadMermaid();
  const id = `mermaid-${Math.random().toString(36).slice(2, 10)}`;
  const { svg } = await mermaid.render(id, source);
  wrapperEl.innerHTML = svg;
}

export async function mountMermaidDiagrams(containerEl) {
  const blocks = containerEl.querySelectorAll('pre > code.language-mermaid:not(.mermaid-mounted)');

  for (const codeEl of blocks) {
    codeEl.classList.add('mermaid-mounted');
    const preEl = codeEl.parentElement;
    const source = codeEl.textContent;
    const wrapper = document.createElement('div');
    wrapper.className = 'mermaid-diagram';

    try {
      await renderInto(wrapper, source);
      preEl.replaceWith(wrapper);
      mountedDiagrams.add({ wrapper, source });
    } catch {
      codeEl.classList.remove('mermaid-mounted');
    }
  }
}

export async function rethemeMermaidDiagrams() {
  for (const entry of mountedDiagrams) {
    if (!entry.wrapper.isConnected) {
      mountedDiagrams.delete(entry);
      continue;
    }
    try {
      await renderInto(entry.wrapper, entry.source);
    } catch {}
  }
}
