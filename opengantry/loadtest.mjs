/**
 * Required runtime order for promote-class calls: verify pass → bind lease → promote.
 */
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { mintVerdictToken, verdictClaimsFor } from '@jeger-ai/opengantry/kernel';

import { VerifyCoalescer } from './src/verify.js';
import { createMiddlewareHandler } from './src/middleware.js';
import { LeaseStore } from './src/lease-store.js';
import { defaultLeaseStorePath } from './src/repo-path.js';
import { writeKeyring, writeMiniGantryRepo } from './tests/helpers/mini-repo.mjs';

const N = 50;
const latencies = [];

async function runLoad() {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-load-repo-'));
  const krDir = fs.mkdtempSync(path.join(os.tmpdir(), 'og-load-'));
  const keyring = writeKeyring(krDir, { orgId: 'load-org', pepper: 'load-pepper' });
  const { missionRel, msnId } = writeMiniGantryRepo(repoRoot, {
    msnId: 'MSN-9001',
    missionRel: '.gitagent/missions/MSN-9001.yaml',
    orgId: 'load-org',
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
    forwardTrigger: async (fid) => ({ ok: true, fid }),
  };
  const middleware = createMiddlewareHandler(state);
  const coalescer = new VerifyCoalescer();

  try {
    const tasks = Array.from({ length: N }, (_, i) => async () => {
      const start = performance.now();
      const result = await middleware({
        function_id: `src::work-${i}`,
        payload: { i },
        context: { msn_id: msnId, worktree_path: repoRoot, mission_rel_path: missionRel },
      });
      latencies.push(performance.now() - start);
      assert.equal(result.ok, true);
    });

    await Promise.all(tasks.map((t) => t()));

    const promoteStart = performance.now();
    const promoteResult = await middleware({
      function_id: 'src::promote',
      payload: {},
      context: {
        msn_id: msnId,
        worktree_path: repoRoot,
        verdict_token: token,
      },
    });
    latencies.push(performance.now() - promoteStart);
    assert.equal(promoteResult.ok, true);

    let verifyRuns = 0;
    await Promise.all(
      Array.from({ length: 10 }, () =>
        coalescer.run('load-key', async () => {
          verifyRuns += 1;
          await new Promise((r) => setTimeout(r, 30));
          return { status: 'passed' };
        }),
      ),
    );
    assert.equal(verifyRuns, 1);

    const sorted = [...latencies].sort((a, b) => a - b);
    const p99 = sorted[Math.floor(sorted.length * 0.99)];
    assert.ok(p99 < 500, `p99 latency ${p99}ms too high`);
    console.log(
      `loadtest: ${N} middleware invocations, p99=${p99.toFixed(2)}ms, verify coalesced to 1 run`,
    );
  } finally {
    if (prevKeyring === undefined) delete process.env.GANTRY_VERDICT_KEYRING;
    else process.env.GANTRY_VERDICT_KEYRING = prevKeyring;
  }
}

runLoad().catch((err) => {
  console.error(err);
  process.exit(1);
});
