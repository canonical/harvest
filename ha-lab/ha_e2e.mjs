import { spawn, execFileSync } from 'node:child_process';
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const WebSocket = require(process.env.WS_MODULE ?? path.resolve(import.meta.dirname, '../web-ui/node_modules/ws'));

const LAB = process.env.LAB_DIR ?? path.join(os.homedir(), '.cache/harvest-ha-lab');
const SERVER_BIN = process.env.SERVER_BIN ?? path.join(os.homedir(), '.cache/harvest-ha-target/debug/knowledge-server');
const UI_DIST = path.resolve(import.meta.dirname, '../web-ui/dist');
const env = Object.fromEntries(fs.readFileSync(path.join(LAB, 'env'), 'utf8').trim().split('\n').map(l => {
  const i = l.indexOf('=');
  return [l.slice(0, i), l.slice(i + 1)];
}));
const LB = `http://${env.LB_IP}`;
const LB_WS = `ws://${env.LB_IP}`;
const API_PATHS = ['/admin', '/agent', '/agents', '/artifacts', '/auth', '/chat-layouts', '/conversations', '/docs', '/graph',
  '/groups', '/health', '/llm', '/machines', '/metrics', '/projects', '/query', '/repositories', '/skills', '/templates',
  '/tool-description', '/version'];

const results = [];
const sleep = ms => new Promise(r => setTimeout(r, ms));
const log = (...a) => console.log(new Date().toISOString().slice(11, 19), ...a);

function record(name, passed, details) {
  results.push({ name, passed, ...details });
  log(passed ? 'PASS' : 'FAIL', name, JSON.stringify(details));
}

function psql(sql) {
  const url = env.DB_URL.replace('connect_timeout=3', 'connect_timeout=5');
  return execFileSync('psql', [url, '-Atc', sql], { encoding: 'utf8' }).trim();
}

function lxc(...args) {
  return execFileSync('lxc', args, { encoding: 'utf8' });
}

const mock = { chunks: 5, delayMs: 5 };
function startMockLlm() {
  const server = http.createServer((req, res) => {
    if (req.url.endsWith('/models')) {
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ data: [{ id: 'mock' }] }));
      return;
    }
    let body = '';
    req.on('data', c => { body += c; });
    req.on('end', async () => {
      const parsed = JSON.parse(body || '{}');
      if (!parsed.stream) {
        res.writeHead(200, { 'content-type': 'application/json' });
        res.end(JSON.stringify({ choices: [{ index: 0, message: { role: 'assistant', content: 'lab title' }, finish_reason: 'stop' }], usage: { prompt_tokens: 1, completion_tokens: 1 } }));
        return;
      }
      res.writeHead(200, { 'content-type': 'text/event-stream' });
      const chunks = mock.chunks;
      const delay = mock.delayMs;
      for (let i = 0; i < chunks; i++) {
        if (res.destroyed) return;
        res.write(`data: ${JSON.stringify({ choices: [{ index: 0, delta: { content: `chunk${i} ` } }] })}\n\n`);
        await sleep(delay);
      }
      res.write(`data: ${JSON.stringify({ choices: [{ index: 0, delta: {}, finish_reason: 'stop' }], usage: { prompt_tokens: 1, completion_tokens: 1 } })}\n\n`);
      res.end('data: [DONE]\n\n');
    });
  });
  return new Promise(resolve => server.listen(0, env.HOST_IP, () => resolve(`http://${env.HOST_IP}:${server.address().port}`)));
}

const nodes = [];
function nodeConfig(i, llmUrl) {
  return `
[server]
host = "${env.HOST_IP}"
port = ${18080 + i}

[database]
url = "${env.DB_URL}"
ca_file = "${env.CA_FILE}"
pool_size = 8
migrate_on_start = ${i === 0}

[cluster]
node_name = "lab-node-${i}"
internal_listen = "${env.HOST_IP}:${19080 + i}"
shared_secret = "lab-cluster-secret"
heartbeat_interval_ms = 1000
node_timeout_ms = 5000
bus_batch_window_ms = 20
drain_grace_secs = 16
drain_timeout_secs = 30

[auth]
jwt_secret = "lab-jwt-secret-that-is-long-enough-for-hs256"
allow_local_login = true

[agent]
max_iterations = 3

[[llm]]
provider = "openai-compatible"
base_url = "${llmUrl}"
api_key = "mock"
model = "mock"
id = "mock"
max_retries = 0
`;
}

