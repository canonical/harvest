<template>
  <svg
    class="segment-mark"
    :class="{ 'segment-mark--spinning': spinning, 'segment-mark--settling': settling }"
    viewBox="0 0 48 48"
    aria-hidden="true"
    xmlns="http://www.w3.org/2000/svg"
  >
    <g ref="turn" class="segment-mark__turn">
      <circle
        v-for="(offset, index) in SEGMENT_OFFSETS"
        :key="index"
        ref="segments"
        class="segment-mark__segment"
        :class="`segment-mark__segment--${index}`"
        cx="24"
        cy="24"
        r="17"
        :pathLength="PATH_LENGTH"
        :style="{ strokeDashoffset: offset }"
      />
    </g>
  </svg>
</template>

<script setup>
import { ref, watch } from 'vue';
import { PATH_LENGTH, REST_DASH, SEGMENT_OFFSETS, angleOf, forwardDelta, settlePlan } from '../../lib/segment-mark.js';

const props = defineProps({
  active: { type: Boolean, default: false },
});

const turn = ref(null);
const segments = ref([]);
const spinning = ref(props.active);
const settling = ref(false);

const nextFrame = () => new Promise(resolve => requestAnimationFrame(resolve));
const currentAngle = () => angleOf(getComputedStyle(turn.value).transform);

function clearInline() {
  [turn.value, ...segments.value].forEach(node => {
    node.style.transform = '';
    node.style.strokeDasharray = '';
    node.style.transitionDuration = '';
  });
}

async function settle() {
  const firstTime = await nextFrame();
  const firstAngle = currentAngle();
  const time = await nextFrame();
  if (props.active || !turn.value) return;
  const angle = currentAngle();
  const speed = forwardDelta(firstAngle, angle) / Math.max((time - firstTime) / 1000, 0.001);
  const { target, seconds } = settlePlan(angle, speed);
  const dashes = segments.value.map(segment => getComputedStyle(segment).strokeDasharray);
  turn.value.style.transform = `rotate(${angle}deg)`;
  segments.value.forEach((segment, index) => { segment.style.strokeDasharray = dashes[index]; });
  [turn.value, ...segments.value].forEach(node => { node.style.transitionDuration = `${seconds}s`; });
  spinning.value = false;
  settling.value = true;
  await nextFrame();
  turn.value.style.transform = `rotate(${target}deg)`;
  segments.value.forEach(segment => { segment.style.strokeDasharray = REST_DASH; });
}

watch(() => props.active, active => {
  if (active) {
    settling.value = false;
    clearInline();
    spinning.value = true;
  } else if (spinning.value) {
    settle();
  }
});
</script>
