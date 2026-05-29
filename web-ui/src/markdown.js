import { marked } from 'marked';
import { markedHighlight } from 'marked-highlight';
import hljs from 'highlight.js';

const CITATION_RE = /\[([^:\]\s]+):([^:\]\s]+):([^:\]\s]+):(\d+)\]/g;

// Configure marked with syntax highlighting and safe HTML
marked.use(
  markedHighlight({
    langPrefix: 'language-',
    highlight(code, lang) {
      const language = hljs.getLanguage(lang) ? lang : 'plaintext';
      return hljs.highlight(code, { language }).value;
    },
  }),
);

marked.use({
  renderer: {
    // No custom link renderer; let default handle links safely
  },
  // Sanitize raw HTML to prevent XSS
  hooks: {
    postprocess(html) {
      return html.replace(/<script[\s\S]*?<\/script>/gi, '');
    },
  },
});

/**
 * Render a markdown string to an HTML string.
 * Citation brackets like [repo:v1:file.rs:42] are converted to links
 * when a URL can be resolved from repoUrlMap, otherwise to spans.
 *
 * @param {string} text
 * @param {Object.<string, string>} [repoUrlMap] — map of repo name → clone URL
 * @returns {string} HTML
 */
export function renderMarkdown(text, repoUrlMap = {}) {
  const withCitations = text.replace(CITATION_RE, (match, repo, version, file, line) => {
    const repoUrl = repoUrlMap[repo];
    const fileUrl = repoUrl ? buildFileUrl(repoUrl, version, file, parseInt(line, 10)) : null;
    const title = `${repo}:${version}:${file}:${line}`;
    const label = `[${repo}:${version}:${file}:${line}]`;
    if (fileUrl) {
      return `<a href="${esc(fileUrl)}" class="citation" target="_blank" rel="noopener noreferrer" title="${esc(title)}">${esc(label)}</a>`;
    }
    const escapedMatch = match.replace(/[<>"&]/g, (c) => ({ '<': '&lt;', '>': '&gt;', '"': '&quot;', '&': '&amp;' }[c]));
    return `<span class="citation" data-citation="${escapedMatch}" title="${esc(title)}">${esc(label)}</span>`;
  });

  return marked.parse(withCitations, { async: false });
}

/**
 * Build a URL pointing to a specific file and line in an upstream repository.
 * Handles GitHub, GitLab, and Bitbucket. SSH clone URLs are converted to HTTPS.
 * Returns null if the URL cannot be determined.
 *
 * @param {string} repoUrl  — clone URL (https or ssh)
 * @param {string} version  — tag or branch name
 * @param {string} file     — relative file path
 * @param {number} line
 * @returns {string|null}
 */
export function buildFileUrl(repoUrl, version, file, line) {
  const base = normalizeRepoUrl(repoUrl);
  if (!base) return null;
  if (base.includes('github.com')) {
    return `${base}/blob/${version}/${file}#L${line}`;
  }
  if (base.includes('gitlab.com') || base.includes('gitlab.')) {
    return `${base}/-/blob/${version}/${file}#L${line}`;
  }
  if (base.includes('bitbucket.org')) {
    return `${base}/src/${version}/${file}#lines-${line}`;
  }
  // Generic fallback — works for most Gitea/Forgejo/cgit instances
  return `${base}/blob/${version}/${file}#L${line}`;
}

function normalizeRepoUrl(url) {
  if (!url) return null;
  // Convert SSH format: git@github.com:owner/repo.git → https://github.com/owner/repo
  let normalized = url.replace(/^git@([^:]+):/, 'https://$1/');
  // Strip .git suffix
  normalized = normalized.replace(/\.git$/, '');
  return normalized;
}

function esc(str) {
  return String(str)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

/**
 * Format a source citation object into a short human-readable label.
 *
 * @param {{repo: string, version: string, file: string, line: number}} source
 * @returns {string}
 */
export function formatCitation({ repo, version, file, line }) {
  const filename = file.split('/').pop();
  return `${repo} ${version} · ${filename}:${line}`;
}

/**
 * Parse [repo:version:file:line] citations from text, returning unique sources.
 *
 * @param {string} text
 * @returns {{repo: string, version: string, file: string, line: number}[]}
 */
export function parseCitations(text) {
  const seen = new Set();
  const results = [];

  for (const match of text.matchAll(CITATION_RE)) {
    const [full, repo, version, file, lineStr] = match;
    if (seen.has(full)) continue;
    seen.add(full);
    results.push({ repo, version, file, line: parseInt(lineStr, 10) });
  }

  return results;
}
