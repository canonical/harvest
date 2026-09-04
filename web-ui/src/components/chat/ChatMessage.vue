<template>
  <div class="message" :class="roleClass">
    <template v-if="msg.role === 'user'">
      <div class="message__sender">
        <div class="message-avatar" :style="{ background: senderColor }">{{ senderInitials }}</div>
        <span class="message__sender-name">{{ msg.username ?? 'You' }}</span>
      </div>
      <div v-if="imageAttachments.length" class="message__img-row">
        <img
          v-for="(a, i) in imageAttachments"
          :key="i"
          class="message__img-thumb"
          :src="a.preview_url"
          :alt="a.name"
          @click="lightboxSrc = a.preview_url"
        />
      </div>
      <div class="message__bubble">
        <div v-if="fileAttachments.length" class="message__attachments">
          <div
            v-for="(a, i) in fileAttachments"
            :key="i"
            class="message__attachment-chip"
          >{{ a.name }}</div>
        </div>
        <div class="message__body">{{ msg.text }}</div>
      </div>
    </template>

    <template v-else>
      <!-- Loading indicator: only while no activity or answer has arrived yet -->
      <span v-if="msg.status === 'loading' && !msg.chain?.length && !msg.pendingAnswer && msg.intent !== 'conversational'" class="loading-orbit">
        <svg viewBox="0 0 16 16" fill="currentColor" aria-hidden="true" xmlns="http://www.w3.org/2000/svg">
          <path d="M.81 7.36a1.92 1.92 0 1 1 3.799.572A1.92 1.92 0 0 1 .81 7.36M8.826 3.033a1.92 1.92 0 1 1 3.755.806 1.92 1.92 0 0 1-3.755-.806M7.04 12.585a4.68 4.68 0 0 1-3.19-2.432 2.76 2.76 0 0 1-1.64.202 6.25 6.25 0 0 0 4.498 3.77c.45.098.908.144 1.364.141a2.74 2.74 0 0 1-.562-1.605 5 5 0 0 1-.47-.076M8.394 12.193a1.92 1.92 0 0 1 3.754.805 1.92 1.92 0 1 1-3.754-.805M12.943 11.89a6.3 6.3 0 0 0 1.22-2.587 6.3 6.3 0 0 0-.905-4.782 2.77 2.77 0 0 1-1.08 1.265 4.7 4.7 0 0 1-.154 4.674c.45.37.77.87.919 1.43M2.56 4.892a2.75 2.75 0 0 1 1.603.41 4.68 4.68 0 0 1 3.77-2.015q.012-.218.057-.433c.088-.411.268-.795.525-1.124A6.31 6.31 0 0 0 2.56 4.892"/>
        </svg>
        <span class="loading-orbit__label">Thinking…</span>
      </span>

      <div v-if="msg.provider_used || durationLabel" class="message__meta-row">
        <div v-if="msg.provider_used" class="provider-badge" :title="msg.provider_used.provider_id">
          <svg class="provider-badge__icon" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true" xmlns="http://www.w3.org/2000/svg">
            <path d="M.81 7.36a1.92 1.92 0 1 1 3.799.572A1.92 1.92 0 0 1 .81 7.36M8.826 3.033a1.92 1.92 0 1 1 3.755.806 1.92 1.92 0 0 1-3.755-.806M7.04 12.585a4.68 4.68 0 0 1-3.19-2.432 2.76 2.76 0 0 1-1.64.202 6.25 6.25 0 0 0 4.498 3.77c.45.098.908.144 1.364.141a2.74 2.74 0 0 1-.562-1.605 5 5 0 0 1-.47-.076M8.394 12.193a1.92 1.92 0 0 1 3.754.805 1.92 1.92 0 1 1-3.754-.805M12.943 11.89a6.3 6.3 0 0 0 1.22-2.587 6.3 6.3 0 0 0-.905-4.782 2.77 2.77 0 0 1-1.08 1.265 4.7 4.7 0 0 1-.154 4.674c.45.37.77.87.919 1.43M2.56 4.892a2.75 2.75 0 0 1 1.603.41 4.68 4.68 0 0 1 3.77-2.015q.012-.218.057-.433c.088-.411.268-.795.525-1.124A6.31 6.31 0 0 0 2.56 4.892"/>
          </svg>
          {{ msg.provider_used.model }} · {{ msg.provider_used.kind }}
        </div>
        <div v-if="durationLabel" class="duration-badge" :class="{ 'duration-badge--live': msg.status === 'loading' }" title="Response generation time">
          <svg class="duration-badge__icon" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3" aria-hidden="true" xmlns="http://www.w3.org/2000/svg">
            <circle cx="8" cy="8" r="6.3"/>
            <path d="M8 4.8V8.2l2.3 1.3" stroke-linecap="round" stroke-linejoin="round"/>
          </svg>
          {{ durationLabel }}
        </div>
      </div>

      <div v-if="msg.intent || (msg.phase && msg.status === 'loading')" class="message__status-bar" :class="{ 'message__status-bar--sticky': msg.status === 'loading' }">
        <div v-if="msg.intent" class="intent-badge" :class="`intent-badge--${msg.intent}`">
          {{ intentLabel }}
        </div>
        <div v-if="msg.phase && msg.status === 'loading'" class="phase-label">
          {{ msg.phase }}
        </div>
      </div>

      <!-- Activity log: preambles + tool calls + confirmable actions, unified left-border track -->
      <div
        v-if="msg.chain?.length"
        class="tc-chain"
        :class="{ 'tc-chain--running': msg.status === 'loading' }"
      >
        <button
          v-if="showChainToggle"
          type="button"
          class="tc-chain__toggle"
          :aria-expanded="String(chainExpanded)"
          @click="chainExpanded = !chainExpanded"
        >
          <svg
            class="tc-chain__toggle-chevron"
            viewBox="0 0 10 10"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
            stroke-linecap="round"
            stroke-linejoin="round"
            xmlns="http://www.w3.org/2000/svg"
            aria-hidden="true"
            :style="chainExpanded ? 'transform: rotate(180deg)' : ''"
          >
            <polyline points="2,3 5,7 8,3"/>
          </svg>
          {{ chainExpanded ? 'Hide steps' : `${collapsibleCount} earlier steps` }}
        </button>

        <div
          ref="chainViewportRef"
          class="tc-chain__viewport"
          :class="{ 'tc-chain__viewport--fixed': !chainExpanded }"
          @scroll="onChainViewportScroll"
        >
          <template v-for="(item, idx) in groupedChain" :key="item.id ?? item.type + item.text">
            <ThinkingBlock
              v-if="item.type === 'thinking'"
              v-show="!isCollapsible(idx) || chainExpanded"
              :text="item.text"
              :streaming="item.streaming ?? false"
            />
            <ToolCallStep
              v-else-if="item.type === 'tool_call'"
              v-show="!isCollapsible(idx) || chainExpanded"
              :step="item"
            />
            <ToolCallGroup
              v-else-if="item.type === 'tool_group'"
              v-show="!isCollapsible(idx) || chainExpanded"
              :items="item.items"
            />
            <ParallelResearchBlock
              v-else-if="item.type === 'parallel_research'"
              v-show="!isCollapsible(idx) || chainExpanded"
              :block="item"
            />

            <div v-else-if="item.type === 'confirm_action'" class="message__confirm" role="group" :aria-label="item.description">
              <p class="message__confirm-text">{{ item.description }}</p>

              <ProvisionSteps v-if="item.steps.length" :steps="item.steps" />

              <div v-if="isLast && item.status === 'pending'" class="confirm-actions">
                <button class="p-button--positive is-dense" type="button" @click="$emit('confirm', item.id)">Confirm</button>
                <button class="p-button--base is-dense" type="button" @click="$emit('deny', item.id)">Cancel</button>
              </div>
              <p v-else-if="item.status === 'running'" class="confirm-status confirm-status--running">
                {{ runningVerb(item.name) }}
              </p>
              <p v-else-if="item.status === 'denied'" class="confirm-status confirm-status--denied">Cancelled</p>
              <p v-else-if="item.status === 'done'" class="confirm-status confirm-status--done">{{ item.resultText }}</p>
              <p v-else-if="item.status === 'error'" class="confirm-status confirm-status--error">{{ item.resultText }}</p>
            </div>
          </template>
        </div>

        <div v-if="isLast && pendingConfirmCount > 1" class="confirm-actions confirm-actions--all">
          <button class="p-button--positive is-dense" type="button" @click="$emit('confirmAll')">Approve all</button>
        </div>
      </div>

      <!-- Final answer: streaming phase (TextDelta before Done fires) -->
      <div v-if="msg.pendingAnswer && !msg.answer" class="message__answer message__answer--streaming">
        <div ref="answerBodyRef" class="message__body" v-html="renderedPendingAnswer"></div>
      </div>

      <!-- Final answer: finalized -->
      <div v-if="msg.answer" class="message__answer">
        <div ref="answerBodyRef" class="message__body" v-html="renderedAnswer" />
      </div>

      <p v-if="msg.status === 'error'" class="message-error">{{ msg.error }}</p>
      <p v-else-if="msg.status === 'done' && !msg.answer && !msg.pendingAnswer" class="message-error message-error--muted">
        No response was generated.
      </p>

      <div v-if="sourceLinks.length" class="source-chips">
        <component
          :is="link.href ? 'a' : 'span'"
          v-for="(link, i) in sourceLinks"
          :key="i"
          class="source-chip"
          :class="{ 'source-chip--inert': !link.href }"
          v-bind="link.href ? { href: link.href, target: '_blank', rel: 'noopener' } : {}"
          :title="link.title"
        >
          <span class="source-chip__num">{{ i + 1 }}</span>
          <span class="source-chip__name">{{ link.src.file }}</span>
        </component>
      </div>

      <div v-if="msg.question" class="message__question">
        <p class="message__question-text">{{ msg.question.question }}</p>
        <div class="question-choices">
          <template v-if="isLast">
            <button
              v-for="c in msg.question.choices"
              :key="c"
              class="btn-choice"
              type="button"
              @click="$emit('choice', c)"
            >{{ c }}</button>
          </template>
          <template v-else>
            <span v-for="c in msg.question.choices" :key="c" class="choice-chip">{{ c }}</span>
          </template>
        </div>
        <div v-if="isLast" class="question-other">
          <input
            v-model="otherText"
            class="question-other-input"
            placeholder="Or type your own…"
            @keydown.enter.prevent="submitOther"
          />
          <button class="question-other-submit" type="button" @click="submitOther">Send</button>
        </div>
      </div>
    </template>

    <Teleport to="body">
      <div v-if="lightboxSrc" class="lightbox" @click.self="lightboxSrc = null">
        <button class="lightbox__close" type="button" @click="lightboxSrc = null">×</button>
        <img class="lightbox__img" :src="lightboxSrc" alt="" @click.stop />
      </div>
    </Teleport>
  </div>
