const AVATAR_COLORS = [
  '#c7162b', '#0e8420', '#0066cc', '#772953',
  '#e95420', '#57407c', '#006e90', '#3c763d',
];

export function avatarColor(name) {
  let h = 0;
  for (const c of String(name ?? '')) h = (h * 31 + c.charCodeAt(0)) & 0x7fffffff;
  return AVATAR_COLORS[h % AVATAR_COLORS.length];
}

export function initials(name) {
  const parts = String(name ?? '').trim().split(/\s+/);
  return parts.length >= 2
    ? (parts[0][0] + parts[parts.length - 1][0]).toUpperCase()
    : (parts[0]?.[0] ?? '?').toUpperCase();
}

export function escapeHtml(str) {
  return String(str ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

export function copyText(text) {
  if (navigator.clipboard?.writeText) {
    return navigator.clipboard.writeText(text);
  }
  return new Promise((resolve, reject) => {
    const ta = document.createElement('textarea');
    ta.value = text;
    ta.style.cssText = 'position:fixed;opacity:0;pointer-events:none';
    document.body.appendChild(ta);
    ta.focus();
    ta.select();
    try {
      document.execCommand('copy') ? resolve() : reject(new Error('execCommand failed'));
    } catch (e) {
      reject(e);
    } finally {
      document.body.removeChild(ta);
    }
  });
}

export function addCopyButtons(containerEl, selector = 'pre') {
  containerEl.querySelectorAll(selector).forEach(pre => {
    if (pre.closest('.code-block')) return;
    const wrapper = document.createElement('div');
    wrapper.className = 'code-block';
    pre.parentNode.insertBefore(wrapper, pre);
    wrapper.appendChild(pre);

    const btn = document.createElement('button');
    btn.className = 'copy-btn';
    btn.textContent = 'Copy';
    btn.addEventListener('click', () => {
      const text = (pre.querySelector('code') ?? pre).innerText;
      copyText(text).then(() => {
        btn.textContent = 'Copied!';
      }).catch(() => {
        btn.textContent = 'Failed';
      }).finally(() => {
        setTimeout(() => { btn.textContent = 'Copy'; }, 2000);
      });
    });
    wrapper.appendChild(btn);
  });
}
