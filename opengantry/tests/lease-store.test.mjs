import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';

import { LEASE_STATES, LeaseStore } from '../src/lease-store.js';
import { resolveLeaseStorePath } from '../src/repo-path.js';

test('lease store rejects path outside .gitagent', () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-ls-path-'));
  assert.throws(
    () => resolveLeaseStorePath(repoRoot, '/tmp/evil-leases.json'),
    /must resolve under/,
    'lease store override outside .gitagent should throw',
  );
});

test('corrupted lease store blocks get', () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-ls-corrupt-'));
  const storePath = resolveLeaseStorePath(repoRoot);
  fs.mkdirSync(path.dirname(storePath), { recursive: true });
  fs.writeFileSync(storePath, '{"leases": null}');
  const store = new LeaseStore(storePath);
  assert.equal(store.corrupted, true, 'invalid leases array should mark store corrupted');
  assert.equal(store.get('MSN-0001'), undefined, 'corrupted store should return no leases');
});

test('unknown lease state marks corrupted on load', () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-ls-badstate-'));
  const storePath = resolveLeaseStorePath(repoRoot);
  fs.mkdirSync(path.dirname(storePath), { recursive: true });
  fs.writeFileSync(
    storePath,
    JSON.stringify({
      leases: [{ msn_id: 'MSN-0001', state: 'not-a-real-state', session_refs: {} }],
    }),
  );
  const store = new LeaseStore(storePath);
  assert.equal(store.corrupted, true, 'unknown lease state should mark store corrupted');
});

test('missing lease state marks corrupted on load', () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-ls-nostate-'));
  const storePath = resolveLeaseStorePath(repoRoot);
  fs.mkdirSync(path.dirname(storePath), { recursive: true });
  fs.writeFileSync(
    storePath,
    JSON.stringify({
      leases: [{ msn_id: 'MSN-0001', session_refs: {} }],
    }),
  );
  const store = new LeaseStore(storePath);
  assert.equal(store.corrupted, true, 'missing lease state should mark store corrupted');
});

test('lease file is written with restrictive permissions', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-ls-mode-'));
  const storePath = resolveLeaseStorePath(repoRoot);
  const store = new LeaseStore(storePath);
  await store.upsert({
    msn_id: 'MSN-0004',
    state: LEASE_STATES.active,
    session_refs: Object.create(null),
  });
  const mode = fs.statSync(storePath).mode & 0o777;
  assert.equal(mode, 0o600, 'lease file should be written with mode 0600');
});

test('get returns clone — caller mutation does not affect store', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-ls-clone-'));
  const store = new LeaseStore(resolveLeaseStorePath(repoRoot));
  await store.upsert({
    msn_id: 'MSN-0001',
    state: LEASE_STATES.active,
    session_refs: Object.create(null),
  });
  const lease = store.get('MSN-0001');
  lease.state = LEASE_STATES.tombstoned;
  assert.equal(
    store.get('MSN-0001')?.state,
    LEASE_STATES.active,
    'mutating get() result should not affect stored lease',
  );
});

test('transition rejects stale from-state', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-ls-cas-'));
  const store = new LeaseStore(resolveLeaseStorePath(repoRoot));
  await store.upsert({
    msn_id: 'MSN-0001',
    state: LEASE_STATES.active,
    session_refs: Object.create(null),
  });
  assert.equal(
    await store.transition('MSN-0001', LEASE_STATES.promoting, LEASE_STATES.active),
    false,
    'transition from wrong state should fail',
  );
  assert.equal(
    store.get('MSN-0001')?.state,
    LEASE_STATES.active,
    'failed transition should not change state',
  );
});

test('tombstone survives late promoting→active transition', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-ls-tomb-'));
  const store = new LeaseStore(resolveLeaseStorePath(repoRoot));
  await store.upsert({
    msn_id: 'MSN-0001',
    state: LEASE_STATES.promoting,
    session_refs: Object.create(null),
  });
  await store.acquireSession('MSN-0001', 'holder-a');
  await store.releaseSession('MSN-0001', 'holder-a');
  assert.equal(
    store.get('MSN-0001')?.state,
    LEASE_STATES.tombstoned,
    'release during promote should tombstone',
  );
  assert.equal(
    await store.transition('MSN-0001', LEASE_STATES.promoting, LEASE_STATES.active),
    false,
    'late promoting→active transition should fail on tombstoned lease',
  );
  assert.equal(
    store.get('MSN-0001')?.state,
    LEASE_STATES.tombstoned,
    'tombstone state should survive failed transition',
  );
});

test('constructor holderId does not break session counting', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-ls-proto-'));
  const store = new LeaseStore(resolveLeaseStorePath(repoRoot));
  await store.upsert({
    msn_id: 'MSN-0001',
    state: 'active',
    session_refs: Object.create(null),
  });
  const afterCtor = await store.acquireSession('MSN-0001', 'constructor');
  assert.equal(afterCtor, null, 'dangerous holder id constructor should be rejected');
  await store.acquireSession('MSN-0001', 'holder-a');
  await store.releaseSession('MSN-0001', 'holder-a');
  const lease = store.get('MSN-0001');
  assert.equal(
    Object.keys(lease.session_refs ?? {}).length,
    0,
    'session refs should be empty after release',
  );
});

test('atomic persist survives reload', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-ls-persist-'));
  const storePath = resolveLeaseStorePath(repoRoot);
  const store = new LeaseStore(storePath);
  await store.upsert({
    msn_id: 'MSN-0002',
    state: 'active',
    session_refs: Object.create(null),
  });
  const reloaded = new LeaseStore(storePath);
  assert.equal(
    reloaded.get('MSN-0002')?.msn_id,
    'MSN-0002',
    'persisted lease should survive reload',
  );
});

test('verdict_expected stripped from persisted lease rows', () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-ls-ve-'));
  const storePath = resolveLeaseStorePath(repoRoot);
  fs.mkdirSync(path.dirname(storePath), { recursive: true });
  fs.writeFileSync(
    storePath,
    JSON.stringify({
      leases: [
        {
          msn_id: 'MSN-0003',
          state: 'active',
          session_refs: {},
          verdict_expected: { msn_id: 'MSN-0003' },
        },
      ],
    }),
  );
  const store = new LeaseStore(storePath);
  const lease = store.get('MSN-0003');
  assert.equal(lease.verdict_expected, undefined, 'verdict_expected should be stripped on load');
});
