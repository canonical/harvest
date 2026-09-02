import { describe, it, expect } from 'vitest';
import { describeToolCall } from '../../src/lib/tool-render.js';

describe('describeToolCall', () => {
  it('describes search_symbols with query and scope', () => {
    const label = describeToolCall('search_symbols', { query: 'retry', repo: 'my-repo' });
    expect(label).toContain('retry');
    expect(label).toContain('my-repo');
  });

  it('describes get_symbol_source with name', () => {
    const label = describeToolCall('get_symbol_source', { name: 'retry_loop' });
    expect(label).toContain('retry_loop');
  });

  it('describes find_callers with function name', () => {
    const label = describeToolCall('find_callers', { function_name: 'retry_loop' });
    expect(label).toContain('retry_loop');
    expect(label.toLowerCase()).toContain('caller');
  });

  it('describes find_callees with function name', () => {
    const label = describeToolCall('find_callees', { function_name: 'retry_loop' });
    expect(label).toContain('retry_loop');
  });

  it('describes run_command with command and hostname', () => {
    const label = describeToolCall('run_command', { command: 'systemctl status nginx', agent_id: 'a1' }, { hostname: 'build-box' });
    expect(label).toContain('systemctl status nginx');
    expect(label).toContain('build-box');
  });

  it('describes run_cypher without leaking the raw query', () => {
    const label = describeToolCall('run_cypher', { query: 'MATCH (n:Function) RETURN n LIMIT 5' });
    expect(label).not.toContain('MATCH');
    expect(label.toLowerCase()).toContain('graph');
  });

  it('describes create_lxd_agent', () => {
    const label = describeToolCall('create_lxd_agent', { name: 'build-runner' });
    expect(label).toContain('build-runner');
  });

  it('describes delete_agent', () => {
    const label = describeToolCall('delete_agent', { agent_id: 'a1' });
    expect(label.toLowerCase()).toContain('delet');
  });

  it('describes list_port_forwards', () => {
    const label = describeToolCall('list_port_forwards', {});
    expect(label.toLowerCase()).toContain('port');
  });

  it('describes create_port_forward', () => {
    const label = describeToolCall('create_port_forward', {});
    expect(label.toLowerCase()).toContain('creat');
  });

  it('describes update_port_forward', () => {
    const label = describeToolCall('update_port_forward', {});
    expect(label.toLowerCase()).toContain('updat');
  });

  it('describes delete_port_forward', () => {
    const label = describeToolCall('delete_port_forward', {});
    expect(label.toLowerCase()).toContain('delet');
  });

  it('describes generate_artifact', () => {
    const label = describeToolCall('generate_artifact', { name: 'config.yml' });
    expect(label).toContain('config.yml');
  });

  it('describes read_provision_bundle', () => {
    const label = describeToolCall('read_provision_bundle', {});
    expect(label.toLowerCase()).toContain('bundle');
  });

  it('describes run_terraform_plan', () => {
    const label = describeToolCall('run_terraform_plan', {});
    expect(label.toLowerCase()).toContain('plan');
  });

  it('falls back to humanized name for unknown tools', () => {
    const label = describeToolCall('some_unknown_tool', {});
    expect(label).toBe('some unknown tool');
  });
});
