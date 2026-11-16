<template>
  <AppModal v-if="pending" :title="pending.title" @close="settleConfirmation(false)">
    <p class="modal-lede">{{ pending.message }}</p>
    <template #actions>
      <button class="p-button--base is-dense" type="button" @click="settleConfirmation(false)">Cancel</button>
      <button
        class="is-dense"
        :class="pending.danger ? 'p-button--negative' : 'p-button--positive'"
        type="button"
        @click="settleConfirmation(true)"
      >{{ pending.confirmLabel }}</button>
    </template>
  </AppModal>
</template>

<script setup>
import AppModal from './AppModal.vue';
import { pendingConfirmation as pending, settleConfirmation } from '../../composables/useConfirm.js';
</script>
