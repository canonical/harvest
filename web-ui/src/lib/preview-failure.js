export function splitFailure(text) {
  const clean = (text ?? '').replace(/^error:\s*/i, '').trim();
  const breakAt = clean.indexOf('\n\n');
  if (breakAt < 0) return { summary: clean, details: '' };
  return { summary: clean.slice(0, breakAt).trim(), details: clean.slice(breakAt + 2).trim() };
}
