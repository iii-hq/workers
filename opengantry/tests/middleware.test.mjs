import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';

import { mintVerdictToken, verdictClaimsFor } from '@jeger-ai/opengantry/kernel';

import { GantryDenied } from '../src/denied.js';
import { LEASE_STATES } from '../src/lease-store.js';
import { resolveLeaseStorePath } from '../src/repo-path.js';
import { createGantryRuntime } from '../src/runtime.js';
import { writeMiniGantryRepo } from './helpers/mini-repo.mjs';
import { withKeyring } from './helpers/with-keyring.mjs';

test('middleware throws on promote without mission binding', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-'));
  const runtime = createGantryRuntime({
    forwardTrigger: async (fid, payload) => ({ ok: true, fid, payload }),
  });
  await assert.rejects(
    () =>
      runtime.middleware({
        function_id: 'src::promote',
        payload: {},
        context: { msn_id: 'MSN-0175', worktree_path: repoRoot },
      }),
    (err) => err instanceof GantryDenied && err.code === 'VERDICT_TOKEN_MISSING',
    'promote without verdict token should be denied',
  );
});

test('middleware throws MSN_ID_MISSING for promote-class calls without msn_id', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-msn-'));
  const runtime = createGantryRuntime({
    forwardTrigger: async () => ({ ok: true }),
  });
  await assert.rejects(
    () =>
      runtime.middleware({
        function_id: 'src::promote',
        payload: { verdict_token: 'token' },
        context: { worktree_path: repoRoot, verdict_token: 'token' },
      }),
    (err) => err instanceof GantryDenied && err.code === 'MSN_ID_MISSING',
    'promote without msn_id should be denied',
  );
});

test('middleware throws when token does not match current mission revision', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-stale-'));
  const krDir = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-kr-'));
  const { missionRel, msnId } = writeMiniGantryRepo(repoRoot);
  const claims = verdictClaimsFor(repoRoot, missionRel);
  await withKeyring(krDir, async (keyring) => {
    const token = mintVerdictToken({ ...claims, keyringPath: keyring });
    fs.appendFileSync(path.join(repoRoot, missionRel), '\n# edited\n');
    const runtime = createGantryRuntime({
      forwardTrigger: async () => ({ ok: true }),
      verdictKeyringPath: keyring,
    });
    const leases = runtime.leaseStoreFor(repoRoot);
    await leases.bindMissionRel(msnId, missionRel);
    await assert.rejects(
      () =>
        runtime.middleware({
          function_id: 'src::promote',
          payload: {},
          context: { msn_id: msnId, worktree_path: repoRoot, verdict_token: token },
        }),
      (err) => err instanceof GantryDenied && err.code === 'VERDICT_TOKEN_INVALID',
      'stale verdict token should be rejected after mission edit',
    );
  });
});

test('middleware allows promote with valid token and mission_rel', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-ok-'));
  const krDir = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-kr2-'));
  const { missionRel, msnId } = writeMiniGantryRepo(repoRoot);
  const claims = verdictClaimsFor(repoRoot, missionRel);
  await withKeyring(krDir, async (keyring) => {
    const token = mintVerdictToken({ ...claims, keyringPath: keyring });
    const runtime = createGantryRuntime({
      forwardTrigger: async (fid, payload) => ({ ok: true, fid, payload }),
      verdictKeyringPath: keyring,
    });
    const leases = runtime.leaseStoreFor(repoRoot);
    await leases.bindMissionRel(msnId, missionRel);
    const result = await runtime.middleware({
      function_id: 'src::promote',
      payload: { branch: 'main' },
      context: { msn_id: msnId, worktree_path: repoRoot, verdict_token: token },
    });
    assert.equal(result.ok, true, 'valid verdict token should allow promote');
  });
});

test('middleware throws on corrupted lease store', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-corrupt-'));
  const storePath = resolveLeaseStorePath(repoRoot);
  fs.mkdirSync(path.dirname(storePath), { recursive: true });
  fs.writeFileSync(storePath, '{"leases": null}');
  const runtime = createGantryRuntime({
    forwardTrigger: async () => ({ ok: true }),
  });
  await assert.rejects(
    () =>
      runtime.middleware({
        function_id: 'math::add',
        payload: {},
        context: { msn_id: 'MSN-1', worktree_path: repoRoot, holder_id: 'h1' },
      }),
    (err) => err instanceof GantryDenied && err.code === 'LEASE_STORE_CORRUPTED',
    'corrupted lease store should deny middleware',
  );
});

test('middleware denies promote when lease is tombstoned', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-tomb-'));
  const krDir = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-tomb-kr-'));
  const { missionRel, msnId } = writeMiniGantryRepo(repoRoot);
  const claims = verdictClaimsFor(repoRoot, missionRel);
  await withKeyring(krDir, async (keyring) => {
    const token = mintVerdictToken({ ...claims, keyringPath: keyring });
    const runtime = createGantryRuntime({
      forwardTrigger: async () => ({ ok: true }),
      verdictKeyringPath: keyring,
    });
    const leases = runtime.leaseStoreFor(repoRoot);
    await leases.bindMissionRel(msnId, missionRel);
    await leases.upsert({
      msn_id: msnId,
      state: LEASE_STATES.tombstoned,
      session_refs: Object.create(null),
      mission_rel: missionRel,
    });
    await assert.rejects(
      () =>
        runtime.middleware({
          function_id: 'src::promote',
          payload: {},
          context: { msn_id: msnId, worktree_path: repoRoot, verdict_token: token },
        }),
      (err) => err instanceof GantryDenied && err.code === 'LEASE_NOT_PROMOTABLE',
      'tombstoned lease should block promote',
    );
  });
});

