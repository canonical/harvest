const KINDS = [
  ['diff --git', 'diff-file'],
  ['+++', 'diff-file'],
  ['---', 'diff-file'],
  ['@@', 'diff-hunk'],
  ['+', 'diff-add'],
  ['-', 'diff-del'],
];

export function classifyDiffLines(diff) {
  return diff.split('\n').map((text) => {
    const match = KINDS.find(([prefix]) => text.startsWith(prefix));
    return { text, kind: match ? match[1] : 'diff-context' };
  });
}

const METADATA = /^(index |new file mode|deleted file mode|old mode|new mode|similarity index|rename from|rename to|Binary files)/;

function pathOf(header) {
  const match = header.match(/^diff --git a\/(.+?) b\/(.+)$/);
  return match ? match[2] : header.replace(/^diff --git\s+/, '');
}

export function diffFiles(diff) {
  const files = [];
  for (const line of classifyDiffLines(diff)) {
    if (line.text.startsWith('diff --git')) {
      files.push({ path: pathOf(line.text), added: 0, removed: 0, lines: [] });
      continue;
    }
    const file = files[files.length - 1];
    if (!file) continue;
    if (line.kind === 'diff-file' || METADATA.test(line.text)) continue;
    if (line.kind === 'diff-add') file.added += 1;
    if (line.kind === 'diff-del') file.removed += 1;
    file.lines.push(line);
  }
  return files;
}

export function diffSummary(commits, files) {
  const added = files.reduce((sum, f) => sum + f.added, 0);
  const removed = files.reduce((sum, f) => sum + f.removed, 0);
  const plural = (n, word) => `${n} ${word}${n === 1 ? '' : 's'}`;
  return `${plural(commits.length, 'commit')} · ${plural(files.length, 'file')} · +${added} −${removed}`;
}
