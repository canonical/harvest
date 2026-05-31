import { describe, it, expect } from 'vitest';
import {
  createDocState,
  setRepos,
  selectRepo,
  selectVersion,
  setDocIndex,
  toggleSection,
  selectPage,
  setPageContent,
  setError,
  getVersionsForRepo,
  SECTION_LABELS,
  SECTION_ORDER,
  renderDocPage,
} from '../src/documentation.js';

// ── State management ──────────────────────────────────────────────────────────

describe('createDocState', () => {
  it('starts with no selection', () => {
    const s = createDocState();
    expect(s.selectedRepo).toBeNull();
    expect(s.selectedVersion).toBeNull();
    expect(s.docIndex).toBeNull();
    expect(s.repos).toEqual([]);
  });

  it('starts with tutorials section expanded', () => {
    const s = createDocState();
    expect(s.expandedSections.has('tutorials')).toBe(true);
  });
});

describe('setRepos', () => {
  it('replaces the repos list', () => {
    const s = setRepos(createDocState(), [{ name: 'repo-a', versions: ['v1.0'] }]);
    expect(s.repos).toHaveLength(1);
    expect(s.repos[0].name).toBe('repo-a');
  });
});

describe('selectRepo', () => {
  it('sets selectedRepo and clears version, docIndex, file', () => {
    let s = createDocState();
    s = setRepos(s, [{ name: 'myrepo', versions: ['v1.0'] }]);
    s = selectVersion(s, 'v1.0');
    s = selectRepo(s, 'myrepo');
    expect(s.selectedRepo).toBe('myrepo');
    expect(s.selectedVersion).toBeNull();
    expect(s.docIndex).toBeNull();
    expect(s.selectedFile).toBeNull();
  });
});

describe('selectVersion', () => {
  it('sets selectedVersion and clears docIndex and file', () => {
    let s = createDocState();
    s = selectVersion(s, 'v2.0');
    expect(s.selectedVersion).toBe('v2.0');
    expect(s.docIndex).toBeNull();
    expect(s.selectedFile).toBeNull();
  });
});

describe('setDocIndex', () => {
  it('stores the doc index and clears loading and error', () => {
    let s = { ...createDocState(), loading: true, error: 'old error' };
    const index = {
      repo: 'myrepo', version: 'v1.0',
      sections: { tutorials: [{ filename: 'a.md', title: 'A' }], 'how-to-guides': [], explanations: [], reference: [] },
    };
    s = setDocIndex(s, index);
    expect(s.docIndex).toBe(index);
    expect(s.loading).toBe(false);
    expect(s.error).toBeNull();
  });
});

describe('toggleSection', () => {
  it('collapses an expanded section', () => {
    let s = createDocState(); // tutorials is expanded
    s = toggleSection(s, 'tutorials');
    expect(s.expandedSections.has('tutorials')).toBe(false);
  });

  it('expands a collapsed section', () => {
    let s = createDocState();
    s = toggleSection(s, 'reference');
    expect(s.expandedSections.has('reference')).toBe(true);
  });

  it('does not mutate the original state', () => {
    const original = createDocState();
    toggleSection(original, 'tutorials');
    expect(original.expandedSections.has('tutorials')).toBe(true);
  });
});

describe('selectPage', () => {
  it('sets section and filename, clears content, starts loading', () => {
    let s = createDocState();
    s = selectPage(s, 'tutorials', 'getting-started.md');
    expect(s.selectedSection).toBe('tutorials');
    expect(s.selectedFile).toBe('getting-started.md');
    expect(s.pageContent).toBeNull();
    expect(s.loading).toBe(true);
  });
});

describe('setPageContent', () => {
  it('stores content and clears loading', () => {
    let s = { ...createDocState(), loading: true };
    s = setPageContent(s, '# Hello');
    expect(s.pageContent).toBe('# Hello');
    expect(s.loading).toBe(false);
  });
});

describe('setError', () => {
  it('stores error message and clears loading', () => {
    let s = { ...createDocState(), loading: true };
    s = setError(s, 'something went wrong');
    expect(s.error).toBe('something went wrong');
    expect(s.loading).toBe(false);
  });
});

// ── getVersionsForRepo ────────────────────────────────────────────────────────

describe('getVersionsForRepo', () => {
  it('returns empty when no repo selected', () => {
    const s = setRepos(createDocState(), [{ name: 'r', versions: ['v1.0'] }]);
    expect(getVersionsForRepo(s)).toEqual([]);
  });

  it('returns versions for the selected repo', () => {
    let s = setRepos(createDocState(), [
      { name: 'repo-a', versions: ['v1.0', 'v2.0'] },
      { name: 'repo-b', versions: ['v3.0'] },
    ]);
    s = selectRepo(s, 'repo-a');
    expect(getVersionsForRepo(s)).toEqual(['v1.0', 'v2.0']);
  });
});

// ── SECTION_LABELS and SECTION_ORDER ─────────────────────────────────────────

describe('SECTION_ORDER', () => {
  it('contains all four Diataxis sections', () => {
    expect(SECTION_ORDER).toContain('tutorials');
    expect(SECTION_ORDER).toContain('how-to-guides');
    expect(SECTION_ORDER).toContain('explanations');
    expect(SECTION_ORDER).toContain('reference');
    expect(SECTION_ORDER).toHaveLength(4);
  });
});

