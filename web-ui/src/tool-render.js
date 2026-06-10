import { renderJsonToHtml, renderPreviewToHtml } from './format.js';
import { escapeHtml as esc } from './utils.js';

export function describeToolCall(name, input, { hostname } = {}) {
  switch (name) {
    case 'list_repositories': return 'Discovering available repositories';
    case 'list_agents':       return 'Checking connected agents';
    case 'list_skills':       return 'Listing available skills';
    case 'search_symbols': {
      const q = input.query ?? 'symbols';
      const kind = input.kind && input.kind !== 'any' ? ` ${input.kind}s` : '';
      const scope = input.repo ? ` in ${input.repo}` : '';
      return `Searching for "${q}"${kind}${scope}`;
    }
    case 'get_symbol_source':
      return `Reading source of ${input.name ?? 'symbol'}`;
    case 'get_file_symbols':
      return `Scanning symbols in ${(input.file ?? 'file').split('/').pop()}`;
    case 'find_callers':
      return `Tracing callers of ${input.function_name ?? 'function'}`;
    case 'find_callees':
      return `Tracing calls made by ${input.function_name ?? 'function'}`;
    case 'get_imports':
      return `Checking imports in ${(input.file ?? 'file').split('/').pop()}`;
    case 'compare_symbol_across_versions': {
      const sym = input.name ?? 'symbol';
      return (input.version_a && input.version_b)
        ? `Comparing ${sym} ${input.version_a} → ${input.version_b}`
        : `Comparing ${sym} across versions`;
    }
    case 'run_cypher': {
      const q = typeof input.query === 'string' ? input.query.trim() : '';
      const snippet = q.length > 38 ? q.slice(0, 38).trimEnd() + '…' : q;
      return snippet ? `Graph query: ${snippet}` : 'Running graph query';
    }
    case 'run_command': {
      const cmd = typeof input.command === 'string'
        ? (input.command.length > 28 ? input.command.slice(0, 28).trimEnd() + '…' : input.command)
        : 'command';
      return `Running "${cmd}" on ${hostname ?? input.agent_id ?? 'agent'}`;
    }
    case 'load_skill':  return `Loading skill ${input.name ?? ''}`.trimEnd();
    default:            return name.replace(/_/g, ' ');
  }
}

export function renderToolCall(tc, i) {
  const label = esc(tc.description || describeToolCall(tc.name, tc.input ?? {}));

  const hasDetail = !!(tc.input || tc.preview);
  const detailHtml = hasDetail ? `
    <div class="tc-step__detail" hidden>
      <span class="tc-step__tool-tag">${esc(tc.name)}</span>
      ${tc.input ? `
        <div class="tc-step__detail-section">
          <div class="tc-step__detail-label">Input</div>
          <div class="tool-data">${renderJsonToHtml(tc.input)}</div>
        </div>` : ''}
      ${tc.preview ? `
        <div class="tc-step__detail-section">
          <div class="tc-step__detail-label">Result</div>
          <div class="tool-data">${renderPreviewToHtml(tc.preview, tc.input?.file ?? null)}</div>
        </div>` : ''}
    </div>
  ` : '';

  return `
    <div class="tc-step tc-step--${tc.status}" data-tc-idx="${i}">
      <div class="tc-step__row${hasDetail ? ' tc-step__row--clickable' : ''}" ${hasDetail ? 'role="button" tabindex="0" aria-expanded="false"' : ''}>
        <i class="p-icon--circle-of-friends tc-step__icon${tc.status === 'running' ? ' tc-step__icon--spinning' : ''}" aria-hidden="true"></i>
        <span class="tc-step__label">${label}</span>
        ${hasDetail ? `
          <svg class="tc-step__chevron" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg" aria-hidden="true"><polyline points="2,3 5,7 8,3"/></svg>` : ''}
      </div>
      ${detailHtml}
    </div>
  `;
}

export function attachTcStepHandlers(container) {
  container.querySelectorAll('.tc-step__row--clickable').forEach(row => {
    const toggle = () => {
      const step = row.closest('.tc-step');
      const detail = step.querySelector('.tc-step__detail');
      const willExpand = detail.hidden;
      detail.hidden = !willExpand;
      row.setAttribute('aria-expanded', String(willExpand));
      step.classList.toggle('tc-step--open', willExpand);
    };
    row.addEventListener('click', toggle);
    row.addEventListener('keydown', e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); toggle(); } });
  });
}
