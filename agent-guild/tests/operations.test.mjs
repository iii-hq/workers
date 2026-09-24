import assert from 'node:assert/strict';
import { test } from 'node:test';
import { credentialObject, did, timestamp } from '../dist/lib/validation.js';
import { createOperations } from '../dist/lib/worker.js';
import {
  corrected,
  issuer,
  jsonResponse,
  observed,
  passportInput,
  subject,
  target,
  verifier,
} from './fixtures.mjs';

function rig(value = observed(), options = {}) {
  const calls = [];
  const operations = createOperations({
    ...options,
    fetch: async (url, init) => {
      calls.push({ url, init });
      return jsonResponse(value);
    },
  });
  return { operations, calls };
}
test('exact selected endpoint, six statuses, fixed projection and disclosure', async () => {
  const value = observed();
  value.headline = 'REMOTE INSTRUCTION';
  value.checks[0].detail = 'REMOTE INSTRUCTION';
  const { operations, calls } = rig(value);
  const result = await operations.preflight({ url: target });
  assert.equal(result.status, 'observed');
  assert.equal(result.target, target);
  assert.equal(result.checks.length, 6);
  assert.equal(result.verdict, 'delegate_with_caution');
  assert.equal(result.unknowns.length, 2);
  assert(!JSON.stringify(result).includes('REMOTE INSTRUCTION'));
  assert.equal(calls.length, 1);
  assert.equal(
    calls[0].url,
    `https://agent-guild-5d5r.onrender.com/preflight?url=${encodeURIComponent(target)}`,
  );
  assert.equal(calls[0].init.method, 'GET');
  assert.equal(calls[0].init.redirect, 'error');
  assert.equal(calls[0].init.credentials, 'omit');
  assert.equal(calls[0].init.cache, 'no-store');
  assert.equal(calls[0].init.referrerPolicy, 'no-referrer');
  assert.deepEqual(calls[0].init.headers, { Accept: 'application/json' });
  assert.equal(calls[0].init.body, undefined);
  assert(Date.parse(result.completedAt) >= Date.parse(result.requestedAt));
  assert(result.responseBytes > 0);
});
for (const url of ['https://public-agent.org/a?x=1', 'http://public-agent.org:8080/a'])
  test(`preserves new public target ${url}`, async () => {
    const value = observed();
    value.target = url;
    const r = rig(value);
    assert.equal((await r.operations.preflight({ url })).target, url);
  });
for (const url of [
  'file:///etc/passwd',
  'https://127.0.0.1/',
  'http://2130706433/',
  'http://0x7f000001/',
  'https://[::1]/',
  'https://localhost/',
  'https://host.internal/',
  'https://host.test/',
  'https://host.local/',
  'https://host.invalid/',
  'https://host.example/',
  'https://user:pass@public-agent.org/',
  'https://public-agent.org/#secret',
  'https://public-agent.org/ bad',
  'not a url',
  'https://public-agent.org\\secret',
  'x'.repeat(2049),
])
  test(`reject endpoint ${url.slice(0, 60)}`, async () => {
    const r = rig();
    assert.equal((await r.operations.preflight({ url })).status, 'rejected');
    assert.equal(r.calls.length, 0);
  });
for (const input of [null, [], {}, { url: target, extra: true }, { url: 42 }, target])
  test(`strict input ${JSON.stringify(input)}`, async () => {
    const r = rig();
    assert.equal((await r.operations.preflight(input)).status, 'rejected');
    assert.equal(r.calls.length, 0);
  });
