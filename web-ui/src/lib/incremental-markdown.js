import { marked } from 'marked';
import { substituteCitations } from './markdown.js';

export function tokenizeDocument(text, repoUrlMap = {}, citationIndex = {}) {
  if (!text) return [];
  const tokens = marked.lexer(substituteCitations(text, repoUrlMap, citationIndex));
  marked.walkTokens(tokens, marked.defaults.walkTokens);
  return tokens.filter(t => t.type !== 'space');
}

export function renderBlock(token) {
  return marked.parser([token], { async: false });
}
