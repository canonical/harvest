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
  const msg = { role: 'assistant', status: 'loading', chain: [], tool_calls: [], answer: null, sources: [], tool_calls_made: 0 };
  return {
    ...state,
    loading: true,
    messages: [...state.messages, msg],
  };
}

export function addThinking(state, { text }) {
  return updateLastAssistant(state, (msg) => {
    const chain = [...(msg.chain ?? []), { type: 'thinking', text }];
    return { ...msg, chain };
  });
}

export function addToolCall(state, { name, input, description = null }) {
  const id = _nextToolCallId++;
  const tc = { type: 'tool_call', id, name, input, status: 'running', preview: null, description };
  return updateLastAssistant(state, (msg) => {
    const chain = [...(msg.chain ?? []), tc];
    return { ...msg, chain, tool_calls: chain.filter(c => c.type === 'tool_call') };
  });
}

export function updateToolCallDescription(state, { id, description }) {
  const messages = state.messages.map((msg) => {
    if (msg.role !== 'assistant') return msg;
    const chain = (msg.chain ?? []).map((item) =>
      item.type === 'tool_call' && item.id === id ? { ...item, description } : item,
    );
    return { ...msg, chain, tool_calls: chain.filter(c => c.type === 'tool_call') };
  });
  return { ...state, messages };
}

export function completeToolCall(state, { name, preview }) {
  return updateLastAssistant(state, (msg) => {
    let marked = false;
    const chain = (msg.chain ?? []).map((item) => {
      if (item.type === 'tool_call' && !marked && item.name === name && item.status === 'running') {
        marked = true;
        return { ...item, status: 'done', preview };
      }
      return item;
    });
    return { ...msg, chain, tool_calls: chain.filter(c => c.type === 'tool_call') };
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
      const chain = m.chain ?? [];
      const tool_calls = chain.filter(c => c.type === 'tool_call');
      return { role: 'assistant', text: m.answer, sources: m.sources ?? [], chain, tool_calls, tool_calls_made: m.tool_calls_made ?? 0 };
    });
}

export function loadFromHistory(messages) {
  const hydrated = messages.map(m => {
    if (m.role === 'user') {
      const msg = { role: 'user', text: m.text, attachments: m.attachments ?? [] };
      if (m.username) msg.username = m.username;
      return msg;
    }
    let chain;
    if (m.chain) {
      chain = m.chain.map(item =>
        item.type === 'tool_call' ? { ...item, status: 'done' } : item,
      );
    } else {
      // backward compat: old format has separate thinking[] + tool_calls[]
      const thinkingItems = (m.thinking ?? []).map(text => ({ type: 'thinking', text }));
      const toolCallItems = (m.tool_calls ?? []).map(tc => ({ type: 'tool_call', ...tc, status: 'done' }));
      chain = [...thinkingItems, ...toolCallItems];
    }
    const tool_calls = chain.filter(c => c.type === 'tool_call');
    return { role: 'assistant', status: 'done', chain, tool_calls, answer: m.text, sources: m.sources ?? [], tool_calls_made: m.tool_calls_made ?? 0 };
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
