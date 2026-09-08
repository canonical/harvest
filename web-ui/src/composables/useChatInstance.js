import { reactive } from 'vue';
import { createConversationThreadState } from '../lib/conversation-thread.js';

const instances = new Map();

export function useChatInstance(tabId) {
  if (!instances.has(tabId)) instances.set(tabId, reactive(createConversationThreadState()));
  return instances.get(tabId);
}

export function disposeChatInstance(tabId) {
  instances.delete(tabId);
}
