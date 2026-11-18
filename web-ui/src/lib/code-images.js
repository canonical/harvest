const STATUS_LABELS = {
  ready: 'Ready',
  building: 'Being prepared…',
  failed: 'Preparation failed',
  none: 'Not prepared yet',
};

const FAILURE_REASONS = {
  lxd_unavailable: 'Harvest could not reach its container service',
  infrastructure: 'The build could not finish',
};

export function imageStatusLabel(image) {
  const label = STATUS_LABELS[image?.status ?? 'none'];
  return image?.status === 'ready' && image.attempt?.status === 'building' ? `${label} · rebuilding…` : label;
}

function failedAttemptNotice(image) {
  const reason = FAILURE_REASONS[image.attempt.failure] ?? FAILURE_REASONS.infrastructure;
  const hasReadyImage = image.status === 'ready';
  return {
    tone: hasReadyImage ? 'caution' : 'negative',
    title: hasReadyImage ? 'The latest rebuild failed' : 'Containers could not be prepared',
    message: hasReadyImage
      ? `${reason}. Sessions keep using the previous containers, and Harvest tries again automatically.`
      : `${reason}. Harvest tries again automatically.`,
    log: image.attempt.log ?? '',
  };
}

function setupWarningNotice(image) {
  return { tone: 'caution', title: 'Part of the setup failed', message: image.setup_warning, log: image.log ?? '' };
}

export function imageNotices(image) {
  if (!image) return [];
  return [
    image.attempt?.status === 'failed' ? failedAttemptNotice(image) : null,
    image.setup_warning ? setupWarningNotice(image) : null,
  ].filter(Boolean);
}
