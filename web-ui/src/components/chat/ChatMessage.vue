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
      <div v-if="showActivity" class="message__status-bar" :class="{ 'message__status-bar--sticky': isLoading }">
        <ActivityLine
          v-if="showActivity"
          :activity="activity"
          :steps="steps"
          :expanded="chainExpanded"
          :elapsed="isLoading ? durationLabel : null"
          :details="runDetails"
          @toggle="chainExpanded = !chainExpanded"
        />
      </div>

      <div
        v-if="msg.chain?.length"
        v-show="chainExpanded || hasConfirmations"
        class="tc-chain"
        :class="{ 'tc-chain--running': isLoading }"
      >
        <div class="tc-chain__list">
          <template v-for="item in groupedChain" :key="item.id ?? item.type + item.text">
            <ThinkingBlock
              v-if="item.type === 'thinking'"
              v-show="chainExpanded"
              :text="item.text"
              :streaming="item.streaming ?? false"
            />
            <ToolCallStep
              v-else-if="item.type === 'tool_call'"
              v-show="chainExpanded"
              :step="item"
            />
            <ToolCallGroup
              v-else-if="item.type === 'tool_group'"
              v-show="chainExpanded"
              :items="item.items"
            />
            <ParallelResearchBlock
              v-else-if="item.type === 'parallel_research'"
              v-show="chainExpanded"
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
      <div v-if="isStreamingAnswer" class="message__answer message__answer--streaming">
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

      <div v-if="sourceGroups.length" class="sources-block">
        <button
          type="button"
          class="sources-toggle"
          :aria-expanded="String(sourcesExpanded)"
          @click="sourcesExpanded = !sourcesExpanded"
        >
          <svg
            class="sources-toggle__chevron"
            viewBox="0 0 10 10"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
            stroke-linecap="round"
            stroke-linejoin="round"
            xmlns="http://www.w3.org/2000/svg"
            aria-hidden="true"
            :style="sourcesExpanded ? 'transform: rotate(180deg)' : ''"
          >
            <polyline points="2,3 5,7 8,3"/>
          </svg>
          Sources
          <span class="sources-toggle__count">{{ sourceLinks.length }}</span>
        </button>

        <div v-show="sourcesExpanded" class="source-groups">
          <div
            v-for="group in sourceGroups"
            :key="`${group.repo}:${group.version}:${group.file}`"
            class="source-group"
          >
            <div class="source-group__file" :title="`${group.repo} ${group.version}`">
              <span class="source-group__file-name">{{ group.file }}</span>
              <span class="source-group__file-meta">{{ group.repo }}</span>
            </div>
            <div class="source-group__locations">
              <component
                :is="loc.href ? 'a' : 'span'"
                v-for="loc in group.locations"
                :key="loc.num"
                class="source-loc"
                :class="{ 'source-loc--inert': !loc.href }"
                v-bind="loc.href ? { href: loc.href, target: '_blank', rel: 'noopener' } : {}"
                :title="loc.title"
              >
                <span class="source-loc__num">{{ loc.num }}</span>
                <span class="source-loc__line">{{ loc.lineLabel }}</span>
              </component>
            </div>
          </div>
        </div>
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
import ActivityLine   from './ActivityLine.vue';
import ProvisionSteps from '../agents/ProvisionSteps.vue';
import { renderMarkdown, buildCitationIndex, buildFileUrl, parseCitations, withoutOpenBracket } from '../../lib/markdown.js';
import { mountInlineGraphs } from '../../lib/inline-graph.js';
import { avatarColor, initials, addCopyButtons, formatDuration } from '../../lib/utils.js';
import { runningVerb } from '../../lib/tool-verbs.js';
import { groupChain } from '../../lib/chain-grouping.js';
import { describeActivity, isAnswerText, stepCount } from '../../lib/agent-activity.js';

const answerBodyRef    = ref(null);
const otherText        = ref('');
const lightboxSrc      = ref(null);
const chainExpanded    = ref(false);

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

const costLabel = computed(() => {
  if (props.msg.role !== 'assistant' || props.msg.status === 'loading') return null;
  const u = props.msg.usage;
  const calls = props.msg.llm_call_count ?? 0;
  const cost = props.msg.cost_microusd ?? 0;
  if (!u && cost === 0) return null;
  const parts = [];
  if (cost > 0) parts.push(formatCost(cost));
  if (u) {
    const inTok = (u.input_tokens ?? 0) + (u.cache_read_tokens ?? 0) + (u.cache_creation_tokens ?? 0);
    const outTok = (u.output_tokens ?? 0) + (u.reasoning_tokens ?? 0);
    parts.push(`${inTok.toLocaleString()} in / ${outTok.toLocaleString()} out`);
  }
  if (calls > 0) parts.unshift(`${calls} call${calls === 1 ? '' : 's'}`);
  return parts.join(' · ') || null;
});

function formatCost(microusd) {
  const dollars = microusd / 1_000_000;
  if (dollars < 0.01) return `$${dollars.toFixed(6)}`;
  if (dollars < 1) return `$${dollars.toFixed(4)}`;
  return `$${dollars.toFixed(2)}`;
}

const groupedChain = computed(() => groupChain(props.msg.chain ?? []));

const isLoading = computed(() => props.msg.status === 'loading');
const isAssistant = computed(() => props.msg.role === 'assistant');
const steps = computed(() => stepCount(props.msg));
const activity = computed(() => describeActivity(props.msg, durationLabel.value));
const runDetails = computed(() =>
  isLoading.value ? [] : [props.msg.provider_used?.model, costLabel.value].filter(Boolean)
);
const showActivity = computed(() =>
  isAssistant.value && (isLoading.value || steps.value > 0 || runDetails.value.length > 0 || Boolean(durationLabel.value))
);
const hasConfirmations = computed(() => (props.msg.chain ?? []).some(i => i.type === 'confirm_action'));
const isStreamingAnswer = computed(() =>
  Boolean(props.msg.pendingAnswer && !props.msg.answer && isAnswerText(props.msg.pendingAnswer))
);

const renderedAnswer = computed(() =>
  props.msg.answer
    ? renderMarkdown(props.msg.answer, props.repoUrlMap, buildCitationIndex(props.msg.sources))
    : ''
);

const renderedPendingAnswer = computed(() => {
  const text = withoutOpenBracket(props.msg.pendingAnswer ?? '');
  return text ? renderMarkdown(text, props.repoUrlMap, buildCitationIndex(parseCitations(text))) : '';
});

onMounted(() => {
  if (answerBodyRef.value) {
    mountInlineGraphs(answerBodyRef.value);
    import('../../lib/mermaid.js').then(({ mountMermaidDiagrams }) => {
      if (answerBodyRef.value) mountMermaidDiagrams(answerBodyRef.value);
    });
    addCopyButtons(answerBodyRef.value);
  }
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

const sourceGroups = computed(() => {
  const groups = [];
  const byKey = new Map();
  sourceLinks.value.forEach((link, i) => {
    const { src } = link;
    const key = `${src.repo}:${src.version}:${src.file}`;
    let group = byKey.get(key);
    if (!group) {
      group = { repo: src.repo, version: src.version ?? 'main', file: src.file, locations: [] };
      byKey.set(key, group);
      groups.push(group);
    }
    group.locations.push({
      num: i + 1,
      href: link.href,
      title: link.title,
      lineLabel: src.line ? (src.end_line ? `${src.line}-${src.end_line}` : `${src.line}`) : 'file',
    });
  });
  return groups;
});

const sourcesExpanded = ref(false);
</script>