function startNode(i, llmUrl) {
  const dir = path.join(LAB, 'nodes');
  fs.mkdirSync(dir, { recursive: true });
  const configPath = path.join(dir, `node-${i}.toml`);
  fs.writeFileSync(configPath, nodeConfig(i, llmUrl));
  const out = fs.openSync(path.join(dir, `node-${i}.log`), 'a');
  const child = spawn(SERVER_BIN, ['--config', configPath], { stdio: ['ignore', out, out], env: { ...process.env, RUST_LOG: 'info' } });
  nodes[i] = { child, base: `http://${env.HOST_IP}:${18080 + i}`, exited: false };
  child.on('exit', () => { nodes[i].exited = true; });
  return nodes[i];
}

async function status(url, opts = {}) {
  try {
    const res = await fetch(url, { ...opts, signal: AbortSignal.timeout(opts.timeout ?? 10000) });
    await res.arrayBuffer();
    return res.status;
  } catch {
    return 0;
  }
}

async function waitFor(check, timeoutMs, what) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await check()) return true;
    await sleep(250);
  }
  throw new Error(`timed out waiting for ${what}`);
}

async function waitNodeReady(i) {
  await waitFor(async () => (await status(`${nodes[i].base}/health/ready`)) === 200, 120000, `node ${i} ready`);
}

function configureHaproxy() {
  const servers = [0, 1, 2].map(i => `    server node${i} ${env.HOST_IP}:${18080 + i} check inter 5s rise 2 fall 3`).join('\n');
  const cfg = fs.readFileSync(path.join(import.meta.dirname, 'haproxy.cfg.template'), 'utf8')
    .replace('@API_PATHS@', API_PATHS.join(' '))
    .replace('@SERVERS@', servers);
  const tmp = path.join(LAB, 'haproxy.cfg');
  fs.writeFileSync(tmp, cfg);
  lxc('file', 'push', tmp, 'harvest-ha-lab/etc/haproxy/haproxy.cfg');
  lxc('exec', 'harvest-ha-lab', '--', 'bash', '-c', 'haproxy -c -f /etc/haproxy/haproxy.cfg >/dev/null && systemctl restart haproxy');
  lxc('exec', 'harvest-ha-lab', '--', 'bash', '-c', 'rm -rf /srv/ui /srv/dist && mkdir -p /srv');
  lxc('file', 'push', '-r', UI_DIST, 'harvest-ha-lab/srv/');
  lxc('exec', 'harvest-ha-lab', '--', 'bash', '-c',
    'mv /srv/dist /srv/ui && echo ok > /srv/ui/ui-health; systemctl stop harvest-ui-static 2>/dev/null; systemctl reset-failed harvest-ui-static 2>/dev/null; ' +
    'systemd-run --unit harvest-ui-static python3 -m http.server 8090 --bind 127.0.0.1 --directory /srv/ui >/dev/null');
}

function resetDatabase() {
  const primary = psql('SELECT inet_server_addr()') === env.PG_A_IP ? 'harvest-pg-a' : 'harvest-pg-b';
  lxc('exec', primary, '--', 'sudo', '-u', 'postgres', 'psql', '-qc',
    "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = 'harvest' AND pid <> pg_backend_pid()");
  lxc('exec', primary, '--', 'sudo', '-u', 'postgres', 'psql', '-qc', 'DROP DATABASE IF EXISTS harvest');
  lxc('exec', primary, '--', 'sudo', '-u', 'postgres', 'psql', '-qc', 'CREATE DATABASE harvest OWNER harvest');
  lxc('exec', primary, '--', 'sudo', '-u', 'postgres', 'psql', '-q', '-d', 'harvest', '-c', 'CREATE EXTENSION IF NOT EXISTS pg_trgm; CREATE EXTENSION IF NOT EXISTS vector;');
}

