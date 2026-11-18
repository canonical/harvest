import { computed, reactive, ref, markRaw, watch } from 'vue';
import { createLayoutOperations } from '../lib/layout-workspace.js';
import { allTabIds, findTab } from '../lib/pane-layout.js';
import {
  SESSION_TAB_KINDS, chatTab, changesTab, jobsTab, previewTab, terminalTab, emptySessionTab, initialTree, tabTitle,
} from '../lib/session-tabs.js';
import {
  createSessionChat, deleteSessionChat, getSessionLayout, saveSessionLayout,
  createSessionPreview, deleteSessionPreview, listSessionPreviews, renameSessionChat,
  createSessionTerminal, endSessionTerminal, listSessionTerminals,
} from '../lib/code-api.js';
import { sharePreview } from '../lib/preview-api.js';
import { showToast } from './useToasts.js';
import { chatUnavailableReason } from '../lib/code-sessions.js';
import { disposeChatInstance, peekChatInstance, setChatDraft, tabActivity } from './useChatInstance.js';

const SAVE_DELAY_MS = 800;
const UNDO_WINDOW_MS = 8000;

export function chatInstanceKey(tab) {
  return `${tab.tabId}:${tab.conversationId ?? 'none'}`;
}


export function useSessionWorkspace(sessionId, { content, tabMenu }) {
  const tree = ref(null);
  const chats = ref([]);
  const previews = ref([]);
  const terminals = ref([]);
  const liveTitles = reactive(new Map());
  const sessionStatus = ref(null);
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
    sessionStatus.value = data.session?.status ?? null;
  }

  async function load(data) {
    setLists(data);
    await refreshTerminals().catch(() => {});
    const stored = await getSessionLayout(sessionId).catch(() => null);
    if (stored?.layout?.root) {
      tree.value = stored.layout;
    } else {
      tree.value = initialTree(chats.value);
      changed();
    }
  }

  const chatTabs = computed(() => {
    if (!tree.value) return [];
    return allTabIds(tree.value.root)
      .map(tabId => findTab(tree.value.root, tabId)?.tab)
      .filter(tab => tab?.kind === 'chat');
  });
  const busyChats = computed(() => chatTabs.value.map(chatInstanceKey).filter(key => peekChatInstance(key)?.loading));
  const turnsFinished = ref(0);

  watch(busyChats, (now, before = []) => {
    if (before.some(key => !now.includes(key))) turnsFinished.value += 1;
  });

  function activityOf(tab, isActive) {
    if (tab.kind !== 'chat') return null;
    return tabActivity(peekChatInstance(chatInstanceKey(tab)), tab.seenReplies, isActive);
  }

  const tabRequest = ref(null);

  function requestTabAction(tabId, action) {
    tabRequest.value = { tabId, action, at: Date.now() };
  }

  function hasTabMenu(tab) {
    return tab.kind === 'chat' || tab.kind === 'preview' || tab.kind === 'terminal';
  }

  async function refreshTerminals() {
    terminals.value = (await listSessionTerminals(sessionId)).terminals ?? [];
  }

  async function newTerminal() {
    const { terminal } = await createSessionTerminal(sessionId);
    terminals.value = [...terminals.value, terminal];
    return terminal;
  }

  async function endTerminal(terminalId) {
    await endSessionTerminal(sessionId, terminalId);
    terminals.value = terminals.value.filter(t => t.id !== terminalId);
  }

  function setLiveTitle(tabId, title) {
    liveTitles.set(tabId, title);
  }

  function showJobs() {
    const existing = findTabOfKind(tree.value.root, 'jobs');
    if (existing) operations.setActiveTab(existing.paneId, existing.tabId);
    else operations.openTab(firstLeafId(tree.value.root), jobsTab());
  }

  async function askAgent(paneId, text) {
    const tab = chatTab(await newChat());
    setChatDraft(chatInstanceKey(tab), text);
    operations.openTab(paneId, tab);
  }

  async function renameChat(chatId, title) {
    await renameSessionChat(sessionId, chatId, title);
    chats.value = chats.value.map(c => (c.id === chatId ? { ...c, title } : c));
  }

  async function newChat() {
    const { chat } = await createSessionChat(sessionId);
    chats.value = [chat, ...chats.value];
    return chat;
  }

  function deleteChat(chatId) {
    const index = chats.value.findIndex(c => c.id === chatId);
    if (index < 0) return;
    const chat = chats.value[index];
    chats.value = chats.value.filter(c => c.id !== chatId);
    showToast({
      message: `Deleted “${chat.title}”`,
      kind: 'information',
      timeout: UNDO_WINDOW_MS,
      action: { label: 'Undo', run: () => { chats.value = [...chats.value.slice(0, index), chat, ...chats.value.slice(index)]; } },
      onExpire: () => deleteSessionChat(sessionId, chatId).catch((e) => showToast({ message: e.message, kind: 'negative' })),
    });
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
    if (kind === 'jobs') return jobsTab();
    if (kind === 'terminal') return terminalTab(await newTerminal());
    return previewTab(previews.value[0] ?? null);
  }

  return reactive({
    id: `session:${sessionId}`,
    sessionId,
    tree,
    chats,
    previews,
    chatUnavailableReason: computed(() => chatUnavailableReason(sessionStatus.value)),
    tabKinds: SESSION_TAB_KINDS,
    content: markRaw(content),
    tabMenu: markRaw(tabMenu),
    turnsFinished,
    tabRequest,
    requestTabAction,
    hasTabMenu,
    activityOf,
    askAgent,
    showJobs,
    renameChat,
    titleOf: (tab) => tabTitle(tab, { chats: chats.value, previews: previews.value, terminals: terminals.value, liveTitles }),
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
    terminals,
    refreshTerminals,
    newTerminal,
    endTerminal,
    setLiveTitle,
  });
}

function firstLeafId(node) {
  return node.type === 'leaf' ? node.id : firstLeafId(node.children[0]);
}

function findTabOfKind(node, kind) {
  if (node.type === 'leaf') {
    const tab = node.tabs.find(t => t.kind === kind);
    return tab ? { paneId: node.id, tabId: tab.tabId } : null;
  }
  for (const child of node.children) {
    const found = findTabOfKind(child, kind);
    if (found) return found;
  }
  return null;
}
