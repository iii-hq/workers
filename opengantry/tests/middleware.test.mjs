import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';

import { mintVerdictToken, verdictClaimsFor } from '@jeger-ai/opengantry/kernel';

import { GantryDenied } from '../src/lib/denied.js';
import { createMiddlewareHandler } from '../src/lib/middleware.js';
import { LEASE_STATES, LeaseStore } from '../src/lib/lease-store.js';
import { defaultLeaseStorePath } from '../src/lib/repo-path.js';
import { writeKeyring, writeMiniGantryRepo } from './helpers/mini-repo.mjs';

test('middleware throws on promote without mission binding', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-'));
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
        context: { msn_id: 'MSN-0175', worktree_path: repoRoot },
      }),
    (err) => err instanceof GantryDenied && err.code === 'VERDICT_TOKEN_MISSING',
  );
});

test('middleware throws when token does not match current mission revision', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-stale-'));
  const krDir = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-kr-'));
  const keyring = writeKeyring(krDir);
  const { missionRel, msnId } = writeMiniGantryRepo(repoRoot);
  const claims = verdictClaimsFor(repoRoot, missionRel);
  const token = mintVerdictToken({ ...claims, keyringPath: keyring });
  fs.appendFileSync(path.join(repoRoot, missionRel), '\n# edited\n');
  const prevKeyring = process.env.GANTRY_VERDICT_KEYRING;
  process.env.GANTRY_VERDICT_KEYRING = keyring;
  const leases = new LeaseStore(defaultLeaseStorePath(repoRoot));
  leases.bindMissionRel(msnId, missionRel);
  const state = {
    leaseStores: new Map([[repoRoot, leases]]),
    forwardTrigger: async () => ({ ok: true }),
  };
  try {
    const middleware = createMiddlewareHandler(state);
    await assert.rejects(
      () =>
        middleware({
          function_id: 'src::promote',
          payload: {},
          context: { msn_id: msnId, worktree_path: repoRoot, verdict_token: token },
        }),
      (err) => err instanceof GantryDenied && err.code === 'VERDICT_TOKEN_INVALID',
    );
  } finally {
    if (prevKeyring === undefined) delete process.env.GANTRY_VERDICT_KEYRING;
    else process.env.GANTRY_VERDICT_KEYRING = prevKeyring;
  }
});

test('middleware allows promote with valid token and mission_rel', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-ok-'));
  const krDir = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-kr2-'));
  const keyring = writeKeyring(krDir);
  const { missionRel, msnId } = writeMiniGantryRepo(repoRoot);
  const claims = verdictClaimsFor(repoRoot, missionRel);
  const token = mintVerdictToken({ ...claims, keyringPath: keyring });
  const prevKeyring = process.env.GANTRY_VERDICT_KEYRING;
  process.env.GANTRY_VERDICT_KEYRING = keyring;
  const leases = new LeaseStore(defaultLeaseStorePath(repoRoot));
  leases.bindMissionRel(msnId, missionRel);
  const state = {
    leaseStores: new Map([[repoRoot, leases]]),
    forwardTrigger: async (fid, payload) => ({ ok: true, fid, payload }),
  };
  try {
    const middleware = createMiddlewareHandler(state);
    const result = await middleware({
      function_id: 'src::promote',
      payload: { branch: 'main' },
      context: { msn_id: msnId, worktree_path: repoRoot, verdict_token: token },
    });
    assert.equal(result.ok, true);
  } finally {
    if (prevKeyring === undefined) delete process.env.GANTRY_VERDICT_KEYRING;
    else process.env.GANTRY_VERDICT_KEYRING = prevKeyring;
  }
});

test('middleware throws on corrupted lease store', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-corrupt-'));
  const storePath = defaultLeaseStorePath(repoRoot);
  fs.mkdirSync(path.dirname(storePath), { recursive: true });
  fs.writeFileSync(storePath, '{"leases": null}');
  const leases = new LeaseStore(storePath);
  const state = {
    leaseStores: new Map([[repoRoot, leases]]),
    forwardTrigger: async () => ({ ok: true }),
  };
  const middleware = createMiddlewareHandler(state);
  await assert.rejects(
    () =>
      middleware({
        function_id: 'math::add',
        payload: {},
        context: { msn_id: 'MSN-1', worktree_path: repoRoot, holder_id: 'h1' },
      }),
    (err) => err instanceof GantryDenied && err.code === 'LEASE_STORE_CORRUPTED',
  );
});
