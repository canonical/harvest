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
      <span v-if="msg.status === 'loading' && !msg.chain?.length && !msg.pendingAnswer" class="loading-orbit">
        <svg viewBox="0 0 16 16" fill="currentColor" aria-hidden="true" xmlns="http://www.w3.org/2000/svg">
          <path d="M.81 7.36a1.92 1.92 0 1 1 3.799.572A1.92 1.92 0 0 1 .81 7.36M8.826 3.033a1.92 1.92 0 1 1 3.755.806 1.92 1.92 0 0 1-3.755-.806M7.04 12.585a4.68 4.68 0 0 1-3.19-2.432 2.76 2.76 0 0 1-1.64.202 6.25 6.25 0 0 0 4.498 3.77c.45.098.908.144 1.364.141a2.74 2.74 0 0 1-.562-1.605 5 5 0 0 1-.47-.076M8.394 12.193a1.92 1.92 0 0 1 3.754.805 1.92 1.92 0 1 1-3.754-.805M12.943 11.89a6.3 6.3 0 0 0 1.22-2.587 6.3 6.3 0 0 0-.905-4.782 2.77 2.77 0 0 1-1.08 1.265 4.7 4.7 0 0 1-.154 4.674c.45.37.77.87.919 1.43M2.56 4.892a2.75 2.75 0 0 1 1.603.41 4.68 4.68 0 0 1 3.77-2.015q.012-.218.057-.433c.088-.411.268-.795.525-1.124A6.31 6.31 0 0 0 2.56 4.892"/>
        </svg>
        <span class="loading-orbit__label">Thinking…</span>
      </span>

      <!-- Activity log: preambles + tool calls + confirmable actions, unified left-border track -->
      <div
        v-if="msg.chain?.length"
        class="tc-chain"
        :class="{ 'tc-chain--running': msg.status === 'loading' }"
      >
        <template v-for="item in msg.chain" :key="item.id ?? item.type + item.text">
          <ThinkingBlock
            v-if="item.type === 'thinking'"
            :text="item.text"
            :streaming="item.streaming ?? false"
          />
          <ToolCallStep v-else-if="item.type === 'tool_call'" :step="item" />

          <div v-else-if="item.type === 'confirm_action'" class="message__confirm">
            <p class="message__confirm-text">{{ item.description }}</p>

            <ProvisionSteps v-if="item.steps.length" :steps="item.steps" />

            <div v-if="isLast && item.status === 'pending'" class="confirm-actions">
              <button class="p-button--negative" type="button" @click="$emit('confirm', item.id)">Confirm</button>
              <button class="p-button--base" type="button" @click="$emit('deny', item.id)">Cancel</button>
            </div>
            <p v-else-if="item.status === 'running'" class="confirm-status confirm-status--running">
              {{ item.name === 'delete_agent' ? 'Deleting…' : 'Creating…' }}
            </p>
            <p v-else-if="item.status === 'denied'" class="confirm-status confirm-status--denied">Cancelled</p>
            <p v-else-if="item.status === 'done'" class="confirm-status confirm-status--done">{{ item.resultText }}</p>
            <p v-else-if="item.status === 'error'" class="confirm-status confirm-status--error">{{ item.resultText }}</p>
          </div>
        </template>

        <div v-if="isLast && pendingConfirmCount > 1" class="confirm-actions confirm-actions--all">
          <button class="p-button--negative" type="button" @click="$emit('confirmAll')">Approve all</button>
        </div>
      </div>

      <!-- Final answer: streaming phase (TextDelta before Done fires) -->
      <div v-if="msg.pendingAnswer && !msg.answer" class="message__bubble message__bubble--streaming">
        <div class="message__body">{{ msg.pendingAnswer }}<span class="answer-cursor" aria-hidden="true">▋</span></div>
      </div>

      <!-- Final answer: finalized -->
      <div v-if="msg.answer" class="message__bubble">
        <div ref="answerBodyRef" class="message__body" v-html="renderedAnswer" />
      </div>

      <p v-if="msg.status === 'error'" class="message-error">{{ msg.error }}</p>

      <div v-if="msg.sources?.length" class="source-chips">
        <a
          v-for="(src, i) in msg.sources"
          :key="i"
          class="source-chip"
          :href="sourceHref(src)"
          :title="`${src.file}:${src.line}`"
          target="_blank"
          rel="noopener"
        >
          <span class="source-chip__num">{{ i + 1 }}</span>
          <span class="source-chip__name">{{ src.file }}</span>
        </a>
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
import { computed, ref, watch, nextTick, onMounted } from 'vue';
import ThinkingBlock from './ThinkingBlock.vue';
import ToolCallStep  from './ToolCallStep.vue';
import ProvisionSteps from '../agents/ProvisionSteps.vue';
import { renderMarkdown, buildCitationIndex } from '../../lib/markdown.js';
import { mountInlineGraphs } from '../../lib/inline-graph.js';
import { avatarColor, initials, addCopyButtons } from '../../lib/utils.js';

const answerBodyRef = ref(null);
const otherText     = ref('');
const lightboxSrc   = ref(null);

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

const renderedAnswer = computed(() =>
  props.msg.answer
    ? renderMarkdown(props.msg.answer, props.repoUrlMap, buildCitationIndex(props.msg.sources))
    : ''
);

onMounted(() => {
  if (answerBodyRef.value) {
    mountInlineGraphs(answerBodyRef.value);
    addCopyButtons(answerBodyRef.value);
  }
});

watch(renderedAnswer, () => nextTick(() => {
  if (answerBodyRef.value) {
    mountInlineGraphs(answerBodyRef.value);
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
  if (!base) return '#';
  return `${base.replace(/\/$/, '')}/blob/${src.version ?? 'main'}/${src.file}#L${src.line}`;
}
</script>
