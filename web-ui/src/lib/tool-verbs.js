const RUNNING_VERBS = {
  create_lxd_agent:    'Creating…',
  delete_agent:        'Deleting…',
  create_port_forward: 'Creating…',
  update_port_forward: 'Updating…',
  delete_port_forward: 'Deleting…',
};

export function runningVerb(name) {
  return RUNNING_VERBS[name] ?? 'Working…';
}

const MAX_QUERY_CHARS = 60;
const MAX_LISTED_ITEMS = 3;

function truncate(text, max) {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

function baseName(path) {
  return String(path).split('/').pop();
}

function shortVersion(version) {
  return version ? String(version).split('/').pop() : '';
}

function listOf(items) {
  const names = items.slice(0, MAX_LISTED_ITEMS);
  const more = items.length - names.length;
  return more > 0 ? `${names.join(', ')} and ${more} more` : names.join(', ');
}

function scopeOf(input) {
  return [input.repo, shortVersion(input.version)].filter(Boolean).join(' ');
}

function quoted(text) {
  return `“${truncate(String(text).trim(), MAX_QUERY_CHARS)}”`;
}

function describeSearchCode(input) {
  const focus = Array.isArray(input.focus) ? input.focus : [];
  if (focus.length) return `Reading ${listOf(focus.map(baseName))}`;
  const scope = scopeOf(input);
  return input.query ? `Searching ${scope || 'the code'} for ${quoted(input.query)}` : `Searching ${scope || 'the code'}`;
}

function describeReadSources(input) {
  const files = (input.files ?? []).map(baseName);
  const names = input.names ?? [];
  const targets = [...files, ...names];
  return targets.length ? `Reading ${listOf(targets)}` : 'Reading source code';
}

const TOOL_DESCRIPTIONS = {
  search_code:                    describeSearchCode,
  read_sources:                   describeReadSources,
  list_repositories:              () => 'Listing indexed repositories',
  search_symbols:                 input => input.query ? `Searching symbols for ${quoted(input.query)}` : 'Searching symbols',
  get_symbol_source:              input => `Reading ${input.name ?? 'a symbol'}`,
  get_file_symbols:               input => `Listing the symbols in ${baseName(input.file ?? 'a file')}`,
  get_imports:                    input => `Reading the imports of ${baseName(input.file ?? 'a file')}`,
  find_callers:                   input => `Finding callers of ${input.function_name ?? 'a function'}`,
  find_callees:                   input => `Finding what ${input.function_name ?? 'a function'} calls`,
  find_subclasses:                input => `Listing subclasses of ${input.class ?? 'a class'}`,
  get_capability_matrix:          input => `Checking ${input.capability ?? 'a capability'} across classes`,
  get_evidence_pack:              () => 'Gathering evidence from the code graph',
  compare_symbol_across_versions: input => `Comparing ${input.name ?? 'a symbol'} between ${shortVersion(input.version_a)} and ${shortVersion(input.version_b)}`,
  run_sql:                        () => 'Querying the code graph',
  list_agents:                    () => 'Listing agents',
  run_command:                    input => input.command ? `Running ${quoted(input.command)}` : 'Running a command',
  list_skills:                    () => 'Listing skills',
  load_skill:                     input => input.name ? `Loading the ${input.name} skill` : 'Loading a skill',
  generate_artifact:              () => 'Generating an artifact',
  propose_parallel_research:      () => 'Splitting the research into parallel leads',
};

function humanize(name) {
  const words = String(name ?? 'tool').replace(/_/g, ' ');
  return `Running ${words}`;
}

export function describeToolCall(name, input = {}) {
  const describe = TOOL_DESCRIPTIONS[name];
  return describe ? describe(input ?? {}) : humanize(name);
}
