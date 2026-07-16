import { describe, it, expect } from 'vitest';
import { renderPreviewToHtml } from '../../src/lib/format.js';

describe('renderPreviewToHtml link preview', () => {
  it('renders a __type: link payload as an anchor with the given href and label', () => {
    const payload = JSON.stringify({ __type: 'link', href: '#/artifacts/abc-123', label: 'Open Deploy report' });
    const html = renderPreviewToHtml(payload);
    expect(html).toContain('<a');
    expect(html).toContain('href="#/artifacts/abc-123"');
    expect(html).toContain('Open Deploy report');
  });

  it('escapes label and href to avoid injecting markup', () => {
    const payload = JSON.stringify({ __type: 'link', href: '"><script>x</script>', label: '<b>x</b>' });
    const html = renderPreviewToHtml(payload);
    expect(html).not.toContain('<script>');
    expect(html).not.toContain('<b>x</b>');
  });
});
