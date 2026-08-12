import {
  provisionLxdAgent, deleteAgent, createPortForward, updatePortForward, deletePortForward, runTerraformArtifact,
} from './api.js';
import { initialProvisionSteps, applyProvisionEvent, isProvisionDone, isProvisionError } from './lxd-provision.js';

function findConfirmAction(threadStore, id) {
  return threadStore.messages.at(-1)?.chain?.find(c => c.type === 'confirm_action' && c.id === id);
}

export async function runConfirmableAction(projectId, action, threadStore) {
  if (action.name === 'create_lxd_agent') {
    threadStore.updateConfirmActionItem(action.id, { status: 'running', steps: initialProvisionSteps() });
    try {
      await provisionLxdAgent(projectId, {
        name: action.input.name,
        description: action.input.description ?? '',
        flavor: action.input.flavor,
      }, (event) => {
        const current = findConfirmAction(threadStore, action.id);
        if (!current) return;
        threadStore.updateConfirmActionItem(action.id, { steps: applyProvisionEvent(current.steps, event) });
        if (isProvisionDone(event)) {
          threadStore.updateConfirmActionItem(action.id, { status: 'done', resultText: `Agent '${action.input.name}' created.` });
        } else if (isProvisionError(event)) {
          threadStore.updateConfirmActionItem(action.id, { status: 'error', resultText: event.message });
        }
      });
    } catch (e) {
      threadStore.updateConfirmActionItem(action.id, { status: 'error', resultText: e.message || 'Failed to create agent' });
    }
  } else if (action.name === 'delete_agent') {
    threadStore.updateConfirmActionItem(action.id, { status: 'running' });
    try {
      await deleteAgent(projectId, action.input.agent_id);
      threadStore.updateConfirmActionItem(action.id, { status: 'done', resultText: 'Agent deleted.' });
    } catch (e) {
      threadStore.updateConfirmActionItem(action.id, { status: 'error', resultText: e.message || 'Failed to delete agent' });
    }
  } else if (action.name === 'create_port_forward') {
    threadStore.updateConfirmActionItem(action.id, { status: 'running' });
    try {
      await createPortForward(projectId, action.input.agent_id, {
        port: action.input.port,
        routeName: action.input.route_name,
      });
      threadStore.updateConfirmActionItem(action.id, { status: 'done', resultText: `Port forward '${action.input.route_name}' created.` });
    } catch (e) {
      threadStore.updateConfirmActionItem(action.id, { status: 'error', resultText: e.message || 'Failed to create port forward' });
    }
  } else if (action.name === 'update_port_forward') {
    threadStore.updateConfirmActionItem(action.id, { status: 'running' });
    try {
      await updatePortForward(projectId, action.input.agent_id, action.input.forward_id, {
        port: action.input.port,
        routeName: action.input.route_name,
      });
      threadStore.updateConfirmActionItem(action.id, { status: 'done', resultText: 'Port forward updated.' });
    } catch (e) {
      threadStore.updateConfirmActionItem(action.id, { status: 'error', resultText: e.message || 'Failed to update port forward' });
    }
  } else if (action.name === 'delete_port_forward') {
    threadStore.updateConfirmActionItem(action.id, { status: 'running' });
    try {
      await deletePortForward(projectId, action.input.agent_id, action.input.forward_id);
      threadStore.updateConfirmActionItem(action.id, { status: 'done', resultText: 'Port forward deleted.' });
    } catch (e) {
      threadStore.updateConfirmActionItem(action.id, { status: 'error', resultText: e.message || 'Failed to delete port forward' });
    }
  } else if (action.name === 'run_terraform_apply' || action.name === 'run_terraform_destroy') {
    threadStore.updateConfirmActionItem(action.id, { status: 'running' });
    const tfAction = action.name === 'run_terraform_apply' ? 'apply' : 'destroy';
    try {
      const result = await runTerraformArtifact(
        projectId, action.input.agent_id, action.input.artifact_id, tfAction, action.input.timeout_secs,
      );
      const ok = result.exit_code === 0;
      threadStore.updateConfirmActionItem(action.id, {
        status: ok ? 'done' : 'error',
        resultText: ok
          ? `terraform ${tfAction} succeeded (exit 0).`
          : `terraform ${tfAction} failed (exit ${result.exit_code}): ${(result.stderr ?? '').slice(0, 300)}`,
      });
    } catch (e) {
      threadStore.updateConfirmActionItem(action.id, { status: 'error', resultText: e.message || `Failed to run terraform ${tfAction}` });
    }
  }
}
