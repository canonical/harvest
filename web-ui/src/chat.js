let _nextToolCallId = 1;

export function createChatState() {
  return { messages: [], loading: false };
}

export function getMessages(state) {
  return state.messages;
}

export function isLoading(state) {
  return state.loading;
}

export function addUserMessage(state, text) {
  return {
    ...state,
    messages: [...state.messages, { role: 'user', text }],
  };
}

export function startAssistantMessage(state) {
  const msg = { role: 'assistant', status: 'loading', tool_calls: [], answer: null, sources: [], tool_calls_made: 0 };
  return {
    ...state,
    loading: true,
    messages: [...state.messages, msg],
  };
}

export function addToolCall(state, { name, input }) {
  const id = _nextToolCallId++;
  return updateLastAssistant(state, (msg) => ({
    ...msg,
    tool_calls: [...msg.tool_calls, { id, name, input, status: 'running', preview: null, description: null }],
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

export function setError(state, error) {
  const next = updateLastAssistant(state, (msg) => ({
    ...msg,
    status: 'error',
    error,
  }));
  return { ...next, loading: false };
}

function updateLastAssistant(state, updater) {
  const messages = [...state.messages];
  const idx = messages.length - 1;
  if (idx >= 0 && messages[idx].role === 'assistant') {
    messages[idx] = updater(messages[idx]);
  }
  return { ...state, messages };
}
