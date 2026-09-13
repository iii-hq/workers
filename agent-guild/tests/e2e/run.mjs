import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { copyFile, mkdir, readFile, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { createServer as createNetServer } from 'node:net';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { registerWorker } from 'iii-sdk';
import { startWorker } from '../../dist/lib/worker.js';
import { corrected, observed, passportInput, target, verifier } from '../fixtures.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const repository = resolve(root, '..');
const results = resolve(process.env.AGENT_GUILD_E2E_RESULTS || `${root}/tests/e2e/results`);
const binary = process.env.III_BIN;
assert(
  binary,
  'III_BIN must point to an already installed iii 0.23.0 engine; this test does not download software.',
);
await mkdir(results, { recursive: true });
const free = createNetServer();
await new Promise((r) => free.listen(0, '127.0.0.1', r));
const port = free.address().port;
await new Promise((r) => free.close(r));
const address = `ws://127.0.0.1:${port}`;
const env = {
  ...process.env,
  III_URL: address,
  III_TELEMETRY_ENABLED: 'false',
  OTEL_ENABLED: 'false',
  III_TRIGGER_PORT: String(port),
};
const config = resolve(results, 'config.yaml');
await writeFile(
  config,
  `workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: ${port}\n`,
);
const log = { engine: '', bundle: '' };
const checks = [];
const requests = [];
function child(args, name) {
  const p = spawn(args[0], args.slice(1), { cwd: results, env, stdio: ['ignore', 'pipe', 'pipe'] });
  p.stdout.on('data', (b) => {
    log[name] += b;
  });
  p.stderr.on('data', (b) => {
    log[name] += b;
  });
  return p;
}
async function stop(p) {
  if (!p || p.exitCode !== null) return;
  p.kill('SIGTERM');
  await Promise.race([
    new Promise((r) => p.once('exit', r)),
    new Promise((r) => setTimeout(r, 1000)),
  ]);
  if (p.exitCode === null) p.kill('SIGKILL');
}
function cli(functionId, payload) {
  const args = ['trigger', functionId, '--json', JSON.stringify(payload), '--port', String(port)];
  const result = spawnSync(binary, args, { cwd: results, env, encoding: 'utf8', timeout: 5000 });
  if (result.status !== 0) throw Error(`CLI ${functionId}: ${result.stderr}`);
  return JSON.parse(result.stdout);
}
let engine, bundle, worker, caller, http;
try {
  const version = spawnSync(binary, ['--version'], { env, encoding: 'utf8' });
  assert.equal(version.status, 0);
  assert.equal(version.stdout.trim(), '0.23.0');
  engine = child([binary, '--config', config, '--no-update-check'], 'engine');
  const readyUntil = Date.now() + 15000;
  while (true) {
    try {
      cli('engine::workers::list', {});
      break;
    } catch (error) {
      if (Date.now() > readyUntil || engine.exitCode !== null) throw error;
      await new Promise((r) => setTimeout(r, 150));
    }
  }
  await writeFile(
    resolve(results, 'workers-baseline.json'),
    JSON.stringify(cli('engine::workers::list', {}), null, 2),
  );
  await writeFile(
    resolve(results, 'triggers-baseline.json'),
    JSON.stringify(cli('engine::triggers::list', { include_internal: false }), null, 2),
  );
  const staged = resolve(results, 'prepared/dist/bundle');
  await mkdir(staged, { recursive: true });
  await copyFile(resolve(root, 'dist/bundle/index.mjs'), resolve(staged, 'index.mjs'));
  await copyFile(resolve(root, 'iii.worker.yaml'), resolve(results, 'prepared/iii.worker.yaml'));
  bundle = child([process.execPath, resolve(staged, 'index.mjs')], 'bundle');
  const collectArgs = [
    resolve(repository, '.github/scripts/collect_worker_interface.py'),
    '--worker',
    'agent-guild',
    '--out',
    resolve(results, 'worker-interface.json'),
    '--wait-seconds',
    '15',
    '--workers-baseline',
    resolve(results, 'workers-baseline.json'),
    '--trigger-types-baseline',
    resolve(results, 'triggers-baseline.json'),
    '--assert-non-empty',
    '--assert-typed-schemas',
  ];
  const capture = spawnSync(process.env.PYTHON || 'python3', collectArgs, {
    cwd: repository,
    env,
    encoding: 'utf8',
    timeout: 25000,
  });
  await writeFile(resolve(results, 'interface-capture.stdout'), capture.stdout || '');
  await writeFile(resolve(results, 'interface-capture.stderr'), capture.stderr || '');
  assert.equal(capture.status, 0, capture.stderr);
  const iface = JSON.parse(await readFile(resolve(results, 'worker-interface.json'), 'utf8'));
  assert.deepEqual(iface.functions.map((f) => f.name).sort(), [
    'agent-guild::preflight',
    'agent-guild::verify_passport',
  ]);
  checks.push({
    name: 'actual bundle registers typed interface through native collector',
    passed: true,
  });
  const details = ['agent-guild::preflight', 'agent-guild::verify_passport'].map((function_id) =>
    cli('engine::functions::info', { function_id }),
  );
  for (const detail of details) assert.equal(detail.metadata.mcp.expose, true);
  await writeFile(resolve(results, 'function-details.json'), JSON.stringify(details, null, 2));
  checks.push({
    name: 'both real registrations carry explicit MCP exposure metadata',
    passed: true,
  });
  // Validation failures in the bundled CLI require no HTTP substitution or external request.
  assert.equal(cli('agent-guild::preflight', { url: 'http://127.0.0.1/' }).status, 'rejected');
  checks.push({ name: 'actual bundle rejects private target through CLI dispatch', passed: true });
  await stop(bundle);
  bundle = undefined;
  let mode = 'normal';
  http = createServer(async (req, res) => {
    let body = '';
    for await (const chunk of req) {
      body += chunk;
      assert(body.length <= 32768);
    }
    requests.push({ method: req.method, path: req.url, body: body || null });
    if (mode === 'redirect') {
      res.writeHead(302, { Location: '/not-followed' });
      res.end();
      return;
    }
    if (mode === 'timeout') {
      setTimeout(() => res.end('{}'), 300).unref();
      return;
    }
    let value = req.method === 'POST' ? verifier(mode !== 'negative') : observed();
    if (mode === 'all-unknown') {
      for (const c of value.checks) c.status = 'unknown';
      value = corrected(value);
    }
    if (mode === 'bad-verdict') value.verdict = 'no_failed_checks';
    res.writeHead(mode === 'paid' ? 402 : 200, { 'Content-Type': 'application/json' });
    res.end(mode === 'oversize' ? ' '.repeat(65537) : JSON.stringify(value));
  });
  await new Promise((r) => http.listen(0, '127.0.0.1', r));
  const origin = `http://127.0.0.1:${http.address().port}`;
  const fixtureFetch = async (url, init) => {
    const selected = new URL(url);
    assert.equal(selected.origin, 'https://agent-guild-5d5r.onrender.com');
    assert(['/preflight', '/credentials/verify'].includes(selected.pathname));
    const response = await fetch(`${origin}${selected.pathname}${selected.search}`, init);
    // Only the HTTP boundary is substituted. URL omission denotes an injected response,
    // while status, headers and the real stream remain unaltered.
    return new Response(response.body, { status: response.status, headers: response.headers });
  };
  worker = startWorker(address, { fetch: fixtureFetch, timeoutMs: 100 });
  caller = registerWorker(address, {
    workerName: 'agent-guild-e2e-caller',
    enableMetricsReporting: false,
    otel: { enabled: false },
    invocationTimeoutMs: 2000,
  });
  worker.client.registerFunction(
    'agent-guild-e2e::inspect',
    async (input) => ({
      input,
      keys: Object.keys(input ?? {}),
      kind: typeof input,
      plain: Object.getPrototypeOf(input) === Object.prototype,
    }),
    { metadata: { internal: true } },
  );
  await new Promise((r) => setTimeout(r, 200));
  const diagnostic = await caller.trigger({
    function_id: 'agent-guild-e2e::inspect',
    payload: { url: target },
  });
  await writeFile(
    resolve(results, 'native-input-diagnostic.json'),
    JSON.stringify(diagnostic, null, 2),
  );
  const spoofed = await caller.trigger({
    function_id: 'agent-guild-e2e::inspect',
    payload: { url: target, _caller_worker_id: 'spoofed' },
  });
  assert.equal(spoofed.input._caller_worker_id, diagnostic.input._caller_worker_id);
  checks.push({ name: 'engine overwrites client-supplied caller metadata', passed: true });
  const invoke = (id, input) =>
    caller.trigger({ function_id: `agent-guild::${id}`, payload: input, timeoutMs: 2000 });
  async function check(name, fn) {
    await fn();
    checks.push({ name, passed: true });
  }
  await check('real SDK registration and cross-client endpoint dispatch', async () => {
    const out = await invoke('preflight', { url: target });
    assert.equal(out.status, 'observed', JSON.stringify(out));
    assert.equal(out.checks.length, 6);
    assert.equal(out.target, target);
  });
  await check('unknown remains unknown through engine protocol', async () => {
    mode = 'all-unknown';
    const out = await invoke('preflight', { url: target });
    assert.equal(out.verdict, 'no_failed_checks');
    assert.equal(out.unknowns.length, 6);
  });
  await check('contradictory response rejected through protocol', async () => {
    mode = 'bad-verdict';
    assert.equal((await invoke('preflight', { url: target })).status, 'rejected');
  });
  await check('passport complete claims and expected DIDs through protocol', async () => {
    mode = 'normal';
    const input = passportInput();
    const out = await invoke('verify_passport', input);
    assert.equal(out.verified, true);
    assert.deepEqual(JSON.parse(requests.at(-1).body), input.credential);
  });
  await check('negative verification remains completed and unverified', async () => {
    mode = 'negative';
    const out = await invoke('verify_passport', passportInput());
    assert.equal(out.status, 'completed');
    assert.equal(out.verified, false);
  });
  await check('raw invalid input rejected without HTTP', async () => {
    const count = requests.length;
    assert.equal((await invoke('preflight', { url: target, extra: true })).code, 'invalid_input');
    const input = passportInput();
    input.expectedSubjectDid = input.expectedIssuerDid;
    assert.equal((await invoke('verify_passport', input)).code, 'subject_mismatch');
    assert.equal(
      (await invoke('preflight', { url: target, _unexpected: 'bad' })).code,
      'invalid_input',
    );
    assert.equal(requests.length, count);
  });
  for (const [next, status] of [
    ['redirect', 'unavailable'],
    ['paid', 'unavailable'],
    ['oversize', 'rejected'],
    ['timeout', 'unavailable'],
  ])
    await check(`protocol ${next} fails without retry`, async () => {
      mode = next;
      const count = requests.length;
      assert.equal((await invoke('preflight', { url: target })).status, status);
      assert.equal(requests.length, count + 1);
    });
  assert(!requests.some((r) => r.path === '/not-followed'));
  assert(!requests.some((r) => r.path.startsWith('/check')));
  await writeFile(
    resolve(results, 'RESULT.json'),
    JSON.stringify(
      {
        engineVersion: version.stdout.trim(),
        sdkVersion: '0.23.0',
        checks,
        requests,
        substitution:
          'Only fixed Guild HTTPS transport mapped to a local HTTP fixture. Engine/SDK/worker classes and cross-client invocation are real. No live Guild, model, payment or independent cryptographic verification.',
      },
      null,
      2,
    ),
  );
  console.log(`PASS: ${checks.length} real engine / SDK / local HTTP checks`);
} finally {
  await writeFile(resolve(results, 'requests.json'), JSON.stringify(requests, null, 2));
  await caller?.shutdown();
  await worker?.shutdown();
  if (http) {
    http.closeAllConnections();
    await new Promise((r) => http.close(r));
  }
  await stop(bundle);
  await stop(engine);
  for (const [name, text] of Object.entries(log))
    await writeFile(resolve(results, `${name}.log`), text);
}