test('middleware denies promote when lineage is dirty', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-dirty-'));
  const krDir = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-dirty-kr-'));
  const { missionRel, msnId } = writeMiniGantryRepo(repoRoot);
  const claims = verdictClaimsFor(repoRoot, missionRel);
  await withKeyring(krDir, async (keyring) => {
    const token = mintVerdictToken({ ...claims, keyringPath: keyring });
    const runtime = createGantryRuntime({
      forwardTrigger: async () => ({ ok: true }),
      verdictKeyringPath: keyring,
    });
    const leases = runtime.leaseStoreFor(repoRoot);
    await leases.upsert({
      msn_id: msnId,
      branch: `gxt/${msnId.toLowerCase()}`,
      state: LEASE_STATES.dirty_rewritten,
      session_refs: Object.create(null),
      mission_rel: missionRel,
    });
    await assert.rejects(
      () =>
        runtime.middleware({
          function_id: 'src::promote',
          payload: {},
          context: { msn_id: msnId, worktree_path: repoRoot, verdict_token: token },
        }),
      (err) => err instanceof GantryDenied && err.code === 'LINEAGE_DIRTY',
      'dirty lineage should block promote',
    );
  });
});

test('verify pass recovers dirty lineage so promote succeeds', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-recover-'));
  const krDir = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-recover-kr-'));
  const { missionRel, msnId } = writeMiniGantryRepo(repoRoot);
  const claims = verdictClaimsFor(repoRoot, missionRel);
  await withKeyring(krDir, async (keyring) => {
    const token = mintVerdictToken({ ...claims, keyringPath: keyring });
    const runtime = createGantryRuntime({
      forwardTrigger: async () => ({ ok: true }),
      verdictKeyringPath: keyring,
    });
    const leases = runtime.leaseStoreFor(repoRoot);
    await leases.upsert({
      msn_id: msnId,
      branch: `gxt/${msnId.toLowerCase()}`,
      state: LEASE_STATES.dirty_rewritten,
      session_refs: Object.create(null),
      mission_rel: missionRel,
    });
    await runtime.onVerifyPassed({
      repo_root: repoRoot,
      msn_id: msnId,
      mission_rel_path: missionRel,
    });
    assert.equal(
      leases.get(msnId)?.state,
      LEASE_STATES.active,
      'verify pass should recover dirty lease to active',
    );
    const result = await runtime.middleware({
      function_id: 'src::promote',
      payload: {},
      context: { msn_id: msnId, worktree_path: repoRoot, verdict_token: token },
    });
    assert.equal(result.ok, true, 'promote should succeed after verify recovery');
  });
});

test('middleware denies scope violation outside TMVC roots', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-scope-'));
  const krDir = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-scope-kr-'));
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
    await assert.rejects(
      () =>
        runtime.middleware({
          function_id: 'outside::promote',
          payload: {},
          context: { msn_id: msnId, worktree_path: repoRoot, verdict_token: token },
        }),
      (err) => err instanceof GantryDenied && err.code === 'SCOPE_VIOLATION',
      'function outside TMVC roots should be denied',
    );
  });
});

test('middleware denies when mission scope load fails', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-scope-load-'));
  const krDir = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-scope-load-kr-'));
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
    fs.writeFileSync(path.join(repoRoot, '.gitagent/foreman/MANIFEST.json'), 'not-json');
    await assert.rejects(
      () =>
        runtime.middleware({
          function_id: 'src::promote',
          payload: {},
          context: { msn_id: msnId, worktree_path: repoRoot, verdict_token: token },
        }),
      (err) => err instanceof GantryDenied && err.code === 'SCOPE_LOAD_FAILED',
      'corrupt manifest should fail scope load',
    );
  });
});

test('middleware restores lease after forward failure and allows retry', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-forward-fail-'));
  const krDir = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-forward-fail-kr-'));
  const { missionRel, msnId } = writeMiniGantryRepo(repoRoot);
  const claims = verdictClaimsFor(repoRoot, missionRel);
  await withKeyring(krDir, async (keyring) => {
    const token = mintVerdictToken({ ...claims, keyringPath: keyring });
    let calls = 0;
    const runtime = createGantryRuntime({
      forwardTrigger: async () => {
        calls += 1;
        if (calls === 1) throw new Error('deploy failed');
        return { ok: true };
      },
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
      (err) => err instanceof Error && err.message === 'deploy failed',
      'first forward failure should propagate',
    );
    const leases = runtime.leaseStoreFor(repoRoot);
    assert.equal(
      leases.get(msnId)?.state,
      LEASE_STATES.active,
      'lease should restore to active after forward failure',
    );
    const result = await runtime.middleware({
      function_id: 'src::promote',
      payload: {},
      context: { msn_id: msnId, worktree_path: repoRoot, verdict_token: token },
    });
    assert.equal(result.ok, true, 'second promote should succeed after lease restore');
  });
});