describe('SECTION_LABELS', () => {
  it('has human-readable labels for all sections', () => {
    for (const s of SECTION_ORDER) {
      expect(SECTION_LABELS[s]).toBeTruthy();
    }
  });
});

// ── renderDocPage ─────────────────────────────────────────────────────────────

function makeIndexState(overrides = {}) {
  let s = createDocState();
  s = setRepos(s, [{ name: 'testrepo', versions: ['v1.0', 'v2.0'] }]);
  s = selectRepo(s, 'testrepo');
  s = selectVersion(s, 'v1.0');
  s = setDocIndex(s, {
    repo: 'testrepo',
    version: 'v1.0',
    sections: {
      tutorials: [{ filename: 'getting-started.md', title: 'Getting Started' }],
      'how-to-guides': [{ filename: 'install.md', title: 'How to Install' }],
      explanations: [],
      reference: [{ filename: 'api.md', title: 'API Reference' }],
    },
  });
  return { ...s, ...overrides };
}

describe('renderDocPage', () => {
  it('renders repo selector with repos', () => {
    const s = setRepos(createDocState(), [{ name: 'myrepo', versions: [] }]);
    const html = renderDocPage(s);
    expect(html).toContain('myrepo');
    expect(html).toContain('doc-repo-select');
  });

  it('renders version selector disabled when no repo selected', () => {
    const s = createDocState();
    const html = renderDocPage(s);
    expect(html).toContain('disabled');
  });

  it('renders version selector enabled when repo is selected', () => {
    let s = setRepos(createDocState(), [{ name: 'r', versions: ['v1.0'] }]);
    s = selectRepo(s, 'r');
    const html = renderDocPage(s);
    expect(html).not.toContain('id="doc-version-select" disabled');
  });

  it('renders empty-tree hint when nothing selected', () => {
    const html = renderDocPage(createDocState());
    expect(html).toContain('Select a repository');
  });

  it('renders all four section headings when docIndex is loaded', () => {
    const s = makeIndexState();
    const html = renderDocPage(s);
    expect(html).toContain('Tutorials');
    expect(html).toContain('How-to Guides');
    expect(html).toContain('Explanations');
    expect(html).toContain('Reference');
  });

  it('renders page titles in the tree', () => {
    const s = makeIndexState();
    const html = renderDocPage(s);
    expect(html).toContain('Getting Started');
    expect(html).toContain('How to Install');
    expect(html).toContain('API Reference');
  });

  it('renders data-section and data-filename on page buttons', () => {
    const s = makeIndexState();
    const html = renderDocPage(s);
    expect(html).toContain('data-section="tutorials"');
    expect(html).toContain('data-filename="getting-started.md"');
  });

  it('marks the active page with aria-current="page"', () => {
    let s = makeIndexState();
    s = selectPage(s, 'tutorials', 'getting-started.md');
    s = setPageContent(s, '# Getting Started');
    const html = renderDocPage(s);
    expect(html).toContain('aria-current="page"');
  });

  it('renders page count per section', () => {
    const s = makeIndexState();
    const html = renderDocPage(s);
    // tutorials: 1, how-to-guides: 1, explanations: 0, reference: 1
    expect(html).toContain('doc-tree__section-count');
  });

  it('collapses sections not in expandedSections', () => {
    let s = makeIndexState();
    s = toggleSection(s, 'tutorials'); // collapse tutorials
    const html = renderDocPage(s);
    // The tutorials list should be hidden
    expect(html).toContain('doc-tree__pages--collapsed');
  });

  it('renders welcome message when doc loaded but no page selected', () => {
    const s = makeIndexState();
    const html = renderDocPage(s);
    expect(html).toContain('doc-welcome');
    expect(html).toContain('testrepo');
    expect(html).toContain('v1.0');
  });

  it('renders loading dots while page content is loading', () => {
    let s = makeIndexState();
    s = selectPage(s, 'tutorials', 'getting-started.md'); // sets loading: true
    const html = renderDocPage(s);
    expect(html).toContain('loading-dots');
  });

  it('renders error message on error state', () => {
    let s = makeIndexState();
    s = selectPage(s, 'tutorials', 'missing.md');
    s = setError(s, 'Page not found.');
    const html = renderDocPage(s);
    expect(html).toContain('Page not found.');
  });

  it('renders markdown content when page is loaded', () => {
    let s = makeIndexState();
    s = selectPage(s, 'tutorials', 'getting-started.md');
    s = setPageContent(s, '# Getting Started\n\nHello world!');
    const html = renderDocPage(s);
    expect(html).toContain('doc-article');
    expect(html).toContain('Getting Started');
  });

  it('escapes HTML in repo and version names', () => {
    let s = createDocState();
    s = setRepos(s, [{ name: '<evil>', versions: ['<v1>'] }]);
    s = selectRepo(s, '<evil>');
    const html = renderDocPage(s);
    expect(html).not.toContain('<evil>');
    expect(html).toContain('&lt;evil&gt;');
  });
});
