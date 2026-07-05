<template>
  <div class="lxd-steps">
    <div v-for="step in steps" :key="step.id" class="lxd-step" :class="`lxd-step--${step.status}`">
      <svg v-if="step.status === 'active'" class="lxd-step__icon lxd-step__icon--spinning" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" xmlns="http://www.w3.org/2000/svg">
        <circle cx="8" cy="8" r="6" stroke-opacity="0.25"/>
        <path d="M14 8a6 6 0 0 0-6-6"/>
      </svg>
      <svg v-else-if="step.status === 'done'" class="lxd-step__icon" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" xmlns="http://www.w3.org/2000/svg">
        <polyline points="3 8 7 12 13 4"/>
      </svg>
      <svg v-else-if="step.status === 'error'" class="lxd-step__icon" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" xmlns="http://www.w3.org/2000/svg">
        <line x1="4" y1="4" x2="12" y2="12"/>
        <line x1="12" y1="4" x2="4" y2="12"/>
      </svg>
      <svg v-else class="lxd-step__icon" viewBox="0 0 16 16" fill="currentColor" xmlns="http://www.w3.org/2000/svg">
        <circle cx="8" cy="8" r="2.5"/>
      </svg>
      <span class="lxd-step__label">{{ step.label }}</span>
      <span v-if="step.detail && step.status !== 'error'" class="lxd-step__detail">{{ step.detail }}</span>
    </div>
  </div>
</template>

<script setup>
defineProps({ steps: { type: Array, required: true } });
</script>
