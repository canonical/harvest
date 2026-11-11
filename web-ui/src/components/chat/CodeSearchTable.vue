<template>
  <table class="cs-table">
    <thead>
      <tr>
        <th scope="col">Symbol</th>
        <th v-if="hasDetail" scope="col">Detail</th>
      </tr>
    </thead>
    <tbody>
      <tr v-for="(row, index) in rows" :key="index">
        <td v-if="row.symbol" class="cs-table__symbol">
          <CodeSearchLink :location="row.symbol" :view="view" @select="emit('select', $event)" />
          <span class="cs-table__location">{{ row.symbol.file }}:{{ row.symbol.line }}</span>
        </td>
        <td v-else class="cs-table__symbol"></td>
        <td v-if="hasDetail">{{ row.detail }}</td>
      </tr>
    </tbody>
  </table>
</template>

<script setup>
import { computed } from 'vue';
import CodeSearchLink from './CodeSearchLink.vue';

const props = defineProps({
  rows: { type: Array, required: true },
  view: { type: Object, required: true },
});

const emit = defineEmits(['select']);

const hasDetail = computed(() => props.rows.some(row => row.detail));
</script>
