import { marked } from 'marked';
import { markedHighlight } from 'marked-highlight';
import hljs from 'highlight.js';
import { escapeHtml as esc } from './utils.js';

// The line number (and range) is optional: a citation can point at a whole
// file ([repo:version:file]) rather than one location, matching how the
// backend's parse_citations treats a missing line as "no specific line".
const CITATION_RE = /\[([^:\]\s]+):([^:\]\s]+):([^:\]\s]+)(?::(\d+(?:[–\-]\d+)?))?\]/g;

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
  extensions: [
    {
      name: 'harvest-graph',
      level: 'block',
      start(src) { return src.indexOf('```harvest-graph'); },
      tokenizer(src) {
        const match = src.match(/^```harvest-graph\n([\s\S]*?)\n```(?:\n|$)/);
        if (match) {
          return { type: 'harvest-graph', raw: match[0], text: match[1] };
        }
      },
      renderer(token) {
        const encoded = encodeURIComponent(token.text);
        return `<div class="inline-graph" data-graph="${encoded}"></div>\n`;
      },
    },
  ],
  hooks: {
    postprocess(html) {
      return html.replace(/<script[\s\S]*?<\/script>/gi, '');
    },
  },
});

const LINE_RANGE_RE = /^(\d+)(?:[–-](\d+))?$/;

export function renderMarkdown(text, repoUrlMap = {}, citationIndex = {}) {
  const withCitations = text.replace(CITATION_RE, (match, repo, version, file, lineRaw) => {
    let startLine = 0, endLine = null;
    if (lineRaw) {
      const [, startStr, endStr] = lineRaw.match(LINE_RANGE_RE) ?? [null, lineRaw, null];
      startLine = parseInt(startStr, 10);
      endLine = endStr ? parseInt(endStr, 10) : null;
    }
    const key = `${repo}:${version}:${file}:${startLine}`;
    const n = citationIndex[key];
    const rawLabel = lineRaw ? `${repo}:${version}:${file}:${lineRaw}` : `${repo}:${version}:${file}`;
    const label = n != null ? `${n}` : rawLabel;
    const lineDisplay = lineRaw ? (endLine ? `${startLine}-${endLine}` : `${startLine}`) : null;
    const title = lineDisplay ? `${repo} ${version} · ${file}:${lineDisplay}` : `${repo} ${version} · ${file}`;
    const repoUrl = repoUrlMap[repo];
    const fileUrl = repoUrl ? buildFileUrl(repoUrl, version, file, startLine, endLine) : null;
    if (fileUrl) {
      return `<a href="${esc(fileUrl)}" class="citation" target="_blank" rel="noopener noreferrer" title="${esc(title)}">${esc(label)}</a>`;
    }
    const escapedMatch = match.replace(/[<>"&]/g, (c) => ({ '<': '&lt;', '>': '&gt;', '"': '&quot;', '&': '&amp;' }[c]));
    return `<span class="citation" data-citation="${escapedMatch}" title="${esc(title)}">${esc(label)}</span>`;
  });

  return marked.parse(withCitations, { async: false });
}

// `line` falsy (0/null/undefined) means "no specific line" — link to the bare
// file with no anchor, rather than a nonsensical #L0.
export function buildFileUrl(repoUrl, version, file, line = null, endLine = null) {
  const base = normalizeRepoUrl(repoUrl);
  if (!base) return null;
  if (base.includes('gitlab.com') || base.includes('gitlab.')) {
    const anchor = line ? `#L${line}${endLine ? `-${endLine}` : ''}` : '';
    return `${base}/-/blob/${version}/${file}${anchor}`;
  }
  if (base.includes('bitbucket.org')) {
    const anchor = line ? `#lines-${line}${endLine ? `:${endLine}` : ''}` : '';
    return `${base}/src/${version}/${file}${anchor}`;
  }
  // GitHub, and the default for any other/self-hosted git host.
  const anchor = line ? `#L${line}${endLine ? `-L${endLine}` : ''}` : '';
  return `${base}/blob/${version}/${file}${anchor}`;
}

function normalizeRepoUrl(url) {
  if (!url) return null;
  let normalized = url.replace(/^git@([^:]+):/, 'https://$1/');
  normalized = normalized.replace(/\.git$/, '');
  return normalized;
}

export function formatCitation({ repo, version, file, line }) {
  const filename = file.split('/').pop();
  return `${repo} ${version} · ${filename}:${line}`;
}

export function buildCitationIndex(sources) {
  const index = {};
  (sources ?? []).forEach((src, i) => {
    index[`${src.repo}:${src.version}:${src.file}:${src.line}`] = i + 1;
  });
  return index;
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
