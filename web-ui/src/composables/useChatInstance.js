import { reactive } from 'vue';
import { createConversationThreadState } from '../lib/conversation-thread.js';

const instances = reactive(new Map());
const drafts = new Map();

export function useChatInstance(tabId) {
  if (!instances.has(tabId)) instances.set(tabId, reactive(createConversationThreadState()));
  return instances.get(tabId);
}

export function peekChatInstance(tabId) {
  return instances.get(tabId) ?? null;
}

export function disposeChatInstance(tabId) {
  instances.delete(tabId);
}

export function repliesIn(instance) {
  return instance?.messages.filter(m => m.role === 'assistant' && m.status !== 'loading').length ?? 0;
}

export function tabActivity(instance, seenReplies, isActive) {
  if (instance?.loading) return 'working';
  if (!isActive && repliesIn(instance) > (seenReplies ?? repliesIn(instance))) return 'unseen';
  return null;
}

export function setChatDraft(tabId, text) {
  drafts.set(tabId, text);
}

export function takeChatDraft(tabId) {
  const draft = drafts.get(tabId) ?? '';
  drafts.delete(tabId);
  return draft;
}