</template>

<script setup>
import { computed, ref, watch, nextTick, onMounted, onUnmounted } from 'vue';
import ThinkingBlock  from './ThinkingBlock.vue';
import ToolCallStep   from './ToolCallStep.vue';
import ToolCallGroup  from './ToolCallGroup.vue';
import ParallelResearchBlock from './ParallelResearchBlock.vue';
import ProvisionSteps from '../agents/ProvisionSteps.vue';
import { renderMarkdown, buildCitationIndex, buildFileUrl } from '../../lib/markdown.js';
import { mountInlineGraphs } from '../../lib/inline-graph.js';
import { avatarColor, initials, addCopyButtons, formatDuration } from '../../lib/utils.js';
import { runningVerb } from '../../lib/tool-verbs.js';

const answerBodyRef    = ref(null);
const otherText        = ref('');
const lightboxSrc      = ref(null);
const chainExpanded    = ref(false);
const chainViewportRef = ref(null);
const chainStuck       = ref(true);

const imageAttachments = computed(() => (props.msg.attachments ?? []).filter(a => a.preview_url));
const fileAttachments  = computed(() => (props.msg.attachments ?? []).filter(a => !a.preview_url));

const props = defineProps({
  msg:        { type: Object, required: true },
  isLast:     { type: Boolean, default: false },
  repoUrlMap: { type: Object, default: () => ({}) },
});

