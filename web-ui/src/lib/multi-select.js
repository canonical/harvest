export function selectionSummary(selected, options, { none, noun }) {
  if (!selected.length || selected.length === options.length) return none;
  if (selected.length === 1) return options.find((option) => option.value === selected[0])?.label ?? none;
  return `${selected.length} ${noun}`;
}

export function matchingOptions(options, text) {
  const needle = text.trim().toLowerCase();
  if (!needle) return options;
  return options.filter((option) => option.label.toLowerCase().includes(needle));
}

export function toggled(selected, value) {
  return selected.includes(value) ? selected.filter((item) => item !== value) : [...selected, value];
}
