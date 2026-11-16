import { describe, it, expect } from 'vitest';
import {
  ACTIVITY_STATE,
  AWAITING_USER,
  MAX_SENTENCE_CHARS,
  MERGING_LEADS,
  REVIEWING,
  THINKING,
  WRITING_ANSWER,
  describeActivity,
  firstLine,
  isAnswerText,
  isContinuation,
  stepCount,
  stepLabel,
} from '../../src/lib/agent-activity.js';

const loading = (patch = {}) => ({ role: 'assistant', status: 'loading', chain: [], pendingAnswer: '', phase: null, ...patch });
const sentence = text => ({ type: 'thinking', text, streaming: false });
const tool = (name, input, status = 'running') => ({ type: 'tool_call', id: Math.random(), name, input, status });

describe('describeActivity while a response is generated', () => {
  const cases = [
    ['nothing has arrived yet', loading(), THINKING],
    ['the agent is writing its sentence', loading({ pendingAnswer: "I'm locating the volume drivers" }), "I'm locating the volume drivers"],
    ['the final answer is streaming', loading({ pendingAnswer: 'x'.repeat(MAX_SENTENCE_CHARS + 1) }), WRITING_ANSWER],
    ['a multi-line text is the answer', loading({ pendingAnswer: 'Cinder ships:\n- LVM' }), WRITING_ANSWER],
    [
      'a tool runs after a sentence',
      loading({ chain: [sentence("I'm reading the support matrix."), tool('read_sources', { files: ['doc/support-matrix.ini'] })] }),
      "I'm reading the support matrix.",
    ],
    [
      'a tool runs without a sentence',
      loading({ chain: [tool('read_sources', { files: ['doc/support-matrix.ini'] })] }),
      'Reading 1 source',
    ],
    [
      'a batch of tools shares the sentence before it',
      loading({ chain: [sentence("I'm comparing two drivers."), tool('read_sources', { files: ['a.py'] }, 'done'), tool('read_sources', { files: ['b.py'] })] }),
      "I'm comparing two drivers.",
    ],
    [
      'a reasoning summary is the latest step',
      loading({ chain: [sentence('**Gathering information on Cinder drivers**\n\nI need to find…')] }),
      'Gathering information on Cinder drivers',
    ],
    ['a tool has finished', loading({ chain: [tool('search_code', { repo: 'cinder' }, 'done')] }), REVIEWING],
    ['the server is synthesizing', loading({ chain: [tool('search_code', {}, 'done')], phase: 'Synthesizing answer' }), 'Synthesizing answer'],
    ['a tool-batch phase does not replace the review', loading({ chain: [tool('search_code', {}, 'done')], phase: 'Reading source' }), REVIEWING],
    [
      'parallel leads are running',
      loading({ chain: [{ type: 'parallel_research', leads: [{}, {}, {}], merging: false, totalDurationMs: null }] }),
      'Researching 3 leads in parallel',
    ],
    [
      'parallel leads are merging',
      loading({ chain: [{ type: 'parallel_research', leads: [{}], merging: true, totalDurationMs: null }] }),
      MERGING_LEADS,
    ],
  ];

  it.each(cases)('shows the right line when %s', (_, msg, expected) => {
    const activity = describeActivity(msg);
    expect(activity.state).toBe(ACTIVITY_STATE.WORKING);
    expect(activity.text).toBe(expected);
  });

  it('waits for the user when an action needs confirmation', () => {
    const msg = loading({ chain: [{ type: 'confirm_action', status: 'pending' }] });
    expect(describeActivity(msg)).toEqual({ state: ACTIVITY_STATE.WAITING, text: AWAITING_USER });
  });
});

describe('describeActivity after a response', () => {
  const steps = [sentence('a'), tool('search_code', {}, 'done'), { type: 'confirm_action', status: 'done' }];

  it('summarises a finished turn with its duration and steps', () => {
    const msg = { status: 'done', chain: steps };
    expect(describeActivity(msg, '11s')).toEqual({ state: ACTIVITY_STATE.DONE, text: 'Worked for 11s · 2 steps' });
  });

  it('summarises without a duration when none is known', () => {
    expect(describeActivity({ status: 'done', chain: steps }).text).toBe('2 steps');
  });

  it('leaves out the step count when the answer needed no steps', () => {
    expect(describeActivity({ status: 'done', chain: [] }, '3s').text).toBe('Worked for 3s');
    expect(describeActivity({ status: 'done', chain: [] }).text).toBe('Done');
  });

  it('marks an interrupted or failed turn as stopped', () => {
    const msg = { status: 'error', chain: steps };
    expect(describeActivity(msg, '4s')).toEqual({ state: ACTIVITY_STATE.STOPPED, text: 'Stopped after 4s · 2 steps' });
  });
});

describe('helpers', () => {
  it('treats one short sentence as a status, not an answer', () => {
    expect(isAnswerText("I'm reading the support matrix.")).toBe(false);
    expect(isAnswerText('line one\nline two')).toBe(true);
    expect(isAnswerText(null)).toBe(false);
  });

  it('takes the first line without markdown emphasis', () => {
    expect(firstLine('  **Bold title**\nrest')).toBe('Bold title');
    expect(firstLine(undefined)).toBe('');
  });

  it('counts thinking, tool and parallel research steps only', () => {
    expect(stepCount({ chain: [sentence('a'), tool('x', {}), { type: 'confirm_action' }, { type: 'parallel_research' }] })).toBe(3);
    expect(stepCount({})).toBe(0);
    expect(stepLabel(1)).toBe('1 step');
    expect(stepLabel(4)).toBe('4 steps');
  });

  it('recognises a growing or settled line as the same line', () => {
    expect(isContinuation("I'm read", "I'm reading the matrix")).toBe(true);
    expect(isContinuation("I'm reading the matrix", "I'm reading the matrix")).toBe(true);
    expect(isContinuation("I'm reading the matrix", 'Reviewing the results')).toBe(false);
    expect(isContinuation('', 'Thinking')).toBe(false);
  });
});
