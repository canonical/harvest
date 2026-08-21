<template>
  <div class="issue-comments" data-testid="issue-comments">
    <p v-if="!comments.length" class="issue-comments__empty">No activity yet.</p>

    <ul v-else class="issue-comments__list">
      <li v-for="c in comments" :key="c.id" class="issue-comments__item" data-testid="issue-comment">
        <div class="issue-comments__header">
          <span
            class="issue-comments__author"
            :class="{ 'issue-comments__author--harvest': c.author_type === 'harvest' }"
          >
            {{ c.author_name }}
            <span v-if="c.author_type === 'harvest'" class="p-chip" data-testid="harvest-badge">Harvest</span>
          </span>
          <span class="issue-comments__time">{{ formatTime(c.created_at) }}</span>
        </div>
        <div class="issue-comments__body" v-html="renderMarkdown(c.body, {}, {})"></div>
      </li>
    </ul>

    <div class="form-group issue-comments__composer">
      <textarea
        v-model="draft"
        data-testid="issue-comment-input"
        rows="2"
        placeholder="Add a comment"
      ></textarea>
      <button
        class="p-button--positive is-dense"
        data-testid="post-comment-btn"
        type="button"
        :disabled="!draft.trim()"
        @click="post"
      >Post comment</button>
    </div>
  </div>
</template>

<script setup>
import { ref } from 'vue';
import { renderMarkdown } from '../../lib/markdown.js';

defineProps({
  comments: { type: Array, default: () => [] },
});
const emit = defineEmits(['post-comment']);

const draft = ref('');

function formatTime(iso) {
  if (!iso) return '';
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
}

function post() {
  const text = draft.value.trim();
  if (!text) return;
  emit('post-comment', text);
  draft.value = '';
}
</script>
