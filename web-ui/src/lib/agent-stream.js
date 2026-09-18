import { ref, computed } from 'vue';
import { describeToolCall } from './tool-render.js';
import { renderMarkdown } from './markdown.js';

export function useAgentStream({
  preparingText = 'Preparing…',
  readyText     = 'Ready',
  failedText    = 'Generation failed',
  writingText   = 'Writing the response…',
} = {}) {
  const finished       = ref(false);
  const error          = ref(null);
  const streamText     = ref('');
  const chain          = ref([]);
  const intent         = ref(null);
  const phase          = ref('');
  const elapsedSeconds = ref(0);
  const files          = ref([]);
  const activeFileId   = ref(null);

  let timerId    = null;
  let startedAt  = 0;
  let fileIdSeq  = 0;

  function startTimer() {
    stopTimer();
    startedAt = Date.now();
    elapsedSeconds.value = 0;
    timerId = setInterval(() => {
      elapsedSeconds.value = Math.floor((Date.now() - startedAt) / 1000);
    }, 1000);
  }

  function stopTimer() {
    if (timerId) {
      clearInterval(timerId);
      timerId = null;
    }
  }

  function finalizeThinking() {
    const last = chain.value.at(-1);
    if (last?.type === 'thinking' && last.streaming) last.streaming = false;
  }

  function findParallelBlock() {
    for (let i = chain.value.length - 1; i >= 0; i--) {
      if (chain.value[i].type === 'parallel_research') return chain.value[i];
    }
    return null;
  }

  function completeToolCall(name, preview) {
    const idx = chain.value.findIndex(s => s.type === 'tool_call' && s.name === name && s.status === 'running');
    if (idx !== -1) chain.value[idx] = { ...chain.value[idx], status: 'done', preview };
  }

  function parseTerraformBundle(content, fallbackPath) {
    try {
      const parsed = JSON.parse(content);
      const entries = parsed && typeof parsed === 'object' && !Array.isArray(parsed)
        ? Object.entries(parsed)
        : [];
      if (entries.length) return entries.map(([path, text]) => ({ path, text: String(text) }));
    } catch {}
    return [{ path: fallbackPath, text: content ?? '' }];
  }

  function handleGenerateArtifactCall(input) {
    const title = input?.title;
    const kind  = input?.kind;
    if (!title || !kind) return;
    const artifactId = input.artifact_id ?? null;
    const subfiles = (kind === 'terraform' || kind === 'terragrunt')
      ? parseTerraformBundle(input.content, title)
      : [{ path: title, text: input.content ?? '' }];
    const existingIdx = artifactId ? files.value.findIndex(f => f.artifactId === artifactId) : -1;
    const entry = {
      id: existingIdx !== -1 ? files.value[existingIdx].id : `file-${fileIdSeq++}`,
      artifactId, title, kind, subfiles,
      status: 'saving',
    };
    files.value = existingIdx !== -1
      ? files.value.map((f, i) => (i === existingIdx ? entry : f))
      : [...files.value, entry];
    activeFileId.value = entry.id;
  }

  function completeGenerateArtifactCall() {
    const idx = files.value.findIndex(f => f.status === 'saving');
    if (idx !== -1) files.value[idx] = { ...files.value[idx], status: 'saved' };
  }

  function handleEvent(event) {
    if (!event) return;
    switch (event.type) {
      case 'intent':
        intent.value = event.mode;
        break;
      case 'phase':
        phase.value = event.label;
        break;
      case 'thinking':
        finalizeThinking();
        chain.value = [...chain.value, { type: 'thinking', text: event.text || '', streaming: false }];
        break;
      case 'thinking_delta': {
        const last = chain.value.at(-1);
        if (last?.type === 'thinking' && last.streaming) {
          last.text += event.text || '';
        } else {
          chain.value = [...chain.value, { type: 'thinking', text: event.text || '', streaming: true }];
        }
        break;
      }
      case 'text_delta':
        finalizeThinking();
        streamText.value += event.text || '';
        break;
      case 'tool_call':
        finalizeThinking();
        chain.value = [...chain.value, {
          type: 'tool_call',
          name: event.name,
          input: event.input,
          status: 'running',
          description: describeToolCall(event.name, event.input ?? {}),
        }];
        if (event.name === 'generate_artifact') handleGenerateArtifactCall(event.input);
        break;
      case 'tool_result':
        completeToolCall(event.name, event.preview);
        if (event.name === 'generate_artifact') completeGenerateArtifactCall();
        break;
      case 'parallel_research_started':
        finalizeThinking();
        chain.value = [...chain.value, {
          type: 'parallel_research',
          leads: (event.leads ?? []).map(label => ({
            label, status: 'running', iterations: null, preview: null, durationMs: null,
          })),
          merging: false,
          totalDurationMs: null,
        }];
        break;
      case 'parallel_research_lead_done': {
        const block = findParallelBlock();
        const lead = block?.leads?.[event.index];
        if (lead) {
          Object.assign(lead, {
            status: 'done',
            iterations: event.iterations,
            preview: event.preview,
            durationMs: event.duration_ms,
          });
        }
        break;
      }
      case 'parallel_research_merge_started': {
        const block = findParallelBlock();
        if (block) {
          block.merging = true;
          block.totalDurationMs = event.duration_ms;
        }
        break;
      }
      case 'done':
        finalizeThinking();
        finished.value = true;
        stopTimer();
        break;
      case 'error':
        error.value = event.message || failedText;
        finished.value = true;
        stopTimer();
        break;
    }
  }

  function reset() {
    finished.value       = false;
    error.value          = null;
    streamText.value     = '';
    chain.value          = [];
    intent.value         = null;
    phase.value          = '';
    files.value          = [];
    activeFileId.value   = null;
    startTimer();
  }

  const isThinking = computed(() => {
    const last = chain.value.at(-1);
    return last?.type === 'thinking' && last.streaming;
  });

  const thinkingText = computed(() =>
    chain.value.filter(c => c.type === 'thinking').map(c => c.text).join('\n\n')
  );

  const runningStep = computed(() => {
    for (let i = chain.value.length - 1; i >= 0; i--) {
      if (chain.value[i].type === 'tool_call' && chain.value[i].status === 'running') return chain.value[i];
    }
    return null;
  });

  const toolCallCount = computed(() => chain.value.filter(c => c.type === 'tool_call').length);

  const statusText = computed(() => {
    if (error.value)       return failedText;
    if (finished.value)    return readyText;
    if (streamText.value)  return writingText;
    if (runningStep.value) return `${runningStep.value.description}…`;
    const research = findParallelBlock();
    if (research && !research.merging) return `Researching ${research.leads.length} leads in parallel…`;
    if (research && research.merging)  return 'Merging research findings…';
    if (isThinking.value)  return 'Thinking…';
    return preparingText;
  });

  const elapsedLabel = computed(() => {
    const s = elapsedSeconds.value;
    if (s < 60) return `${s}s`;
    return `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, '0')}s`;
  });

  const renderedStream = computed(() => streamText.value ? renderMarkdown(streamText.value, {}, {}) : '');

  return {
    finished, error, streamText, chain, intent, phase, elapsedSeconds,
    files, activeFileId, thinkingText,
    statusText, elapsedLabel, renderedStream, toolCallCount,
    handleEvent, reset, startTimer, stopTimer,
  };
}