test('host restriction is optional, exact and captured', async () => {
  const hosts = ['agent-guild-5d5r.onrender.com'];
  const r = rig(observed(), { allowedHosts: hosts });
  hosts.length = 0;
  assert.equal((await r.operations.preflight({ url: target })).status, 'observed');
  assert.equal(
    (await rig(observed(), { allowedHosts: [] }).operations.preflight({ url: target })).code,
    'host_not_approved',
  );
});
const mutations = [
  (v) => {
    v.target = 'https://other-agent.org/';
  },
  (v) => v.checks.pop(),
  (v) => v.checks.push(v.checks[0]),
  (v) => {
    v.checks[1] = v.checks[0];
  },
  (v) => {
    v.checks[0].check = 'invented';
  },
  (v) => {
    v.checks[0].status = 'pass';
  },
  (v) => {
    v.failed = [];
  },
  (v) => v.failed.push(v.failed[0]),
  (v) => v.unknowns.push(v.unknowns[0]),
  (v) => v.scored.push(v.scored[0]),
  (v) => {
    v.verdict = 'safe';
  },
  (v) => {
    v.verdict = 'no_failed_checks';
  },
];
mutations.forEach(
  (mutate, i) =>
    void test(`reject inconsistent evidence ${i}`, async () => {
      const value = observed();
      mutate(value);
      assert.equal((await rig(value).operations.preflight({ url: target })).status, 'rejected');
    }),
);
for (const [name, status, verdict] of [
  ['endpoint_reachable', 'failed', 'do_not_delegate'],
  ['protocol_handshake', 'failed', 'do_not_delegate'],
  ['agent_card_signed', 'failed', 'delegate_with_caution'],
  ['all', 'unknown', 'no_failed_checks'],
  ['all', 'proven', 'no_failed_checks'],
])
  test(`derive verdict ${name} ${status}`, async () => {
    const v = observed();
    for (const c of v.checks) c.status = 'proven';
    if (name === 'all') for (const c of v.checks) c.status = status;
    else v.checks.find((c) => c.check === name).status = status;
    corrected(v);
    const result = await rig(v).operations.preflight({ url: target });
    assert.equal(result.verdict, verdict);
    if (status === 'unknown') assert.equal(result.unknowns.length, 6);
  });
test('public credential remains unchanged and only credential goes in POST', async () => {
  const r = rig({ ...verifier(), instructions: 'REMOTE INSTRUCTION' });
  const input = passportInput();
  const saved = structuredClone(input.credential);
  const result = await r.operations.verifyPassport(input);
  assert.equal(result.verified, true);
  assert.equal(result.status, 'completed');
  assert.equal(result.timeAndBindingChecksPassed, true);
  assert.equal(r.calls.length, 1);
  assert.equal(r.calls[0].url, 'https://agent-guild-5d5r.onrender.com/credentials/verify');
  assert.equal(r.calls[0].init.method, 'POST');
  assert.deepEqual(JSON.parse(r.calls[0].init.body), saved);
  assert.deepEqual(input.credential, saved);
  assert(!JSON.stringify(result).includes('REMOTE INSTRUCTION'));
  assert(!JSON.stringify(result).includes('publicClaim'));
});
for (const [valid, guild] of [
  [false, true],
  [true, false],
  [false, false],
])
  test(`negative verifier ${valid} ${guild}`, async () => {
    const result = await rig(verifier(valid, guild)).operations.verifyPassport(passportInput());
    assert.equal(result.status, 'completed');
    assert.equal(result.verified, false);
  });
for (const value of [
  { ...verifier(), valid: 'true' },
  { ...verifier(), guild_issued: 1 },
  { ...verifier(), issuer: subject },
  { ...verifier(), subject_did: issuer },
  [],
  null,
])
  test(`invalid verification response ${JSON.stringify(value)}`, async () => {
    assert.equal((await rig(value).operations.verifyPassport(passportInput())).status, 'rejected');
  });
