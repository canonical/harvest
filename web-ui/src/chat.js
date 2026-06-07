let _nextToolCallId = 1;
let _nextAttachmentId = 1;

export function createChatState() {
  return { messages: [], loading: false, pendingAttachments: [], suggestions: [] };
}

export function getMessages(state) {
  return state.messages;
}

export function isLoading(state) {
  return state.loading;
}

export function addUserMessage(state, text, username = null, attachments = []) {
  const msg = { role: 'user', text, attachments };
  if (username) msg.username = username;
  return { ...state, messages: [...state.messages, msg], suggestions: [] };
}

export function setSuggestions(state, choices) {
  return { ...state, suggestions: choices };
}

export function addPendingAttachment(state, attachment) {
  const att = { ...attachment, id: attachment.id ?? _nextAttachmentId++ };
  return { ...state, pendingAttachments: [...state.pendingAttachments, att] };
}

export function removePendingAttachment(state, id) {
  return { ...state, pendingAttachments: state.pendingAttachments.filter(a => a.id !== id) };
}

export function getPendingAttachments(state) {
  return state.pendingAttachments;
}

export function clearPendingAttachments(state) {
  return { ...state, pendingAttachments: [] };
}

export function startAssistantMessage(state) {
  const msg = { role: 'assistant', status: 'loading', tool_calls: [], answer: null, sources: [], tool_calls_made: 0 };
  return {
    ...state,
    loading: true,
    messages: [...state.messages, msg],
  };
}

export function addToolCall(state, { name, input, description = null }) {
  const id = _nextToolCallId++;
  return updateLastAssistant(state, (msg) => ({
    ...msg,
    tool_calls: [...msg.tool_calls, { id, name, input, status: 'running', preview: null, description }],
  }));
}

export function updateToolCallDescription(state, { id, description }) {
  const messages = state.messages.map((msg) => {
    if (msg.role !== 'assistant') return msg;
    const tool_calls = msg.tool_calls.map((tc) =>
      tc.id === id ? { ...tc, description } : tc,
    );
    return { ...msg, tool_calls };
  });
  return { ...state, messages };
}

export function completeToolCall(state, { name, preview }) {
  return updateLastAssistant(state, (msg) => {
    let marked = false;
    const tool_calls = msg.tool_calls.map((tc) => {
      if (!marked && tc.name === name && tc.status === 'running') {
        marked = true;
        return { ...tc, status: 'done', preview };
      }
      return tc;
    });
    return { ...msg, tool_calls };
  });
}

export function finalizeAssistantMessage(state, { answer, sources, tool_calls_made }) {
  const next = updateLastAssistant(state, (msg) => ({
    ...msg,
    status: 'done',
    answer,
    sources,
    tool_calls_made,
  }));
  return { ...next, loading: false };
}

export function setQuestion(state, { question, choices }) {
  return updateLastAssistant(state, (msg) => ({
    ...msg,
    question: { question, choices },
  }));
}

export function setError(state, error) {
  const next = updateLastAssistant(state, (msg) => ({
    ...msg,
    status: 'error',
    error,
  }));
  return { ...next, loading: false };
}

export function getSaveableMessages(state) {
  return state.messages
    .filter(m => m.role === 'user' || (m.role === 'assistant' && m.status === 'done' && m.answer))
    .map(m => {
      if (m.role === 'user') {
        const saved = { role: 'user', text: m.text, attachments: m.attachments ?? [] };
        if (m.username) saved.username = m.username;
        return saved;
      }
      return { role: 'assistant', text: m.answer, sources: m.sources ?? [], tool_calls: m.tool_calls ?? [], tool_calls_made: m.tool_calls_made ?? 0 };
    });
}

export function loadFromHistory(messages) {
  const hydrated = messages.map(m => {
    if (m.role === 'user') {
      const msg = { role: 'user', text: m.text, attachments: m.attachments ?? [] };
      if (m.username) msg.username = m.username;
      return msg;
    }
    const tool_calls = (m.tool_calls ?? []).map(tc => ({ ...tc, status: 'done' }));
    return { role: 'assistant', status: 'done', tool_calls, answer: m.text, sources: m.sources ?? [], tool_calls_made: m.tool_calls_made ?? 0 };
  });
  return { messages: hydrated, loading: false, pendingAttachments: [] };
}

function updateLastAssistant(state, updater) {
  const messages = [...state.messages];
  const idx = messages.length - 1;
  if (idx >= 0 && messages[idx].role === 'assistant') {
    messages[idx] = updater(messages[idx]);
  }
  return { ...state, messages };
}
