import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';

import { mintVerdictToken, verdictClaimsFor } from '@jeger-ai/opengantry/kernel';

import { GantryDenied } from '../src/denied.js';
import { createGantryRuntime } from '../src/runtime.js';
import { writeMiniGantryRepo } from './helpers/mini-repo.mjs';
import { withKeyring } from './helpers/with-keyring.mjs';

test('onVerifyPassed pins mission_rel on lease store', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-vp-pin-'));
  const { missionRel, msnId } = writeMiniGantryRepo(repoRoot);
  const runtime = createGantryRuntime({
    forwardTrigger: async () => ({ ok: true }),
  });
  await runtime.onVerifyPassed({
    repo_root: repoRoot,
    msn_id: msnId,
    mission_rel_path: missionRel,
  });
  const leases = runtime.leaseStoreFor(repoRoot);
  assert.equal(leases.get(msnId)?.mission_rel, missionRel, 'verify pass should bind mission_rel');
});

test('minted token promotes when mission_rel is bound', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-vp-promote-'));
  const krDir = fs.mkdtempSync(path.join(os.tmpdir(), 'og-vp-kr-'));
  const { missionRel, msnId } = writeMiniGantryRepo(repoRoot);
  const claims = verdictClaimsFor(repoRoot, missionRel);
  await withKeyring(krDir, async (keyring) => {
    const token = mintVerdictToken({ ...claims, keyringPath: keyring });
    const runtime = createGantryRuntime({
      forwardTrigger: async () => ({ ok: true }),
      verdictKeyringPath: keyring,
    });
    await runtime.onVerifyPassed({
      repo_root: repoRoot,
      msn_id: msnId,
      mission_rel_path: missionRel,
    });
    const result = await runtime.middleware({
      function_id: 'src::promote',
      payload: {},
      context: { msn_id: msnId, worktree_path: repoRoot, verdict_token: token },
    });
    assert.equal(result.ok, true, 'minted token should allow promote when mission is bound');
  });
});

test('mission edited after mint denies promote', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-vp-deny-'));
  const krDir = fs.mkdtempSync(path.join(os.tmpdir(), 'og-vp-kr2-'));
  const { missionRel, msnId } = writeMiniGantryRepo(repoRoot);
  const claims = verdictClaimsFor(repoRoot, missionRel);
  await withKeyring(krDir, async (keyring) => {
    const token = mintVerdictToken({ ...claims, keyringPath: keyring });
    fs.appendFileSync(path.join(repoRoot, missionRel), '\n# tamper\n');
    const runtime = createGantryRuntime({
      forwardTrigger: async () => ({ ok: true }),
      verdictKeyringPath: keyring,
    });
    await runtime.onVerifyPassed({
      repo_root: repoRoot,
      msn_id: msnId,
      mission_rel_path: missionRel,
    });
    await assert.rejects(
      () =>
        runtime.middleware({
          function_id: 'src::promote',
          payload: {},
          context: { msn_id: msnId, worktree_path: repoRoot, verdict_token: token },
        }),
      (err) => err instanceof GantryDenied && err.code === 'VERDICT_TOKEN_INVALID',
      'mission edit after mint should invalidate verdict token',
    );
  });
});

test('onVerifyPassed throws when lease store is corrupted', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-vp-corrupt-'));
  const { missionRel, msnId } = writeMiniGantryRepo(repoRoot);
  const storePath = path.join(repoRoot, '.gitagent', 'leases.json');
  fs.mkdirSync(path.dirname(storePath), { recursive: true });
  fs.writeFileSync(storePath, '{"leases": null}');
  const runtime = createGantryRuntime({
    forwardTrigger: async () => ({ ok: true }),
  });
  await assert.rejects(
    () =>
      runtime.onVerifyPassed({
        repo_root: repoRoot,
        msn_id: msnId,
        mission_rel_path: missionRel,
      }),
    (err) => err instanceof GantryDenied && err.code === 'LEASE_STORE_CORRUPTED',
    'corrupted lease store should fail verify bind',
  );
});

test('missing org config denies promote', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-vp-org-'));
  const krDir = fs.mkdtempSync(path.join(os.tmpdir(), 'og-vp-kr3-'));
  const { missionRel, msnId } = writeMiniGantryRepo(repoRoot);
  const claims = verdictClaimsFor(repoRoot, missionRel);
  await withKeyring(krDir, async (keyring) => {
    const token = mintVerdictToken({ ...claims, keyringPath: keyring });
    fs.unlinkSync(path.join(repoRoot, '.gitagent/foreman/ORG.export.local'));
    delete process.env.GANTRY_ORG_ID;
    const runtime = createGantryRuntime({
      forwardTrigger: async () => ({ ok: true }),
      verdictKeyringPath: keyring,
    });
    await runtime.onVerifyPassed({
      repo_root: repoRoot,
      msn_id: msnId,
      mission_rel_path: missionRel,
    });
    await assert.rejects(
      () =>
        runtime.middleware({
          function_id: 'src::promote',
          payload: {},
          context: { msn_id: msnId, worktree_path: repoRoot, verdict_token: token },
        }),
      (err) => err instanceof GantryDenied && err.code === 'ORG_ID_MISSING',
      'missing org config should deny promote',
    );
  });
});
