import { fetchDocIndex, fetchDocPage } from './api.js';
import { renderMarkdown } from './markdown.js';

// ── State ─────────────────────────────────────────────────────────────────────

export function createDocState() {
  return {
    repos: [],
    selectedRepo: null,
    selectedVersion: null,
    docIndex: null,       // { repo, version, sections: { tutorials, how-to-guides, explanations, reference } }
    expandedSections: new Set(['tutorials']),
    selectedSection: null,
    selectedFile: null,
    pageContent: null,    // raw markdown string
    loading: false,
    error: null,
  };
}

export function setRepos(state, repos) {
  return { ...state, repos };
}

export function selectRepo(state, repo) {
  return {
    ...state,
    selectedRepo: repo,
    selectedVersion: null,
    docIndex: null,
    selectedSection: null,
    selectedFile: null,
    pageContent: null,
    error: null,
  };
}

export function selectVersion(state, version) {
  return {
    ...state,
    selectedVersion: version,
    docIndex: null,
    selectedSection: null,
    selectedFile: null,
    pageContent: null,
    error: null,
  };
}

export function setDocIndex(state, docIndex) {
  return { ...state, docIndex, loading: false, error: null };
}

export function toggleSection(state, section) {
  const expanded = new Set(state.expandedSections);
  if (expanded.has(section)) {
    expanded.delete(section);
  } else {
    expanded.add(section);
  }
  return { ...state, expandedSections: expanded };
}

export function selectPage(state, section, filename) {
  return { ...state, selectedSection: section, selectedFile: filename, pageContent: null, loading: true, error: null };
}

export function setPageContent(state, content) {
  return { ...state, pageContent: content, loading: false };
}

export function setLoading(state, loading) {
  return { ...state, loading };
}

export function setError(state, error) {
  return { ...state, error, loading: false };
}

// ── Derived helpers ───────────────────────────────────────────────────────────

export function getVersionsForRepo(state) {
  if (!state.selectedRepo) return [];
  const repo = state.repos.find(r => r.name === state.selectedRepo);
  return repo ? repo.versions : [];
}

export const SECTION_LABELS = {
  'tutorials': 'Tutorials',
  'how-to-guides': 'How-to Guides',
  'explanations': 'Explanations',
  'reference': 'Reference',
};

export const SECTION_ORDER = ['tutorials', 'how-to-guides', 'explanations', 'reference'];

// ── HTML renderers ────────────────────────────────────────────────────────────

export function renderDocPage(state) {
  const repoOptions = state.repos
    .map(r => `<option value="${esc(r.name)}" ${state.selectedRepo === r.name ? 'selected' : ''}>${esc(r.name)}</option>`)
    .join('');

  const versions = getVersionsForRepo(state);
  const versionOptions = versions
    .map(v => `<option value="${esc(v)}" ${state.selectedVersion === v ? 'selected' : ''}>${esc(v)}</option>`)
    .join('');

  const tree = state.docIndex ? renderTree(state) : renderEmptyTree(state);
  const content = renderContent(state);

  return `
    <div class="doc-toolbar">
      <div class="doc-selector">
        <label class="doc-selector__label" for="doc-repo-select">
          <svg width="13" height="13" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true"><path d="M2 2.5A2.5 2.5 0 0 1 4.5 0h8.75a.75.75 0 0 1 .75.75v12.5a.75.75 0 0 1-.75.75h-2.5a.75.75 0 0 1 0-1.5h1.75v-2h-8a1 1 0 0 0-.714 1.7.75.75 0 1 1-1.072 1.05A2.495 2.495 0 0 1 2 11.5Zm10.5-1h-8a1 1 0 0 0-1 1v6.708A2.486 2.486 0 0 1 4.5 9h8Z"/></svg>
          Repository
        </label>
        <select class="doc-select" id="doc-repo-select" aria-label="Select repository">
          <option value="">Choose a repository…</option>
          ${repoOptions}
        </select>
      </div>
      <svg class="doc-selector__sep" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="5.5 3.5 10.5 8 5.5 12.5"/></svg>
      <div class="doc-selector">
        <label class="doc-selector__label" for="doc-version-select">
          <svg width="13" height="13" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true"><path d="M1 7.775V2.75C1 1.784 1.784 1 2.75 1h5.025c.464 0 .91.184 1.238.513l6.25 6.25a1.75 1.75 0 0 1 0 2.474l-5.026 5.026a1.75 1.75 0 0 1-2.474 0l-6.25-6.25A1.752 1.752 0 0 1 1 7.775Zm1.5 0c0 .066.026.13.073.177l6.25 6.25a.25.25 0 0 0 .354 0l5.025-5.025a.25.25 0 0 0 0-.354l-6.25-6.25a.25.25 0 0 0-.177-.073H2.75a.25.25 0 0 0-.25.25ZM6 5a1 1 0 1 1 0 2 1 1 0 0 1 0-2Z"/></svg>
          Version
        </label>
        <select class="doc-select" id="doc-version-select" aria-label="Select version" ${!state.selectedRepo ? 'disabled' : ''}>
          <option value="">Choose a version…</option>
          ${versionOptions}
        </select>
      </div>
    </div>
    <div class="doc-layout">
      <nav class="doc-tree" aria-label="Documentation sections">
        ${tree}
      </nav>
      <main class="doc-content" id="doc-content-area">
        ${content}
      </main>
    </div>
  `;
}