const emit = defineEmits(['choice', 'confirm', 'deny', 'confirmAll']);

const roleClass = computed(() =>
  props.msg.role === 'user' ? 'message--user' : 'message--assistant'
);

const senderInitials = computed(() => initials(props.msg.username ?? 'You'));
const senderColor    = computed(() => avatarColor(props.msg.username ?? 'You'));

const pendingConfirmCount = computed(() =>
  (props.msg.chain ?? []).filter(i => i.type === 'confirm_action' && i.status === 'pending').length
);

const tickNow = ref(Date.now());
let tickTimer = null;

watch(() => props.msg.status, (status) => {
  if (status === 'loading' && props.msg.role === 'assistant') {
    if (!tickTimer) tickTimer = setInterval(() => { tickNow.value = Date.now(); }, 200);
  } else if (tickTimer) {
    clearInterval(tickTimer);
    tickTimer = null;
  }
}, { immediate: true });

onUnmounted(() => { if (tickTimer) clearInterval(tickTimer); });

const durationLabel = computed(() => {
  if (props.msg.role !== 'assistant') return null;
  if (props.msg.status === 'loading') {
    return props.msg.startedAt ? formatDuration(tickNow.value - props.msg.startedAt) : null;
  }
  return props.msg.durationMs != null ? formatDuration(props.msg.durationMs) : null;
});

const GROUPABLE_MIN_RUN = 3;

