import { defineStore } from 'pinia';
import { createConversationThreadState } from '../lib/conversation-thread.js';

const storeHooks = new Map();

export function useIssueChatStore(issueId) {
  if (!storeHooks.has(issueId)) {
    storeHooks.set(issueId, defineStore(`issue-chat-${issueId}`, () => createConversationThreadState()));
  }
  return storeHooks.get(issueId)();
}
