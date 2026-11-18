async function previewFetch(url, options = {}) {
  const response = await fetch(url, {
    credentials: 'same-origin',
    ...options,
    headers: { 'Content-Type': 'application/json', ...(options.headers ?? {}) },
  });
  if (response.status === 204) return null;
  const data = await response.json().catch(() => ({}));
  if (!response.ok) {
    const error = new Error(data.error || `Request failed (${response.status})`);
    error.status = response.status;
    throw error;
  }
  return data;
}

const post = (body) => ({ method: 'POST', body: JSON.stringify(body ?? {}) });

export const AUDIENCES = Object.freeze([
  { id: 'owner', label: 'Only me' },
  { id: 'repository', label: 'People with access to the repository' },
  { id: 'link', label: 'Anyone with the link' },
]);

export const getPreview = (id) => previewFetch(`/previews/${id}`);
export const enterPreview = (id, to) => previewFetch(`/previews/${id}/enter`, post(to ? { to } : {}));
export const resetPreview = (id) => previewFetch(`/previews/${id}/reset`, post());
export const sendPreviewRequest = (id, request) => previewFetch(`/previews/${id}/request`, post(request));
export const execPreview = (id, command) => previewFetch(`/previews/${id}/exec`, post({ command }));
export const sharePreview = (previewId, audience) =>
  previewFetch(`/code/previews/${previewId}/share`, { method: 'PUT', body: JSON.stringify({ audience }) });

export function canFrameHere(info, origin = window.location.origin) {
  return !info?.harvest_origin || info.harvest_origin === origin;
}

export function shareUrl(id, origin = window.location.origin) {
  return `${origin}/#/preview/${id}`;
}

export function parseHeaderLines(text) {
  return Object.fromEntries(
    (text ?? '')
      .split('\n')
      .map((line) => line.trim())
      .filter((line) => line.includes(':'))
      .map((line) => {
        const at = line.indexOf(':');
        return [line.slice(0, at).trim(), line.slice(at + 1).trim()];
      })
      .filter(([name]) => name),
  );
}
