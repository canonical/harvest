<template>
  <svg
    class="raccoon"
    :class="`raccoon--${mode}`"
    :viewBox="VIEW_BOX"
    aria-hidden="true"
    xmlns="http://www.w3.org/2000/svg"
  >
    <defs>
      <mask :id="maskId" maskUnits="userSpaceOnUse">
        <circle :cx="CENTER.x" :cy="CENTER.y" :r="DISC_RADIUS" fill="#fff" />
        <g class="raccoon__face">
          <path :d="FACE" fill="#000" />
          <path class="raccoon__nose" :d="NOSE" fill="#fff" />
          <path v-for="(eye, index) in EYES" :key="index" class="raccoon__eye" :d="eye" fill="#000" />
        </g>
      </mask>
    </defs>
    <g class="raccoon__rays">
      <g v-for="(segments, index) in RAYS" :key="index" class="raccoon__ray" :style="isWorking ? { animationDelay: rayDelay(index) } : null">
        <path v-for="([d, width], part) in segments" :key="part" :d="d" :stroke-width="width" />
      </g>
    </g>
    <circle :cx="CENTER.x" :cy="CENTER.y" :r="DISC_RADIUS" :mask="`url(#${maskId})`" fill="currentColor" />
  </svg>
</template>

<script>
let marks = 0;
</script>

<script setup>
import { computed } from 'vue';
import { CENTER, DISC_RADIUS, EYES, FACE, NOSE, RACCOON_MODE, RAYS, VIEW_BOX, rayDelay } from '../../lib/raccoon-mark.js';

const props = defineProps({
  mode: {
    type: String,
    default: RACCOON_MODE.RESTING,
    validator: value => Object.values(RACCOON_MODE).includes(value),
  },
});

const isWorking = computed(() => props.mode === RACCOON_MODE.WORKING);

const maskId = `raccoon-mask-${++marks}`;
</script>
