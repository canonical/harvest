export const JOB_KIND_LABELS = Object.freeze({
  prepare: 'Environment',
  command: 'Command',
  checks: 'Checks',
  review: 'Review',
});

const STATUS_LABELS = Object.freeze({
  running: 'Running',
  succeeded: 'Passed',
  failed: 'Failed',
  cancelled: 'Stopped',
});

export function statusLabel(job) {
  if (job.kind === 'review' && job.status === 'succeeded') return 'Done';
  return STATUS_LABELS[job.status] ?? job.status;
}

export function isRunning(job) {
  return job?.status === 'running';
}

export function canStop(job) {
  return isRunning(job) && job.kind !== 'review';
}

export function needsFix(job) {
  return job?.kind === 'checks' && job.status === 'failed';
}

export function latestOfKind(jobs, kind) {
  return jobs.find(job => job.kind === kind) ?? null;
}

export function checksBadge(jobs) {
  const checks = latestOfKind(jobs, 'checks');
  if (!checks) return null;
  if (isRunning(checks)) return { label: 'Checks running…', kind: 'running' };
  if (checks.status === 'succeeded') return { label: 'Checks passed', kind: 'passed' };
  if (checks.status === 'failed') return { label: 'Checks failed', kind: 'failed' };
  return null;
}

export function duration(job, now = Date.now()) {
  const start = Date.parse(job.created_at);
  const end = job.finished_at ? Date.parse(job.finished_at) : now;
  if (Number.isNaN(start) || Number.isNaN(end)) return '';
  const seconds = Math.max(0, Math.round((end - start) / 1000));
  return seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}

export function fixRequest(job) {
  const output = (job.output ?? '').trim().split('\n').slice(-60).join('\n');
  return `The repository's checks failed after the last changes:\n\n${output}\n\nPlease fix what failed and run the checks again.`;
}
