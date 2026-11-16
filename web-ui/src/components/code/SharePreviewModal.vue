<template>
  <AppModal title="Share preview" @close="emit('close')">
    <fieldset class="share-audiences">
      <legend>Who can open it</legend>
      <label v-for="option in AUDIENCES" :key="option.id" class="share-audience">
        <input type="radio" name="preview-audience" :value="option.id" :checked="preview.audience === option.id" :disabled="saving" @change="choose(option.id)" />
        <span class="share-audience__label">{{ option.label }}</span>
        <span class="share-audience__description">{{ AUDIENCE_DESCRIPTIONS[option.id] }}</span>
      </label>
    </fieldset>
    <div v-if="preview.audience !== 'owner'" class="share-link">
      <label :for="linkId">Link</label>
      <div class="share-link__row">
        <input :id="linkId" :value="link" type="text" readonly @focus="$event.target.select()" />
        <button class="p-button--base is-dense" type="button" @click="copy">Copy link</button>
      </div>
    </div>
    <p class="modal-lede">Everyone except you gets their own copy of the session's container, so nobody sees another person's data.</p>
    <template #actions>
      <button class="p-button--positive is-dense" type="button" @click="emit('close')">Done</button>
    </template>
  </AppModal>
</template>

<script setup>
import { computed, ref } from 'vue';
import AppModal from '../ui/AppModal.vue';
import { AUDIENCES, shareUrl } from '../../lib/preview-api.js';
import { showToast } from '../../composables/useToasts.js';

const props = defineProps({
  preview: { type: Object, required: true },
  share: { type: Function, required: true },
});
const emit = defineEmits(['close']);

const AUDIENCE_DESCRIPTIONS = {
  owner: 'Only you, while the session runs.',
  repository: 'People who can access this repository in Harvest.',
  link: 'Anyone with the link, signed in or not.',
};

const linkId = `share-link-${Math.random().toString(36).slice(2, 8)}`;
const saving = ref(false);
const link = computed(() => shareUrl(props.preview.id));

async function choose(audience) {
  saving.value = true;
  try {
    await props.share(props.preview.id, audience);
  } catch (e) {
    showToast({ message: e.message, kind: 'negative' });
  } finally {
    saving.value = false;
  }
}

async function copy() {
  try {
    await navigator.clipboard.writeText(link.value);
    showToast({ message: 'Link copied' });
  } catch {
    showToast({ message: 'Copy the link from the field instead', kind: 'caution' });
  }
}
</script>
