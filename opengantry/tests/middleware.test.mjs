import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';

import { mintVerdictToken } from '@jeger-ai/opengantry/kernel';

import { createMiddlewareHandler } from '../src/lib/middleware.js';
import { LeaseStore } from '../src/lib/lease-store.js';
import { defaultLeaseStorePath } from '../src/lib/repo-path.js';
import { bindVerdictLease } from './helpers/lease-fixtures.mjs';

test('middleware denies promote without a verify pass', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-'));
  const state = {
    leaseStores: new Map(),
    forwardTrigger: async (fid, payload) => ({ ok: true, fid, payload }),
  };
  const middleware = createMiddlewareHandler(state);
  const result = await middleware({
    function_id: 'demo::promote',
    payload: {},
    context: { msn_id: 'MSN-0175', worktree_path: repoRoot },
  });
  assert.equal(result.status, 'failed');
});

test('middleware denies promote when token does not match bound claims', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-stale-'));
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-kr-'));
  const keyring = path.join(dir, 'pepper-keyring.json');
  fs.writeFileSync(
    keyring,
    JSON.stringify([{ org_id: 'demo-org', pepper_version: 1, pepper: 'demo-pepper' }]),
    { mode: 0o600 },
  );
  const stale = {
    msn_id: 'MSN-0175',
    mission_sha256: 'old-sha',
    findings_digest: 'dig',
    gate_command: 'npm test',
    org_id: 'demo-org',
  };
  const bound = { ...stale, mission_sha256: 'new-sha' };
  const token = mintVerdictToken({ ...stale, keyringPath: keyring });
  const prevKeyring = process.env.GANTRY_VERDICT_KEYRING;
  process.env.GANTRY_VERDICT_KEYRING = keyring;
  const storePath = defaultLeaseStorePath(repoRoot);
  const leases = new LeaseStore(storePath);
  bindVerdictLease(leases, 'MSN-0175', bound);
  const state = {
    leaseStores: new Map([[repoRoot, leases]]),
    forwardTrigger: async () => ({ ok: true }),
  };
  try {
    const middleware = createMiddlewareHandler(state);
    const result = await middleware({
      function_id: 'demo::promote',
      payload: {},
      context: { msn_id: 'MSN-0175', worktree_path: repoRoot, verdict_token: token },
    });
    assert.equal(result.status, 'failed');
  } finally {
    if (prevKeyring === undefined) delete process.env.GANTRY_VERDICT_KEYRING;
    else process.env.GANTRY_VERDICT_KEYRING = prevKeyring;
  }
});

test('middleware allows promote with bound matching verdict token', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-ok-'));
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-kr2-'));
  const keyring = path.join(dir, 'pepper-keyring.json');
  fs.writeFileSync(
    keyring,
    JSON.stringify([{ org_id: 'demo-org', pepper_version: 1, pepper: 'demo-pepper' }]),
    { mode: 0o600 },
  );
  const expected = {
    msn_id: 'MSN-0175',
    mission_sha256: 'sha',
    findings_digest: 'dig',
    gate_command: 'npm test',
    org_id: 'demo-org',
  };
  const token = mintVerdictToken({ ...expected, keyringPath: keyring });
  const prevKeyring = process.env.GANTRY_VERDICT_KEYRING;
  process.env.GANTRY_VERDICT_KEYRING = keyring;
  const storePath = defaultLeaseStorePath(repoRoot);
  const leases = new LeaseStore(storePath);
  bindVerdictLease(leases, 'MSN-0175', expected);
  const state = {
    leaseStores: new Map([[repoRoot, leases]]),
    forwardTrigger: async (fid, payload) => ({ ok: true, fid, payload }),
  };
  try {
    const middleware = createMiddlewareHandler(state);
    const result = await middleware({
      function_id: 'demo::promote',
      payload: { branch: 'main' },
      context: { msn_id: 'MSN-0175', worktree_path: repoRoot, verdict_token: token },
    });
    assert.equal(result.ok, true);
  } finally {
    if (prevKeyring === undefined) delete process.env.GANTRY_VERDICT_KEYRING;
    else process.env.GANTRY_VERDICT_KEYRING = prevKeyring;
  }
});
