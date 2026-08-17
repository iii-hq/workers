import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';

import { LEASE_STATES, LeaseStore } from '../src/lib/lease-store.js';
import { defaultLeaseStorePath } from '../src/lib/repo-path.js';

test('lease store rejects path outside .gitagent', () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-ls-path-'));
  const prev = process.env.GANTRY_III_LEASE_STORE;
  process.env.GANTRY_III_LEASE_STORE = '/tmp/evil-leases.json';
  try {
    assert.throws(() => defaultLeaseStorePath(repoRoot), /must resolve under/);
  } finally {
    if (prev === undefined) delete process.env.GANTRY_III_LEASE_STORE;
    else process.env.GANTRY_III_LEASE_STORE = prev;
  }
});

test('corrupted lease store blocks get', () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-ls-corrupt-'));
  const storePath = defaultLeaseStorePath(repoRoot);
  fs.mkdirSync(path.dirname(storePath), { recursive: true });
  fs.writeFileSync(storePath, '{"leases": null}');
  const store = new LeaseStore(storePath);
  assert.equal(store.corrupted, true);
  assert.equal(store.get('MSN-0001'), undefined);
});

test('unknown lease state marks corrupted on load', () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-ls-badstate-'));
  const storePath = defaultLeaseStorePath(repoRoot);
  fs.mkdirSync(path.dirname(storePath), { recursive: true });
  fs.writeFileSync(
    storePath,
    JSON.stringify({
      leases: [{ msn_id: 'MSN-0001', state: 'not-a-real-state', session_refs: {} }],
    }),
  );
  const store = new LeaseStore(storePath);
  assert.equal(store.corrupted, true);
});

test('get returns clone — caller mutation does not affect store', () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-ls-clone-'));
  const store = new LeaseStore(defaultLeaseStorePath(repoRoot));
  store.upsert({
    msn_id: 'MSN-0001',
    state: LEASE_STATES.active,
    session_refs: Object.create(null),
  });
  const lease = store.get('MSN-0001');
  lease.state = LEASE_STATES.tombstoned;
  assert.equal(store.get('MSN-0001')?.state, LEASE_STATES.active);
});

test('transition rejects stale from-state', () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-ls-cas-'));
  const store = new LeaseStore(defaultLeaseStorePath(repoRoot));
  store.upsert({
    msn_id: 'MSN-0001',
    state: LEASE_STATES.active,
    session_refs: Object.create(null),
  });
  assert.equal(store.transition('MSN-0001', LEASE_STATES.promoting, LEASE_STATES.active), false);
  assert.equal(store.get('MSN-0001')?.state, LEASE_STATES.active);
});

test('tombstone survives late promoting→active transition', () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-ls-tomb-'));
  const store = new LeaseStore(defaultLeaseStorePath(repoRoot));
  store.upsert({
    msn_id: 'MSN-0001',
    state: LEASE_STATES.promoting,
    session_refs: Object.create(null),
  });
  store.acquireSession('MSN-0001', 'holder-a');
  store.releaseSession('MSN-0001', 'holder-a');
  assert.equal(store.get('MSN-0001')?.state, LEASE_STATES.tombstoned);
  assert.equal(store.transition('MSN-0001', LEASE_STATES.promoting, LEASE_STATES.active), false);
  assert.equal(store.get('MSN-0001')?.state, LEASE_STATES.tombstoned);
});

test('constructor holderId does not break session counting', () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-ls-proto-'));
  const store = new LeaseStore(defaultLeaseStorePath(repoRoot));
  store.upsert({
    msn_id: 'MSN-0001',
    state: 'active',
    session_refs: Object.create(null),
  });
  const afterCtor = store.acquireSession('MSN-0001', 'constructor');
  assert.equal(afterCtor, null);
  store.acquireSession('MSN-0001', 'holder-a');
  store.releaseSession('MSN-0001', 'holder-a');
  const lease = store.get('MSN-0001');
  assert.equal(Object.keys(lease.session_refs ?? {}).length, 0);
});

test('atomic persist survives reload', () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-ls-persist-'));
  const storePath = defaultLeaseStorePath(repoRoot);
  const store = new LeaseStore(storePath);
  store.upsert({
    msn_id: 'MSN-0002',
    state: 'active',
    session_refs: Object.create(null),
  });
  const reloaded = new LeaseStore(storePath);
  assert.equal(reloaded.get('MSN-0002')?.msn_id, 'MSN-0002');
});

test('verdict_expected stripped from persisted lease rows', () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-ls-ve-'));
  const storePath = defaultLeaseStorePath(repoRoot);
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
  assert.equal(lease.verdict_expected, undefined);
});
