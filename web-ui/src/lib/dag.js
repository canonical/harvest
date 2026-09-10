export function topologicalSort(steps) {
  const map = new Map();
  for (const s of steps) map.set(s.id, s);

  const inDegree = new Map();
  for (const s of steps) inDegree.set(s.id, 0);
  for (const s of steps) {
    for (const dep of s.depends_on ?? []) {
      if (map.has(dep)) {
        inDegree.set(s.id, inDegree.get(s.id) + 1);
      }
    }
  }

  const queue = [];
  for (const [id, deg] of inDegree) {
    if (deg === 0) queue.push(id);
  }
  queue.sort((a, b) => {
    const sa = map.get(a);
    const sb = map.get(b);
    return (sa.step_index ?? 0) - (sb.step_index ?? 0);
  });

  const visited = new Set();
  const result = [];

  while (queue.length > 0) {
    const id = queue.shift();
    if (visited.has(id)) continue;
    visited.add(id);
    result.push(map.get(id));

    for (const s of steps) {
      if ((s.depends_on ?? []).includes(id)) {
        const newDeg = inDegree.get(s.id) - 1;
        inDegree.set(s.id, newDeg);
        if (newDeg === 0 && !visited.has(s.id)) {
          queue.push(s.id);
          queue.sort((a, b) => {
            const sa = map.get(a);
            const sb = map.get(b);
            return (sa.step_index ?? 0) - (sb.step_index ?? 0);
          });
        }
      }
    }
  }

  const orphaned = steps.filter(s => !visited.has(s.id));
  return [...result, ...orphaned];
}

export function stepNumber(steps, stepId) {
  const idx = steps.findIndex(s => s.id === stepId);
  return idx >= 0 ? idx + 1 : null;
}