class Session {
  constructor() { this.token = null; }
  headers(extra = {}) { return { authorization: `Bearer ${this.token}`, 'content-type': 'application/json', ...extra }; }
  async json(method, p, body) {
    const res = await fetch(`${LB}${p}`, { method, headers: this.headers(), body: body ? JSON.stringify(body) : undefined, signal: AbortSignal.timeout(30000) });
    const text = await res.text();
    let data = null;
    try { data = JSON.parse(text); } catch { data = text; }
    return { status: res.status, data, headers: res.headers };
  }
}

async function sse(url, headers, onEvent) {
  const controller = new AbortController();
  const res = await fetch(url, { headers, signal: controller.signal });
  if (res.status !== 200) throw new Error(`SSE ${url} -> ${res.status}`);
  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';
  const done = (async () => {
    try {
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        let idx;
        while ((idx = buffer.indexOf('\n\n')) >= 0) {
          const block = buffer.slice(0, idx);
          buffer = buffer.slice(idx + 2);
          for (const line of block.split('\n')) {
            if (line.startsWith('data:')) {
              try { onEvent(JSON.parse(line.slice(5).trim())); } catch {}
            }
          }
        }
      }
    } catch {}
  })();
  return { close: () => controller.abort(), done };
}

class EventLog {
  constructor() { this.events = []; this.waiters = []; }
  push(e) {
    this.events.push(e);
    for (const w of [...this.waiters]) if (w.pred(e)) { w.resolve(e); this.waiters.splice(this.waiters.indexOf(w), 1); }
  }
  wait(pred, timeoutMs, what) {
    const found = this.events.find(pred);
    if (found) return Promise.resolve(found);
    return new Promise((resolve, reject) => {
      const w = { pred, resolve };
      this.waiters.push(w);
      setTimeout(() => reject(new Error(`timed out waiting for ${what}`)), timeoutMs);
    });
  }
}

class Watcher {
  constructor(session, projectId, convId, base = LB) {
    this.session = session; this.projectId = projectId; this.convId = convId; this.base = base;
    this.log = new EventLog(); this.reconnects = 0; this.stopped = false;
  }
  async start() {
    const url = `${this.base}/projects/${this.projectId}/events?conv=${this.convId}`;
    (async () => {
      while (!this.stopped) {
        try {
          const stream = await sse(url, this.session.headers(), e => this.log.push(e));
          this.stream = stream;
          await stream.done;
        } catch {}
        if (this.stopped) break;
        this.reconnects += 1;
        await sleep(500);
      }
    })();
    await this.log.wait(e => e.type === 'presence', 15000, 'presence');
  }
  stop() { this.stopped = true; this.stream?.close(); }
}

class LoadLoop {
  constructor(session, projectId) { this.session = session; this.projectId = projectId; this.ok = 0; this.failed = []; this.running = false; }
  start() {
    this.running = true;
    this.loop = (async () => {
      while (this.running) {
        const started = Date.now();
        const code = await status(`${LB}/projects/${this.projectId}`, { headers: this.session.headers(), timeout: 15000 });
        if (code === 200) this.ok += 1; else this.failed.push({ at: new Date().toISOString(), code });
        await sleep(Math.max(0, 100 - (Date.now() - started)));
      }
    })();
  }
  async stop() { this.running = false; await this.loop; return { ok: this.ok, failed: this.failed.length, failures: this.failed.slice(0, 10) }; }
}

