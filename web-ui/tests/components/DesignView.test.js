import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, flushPromises } from '@vue/test-utils';

vi.mock('../../src/lib/api.js', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    getProjectDeploymentSingle: vi.fn(),
  };
});

import DesignView from '../../src/views/DesignView.vue';
import * as api from '../../src/lib/api.js';

const DEPLOYMENT_NO_DESIGN = {
  id: 'd1', name: 'MyProject', infra_state: 'none', design_doc: null,
};

const DEPLOYMENT_WITH_DESIGN = {
  id: 'd1', name: 'MyProject', infra_state: 'none',
  design_doc: { id: 'a1', title: 'Design doc' },
};

function mountView() {
  return mount(DesignView, { props: { projectId: 'proj-1' } });
}

describe('DesignView', () => {
  beforeEach(() => { vi.restoreAllMocks(); });

  it('fetches the project deployment on mount', async () => {
    api.getProjectDeploymentSingle.mockResolvedValue(structuredClone(DEPLOYMENT_NO_DESIGN));
    const w = mountView();
    await flushPromises();
    expect(api.getProjectDeploymentSingle).toHaveBeenCalledWith('proj-1');
  });

  it('shows the deployment name in the header', async () => {
    api.getProjectDeploymentSingle.mockResolvedValue(structuredClone(DEPLOYMENT_WITH_DESIGN));
    const w = mountView();
    await flushPromises();
    expect(w.text()).toContain('MyProject');
  });

  it('renders the DesignPanel with the fetched deployment', async () => {
    api.getProjectDeploymentSingle.mockResolvedValue(structuredClone(DEPLOYMENT_WITH_DESIGN));
    const w = mountView();
    await flushPromises();
    expect(w.findComponent({ name: 'DesignPanel' }).exists()).toBe(true);
  });

  it('shows a loading indicator while fetching', async () => {
    let resolveFn;
    api.getProjectDeploymentSingle.mockReturnValue(new Promise(r => { resolveFn = r; }));
    const w = mountView();
    await flushPromises();
    expect(w.find('[data-testid="design-loading"]').exists()).toBe(true);
    resolveFn(structuredClone(DEPLOYMENT_NO_DESIGN));
    await flushPromises();
  });
});
