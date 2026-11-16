export const GROUPABLE_MIN_RUN = 3;

export function groupChain(chain) {
  const rows = [];
  let i = 0;
  while (i < chain.length) {
    const item = chain[i];
    if (item.type !== 'tool_call') {
      rows.push(item);
      i++;
      continue;
    }
    let j = i + 1;
    while (j < chain.length && chain[j].type === 'tool_call' && chain[j].name === item.name) j++;
    const run = chain.slice(i, j);
    if (run.length >= GROUPABLE_MIN_RUN) {
      rows.push({ type: 'tool_group', id: `group-${item.id ?? i}`, items: run });
    } else {
      rows.push(...run);
    }
    i = j;
  }
  return rows;
}
