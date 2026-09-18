import { marked } from 'marked';
import { markedHighlight } from 'marked-highlight';
import hljs from 'highlight.js';
import { escapeHtml as esc } from './utils.js';

// The line number (and range) is optional: a citation can point at a whole
// file ([repo:version:file]) rather than one location, matching how the
// backend's parse_citations treats a missing line as "no specific line".
const BRACKET_RE = /\[([^\[\]]+)\]/g;
const CITATION_ONE_RE = /^([^:\s]+):([^:\s]+):([^:\s]+)(?::(\d+(?:[–-]\d+)?(?:,\d+(?:[–-]\d+)?)*))?$/;

function splitCitationBody(body) {
  const groups = [];
  for (const piece of body.split(',')) {
    const trimmed = piece.trim();
    if ((trimmed.match(/:/g) || []).length >= 2 || groups.length === 0) {
      groups.push(trimmed);
    } else {
      groups[groups.length - 1] = `${groups[groups.length - 1]},${trimmed}`;
    }
  }
  return groups;
}

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

const LINE_RANGE_RE = /^(\d+)(?:[–-](\d+))?/;

function renderOneCitation(repo, version, file, lineRaw, repoUrlMap, citationIndex) {
  let startLine = 0, endLine = null;
  if (lineRaw) {
    const [, startStr, endStr] = lineRaw.match(LINE_RANGE_RE);
    startLine = parseInt(startStr, 10);
    endLine = endStr ? parseInt(endStr, 10) : null;
  }
  const key = `${repo}:${version}:${file}:${startLine}`;
  const n = citationIndex[key];
  const rawLabel = lineRaw ? `${repo}:${version}:${file}:${lineRaw}` : `${repo}:${version}:${file}`;
  const label = n != null ? `${n}` : rawLabel;
  const title = lineRaw ? `${repo} ${version} · ${file}:${lineRaw}` : `${repo} ${version} · ${file}`;
  const repoUrl = repoUrlMap[repo];
  const fileUrl = repoUrl ? buildFileUrl(repoUrl, version, file, startLine, endLine) : null;
  if (fileUrl) {
    return `<a href="${esc(fileUrl)}" class="citation" target="_blank" rel="noopener noreferrer" title="${esc(title)}">${esc(label)}</a>`;
  }
  const escapedMatch = `[${repo}:${version}:${file}${lineRaw ? `:${lineRaw}` : ''}]`.replace(/[<>"&]/g, (c) => ({ '<': '&lt;', '>': '&gt;', '"': '&quot;', '&': '&amp;' }[c]));
  return `<span class="citation" data-citation="${escapedMatch}" title="${esc(title)}">${esc(label)}</span>`;
}

export function substituteCitations(text, repoUrlMap = {}, citationIndex = {}) {
  return text.replace(BRACKET_RE, (match, body) => {
    const groups = splitCitationBody(body).map((g) => g.match(CITATION_ONE_RE));
    if (groups.some((g) => !g)) return match;
    return groups
      .map(([, repo, version, file, lineRaw]) => renderOneCitation(repo, version, file, lineRaw, repoUrlMap, citationIndex))
      .join(', ');
  });
}

export function renderMarkdown(text, repoUrlMap = {}, citationIndex = {}) {
  return marked.parse(substituteCitations(text, repoUrlMap, citationIndex), { async: false });
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

  for (const bracketMatch of text.matchAll(BRACKET_RE)) {
    for (const group of splitCitationBody(bracketMatch[1])) {
      const m = group.match(CITATION_ONE_RE);
      if (!m) continue;
      const [full, repo, version, file, lineStr] = m;
      if (seen.has(full)) continue;
      seen.add(full);
      results.push({ repo, version, file, line: parseInt(lineStr, 10) });
    }
  }

  return results;
}