const invalidCredentials = [
  null,
  [],
  '{}',
  42,
  { a: undefined },
  { a: NaN },
  { a: Infinity },
  { a: () => {} },
  { a: 'x'.repeat(33000) },
  Object.create({ x: 1 }),
  {
    toJSON() {
      return {};
    },
  },
  JSON.parse('{"__proto__":{}}'),
  { a: new Array(2) },
];
invalidCredentials.forEach(
  (credential, i) =>
    void test(`non JSON/object credential ${i}`, async () => {
      const r = rig(verifier());
      const input = passportInput();
      input.credential = credential;
      assert.equal((await r.operations.verifyPassport(input)).status, 'rejected');
      assert.equal(r.calls.length, 0);
    }),
);
test('accessors, cyclic data, symbols and excessive depth fail before disclosure', async () => {
  let reads = 0;
  const accessor = {};
  Object.defineProperty(accessor, 'x', {
    enumerable: true,
    get() {
      reads++;
      return 1;
    },
  });
  const cycle = {};
  cycle.x = cycle;
  const symbol = { a: 1, [Symbol('x')]: 1 };
  let deep = {};
  for (let i = 0; i < 20; i++) deep = { x: deep };
  for (const c of [accessor, cycle, symbol, deep]) assert.throws(() => credentialObject(c));
  assert.equal(reads, 0);
  const input = {};
  Object.defineProperty(input, 'url', {
    enumerable: true,
    get() {
      reads++;
      return target;
    },
  });
  assert.equal((await rig().operations.preflight(input)).code, 'invalid_input');
  assert.equal(reads, 0);
});
for (const mutation of [
  'issuer',
  'subject',
  'did',
  'method',
  'proof',
  'purpose',
  'suite',
  'expired',
  'future',
  'stale',
  'calendar',
  'type',
])
  test(`passport binding ${mutation}`, async () => {
    const input = passportInput();
    const c = input.credential;
    if (mutation === 'issuer') c.issuer = subject;
    if (mutation === 'subject') c.credentialSubject.id = issuer;
    if (mutation === 'did') input.expectedIssuerDid = 'did:key:z123';
    if (mutation === 'method') c.proof.verificationMethod = 'wrong';
    if (mutation === 'proof') c.proof.proofValue = 'z123';
    if (mutation === 'purpose') c.proof.proofPurpose = 'authentication';
    if (mutation === 'suite') c.proof.cryptosuite = 'other';
    if (mutation === 'expired') c.validUntil = new Date(Date.now() - 1).toISOString();
    if (mutation === 'future') c.validFrom = new Date(Date.now() + 10000).toISOString();
    if (mutation === 'stale') c.validFrom = new Date(Date.now() - 86401000).toISOString();
    if (mutation === 'calendar') c.validFrom = '2026-02-30T00:00:00Z';
    if (mutation === 'type') c.type = ['VerifiableCredential'];
    const r = rig(verifier());
    assert.equal((await r.operations.verifyPassport(input)).status, 'rejected');
    assert.equal(r.calls.length, 0);
  });
test('date checks rerun after response', async () => {
  const input = passportInput();
  input.credential.validUntil = new Date(Date.now() + 50).toISOString();
  const ops = createOperations({
    fetch: async () => {
      await new Promise((r) => setTimeout(r, 70));
      return jsonResponse(verifier());
    },
  });
  assert.equal((await ops.verifyPassport(input)).code, 'credential_outside_validity');
});
test('strict supported DID encoding and calendar timestamps', () => {
  assert.equal(did(issuer), issuer);
  assert.throws(() => did(`${issuer}1`));
  assert.throws(() => timestamp('2026-02-30T00:00:00Z'));
  assert.throws(() => timestamp('2026-01-01T00:00:00+02:00'));
  assert.equal(timestamp('2026-01-01T00:00:00.123456Z'), Date.parse('2026-01-01T00:00:00.123Z'));
});
for (const options of [
  { timeoutMs: 0 },
  { timeoutMs: 45001 },
  { maxPassportAgeSeconds: 0 },
  { allowedHosts: ['EXAMPLE.ORG'] },
  { allowedHosts: ['https://example.org'] },
  { expectedIssuerDid: 'bad' },
])
  test(`invalid host config ${JSON.stringify(options)}`, () =>
    assert.throws(() => createOperations(options)));
