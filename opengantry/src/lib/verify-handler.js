import { verifyMission } from '@jeger-ai/opengantry/kernel';

import { getGovernanceBundle } from './governance-context.js';
import { getLeaseStore } from './middleware.js';
import { resolveVerifyRepoRoot } from './repo-path.js';
import { runPracticesScan, practicesFailedPayload } from './iii-practices/scan.mjs';
import { VerifyCoalescer } from './verify-coalescer.js';
import { GantryDenied } from './denied.js';

function verifySaturatedPayload() {
  return {
    status: 'failed',
    error_code: 'GXT_VERIFY_SATURATED',
    findings: [
      {
        failed_gate: 'gate',
        resolution_hint: 'verify queue saturated; retry later',
      },
    ],
  };
}

export function onVerifyPassed(state, data) {
  const repoRoot = data?.repo_root;
  if (!data?.msn_id || !data?.mission_rel_path || !repoRoot) return;
  const resolved = resolveVerifyRepoRoot(repoRoot);
  const leases = getLeaseStore(state, resolved);
  if (!leases.corrupted) {
    leases.bindMissionRel(data.msn_id, data.mission_rel_path);
  }
  try {
    getGovernanceBundle(state, resolved, data.mission_rel_path);
  } catch {
    /* scope bind is best-effort; middleware surfaces load errors */
  }
}

export async function runVerify(data, { allowlistRoot, skipPracticesScan } = {}) {
  const repoRoot = resolveVerifyRepoRoot(data.repo_root);
  if (!skipPracticesScan) {
    const { findings, logs } = await runPracticesScan(repoRoot, { allowlistRoot });
    for (const line of logs) console.log(line);
    if (findings.length) return practicesFailedPayload(findings);
  }
  return verifyMission({
    repoRoot,
    missionRelPath: data.mission_rel_path,
    options: data.options ?? { skipStaleEvidence: true },
  });
}

export function createVerifyHandler(state, { allowlistRoot } = {}) {
  return async function gantryVerify(data) {
    const repoRoot = data?.repo_root;
    const key = JSON.stringify({
      repo_root: repoRoot ?? '',
      msn_id: data?.msn_id ?? '',
      mission_rel_path: data?.mission_rel_path ?? '',
      options: data?.options ?? null,
    });
    const coalescer = state.coalescer;
    const result = await coalescer.run(key, () => runVerify(data, { allowlistRoot }));
    if (result?.error_code === 'GXT_VERIFY_SATURATED') {
      return verifySaturatedPayload();
    }
    if (result?.status === 'passed' && data?.msn_id && data?.mission_rel_path && repoRoot) {
      onVerifyPassed(state, data);
    }
    return result;
  };
}