function renderEmptyTree(state) {
  if (!state.selectedRepo || !state.selectedVersion) {
    return `<p class="doc-tree__hint">Select a repository and version to browse documentation.</p>`;
  }
  if (state.loading) {
    return `<div class="loading-dots"><span></span><span></span><span></span></div>`;
  }
  if (state.error) {
    return `<p class="doc-tree__hint doc-tree__hint--error">${esc(state.error)}</p>`;
  }
  return `<p class="doc-tree__hint">No documentation generated yet for ${esc(state.selectedRepo)}:${esc(state.selectedVersion)}.</p>`;
}

function renderTree(state) {
  const { docIndex, expandedSections, selectedSection, selectedFile } = state;
  return SECTION_ORDER.map(section => {
    const pages = docIndex.sections[section] ?? [];
    const isExpanded = expandedSections.has(section);
    const label = SECTION_LABELS[section] ?? section;
    const pageItems = pages.map(({ filename, title }) => {
      const isActive = selectedSection === section && selectedFile === filename;
      return `
        <li class="doc-tree__page ${isActive ? 'doc-tree__page--active' : ''}">
          <button
            class="doc-tree__page-btn"
            data-section="${esc(section)}"
            data-filename="${esc(filename)}"
            aria-current="${isActive ? 'page' : 'false'}"
          >${esc(title)}</button>
        </li>
      `;
    }).join('');

    return `
      <div class="doc-tree__section">
        <button
          class="doc-tree__section-btn ${isExpanded ? 'doc-tree__section-btn--expanded' : ''}"
          data-section-toggle="${esc(section)}"
          aria-expanded="${isExpanded}"
        >
          <span class="doc-tree__section-chevron">${isExpanded ? '▾' : '▸'}</span>
          ${esc(label)}
          <span class="doc-tree__section-count">${pages.length}</span>
        </button>
        <ul class="doc-tree__pages ${isExpanded ? '' : 'doc-tree__pages--collapsed'}" aria-hidden="${!isExpanded}">
          ${pageItems || `<li class="doc-tree__empty">No pages</li>`}
        </ul>
      </div>
    `;
  }).join('');
}

function renderContent(state) {
  if (!state.selectedFile) {
    if (state.docIndex) {
      const total = SECTION_ORDER.reduce((n, s) => n + (state.docIndex.sections[s]?.length ?? 0), 0);
      return `
        <div class="doc-welcome">
          <h2 class="doc-welcome__title">${esc(state.docIndex.repo)} ${esc(state.docIndex.version)}</h2>
          <p class="doc-welcome__body">
            ${total} page${total !== 1 ? 's' : ''} of documentation available.
            Select a page from the sidebar to read it.
          </p>
        </div>
      `;
    }
    return `
      <div class="doc-welcome">
        <p class="doc-welcome__icon">📖</p>
        <p class="doc-welcome__body">Select a repository and version, then browse the documentation sections on the left.</p>
      </div>
    `;
  }
  if (state.loading) {
    return `<div class="doc-content-loading"><div class="loading-dots"><span></span><span></span><span></span></div></div>`;
  }
  if (state.error) {
    return `<div class="doc-content-error"><strong>Error:</strong> ${esc(state.error)}</div>`;
  }
  if (state.pageContent) {
    return `
      <article class="doc-article">
        <div class="doc-article__body">${renderMarkdown(state.pageContent, {}, {})}</div>
      </article>
    `;
  }
  return '';
}

function esc(str) {
  return String(str ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

// ── Page controller ───────────────────────────────────────────────────────────

export function initDocumentationPage(containerEl, initialRepos) {
  let state = createDocState();
  state = setRepos(state, initialRepos);

  function rerender() {
    containerEl.innerHTML = renderDocPage(state);
    attachListeners();
  }

  function attachListeners() {
    const repoSelect = containerEl.querySelector('#doc-repo-select');
    const versionSelect = containerEl.querySelector('#doc-version-select');

    repoSelect?.addEventListener('change', () => {
      state = selectRepo(state, repoSelect.value || null);
      rerender();
    });

    versionSelect?.addEventListener('change', async () => {
      if (!versionSelect.value) return;
      state = selectVersion(state, versionSelect.value);
      state = setLoading(state, true);
      rerender();
      try {
        const index = await fetchDocIndex(state.selectedRepo, state.selectedVersion);
        if (index) {
          state = setDocIndex(state, index);
        } else {
          state = setError(state, 'No documentation generated yet for this version.');
        }
      } catch (e) {
        state = setError(state, e.message);
      }
      rerender();
    });

    containerEl.querySelectorAll('[data-section-toggle]').forEach(btn => {
      btn.addEventListener('click', () => {
        state = toggleSection(state, btn.dataset.sectionToggle);
        rerender();
      });
    });

    containerEl.querySelectorAll('[data-section][data-filename]').forEach(btn => {
      btn.addEventListener('click', async () => {
        const { section, filename } = btn.dataset;
        state = selectPage(state, section, filename);
        rerender();
        try {
          const content = await fetchDocPage(
            state.selectedRepo,
            state.selectedVersion,
            section,
            filename,
          );
          state = content !== null
            ? setPageContent(state, content)
            : setError(state, 'Page not found.');
        } catch (e) {
          state = setError(state, e.message);
        }
        rerender();
      });
    });
  }

  rerender();
  return {
    update(repos) {
      state = setRepos(state, repos);
      rerender();
    },
  };
}
