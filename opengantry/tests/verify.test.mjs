import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';

import { isPromoteClassFunctionId } from '@jeger-ai/opengantry/kernel';

import { LEASE_STATES } from '../src/lease-store.js';
import { isReservedGovernanceFunctionId } from '../src/namespace.js';
import { defaultLeaseStorePath } from '../src/repo-path.js';
import { createGantryRuntime } from '../src/runtime.js';
import { VerifyCoalescer, VerifyCoalescerSaturationError } from '../src/verify.js';

test('verify coalescing collapses concurrent runs to one execution', async () => {
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
  assert.equal(a.status, 'passed', 'first coalesced call should pass');
  assert.equal(b.status, 'passed', 'second coalesced call should pass');
  assert.equal(runs, 1, 'expected the second concurrent verify to reuse the in-flight promise');
});

test('verify coalescer throws when maxInFlight is reached', async () => {
  const coalescer = new VerifyCoalescer();
  coalescer.maxInFlight = 1;
  const blocker = coalescer.run('block', async () => {
    await new Promise((r) => setTimeout(r, 50));
    return { status: 'passed' };
  });
  await assert.rejects(
    () => coalescer.run('other', async () => ({ status: 'passed' })),
    (err) => err instanceof VerifyCoalescerSaturationError,
    'expected saturation when maxInFlight concurrent verifies are already running',
  );
  await blocker;
});

test('promote-class function ids are detected', () => {
  assert.equal(isPromoteClassFunctionId('demo::push'), true, 'push should be promote-class');
  assert.equal(isPromoteClassFunctionId('math::add'), false, 'add should not be promote-class');
});

test('reserved governance namespace blocks gantry squatting', () => {
  assert.equal(
    isReservedGovernanceFunctionId('gantry::verify'),
    true,
    'gantry::verify should be reserved',
  );
  assert.equal(
    isReservedGovernanceFunctionId('demo::work'),
    false,
    'unrelated namespaces should not be reserved',
  );
});

test('tombstone on disconnect while promoting', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-v-tomb-'));
  const runtime = createGantryRuntime({
    forwardTrigger: async () => ({ ok: true }),
  });
  const leases = runtime.leaseStoreFor(repoRoot);
  await leases.upsert({
    msn_id: 'MSN-0155',
    branch: 'gxt/msn-0155',
    state: LEASE_STATES.promoting,
    session_refs: Object.create(null),
  });
  await leases.acquireSession('MSN-0155', 'rogue');
  await leases.releaseSession('MSN-0155', 'rogue');
  const lease = leases.get('MSN-0155');
  assert.equal(
    lease.state,
    LEASE_STATES.tombstoned,
    'promoting lease should tombstone when last session disconnects',
  );
});

test('default lease store path resolves under .gitagent', () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-v-lease-path-'));
  const storePath = defaultLeaseStorePath(repoRoot);
  assert.equal(
    storePath,
    path.join(repoRoot, '.gitagent', 'leases.json'),
    'lease store should default to .gitagent/leases.json',
  );
});
