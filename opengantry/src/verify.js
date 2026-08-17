import { verifyMission } from '@jeger-ai/opengantry/kernel';

import { GantryDenied } from './denied.js';
import { getGovernanceBundle, getLeaseStore } from './middleware.js';
import { resolveVerifyRepoRoot } from './repo-path.js';

/** Single-flight verify coalescing keyed by caller-supplied cache key. */
export class VerifyCoalescer {
  constructor() {
    this.inFlight = new Map();
    this.maxQueue = 32;
  }

  async run(key, fn) {
    if (this.inFlight.has(key)) {
      return this.inFlight.get(key);
    }
    if (this.inFlight.size >= this.maxQueue) {
      return {
        status: 'failed',
        error_code: 'GXT_VERIFY_SATURATED',
      };
    }
    const promise = fn().finally(() => {
      this.inFlight.delete(key);
    });
    this.inFlight.set(key, promise);
    return promise;
  }
}

export function createWorkerState() {
  return {
    leaseStores: undefined,
    governance: undefined,
    coalescer: new VerifyCoalescer(),
    forwardTrigger: async (function_id, payload) => ({ ok: true, function_id, payload }),
  };
}

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

function verifyBindFailedPayload(hint) {
  return {
    status: 'failed',
    error_code: 'GXT_VERIFY_BIND_FAILED',
    findings: [
      {
        failed_gate: 'gate',
        resolution_hint: `verdict scope bind failed: ${hint}`,
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

export async function runVerify(data) {
  const repoRoot = resolveVerifyRepoRoot(data.repo_root);
  return verifyMission({
    repoRoot,
    missionRelPath: data.mission_rel_path,
    options: data.options ?? { skipStaleEvidence: true },
  });
}

export function createVerifyHandler(state) {
  return async function gantryVerify(data) {
    const repoRoot = data?.repo_root;
    const key = JSON.stringify({
      repo_root: repoRoot ?? '',
      msn_id: data?.msn_id ?? '',
      mission_rel_path: data?.mission_rel_path ?? '',
      options: data?.options ?? null,
    });
    const coalescer = state.coalescer;
    const result = await coalescer.run(key, () => runVerify(data));
    if (result?.error_code === 'GXT_VERIFY_SATURATED') {
      return verifySaturatedPayload();
    }
    if (result?.status === 'passed' && data?.msn_id && data?.mission_rel_path && repoRoot) {
      try {
        onVerifyPassed(state, data);
      } catch (e) {
        const hint =
          e instanceof GantryDenied ? e.hint : e instanceof Error ? e.message : String(e);
        return verifyBindFailedPayload(hint);
      }
    }
    return result;
  };
}
