import { defineStore } from 'pinia';
import { createConversationThreadState } from '../lib/conversation-thread.js';

export const useChatStore = defineStore('chat', () => createConversationThreadState());
