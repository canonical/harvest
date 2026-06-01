import { marked } from 'marked';
import { markedHighlight } from 'marked-highlight';
import hljs from 'highlight.js';

const CITATION_RE = /\[([^:\]\s]+):([^:\]\s]+):([^:\]\s]+):(\d+)\]/g;

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
  },
  hooks: {
    postprocess(html) {
      return html.replace(/<script[\s\S]*?<\/script>/gi, '');
    },
  },
});

export function renderMarkdown(text, repoUrlMap = {}, citationIndex = {}) {
  const withCitations = text.replace(CITATION_RE, (match, repo, version, file, line) => {
    const key = `${repo}:${version}:${file}:${line}`;
    const n = citationIndex[key];
    const label = n != null ? `[${n}]` : `[${repo}:${version}:${file}:${line}]`;
    const title = `${repo} ${version} · ${file}:${line}`;
    const repoUrl = repoUrlMap[repo];
    const fileUrl = repoUrl ? buildFileUrl(repoUrl, version, file, parseInt(line, 10)) : null;
    if (fileUrl) {
      return `<a href="${esc(fileUrl)}" class="citation" target="_blank" rel="noopener noreferrer" title="${esc(title)}">${esc(label)}</a>`;
    }
    const escapedMatch = match.replace(/[<>"&]/g, (c) => ({ '<': '&lt;', '>': '&gt;', '"': '&quot;', '&': '&amp;' }[c]));
    return `<span class="citation" data-citation="${escapedMatch}" title="${esc(title)}">${esc(label)}</span>`;
  });

  return marked.parse(withCitations, { async: false });
}

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
  return `${base}/blob/${version}/${file}#L${line}`;
}

function normalizeRepoUrl(url) {
  if (!url) return null;
  let normalized = url.replace(/^git@([^:]+):/, 'https://$1/');
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

export function formatCitation({ repo, version, file, line }) {
  const filename = file.split('/').pop();
  return `${repo} ${version} · ${filename}:${line}`;
}

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
