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
 * Citation brackets like [repo:v1:file.rs:42] are converted to
 * highlighted <span data-citation> elements before parsing.
 *
 * @param {string} text
 * @returns {string} HTML
 */
export function renderMarkdown(text) {
  // Replace citations with a highlighted span before markdown parsing
  const withCitations = text.replace(CITATION_RE, (match, repo, version, file, line) => {
    const escaped = match.replace(/[<>"&]/g, (c) => ({ '<': '&lt;', '>': '&gt;', '"': '&quot;', '&': '&amp;' }[c]));
    return `<span class="citation" data-citation="${escaped}" title="${repo}:${version}:${file}:${line}">[${repo}:${version}:${file}:${line}]</span>`;
  });

  return marked.parse(withCitations, { async: false });
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