const groupedChain = computed(() => {
  const chain = props.msg.chain ?? [];
  const rows = [];
  let i = 0;
  while (i < chain.length) {
    const item = chain[i];
    if (item.type !== 'tool_call') {
      rows.push(item);
      i++;
      continue;
    }
    let j = i + 1;
    while (j < chain.length && chain[j].type === 'tool_call' && chain[j].name === item.name) j++;
    const run = chain.slice(i, j);
    if (run.length >= GROUPABLE_MIN_RUN) {
      rows.push({ type: 'tool_group', id: `group-${item.id ?? i}`, items: run });
    } else {
      rows.push(...run);
    }
    i = j;
  }
  return rows;
});

const TAIL_SIZE = 5;

const tailStartIndex = computed(() => Math.max(0, groupedChain.value.length - TAIL_SIZE));

const collapsibleIndexes = computed(() => {
  const rows = groupedChain.value;
  const indexes = [];
  rows.forEach((item, idx) => {
    if (idx < tailStartIndex.value && item.type !== 'confirm_action') indexes.push(idx);
  });
  return indexes;
});

const collapsibleCount = computed(() => collapsibleIndexes.value.length);
const showChainToggle  = computed(() => collapsibleCount.value > 0);

function isCollapsible(idx) {
  return showChainToggle.value && collapsibleIndexes.value.includes(idx);
}

const chainSignature = computed(() => {
  const chain = props.msg.chain ?? [];
  const last = chain.at(-1);
  return `${chain.length}:${last?.text?.length ?? 0}:${last?.status ?? ''}:${last?.preview ?? ''}`;
});

function scrollChainViewportToBottom() {
  nextTick(() => {
    const el = chainViewportRef.value;
    if (el) el.scrollTop = el.scrollHeight;
  });
}

function onChainViewportScroll() {
  const el = chainViewportRef.value;
  if (!el) return;
  chainStuck.value = el.scrollHeight - el.scrollTop - el.clientHeight <= 24;
}

watch(chainSignature, () => {
  if (!chainExpanded.value && chainStuck.value) scrollChainViewportToBottom();
});

watch(chainExpanded, (expanded) => {
  if (!expanded) {
    chainStuck.value = true;
    scrollChainViewportToBottom();
  }
});

const intentLabel = computed(() => {
  switch (props.msg.intent) {
    case 'conversational': return 'Answering';
    case 'research':      return 'Researching';
    case 'action':        return 'Executing';
    case 'hybrid':        return 'Researching → Executing';
    default:              return '';
  }
});

const renderedAnswer = computed(() =>
  props.msg.answer
    ? renderMarkdown(props.msg.answer, props.repoUrlMap, buildCitationIndex(props.msg.sources))
    : ''
);

const renderedPendingAnswer = computed(() =>
  props.msg.pendingAnswer
    ? renderMarkdown(props.msg.pendingAnswer, props.repoUrlMap, buildCitationIndex(props.msg.sources))
    : ''
);

onMounted(() => {
  if (answerBodyRef.value) {
    mountInlineGraphs(answerBodyRef.value);
    import('../../lib/mermaid.js').then(({ mountMermaidDiagrams }) => {
      if (answerBodyRef.value) mountMermaidDiagrams(answerBodyRef.value);
    });
    addCopyButtons(answerBodyRef.value);
  }
  scrollChainViewportToBottom();
});

watch([renderedAnswer, renderedPendingAnswer], () => nextTick(() => {
  if (answerBodyRef.value) {
    mountInlineGraphs(answerBodyRef.value);
    import('../../lib/mermaid.js').then(({ mountMermaidDiagrams }) => {
      if (answerBodyRef.value) mountMermaidDiagrams(answerBodyRef.value);
    });
    addCopyButtons(answerBodyRef.value);
  }
}));

function submitOther() {
  const text = otherText.value.trim();
  if (!text) return;
  otherText.value = '';
  emit('choice', text);
}

function sourceHref(src) {
  const base = props.repoUrlMap[src.repo];
  if (!base) return null;
  return buildFileUrl(base, src.version ?? 'main', src.file, src.line, src.end_line ?? null);
}

function sourceTitle(src) {
  if (!src.line) return `${src.repo} ${src.version ?? 'main'} · ${src.file}`;
  const lineDisplay = src.end_line ? `${src.line}-${src.end_line}` : `${src.line}`;
  return `${src.repo} ${src.version ?? 'main'} · ${src.file}:${lineDisplay}`;
}

const sourceLinks = computed(() => (props.msg.sources ?? []).map(src => ({
  src,
  href: sourceHref(src),
  title: sourceTitle(src),
})));
</script>
