<template>
  <div class="message" :class="roleClass">
    <template v-if="msg.role === 'user'">
      <div class="message__sender">
        <div class="message-avatar" :style="{ background: senderColor }">{{ senderInitials }}</div>
        <span class="message__sender-name">{{ msg.username ?? 'You' }}</span>
      </div>
      <div class="message__bubble">
        <div class="message__body">{{ msg.text }}</div>
        <div v-if="msg.attachments?.length" class="message-attachments">
          <img
            v-for="(a, i) in msg.attachments.filter(x => x.preview_url)"
            :key="i"
            class="message-attachment-thumb"
            :src="a.preview_url"
            :alt="a.name"
          />
        </div>
      </div>
    </template>

    <template v-else>
      <!-- Loading indicator: only while no activity or answer has arrived yet -->
      <span v-if="msg.status === 'loading' && !msg.chain?.length && !msg.pendingAnswer" class="loading-dots">
        <span>.</span><span>.</span><span>.</span>
      </span>

      <!-- Activity log: preambles + tool calls, unified left-border track -->
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
        </template>
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
  </div>
</template>

<script setup>
import { computed, ref, watch, nextTick, onMounted } from 'vue';
import ThinkingBlock from './ThinkingBlock.vue';
import ToolCallStep  from './ToolCallStep.vue';
import { renderMarkdown } from '../../lib/markdown.js';
import { mountInlineGraphs } from '../../lib/inline-graph.js';
import { avatarColor, initials } from '../../lib/utils.js';

const answerBodyRef = ref(null);
const otherText     = ref('');

const props = defineProps({
  msg:        { type: Object, required: true },
  isLast:     { type: Boolean, default: false },
  repoUrlMap: { type: Object, default: () => ({}) },
});

const emit = defineEmits(['choice']);

const roleClass = computed(() =>
  props.msg.role === 'user' ? 'message--user' : 'message--assistant'
);

const senderInitials = computed(() => initials(props.msg.username ?? 'You'));
const senderColor    = computed(() => avatarColor(props.msg.username ?? 'You'));

const renderedAnswer = computed(() =>
  props.msg.answer ? renderMarkdown(props.msg.answer) : ''
);

onMounted(() => {
  if (answerBodyRef.value) mountInlineGraphs(answerBodyRef.value);
});

watch(renderedAnswer, () => nextTick(() => {
  if (answerBodyRef.value) mountInlineGraphs(answerBodyRef.value);
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
