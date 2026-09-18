import { isDarkTheme } from './theme.js';

let mermaidInstance = null;
let loadingPromise = null;
let initializedTheme = null;
const mountedDiagrams = new Set();

async function loadMermaidModule() {
  if (mermaidInstance) return mermaidInstance;
  if (loadingPromise) {
    await loadingPromise;
    return mermaidInstance;
  }
  loadingPromise = (async () => {
    const mod = await import('mermaid');
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
  const sandbox = document.createElement('div');
  sandbox.style.position = 'absolute';
  sandbox.style.top = '-9999px';
  sandbox.style.left = '-9999px';
  sandbox.style.visibility = 'hidden';
  document.body.appendChild(sandbox);
  try {
    const { svg } = await mermaid.render(id, source, sandbox);
    wrapperEl.innerHTML = svg;
  } finally {
    sandbox.remove();
  }
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
    } catch (err) {
      codeEl.classList.remove('mermaid-mounted');
      const alreadyNoted = preEl.previousElementSibling?.classList?.contains('mermaid-error-note');
      if (!alreadyNoted) {
        const note = document.createElement('div');
        note.className = 'mermaid-error-note';
        note.textContent = 'Diagram failed to render — showing raw source below.';
        preEl.before(note);
      }
      console.error('mermaid diagram failed to render', err);
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
