import hljs from 'highlight.js';
import { fetchSymbolSource } from './api.js';
import { langFromPath } from './format.js';
import { escapeHtml as esc } from './utils.js';

const $panel = () => document.getElementById('repo-source-panel');
const $id    = id => document.getElementById(id);

export function initSourcePanel() {
  $id('source-panel-close').addEventListener('click', closeSourcePanel);
  $id('source-panel-expand').addEventListener('click', toggleExpandPanel);
}

export function closeSourcePanel() {
  const panel = $panel();
  panel?.classList.remove('is-open');
  panel?.classList.remove('is-expanded');
  _resetExpandBtn();
}

function toggleExpandPanel() {
  const panel = $panel();
  const expanded = panel?.classList.toggle('is-expanded');
  const btn = $id('source-panel-expand');
  if (!btn) return;
  btn.querySelector('i').className = expanded ? 'p-icon--chevron-right' : 'p-icon--chevron-left';
  btn.setAttribute('aria-label', expanded ? 'Shrink source panel' : 'Expand source panel');
}

function _resetExpandBtn() {
  const btn = $id('source-panel-expand');
  if (!btn) return;
  btn.querySelector('i').className = 'p-icon--chevron-left';
  btn.setAttribute('aria-label', 'Expand source panel');
}

export function openSourcePanel(data, repo, version, sections = []) {
  const kind = (data.kind ?? 'function').toLowerCase();
  const badge = $id('source-kind-badge');
  badge.textContent = kind;
  badge.className = `source-kind-badge source-kind-badge--${kind}`;

  $id('source-symbol-name').textContent = data.label ?? data.name ?? '';
  $id('source-symbol-file').textContent =
    data.file ? `${data.file}  ·  line ${data.start_line ?? '?'}` : '';

  $id('source-code').textContent = '// Loading…';
  $id('source-code').className = '';

  renderSections($id('source-relations'), sections);
  $panel()?.classList.add('is-open');

  if (repo && version && data.file) {
    loadSource(data, repo, version);
  }
}

function renderSections(relEl, sections) {
  const visible = sections.filter(s => s.chips.length > 0);
  relEl.innerHTML = '';
  visible.forEach((s, si) => {
    const section = document.createElement('div');
    section.className = 'source-relations__section';

    const label = document.createElement('div');
    label.className = 'source-relations__label';
    label.textContent = s.label;
    section.appendChild(label);

    const chips = document.createElement('div');
    chips.className = 'source-relations__chips';
    s.chips.forEach(chip => {
      const btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'relation-chip';
      btn.textContent = chip.name;
      if (chip.title) btn.title = chip.title;
      if (chip.onClick) btn.addEventListener('click', chip.onClick);
      chips.appendChild(btn);
    });
    section.appendChild(chips);
    relEl.appendChild(section);
  });
}

async function loadSource(data, repo, version) {
  const codeEl = $id('source-code');
  try {
    const sym = await fetchSymbolSource(repo, version, data.file, data.label ?? data.name);
    if (!sym?.source) { codeEl.textContent = '// Source not available'; return; }
    const lang = langFromPath(data.file);
    if (lang !== 'plaintext') {
      const result = hljs.highlight(sym.source, { language: lang, ignoreIllegals: true });
      codeEl.innerHTML = result.value;
      codeEl.className = `hljs language-${lang}`;
    } else {
      codeEl.textContent = sym.source;
    }
  } catch (err) {
    codeEl.textContent = `// Error: ${err.message}`;
  }
}
