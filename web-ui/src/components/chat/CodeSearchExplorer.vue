<template>
  <div class="cs">
    <div class="cs-summary">
      <span class="cs-badge">{{ summary.items }} code item{{ summary.items === 1 ? '' : 's' }} · {{ summary.files }} file{{ summary.files === 1 ? '' : 's' }}</span>
      <span v-if="summary.documentation" class="cs-badge cs-badge--unverified">{{ summary.documentation }} documentation · not verified</span>
      <span v-if="summary.gaps" class="cs-badge cs-badge--gap">{{ summary.gaps }} gap{{ summary.gaps === 1 ? '' : 's' }}</span>
      <span v-if="summary.tier" class="cs-badge">{{ summary.tier }}<template v-if="summary.milliseconds != null"> · {{ summary.milliseconds }} ms</template></span>
    </div>

    <div class="cs-panes">
      <div class="cs-side">
        <div class="cs-tabs" role="tablist" aria-label="Search results">
          <button
            v-for="entry in tabs"
            :key="entry.id"
            type="button"
            role="tab"
            class="cs-tab"
            :aria-selected="String(tab === entry.id)"
            :disabled="entry.count === 0"
            @click="tab = entry.id"
          >
            {{ entry.label }} <span class="cs-tab__count">{{ entry.count }}</span>
          </button>
        </div>

        <div class="cs-list" role="listbox" tabindex="0" :aria-label="`${activeLabel} results`" @keydown="onKeydown">
          <template v-if="tab === TAB.CODE">
            <template v-for="group in groups" :key="group.file || 'tables'">
              <div class="cs-list__file" :title="group.file">{{ group.file || 'Tables' }}</div>
              <button
                v-for="entry in group.entries"
                :key="entry.index"
                type="button"
                role="option"
                class="cs-list__entry"
                :aria-selected="String(entry.index === selectedItem)"
                @click="selectItem(entry.index)"
              >
                <span class="cs-kind cs-kind--small" :class="`cs-kind--${entry.item.kind}`">{{ kindLetter(entry.item.kind) }}</span>
                <span class="cs-list__name">{{ entry.item.name }}</span>
              </button>
            </template>
          </template>
          <template v-else-if="tab === TAB.DOCS">
            <button
              v-for="(entry, index) in view.documentation"
              :key="index"
              type="button"
              role="option"
              class="cs-list__entry"
              :aria-selected="String(index === selectedDocument)"
              @click="selectedDocument = index"
            >
              <span class="cs-kind cs-kind--small cs-kind--doc">D</span>
              <span class="cs-list__name">{{ entry.name }}</span>
            </button>
          </template>
          <template v-else>
            <p v-for="(gap, index) in view.gaps" :key="index" class="cs-list__gap">{{ gap }}</p>
          </template>
        </div>
      </div>

      <div class="cs-main">
        <CodeSearchItem
          v-if="tab === TAB.CODE && currentItem"
          :item="currentItem"
          :view="view"
          @select="selectLocation"
        />
        <CodeSearchDocument
          v-else-if="tab === TAB.DOCS && currentDocument"
          :documentation="currentDocument"
          :view="view"
          @select="selectLocation"
        />
        <p v-else class="cs-note">These are things the search could not find. The agent may read them next.</p>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed, ref } from 'vue';
import CodeSearchItem from './CodeSearchItem.vue';
import CodeSearchDocument from './CodeSearchDocument.vue';
import { TAB, firstTab, groupByFile, itemIndexAt, kindLetter, summarize } from '../../lib/code-search-view.js';

const props = defineProps({
  view: { type: Object, required: true },
});

const tab = ref(firstTab(props.view));
const selectedItem = ref(0);
const selectedDocument = ref(0);

const summary = computed(() => summarize(props.view));
const groups = computed(() => groupByFile(props.view.items));
const itemOrder = computed(() => groups.value.flatMap(group => group.entries.map(entry => entry.index)));
const currentItem = computed(() => props.view.items[selectedItem.value] ?? null);
const currentDocument = computed(() => props.view.documentation[selectedDocument.value] ?? null);

const tabs = computed(() => [
  { id: TAB.CODE, label: 'Code', count: props.view.items.length },
  { id: TAB.DOCS, label: 'Docs', count: props.view.documentation.length },
  { id: TAB.GAPS, label: 'Gaps', count: props.view.gaps.length },
]);
const activeLabel = computed(() => tabs.value.find(t => t.id === tab.value)?.label ?? '');

function selectItem(index) {
  tab.value = TAB.CODE;
  selectedItem.value = index;
}

function selectLocation(location) {
  const index = itemIndexAt(props.view, location);
  if (index >= 0) selectItem(index);
}

function step(order, current, delta) {
  const position = order.indexOf(current);
  const next = Math.min(order.length - 1, Math.max(0, position + delta));
  return order[next] ?? current;
}

const KEY_DELTAS = { ArrowDown: 1, ArrowUp: -1 };

function onKeydown(event) {
  const delta = KEY_DELTAS[event.key];
  if (!delta) return;
  event.preventDefault();
  if (tab.value === TAB.CODE) {
    selectedItem.value = step(itemOrder.value, selectedItem.value, delta);
  } else if (tab.value === TAB.DOCS) {
    const order = props.view.documentation.map((_, index) => index);
    selectedDocument.value = step(order, selectedDocument.value, delta);
  }
}
</script>
