import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import {
  mintVerdictToken,
  verifyVerdictToken,
  verdictClaimsFor,
  isPromoteClassFunctionId,
} from '@jeger-ai/opengantry/kernel';

import { GantryDenied } from './src/denied.js';
import { LeaseStore, LEASE_STATES } from './src/lease-store.js';
import { createMiddlewareHandler, isReservedGovernanceFunctionId } from './src/middleware.js';
import { VerifyCoalescer } from './src/verify.js';
import { defaultLeaseStorePath, resolveVerifyRepoRoot } from './src/repo-path.js';
import { writeKeyring, writeMiniGantryRepo } from './tests/helpers/mini-repo.mjs';

function pass(label) {
  console.log(`PASS ${label}`);
}

function testKernelExports() {
  assert.equal(typeof verdictClaimsFor, 'function');
  assert.equal(typeof mintVerdictToken, 'function');
  assert.equal(typeof verifyVerdictToken, 'function');
  pass('kernel exports');
}

function testVerdictTokenRoundTrip() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'og-demo-vt-'));
  const keyring = writeKeyring(dir);
  const expected = {
    msn_id: 'MSN-0155',
    mission_sha256: 'sha',
    findings_digest: 'dig',
    gate_command: 'npm test',
    org_id: 'demo-org',
  };
  const token = mintVerdictToken({ ...expected, keyringPath: keyring });
  assert.ok(verifyVerdictToken({ token, expected, keyringPath: keyring }));
  pass('verdict token round-trip');
}

function testReservedNamespace() {
  assert.equal(isReservedGovernanceFunctionId('gantry::verify'), true);
  assert.equal(isReservedGovernanceFunctionId('demo::work'), false);
  pass('reserved namespace guard');
}

async function testMiddlewarePromoteDenied() {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-demo-repo-'));
  const state = {
    leaseStores: new Map(),
    forwardTrigger: async (fid, payload) => ({ ok: true, fid, payload }),
  };
  const middleware = createMiddlewareHandler(state);
  await assert.rejects(
    () =>
      middleware({
        function_id: 'src::promote',
        payload: {},
        context: { msn_id: 'MSN-0155', worktree_path: repoRoot },
      }),
    (err) => err instanceof GantryDenied,
  );
  pass('middleware throws on promote without verdict (fail-closed)');
}

async function testMiddlewarePromoteAllowed() {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-demo-repo2-'));
  const krDir = fs.mkdtempSync(path.join(os.tmpdir(), 'og-demo-vt2-'));
  const keyring = writeKeyring(krDir);
  const { missionRel, msnId } = writeMiniGantryRepo(repoRoot, {
    msnId: 'MSN-0155',
    missionRel: '.gitagent/missions/MSN-0155.yaml',
  });
  const expected = verdictClaimsFor(repoRoot, missionRel);
  const token = mintVerdictToken({ ...expected, keyringPath: keyring });
  const prevKeyring = process.env.GANTRY_VERDICT_KEYRING;
  process.env.GANTRY_VERDICT_KEYRING = keyring;
  const storePath = defaultLeaseStorePath(repoRoot);
  const leases = new LeaseStore(storePath);
  leases.bindMissionRel(msnId, missionRel);
  const state = {
    leaseStores: new Map([[repoRoot, leases]]),
    forwardTrigger: async (fid, payload) => ({ ok: true, fid, payload }),
  };
  const middleware = createMiddlewareHandler(state);
  try {
    const result = await middleware({
      function_id: 'src::promote',
      payload: { branch: 'gxt/msn-0155' },
      context: {
        msn_id: msnId,
        worktree_path: repoRoot,
        verdict_token: token,
      },
    });
    assert.equal(result.ok, true);
    pass('middleware allows promote with verdict token');
  } finally {
    if (prevKeyring === undefined) delete process.env.GANTRY_VERDICT_KEYRING;
    else process.env.GANTRY_VERDICT_KEYRING = prevKeyring;
  }
}

async function testMiddlewareMissingPathThrows() {
  const state = {
    leaseStores: new Map(),
    forwardTrigger: async () => ({ ok: true }),
  };
  const middleware = createMiddlewareHandler(state);
  await assert.rejects(
    () =>
      middleware({
        function_id: 'demo::work',
        payload: {},
        context: { msn_id: 'MSN-0155' },
      }),
    /worktree_path or context\.repo_root required/,
  );
  pass('middleware throws when repo path missing');
}

function testDurableLeaseStorePath() {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-demo-lease-'));
  const storePath = defaultLeaseStorePath(repoRoot);
  assert.equal(storePath, path.join(repoRoot, '.gitagent', 'leases.json'));
  const leases = new LeaseStore(storePath);
  leases.upsert({
    msn_id: 'MSN-0159',
    branch: 'gxt/msn-0159',
    state: LEASE_STATES.active,
    session_refs: {},
  });
  assert.ok(fs.existsSync(storePath));
  pass('lease store persists under .gitagent/leases.json');
}

function testVerifyRequiresAbsoluteRepoRoot() {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-demo-abs-'));
  writeMiniGantryRepo(repoRoot, {
    msnId: 'MSN-0155',
    missionRel: '.gitagent/missions/MSN-0155.yaml',
  });
  assert.throws(() => resolveVerifyRepoRoot('target-repo'), /repo_root must be an absolute path/);
  assert.throws(() => resolveVerifyRepoRoot(undefined), /repo_root required/);
  assert.equal(resolveVerifyRepoRoot(repoRoot), repoRoot);
  pass('verify requires absolute repo_root with .gitagent present');
}

async function testVerifyCoalescing() {
  const coalescer = new VerifyCoalescer();
  let runs = 0;
  const key = 'repo:msn';
  const p1 = coalescer.run(key, async () => {
    runs += 1;
    await new Promise((r) => setTimeout(r, 20));
    return { status: 'passed' };
  });
  const p2 = coalescer.run(key, async () => {
    runs += 1;
    return { status: 'passed' };
  });
  const [a, b] = await Promise.all([p1, p2]);
  assert.equal(a.status, 'passed');
  assert.equal(b.status, 'passed');
  assert.equal(runs, 1);
  pass('verify coalescing single-flight');
}

function testPromoteClassDetection() {
  assert.equal(isPromoteClassFunctionId('demo::push'), true);
  assert.equal(isPromoteClassFunctionId('math::add'), false);
  pass('promote-class detection');
}

function testTombstoneState() {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-demo-tomb-'));
  const storePath = defaultLeaseStorePath(repoRoot);
  const leases = new LeaseStore(storePath);
  leases.upsert({
    msn_id: 'MSN-0155',
    branch: 'gxt/msn-0155',
    state: LEASE_STATES.promoting,
    session_refs: Object.create(null),
  });
  leases.acquireSession('MSN-0155', 'rogue');
  leases.releaseSession('MSN-0155', 'rogue');
  const lease = leases.get('MSN-0155');
  assert.equal(lease.state, LEASE_STATES.tombstoned);
  pass('tombstone on disconnect while promoting');
}

async function main() {
  testKernelExports();
  testVerdictTokenRoundTrip();
  testReservedNamespace();
  await testMiddlewarePromoteDenied();
  await testMiddlewarePromoteAllowed();
  await testMiddlewareMissingPathThrows();
  testDurableLeaseStorePath();
  testVerifyRequiresAbsoluteRepoRoot();
  await testVerifyCoalescing();
  testPromoteClassDetection();
  testTombstoneState();
  console.log('demo.mjs: all checks passed');
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
