import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';

import { mintVerdictToken, verdictClaimsFor } from '@jeger-ai/opengantry/kernel';

import { GantryDenied } from '../src/lib/denied.js';
import { createMiddlewareHandler } from '../src/lib/middleware.js';
import { getLeaseStore } from '../src/lib/middleware.js';
import { onVerifyPassed } from '../src/lib/verify-handler.js';
import { createWorkerState } from '../src/lib/worker-state.js';
import { writeKeyring, writeMiniGantryRepo } from './helpers/mini-repo.mjs';

test('onVerifyPassed pins mission_rel on lease store', () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-vp-pin-'));
  const { missionRel, msnId } = writeMiniGantryRepo(repoRoot);
  const state = createWorkerState();
  onVerifyPassed(state, {
    repo_root: repoRoot,
    msn_id: msnId,
    mission_rel_path: missionRel,
  });
  const leases = getLeaseStore(state, repoRoot);
  assert.equal(leases.get(msnId)?.mission_rel, missionRel);
});

test('minted token promotes when mission_rel is bound', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-vp-promote-'));
  const krDir = fs.mkdtempSync(path.join(os.tmpdir(), 'og-vp-kr-'));
  const keyring = writeKeyring(krDir);
  const { missionRel, msnId } = writeMiniGantryRepo(repoRoot);
  const claims = verdictClaimsFor(repoRoot, missionRel);
  const token = mintVerdictToken({ ...claims, keyringPath: keyring });
  const state = createWorkerState();
  onVerifyPassed(state, { repo_root: repoRoot, msn_id: msnId, mission_rel_path: missionRel });
  const prevKeyring = process.env.GANTRY_VERDICT_KEYRING;
  process.env.GANTRY_VERDICT_KEYRING = keyring;
  state.forwardTrigger = async () => ({ ok: true });
  const middleware = createMiddlewareHandler(state);
  try {
    const result = await middleware({
      function_id: 'src::promote',
      payload: {},
      context: { msn_id: msnId, worktree_path: repoRoot, verdict_token: token },
    });
    assert.equal(result.ok, true);
  } finally {
    if (prevKeyring === undefined) delete process.env.GANTRY_VERDICT_KEYRING;
    else process.env.GANTRY_VERDICT_KEYRING = prevKeyring;
  }
});

test('mission edited after mint denies promote', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-vp-deny-'));
  const krDir = fs.mkdtempSync(path.join(os.tmpdir(), 'og-vp-kr2-'));
  const keyring = writeKeyring(krDir);
  const { missionRel, msnId } = writeMiniGantryRepo(repoRoot);
  const claims = verdictClaimsFor(repoRoot, missionRel);
  const token = mintVerdictToken({ ...claims, keyringPath: keyring });
  fs.appendFileSync(path.join(repoRoot, missionRel), '\n# tamper\n');
  const state = createWorkerState();
  onVerifyPassed(state, { repo_root: repoRoot, msn_id: msnId, mission_rel_path: missionRel });
  const prevKeyring = process.env.GANTRY_VERDICT_KEYRING;
  process.env.GANTRY_VERDICT_KEYRING = keyring;
  state.forwardTrigger = async () => ({ ok: true });
  const middleware = createMiddlewareHandler(state);
  try {
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

test('missing org config denies promote', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-vp-org-'));
  const krDir = fs.mkdtempSync(path.join(os.tmpdir(), 'og-vp-kr3-'));
  const keyring = writeKeyring(krDir);
  const { missionRel, msnId } = writeMiniGantryRepo(repoRoot);
  const claims = verdictClaimsFor(repoRoot, missionRel);
  const token = mintVerdictToken({ ...claims, keyringPath: keyring });
  fs.unlinkSync(path.join(repoRoot, '.gitagent/foreman/ORG.export.local'));
  delete process.env.GANTRY_ORG_ID;
  const state = createWorkerState();
  onVerifyPassed(state, { repo_root: repoRoot, msn_id: msnId, mission_rel_path: missionRel });
  const prevKeyring = process.env.GANTRY_VERDICT_KEYRING;
  process.env.GANTRY_VERDICT_KEYRING = keyring;
  state.forwardTrigger = async () => ({ ok: true });
  const middleware = createMiddlewareHandler(state);
  try {
    await assert.rejects(
      () =>
        middleware({
          function_id: 'src::promote',
          payload: {},
          context: { msn_id: msnId, worktree_path: repoRoot, verdict_token: token },
        }),
      (err) => err instanceof GantryDenied && err.code === 'ORG_ID_MISSING',
    );
  } finally {
    if (prevKeyring === undefined) delete process.env.GANTRY_VERDICT_KEYRING;
    else process.env.GANTRY_VERDICT_KEYRING = prevKeyring;
  }
});
