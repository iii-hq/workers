/**
 * Required runtime order for promote-class calls: verify pass → bind lease → promote.
 */
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { mintVerdictToken, verdictClaimsFor } from '@jeger-ai/opengantry/kernel';

import { VerifyCoalescer } from './src/verify.js';
import { createMiddlewareHandler } from './src/middleware.js';
import { LeaseStore } from './src/lease-store.js';
import { defaultLeaseStorePath } from './src/repo-path.js';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const OPENGANTRY_ROOT = path.resolve(__dirname, '../../opengantry');

const N = 50;
const latencies = [];

function writeMiniRepo(repoRoot) {
  const schemaSrc = path.join(OPENGANTRY_ROOT, '.gitagent/planner/MISSION.schema.yaml');
  fs.mkdirSync(path.join(repoRoot, '.gitagent/planner'), { recursive: true });
  fs.copyFileSync(schemaSrc, path.join(repoRoot, '.gitagent/planner/MISSION.schema.yaml'));
  fs.mkdirSync(path.join(repoRoot, '.gitagent/foreman'), { recursive: true });
  fs.mkdirSync(path.join(repoRoot, '.gitagent/missions'), { recursive: true });
  const missionRel = '.gitagent/missions/MSN-9001.yaml';
  fs.writeFileSync(
    path.join(repoRoot, missionRel),
    'msn_id: MSN-9001\nskill_key: gantry\ngate_command: npm test\ntrace_rows: []\n',
  );
  fs.writeFileSync(
    path.join(repoRoot, '.gitagent/foreman/MANIFEST.json'),
    JSON.stringify({
      schema_version: '0.5.0',
      skills: {
        gantry: {
          desc: 't',
          trust_threshold: 'Tier-2',
          tmvc_roots: ['src/'],
          forbidden_zones: [],
          gate_commands: ['npm test'],
        },
      },
      path_risks: {},
      risk_keywords: [],
      perimeter_protected: [],
    }),
  );
  fs.writeFileSync(
    path.join(repoRoot, '.gitagent/foreman/ORG.export.local'),
    JSON.stringify({ org_id: 'load-org' }),
    { mode: 0o600 },
  );
  return missionRel;
}

async function runLoad() {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-load-repo-'));
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'og-load-'));
  const keyring = path.join(dir, 'pepper-keyring.json');
  fs.writeFileSync(
    keyring,
    JSON.stringify([{ org_id: 'load-org', pepper_version: 1, pepper: 'load-pepper' }]),
    { mode: 0o600 },
  );
  const missionRel = writeMiniRepo(repoRoot);
  const expected = verdictClaimsFor(repoRoot, missionRel);
  const token = mintVerdictToken({ ...expected, keyringPath: keyring });
  const prevKeyring = process.env.GANTRY_VERDICT_KEYRING;
  process.env.GANTRY_VERDICT_KEYRING = keyring;
  const storePath = defaultLeaseStorePath(repoRoot);
  const leases = new LeaseStore(storePath);
  leases.bindMissionRel('MSN-9001', missionRel);
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
        function_id: i % 5 === 0 ? 'src::promote' : `src/work-${i}`,
        payload: { i },
        context:
          i % 5 === 0
            ? {
                msn_id: 'MSN-9001',
                worktree_path: repoRoot,
                verdict_token: token,
              }
            : { msn_id: 'MSN-9001', worktree_path: repoRoot, mission_rel_path: missionRel },
      });
      latencies.push(performance.now() - start);
      assert.equal(result.ok, true);
    });

    await Promise.all(tasks.map((t) => t()));

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