class FakeAgent {
  constructor(installToken) { this.token = installToken; this.log = new EventLog(); this.stopped = false; this.connects = 0; }
  async start() {
    (async () => {
      while (!this.stopped) {
        try {
          const stream = await sse(`${LB}/agent/events?hostname=lab-agent`, { authorization: `Bearer ${this.token}` }, e => this.handle(e));
          this.connects += 1;
          this.stream = stream;
          await stream.done;
        } catch {}
        if (this.stopped) break;
        await sleep(1000);
      }
    })();
    await this.log.wait(e => e.type === 'registered' || e.type === 'hello_ack', 20000, 'agent registration');
  }
  async handle(e) {
    this.log.push(e);
    if (e.type === 'registered') this.token = e.agent_token;
    if (e.type === 'execute') {
      await fetch(`${LB}/agent/results`, {
        method: 'POST',
        headers: { authorization: `Bearer ${this.token}`, 'content-type': 'application/json' },
        body: JSON.stringify({ request_id: e.request_id, stdout: `lab ran: ${e.command}`, stderr: '', exit_code: 0 }),
      });
    }
    if (e.type === 'open_shell') {
      const ws = new WebSocket(`${LB_WS}/agent/console/${e.session_id}`, { headers: { authorization: `Bearer ${this.token}` } });
      this.console = ws;
      ws.on('open', () => ws.send(JSON.stringify({ type: 'ready' })));
      ws.on('message', (data, isBinary) => { if (isBinary) ws.send(Buffer.concat([Buffer.from('echo:'), data]), { binary: true }); });
    }
  }
  stop() { this.stopped = true; this.stream?.close(); this.console?.close(); }
}

async function chat(session, projectId, convId, query) {
  return (await session.json('POST', `/projects/${projectId}/query/stream`, { query, conversation_id: convId })).status;
}

function expectedAnswer(n) { return Array.from({ length: n }, (_, i) => `chunk${i} `).join(''); }
function answerFrom(events, convId) {
  return events.filter(e => e.conv_id === convId && e.type === 'text_delta').map(e => e.text).join('');
}

