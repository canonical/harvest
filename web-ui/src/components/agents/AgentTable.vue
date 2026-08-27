<template>
  <table class="p-table" data-testid="agent-table">
    <thead>
      <tr>
        <th>Status</th>
        <th>Hostname</th>
        <th>Last seen</th>
        <th v-if="showActions">Actions</th>
      </tr>
    </thead>
    <tbody>
      <tr v-for="agent in agents" :key="agent.id">
        <td>
          <span
            class="p-label agent-status"
            :class="agent.online ? 'p-label--positive agent-status--online' : 'agent-status--offline'"
          >
            {{ agent.online ? 'Online' : 'Offline' }}
          </span>
        </td>
        <td>
          <span class="agent-type-icon" :title="agent.provider === 'lxd' ? 'Harvest-managed (LXD)' : 'Manually installed'">
            <svg v-if="agent.provider === 'lxd'" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="width: 16px; height: 16px" aria-hidden="true"><path d="M18 10h-1.26A8 8 0 1 0 9 20h9a5 5 0 0 0 0-10z"/></svg>
            <svg v-else xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="width: 16px; height: 16px" aria-hidden="true"><rect x="2" y="2" width="20" height="8" rx="2" ry="2"/><rect x="2" y="14" width="20" height="8" rx="2" ry="2"/><line x1="6" y1="6" x2="6.01" y2="6"/><line x1="6" y1="18" x2="6.01" y2="18"/></svg>
          </span>
          {{ agent.hostname || agent.id }}
          <span v-if="agent.provider === 'lxd'" class="agent-provider-badge">LXD</span>
        </td>
        <td>{{ relativeTime(agent.last_seen) }}</td>
        <td v-if="showActions">
          <div class="agent-row-actions">
            <router-link
              :to="`/agents/${agent.id}/console`"
              class="console-icon-btn"
              title="Open console"
              aria-label="Open console"
            >
              <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="3" width="20" height="14" rx="2" ry="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/></svg>
            </router-link>
            <button
              class="console-icon-btn console-icon-btn--danger"
              type="button"
              title="Delete agent"
              aria-label="Delete agent"
              @click="$emit('delete', agent)"
            >
              <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><line x1="10" y1="11" x2="10" y2="17"/><line x1="14" y1="11" x2="14" y2="17"/></svg>
            </button>
          </div>
        </td>
      </tr>
    </tbody>
  </table>
</template>

<script setup>
defineProps({
  agents:     { type: Array, default: () => [] },
  showActions: { type: Boolean, default: true },
});
defineEmits(['delete']);

function relativeTime(iso) {
  if (!iso) return '—';
  const s = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
  if (s < 60)    return 'just now';
  if (s < 3600)  return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}
</script>
