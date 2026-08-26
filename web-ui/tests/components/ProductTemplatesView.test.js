import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { createRouter, createMemoryHistory } from 'vue-router';

import ProductTemplatesView from '../../src/views/ProductTemplatesView.vue';

function makeRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/product-templates', component: ProductTemplatesView }],
  });
}

async function mountView() {
  const router = makeRouter();
  router.push('/product-templates');
  await router.isReady();
  const w = mount(ProductTemplatesView, { global: { plugins: [createPinia(), router] } });
  await flushPromises();
  return w;
}

describe('ProductTemplatesView', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('renders the page header', async () => {
    const w = await mountView();
    expect(w.text()).toMatch(/product templates/i);
  });

  it('renders an empty placeholder body', async () => {
    const w = await mountView();
    expect(w.find('[data-testid="product-templates-empty"]').exists()).toBe(true);
  });
});