async function main() {
  const report = { started: new Date().toISOString(), environment: { lb: env.LB_IP, nodes: 3, postgres: [env.PG_A_IP, env.PG_B_IP] } };
  resetDatabase();
  const llmUrl = await startMockLlm();
  startNode(0, llmUrl);
  await waitNodeReady(0);
  startNode(1, llmUrl); startNode(2, llmUrl);
  await waitNodeReady(1); await waitNodeReady(2);
  configureHaproxy();
  await waitFor(async () => (await status(`${LB}/health/ready`)) === 200 && (await status(`${LB}/ui-health`)) === 200, 60000, 'load balancer');
  await sleep(11000);

  const seenNodes = new Set();
  for (let i = 0; i < 60 && seenNodes.size < 3; i++) {
    const res = await fetch(`${LB}/health/ready`);
    seenNodes.add((await res.json()).node_id);
  }
  const index = await (await fetch(`${LB}/`)).text();
  record('haproxy routes API paths to all three servers and everything else to the web UI', seenNodes.size === 3 && index.includes('<div id="app">'),
    { distinct_server_nodes: seenNodes.size, ui_served: index.includes('<div id="app">') });

  const admin = new Session();
  const reg = await fetch(`${LB}/auth/register`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ email: 'lab@example.com', name: 'Lab Admin', password: 'lab-password' }) });
  admin.token = reg.headers.get('set-cookie').split(';')[0].replace('token=', '');
  const group = await admin.json('POST', '/admin/groups', { name: 'lab', description: '' });
  const project = await admin.json('POST', '/projects', { name: 'lab', group_id: group.data.id });
  const pid = project.data.id;

  {
    mock.chunks = 12; mock.delayMs = 30;
    const conv = (await admin.json('POST', `/projects/${pid}/conversations`, {})).data.id;
    const watchers = [new Watcher(admin, pid, conv), new Watcher(admin, pid, conv), new Watcher(admin, pid, conv)];
    for (const w of watchers) await w.start();
    const code = await chat(admin, pid, conv, 'hello through haproxy');
    await Promise.all(watchers.map(w => w.log.wait(e => e.type === 'done' && e.conv_id === conv, 30000, 'done')));
    const answers = watchers.map(w => answerFrom(w.log.events, conv));
    const owner = psql(`SELECT count(*) FROM project_presence WHERE project_id = '${pid}'`);
    const presenceNodes = psql(`SELECT count(DISTINCT node_id) FROM project_presence WHERE project_id = '${pid}'`);
    record('a chat turn streams identically to watchers attached to different nodes', code === 200 && answers.every(a => a === expectedAnswer(12)),
      { send_status: code, watchers: watchers.length, watcher_connections: Number(owner), distinct_watcher_nodes: Number(presenceNodes), answers_match: answers.map(a => a === expectedAnswer(12)) });
    watchers.forEach(w => w.stop());
  }

  const installToken = psql(`SELECT install_token FROM projects WHERE id = '${pid}'`);
  const agent = new FakeAgent(installToken);
  await agent.start();
  const agentId = psql(`SELECT id FROM machines WHERE project_id = '${pid}' LIMIT 1`);

  {
    let okCount = 0; const owners = new Set();
    for (let i = 0; i < 6; i++) {
      const res = await admin.json('POST', `/projects/${pid}/agents/${agentId}/execute`, { command: `uptime ${i}`, timeout_secs: 15 });
      if (res.status === 200 && res.data.stdout === `lab ran: uptime ${i}`) okCount += 1;
    }
    owners.add(psql(`SELECT node_id FROM agent_connections WHERE agent_id = '${agentId}'`));
    record('agent commands requested through any node reach the agent connected to one node', okCount === 6, { commands: 6, succeeded: okCount, agent_owner: [...owners][0] });
  }

  {
    const res = await fetch(`${LB}/auth/login`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ email: 'lab@example.com', password: 'lab-password' }) });
    const cookie = res.headers.get('set-cookie').split(';')[0];
    const browser = new WebSocket(`${LB_WS}/projects/${pid}/agents/${agentId}/console?cols=80&rows=24`, { headers: { cookie } });
    const received = [];
    browser.on('message', (data, isBinary) => received.push(isBinary ? data.toString() : String(data)));
    let closedAt = null;
    browser.on('close', () => { closedAt = Date.now(); });
    await waitFor(async () => received.length > 0, 20000, 'console ready');
    const openedAt = Date.now();
    await sleep(70000);
    const aliveAfterIdle = browser.readyState === WebSocket.OPEN;
    if (aliveAfterIdle) browser.send(Buffer.from('ping-after-idle'), { binary: true });
    await sleep(2000);
    const echoed = received.some(m => m.includes('echo:ping-after-idle'));
    record('a console websocket stays open through haproxy after 70 s idle (client timeout 50 s)', aliveAfterIdle && echoed,
      { idle_seconds: 70, open_after_idle: aliveAfterIdle, echoed, closed_after_ms: closedAt ? closedAt - openedAt : null });
    browser.close();
  }

  {
    mock.chunks = 400; mock.delayMs = 50;
    const conv = (await admin.json('POST', `/projects/${pid}/conversations`, {})).data.id;
    const load = new LoadLoop(admin, pid); load.start();
    await chat(admin, pid, conv, 'this node will crash');
    let ownerNode = '';
    await waitFor(async () => { ownerNode = psql(`SELECT node_id FROM active_turns WHERE conv_id = '${conv}'`); return ownerNode !== ''; }, 20000, 'turn owner');
    const victim = Number(ownerNode.match(/lab-node-(\d)/)[1]);
    const watcher = new Watcher(admin, pid, conv, nodes[(victim + 1) % 3].base);
    await watcher.start();
    await watcher.log.wait(e => e.type === 'text_delta' && e.conv_id === conv, 20000, 'first delta');
    const killedAt = Date.now();
    nodes[victim].child.kill('SIGKILL');
    await watcher.log.wait(e => e.type === 'turn_aborted' && e.conv_id === conv, 30000, 'turn_aborted');
    const detectedMs = Date.now() - killedAt;
    await sleep(20000);
    mock.chunks = 3; mock.delayMs = 5;
    const retry = await chat(admin, pid, conv, 'retry on a survivor');
    await watcher.log.wait(e => e.type === 'done' && e.conv_id === conv, 30000, 'done after retry');
    const loadResult = await load.stop();
    watcher.stop();
    record('crashing the node that runs a turn aborts it cluster-wide, frees the lock and causes no request failures',
      loadResult.failed === 0 && retry === 200 && detectedMs < 15000,
      { killed_node: ownerNode, detection_ms: detectedMs, retry_status: retry, api_requests_ok: loadResult.ok, api_requests_failed: loadResult.failed, failures: loadResult.failures });
    startNode(victim, llmUrl);
    await waitNodeReady(victim);
    await sleep(11000);
  }

  {
    const load = new LoadLoop(admin, pid); load.start();
    let turns = 0; let turnFailures = 0; let turning = true;
    const conv = (await admin.json('POST', `/projects/${pid}/conversations`, {})).data.id;
    mock.chunks = 3; mock.delayMs = 5;
    const turnLoop = (async () => {
      while (turning) {
        const code = await chat(admin, pid, conv, `rolling ${turns}`);
        if (code === 200 || code === 409) turns += 1; else turnFailures += 1;
        await sleep(1500);
      }
    })();
    const drainTimes = [];
    for (const i of [0, 1, 2]) {
      const started = Date.now();
      nodes[i].child.kill('SIGTERM');
      await waitFor(async () => nodes[i].exited, 90000, `node ${i} exit`);
      drainTimes.push(Date.now() - started);
      startNode(i, llmUrl);
      await waitNodeReady(i);
      await sleep(12000);
    }
    turning = false; await turnLoop;
    const loadResult = await load.stop();
    record('a rolling restart of every node (SIGTERM drain) causes no failed requests', loadResult.failed === 0 && turnFailures === 0,
      { api_requests_ok: loadResult.ok, api_requests_failed: loadResult.failed, turns_sent: turns, turn_failures: turnFailures, drain_ms: drainTimes, failures: loadResult.failures });
  }

  {
    const load = new LoadLoop(admin, pid); load.start();
    await sleep(3000);
    const before = psql('SELECT inet_server_addr()');
    const [oldPrimary, newPrimary] = before === env.PG_A_IP ? ['harvest-pg-a', 'harvest-pg-b'] : ['harvest-pg-b', 'harvest-pg-a'];
    const started = Date.now();
    lxc('exec', oldPrimary, '--', 'systemctl', 'stop', 'postgresql@16-main');
    lxc('exec', newPrimary, '--', 'sudo', '-u', 'postgres', '/usr/lib/postgresql/16/bin/pg_ctl', 'promote', '-D', '/var/lib/postgresql/16/main');
    await waitFor(async () => {
      for (let i = 0; i < 3; i++) if ((await status(`${nodes[i].base}/health/ready`)) !== 200) return false;
      return true;
    }, 120000, 'nodes ready after failover');
    const recoveredMs = Date.now() - started;
    await sleep(5000);
    const failuresAtRecovery = load.failed.length;
    await sleep(10000);
    const loadResult = await load.stop();
    const after = psql('SELECT inet_server_addr()');
    const conv = (await admin.json('POST', `/projects/${pid}/conversations`, {})).data.id;
    const watcher = new Watcher(admin, pid, conv); await watcher.start();
    const code = await chat(admin, pid, conv, 'after failover');
    await watcher.log.wait(e => e.type === 'done' && e.conv_id === conv, 30000, 'done after failover');
    watcher.stop();
    const restarted = nodes.filter(n => n.exited).length;
    record('a PostgreSQL primary failover is absorbed without restarting any Harvest node',
      restarted === 0 && code === 200 && loadResult.failed === failuresAtRecovery,
      { old_primary: before, new_primary: after, recovered_ms: recoveredMs, api_requests_ok: loadResult.ok,
        api_requests_failed_during_failover: failuresAtRecovery, api_requests_failed_after_recovery: loadResult.failed - failuresAtRecovery,
        harvest_nodes_restarted: restarted, chat_after_failover: code });
  }

  agent.stop();
  report.finished = new Date().toISOString();
  report.results = results;
  report.passed = results.filter(r => r.passed).length;
  report.failed = results.filter(r => !r.passed).length;
  fs.writeFileSync(path.join(LAB, 'report.json'), JSON.stringify(report, null, 2));
  log(`${report.passed} passed, ${report.failed} failed`);
  for (const n of nodes) n?.child.kill('SIGKILL');
  process.exit(report.failed === 0 ? 0 : 1);
}

main().catch(e => {
  console.error(e);
  for (const n of nodes) n?.child.kill('SIGKILL');
  results.push({ name: 'harness', passed: false, error: String(e) });
  fs.writeFileSync(path.join(LAB, 'report.json'), JSON.stringify({ results }, null, 2));
  process.exit(2);
});
