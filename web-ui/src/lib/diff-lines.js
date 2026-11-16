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
