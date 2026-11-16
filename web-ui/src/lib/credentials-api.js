async function request(url, options = {}) {
  const res = await fetch(url, { headers: { 'Content-Type': 'application/json' }, ...options });
  if (!res.ok) {
    const data = await res.json().catch(() => ({}));
    throw new Error(data.error || `Request failed (${res.status})`);
  }
  return res.status === 204 ? null : res.json();
}

export const fetchCredentials = () => request('/me/credentials');
export const generateSshKey   = () => request('/me/ssh-key', { method: 'PUT', body: JSON.stringify({}) });
export const importSshKey     = (privateKey) => request('/me/ssh-key', { method: 'PUT', body: JSON.stringify({ private_key: privateKey }) });
export const deleteSshKey     = () => request('/me/ssh-key', { method: 'DELETE' });
export const saveAccount      = (account) => request('/me/accounts', { method: 'PUT', body: JSON.stringify(account) });
export const deleteAccount    = (host) => request(`/me/accounts/${encodeURIComponent(host)}`, { method: 'DELETE' });
