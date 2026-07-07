import { defineStore } from 'pinia';
import { ref, computed } from 'vue';

let _nextId = 1;

export const useChatStore = defineStore('chat', () => {
  const messages          = ref([]);
  const loading           = ref(false);
  const pendingAttachments = ref([]);
  const suggestions       = ref([]);

  function lastAssistant() {
    return messages.value.findLast(m => m.role === 'assistant');
  }

  function addUserMessage(text, username = null, attachments = []) {
    const msg = { role: 'user', text, attachments: attachments ?? [] };
    if (username) msg.username = username;
    messages.value.push(msg);
  }

  function startAssistantMessage() {
    messages.value.push({
      role: 'assistant', status: 'loading',
      chain: [], tool_calls: [], answer: null, pendingAnswer: '',
      sources: [], tool_calls_made: 0,
    });
    loading.value = true;
  }

  function finalizeAssistantMessage({ answer, sources, tool_calls_made }) {
    const msg = lastAssistant();
    if (!msg) return;
    msg.status = 'done';
    msg.answer = answer ?? msg.pendingAnswer ?? '';
    msg.pendingAnswer = '';
    for (const item of msg.chain) {
      if (item.type === 'thinking' && item.streaming) item.streaming = false;
    }
    msg.sources = sources ?? [];
    msg.tool_calls_made = tool_calls_made ?? 0;
    loading.value = false;
  }

  function setError(error) {
    const msg = lastAssistant();
    if (!msg) return;
    msg.status = 'error';
    msg.error  = error;
    loading.value = false;
  }

  function addThinking(text) {
    const msg = lastAssistant();
    if (!msg) return;
    const last = msg.chain.at(-1);
    if (last?.type === 'thinking' && last.streaming) {
      last.streaming = false;
    }
    msg.chain.push({ type: 'thinking', text, streaming: false });
  }

  function addThinkingDelta(text) {
    const msg = lastAssistant();
    if (!msg) return;
    const last = msg.chain.at(-1);
    if (last?.type === 'thinking' && last.streaming) {
      last.text += text;
    } else {
      msg.chain.push({ type: 'thinking', text, streaming: true });
    }
  }

  function addTextDelta(text) {
    const msg = lastAssistant();
    if (!msg) return;
    msg.pendingAnswer = (msg.pendingAnswer ?? '') + text;
  }

  function addToolCall(name, input, description = null) {
    const msg = lastAssistant();
    if (!msg) return;
    if (msg.pendingAnswer) {
      msg.chain.push({ type: 'thinking', text: msg.pendingAnswer, streaming: false });
      msg.pendingAnswer = '';
    }
    const last = msg.chain.at(-1);
    if (last?.type === 'thinking' && last.streaming) last.streaming = false;
    const tc = { type: 'tool_call', id: _nextId++, name, input, status: 'running', preview: null, description };
    msg.chain.push(tc);
    msg.tool_calls.push(tc);
  }

  function completeToolCall(name, preview) {
    const msg = lastAssistant();
    if (!msg) return;
    let marked = false;
    for (const item of msg.chain) {
      if (item.type === 'tool_call' && !marked && item.name === name && item.status === 'running') {
        item.status  = 'done';
        item.preview = preview;
        marked = true;
      }
    }
  }

  function updateToolCallDescription(id, description) {
    for (const msg of messages.value) {
      if (msg.role !== 'assistant') continue;
      const tc = msg.chain.find(c => c.type === 'tool_call' && c.id === id);
      if (tc) { tc.description = description; break; }
    }
  }

  function setQuestion(question, choices) {
    const msg = lastAssistant();
    if (!msg) return;
    msg.question = { question, choices };
  }

  function addConfirmAction(id, name, input, description) {
    const msg = lastAssistant();
    if (!msg) return;
    if (msg.pendingAnswer) {
      msg.chain.push({ type: 'thinking', text: msg.pendingAnswer, streaming: false });
      msg.pendingAnswer = '';
    }
    const last = msg.chain.at(-1);
    if (last?.type === 'thinking' && last.streaming) last.streaming = false;
    msg.chain.push({ type: 'confirm_action', id, name, input, description, status: 'pending', steps: [], resultText: '' });
  }

  function updateConfirmActionItem(id, patch) {
    const item = lastAssistant()?.chain?.find(c => c.type === 'confirm_action' && c.id === id);
    if (!item) return;
    Object.assign(item, patch);
  }

  function resumeAssistantMessage() {
    const msg = lastAssistant();
    if (!msg) return;
    msg.status = 'loading';
    loading.value = true;
  }

  function setSuggestions(choices) {
    suggestions.value = choices ?? [];
  }

  function addPendingAttachment(attachment) {
    pendingAttachments.value.push({ ...attachment, id: _nextId++ });
  }

  function removePendingAttachment(id) {
    pendingAttachments.value = pendingAttachments.value.filter(a => a.id !== id);
  }

  function clearPendingAttachments() {
    pendingAttachments.value = [];
  }

  const saveableMessages = computed(() => {
    return messages.value
      .filter(m => m.role === 'user' || (m.role === 'assistant' && m.status === 'done'))
      .map(m => {
        if (m.role === 'user') {
          const saved = { role: 'user', text: m.text };
          if (m.username) saved.username = m.username;
          if (m.attachments?.length) saved.attachments = m.attachments;
          return saved;
        }
        const chain = m.chain ?? [];
        return {
          role: 'assistant', text: m.answer,
          sources: m.sources ?? [],
          chain,
          tool_calls: chain.filter(c => c.type === 'tool_call'),
          tool_calls_made: m.tool_calls_made ?? 0,
        };
      });
  });

  function loadFromHistory(history) {
    reset();
    for (const m of history) {
      if (m.role === 'user') {
        const attachments = (m.attachments ?? []).map(a => ({
          ...a,
          preview_url: a.preview_url
            ?? (a.data && a.mime_type?.startsWith('image/')
                ? `data:${a.mime_type};base64,${a.data}`
                : null),
        }));
        const msg = { role: 'user', text: m.text, attachments };
        if (m.username) msg.username = m.username;
        messages.value.push(msg);
      } else {
        let chain;
        if (m.chain) {
          chain = m.chain.map(item => {
            if (item.type === 'tool_call') return { ...item, status: 'done' };
            if (item.type === 'confirm_action') {
              const { result_text, ...rest } = item;
              return {
                ...rest,
                status: item.status ?? 'pending',
                steps: item.steps ?? [],
                resultText: result_text ?? '',
              };
            }
            return item;
          });
        } else {
          const thinkingItems  = (m.thinking  ?? []).map(text => ({ type: 'thinking', text }));
          const toolCallItems  = (m.tool_calls ?? []).map(tc  => ({ type: 'tool_call', ...tc, status: 'done' }));
          chain = [...thinkingItems, ...toolCallItems];
        }
        const msg = {
          role: 'assistant', status: 'done',
          chain,
          tool_calls: chain.filter(c => c.type === 'tool_call'),
          answer: m.text,
          sources: m.sources ?? [],
          tool_calls_made: m.tool_calls_made ?? 0,
        };
        if (m.question) msg.question = m.question;
        messages.value.push(msg);
      }
    }
    loading.value = false;
  }

  function reset() {
    messages.value = [];
    loading.value  = false;
    pendingAttachments.value = [];
    suggestions.value = [];
  }

  return {
    messages, loading, pendingAttachments, suggestions,
    addUserMessage, startAssistantMessage, finalizeAssistantMessage,
    setError, addThinking, addThinkingDelta, addTextDelta, addToolCall,
    completeToolCall, updateToolCallDescription,
    setQuestion, addConfirmAction, updateConfirmActionItem, resumeAssistantMessage, setSuggestions,
    addPendingAttachment, removePendingAttachment, clearPendingAttachments,
    saveableMessages, loadFromHistory, reset,
  };
});
