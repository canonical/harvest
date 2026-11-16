import { describeToolCall } from './tool-render.js';

export const MAX_SENTENCE_CHARS = 160;

export const ACTIVITY_STATE = Object.freeze({
  WORKING: 'working',
  WAITING: 'waiting',
  DONE:    'done',
  STOPPED: 'stopped',
});

export const THINKING        = 'Thinking';
export const WRITING_ANSWER  = 'Writing the answer';
export const REVIEWING       = 'Reviewing the results';
export const AWAITING_USER   = 'Waiting for your confirmation';
export const MERGING_LEADS   = 'Merging the parallel findings';

const ANSWER_PHASES = new Set(['Synthesizing answer', 'Continuing…']);
const STEP_TYPES = new Set(['thinking', 'tool_call', 'parallel_research']);
const MARKUP = /[*_`#>]+/g;

export function isAnswerText(text) {
  const trimmed = (text ?? '').trim();
  return trimmed.length > MAX_SENTENCE_CHARS || trimmed.includes('\n');
}

export function firstLine(text) {
  return (text ?? '')
    .trim()
    .split('\n')[0]
    .replace(MARKUP, '')
    .trim();
}

export function stepCount(msg) {
  return (msg.chain ?? []).filter(item => STEP_TYPES.has(item.type)).length;
}

export function stepLabel(count) {
  return `${count} step${count === 1 ? '' : 's'}`;
}

export function isContinuation(previous, next) {
  if (!previous || !next) return false;
  return next.startsWith(previous) || previous.startsWith(next);
}

function isRunningTool(item) {
  return item.type === 'tool_call' && item.status === 'running';
}

function isPendingConfirmation(item) {
  return item.type === 'confirm_action' && item.status === 'pending';
}

function sentenceBefore(chain, index) {
  let cursor = index - 1;
  while (cursor >= 0 && chain[cursor].type === 'tool_call') cursor -= 1;
  const candidate = chain[cursor];
  if (candidate?.type !== 'thinking' || isAnswerText(candidate.text)) return null;
  return firstLine(candidate.text) || null;
}

function describeRunningTool(chain, index) {
  const tool = chain[index];
  return sentenceBefore(chain, index) ?? describeToolCall(tool.name, tool.input);
}

function describeParallelResearch(block) {
  if (block.merging) return MERGING_LEADS;
  const leads = block.leads?.length ?? 0;
  return `Researching ${leads} leads in parallel`;
}

function isParallelResearchRunning(item) {
  return item?.type === 'parallel_research' && item.totalDurationMs == null;
}

function describeWork(msg) {
  const chain = msg.chain ?? [];
  if (chain.some(isPendingConfirmation)) return { state: ACTIVITY_STATE.WAITING, text: AWAITING_USER };
  const working = text => ({ state: ACTIVITY_STATE.WORKING, text });
  const pending = msg.pendingAnswer ?? '';
  if (pending.trim()) return working(isAnswerText(pending) ? WRITING_ANSWER : firstLine(pending));
  const running = chain.findLastIndex(isRunningTool);
  if (running >= 0) return working(describeRunningTool(chain, running));
  const last = chain.at(-1);
  if (isParallelResearchRunning(last)) return working(describeParallelResearch(last));
  if (ANSWER_PHASES.has(msg.phase)) return working(msg.phase);
  if (!last) return working(THINKING);
  if (last.type === 'thinking') return working(firstLine(last.text) || THINKING);
  return working(REVIEWING);
}

function outcomeText(lead, steps, fallback) {
  return [lead, steps ? stepLabel(steps) : null].filter(Boolean).join(' · ') || fallback;
}

function describeOutcome(msg, duration) {
  const steps = stepCount(msg);
  if (msg.status === 'error') {
    return { state: ACTIVITY_STATE.STOPPED, text: outcomeText(duration ? `Stopped after ${duration}` : 'Stopped', steps, 'Stopped') };
  }
  return { state: ACTIVITY_STATE.DONE, text: outcomeText(duration && `Worked for ${duration}`, steps, 'Done') };
}

export function describeActivity(msg, duration = null) {
  return msg.status === 'loading' ? describeWork(msg) : describeOutcome(msg, duration);
}
