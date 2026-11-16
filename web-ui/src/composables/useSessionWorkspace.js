import { reactive, ref, markRaw } from 'vue';
import { createLayoutOperations } from '../lib/layout-workspace.js';
import {
  SESSION_TAB_KINDS, chatTab, changesTab, previewTab, emptySessionTab, initialTree, tabTitle,
} from '../lib/session-tabs.js';
import {
  createSessionChat, deleteSessionChat, getSessionLayout, saveSessionLayout,
  createSessionPreview, deleteSessionPreview, listSessionPreviews,
} from '../lib/code-api.js';
import { sharePreview } from '../lib/preview-api.js';
import { disposeChatInstance } from './useChatInstance.js';

const SAVE_DELAY_MS = 800;

export function chatInstanceKey(tab) {
  return `${tab.tabId}:${tab.conversationId ?? 'none'}`;
}

export function useSessionWorkspace(sessionId, content) {
  const tree = ref(null);
  const chats = ref([]);
  const previews = ref([]);
  let saveTimer = null;

  function save() {
    saveTimer = null;
    if (tree.value) saveSessionLayout(sessionId, tree.value).catch(() => {});
  }

  function changed() {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(save, SAVE_DELAY_MS);
  }

  function flush() {
    if (saveTimer) {
      clearTimeout(saveTimer);
      save();
    }
  }

  const operations = createLayoutOperations({
    tree,
    changed,
    emptyTab: emptySessionTab,
    disposeTab: (tab) => { if (tab.kind === 'chat') disposeChatInstance(chatInstanceKey(tab)); },
  });

  function setLists(data) {
    chats.value = data.chats ?? [];
    previews.value = data.previews ?? [];
  }

  async function load(data) {
    setLists(data);
    const stored = await getSessionLayout(sessionId).catch(() => null);
    if (stored?.layout?.root) {
      tree.value = stored.layout;
    } else {
      tree.value = initialTree(chats.value);
      changed();
    }
  }

  async function newChat() {
    const { chat } = await createSessionChat(sessionId);
    chats.value = [chat, ...chats.value];
    return chat;
  }

  async function deleteChat(chatId) {
    await deleteSessionChat(sessionId, chatId);
    chats.value = chats.value.filter(c => c.id !== chatId);
  }

  async function refreshPreviews() {
    previews.value = (await listSessionPreviews(sessionId)).previews ?? [];
  }

  async function createPreview(settings) {
    const { preview } = await createSessionPreview(sessionId, settings);
    await refreshPreviews();
    return preview;
  }

  async function deletePreview(previewId) {
    await deleteSessionPreview(previewId);
    previews.value = previews.value.filter(p => p.id !== previewId);
  }

  async function shareWith(previewId, audience) {
    await sharePreview(previewId, audience);
    previews.value = previews.value.map(p => (p.id === previewId ? { ...p, audience } : p));
  }

  async function createTab(kind) {
    if (kind === 'chat') return chatTab(await newChat());
    if (kind === 'changes') return changesTab();
    return previewTab(previews.value[0] ?? null);
  }

  return reactive({
    id: `session:${sessionId}`,
    sessionId,
    tree,
    chats,
    previews,
    tabKinds: SESSION_TAB_KINDS,
    content: markRaw(content),
    titleOf: (tab) => tabTitle(tab, { chats: chats.value, previews: previews.value }),
    createTab,
    ...operations,
    load,
    setLists,
    flush,
    newChat,
    deleteChat,
    refreshPreviews,
    createPreview,
    deletePreview,
    shareWith,
  });
}
