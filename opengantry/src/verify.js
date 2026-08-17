import { verifyMission } from '@jeger-ai/opengantry/kernel';

import { GantryDenied } from './denied.js';
import { getGovernanceBundle, getLeaseStore } from './stores.js';
import { resolveVerifyRepoRoot } from './repo-path.js';

/** Thrown when verify coalescer is at maxInFlight capacity. */
export class VerifyCoalescerSaturationError extends Error {
  constructor() {
    super('verify queue saturated');
    this.name = 'VerifyCoalescerSaturationError';
    this.code = 'GXT_VERIFY_SATURATED';
  }
}

/** Single-flight verify coalescing keyed by caller-supplied cache key. */
export class VerifyCoalescer {
  constructor() {
    this.inFlight = new Map();
    this.maxInFlight = 32;
  }

  async run(key, fn) {
    if (this.inFlight.has(key)) {
      return this.inFlight.get(key);
    }
    if (this.inFlight.size >= this.maxInFlight) {
      throw new VerifyCoalescerSaturationError();
    }
    const promise = fn().finally(() => {
      this.inFlight.delete(key);
    });
    this.inFlight.set(key, promise);
    return promise;
  }
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

export async function onVerifyPassed(deps, data) {
  const resolved = resolveVerifyRepoRoot(data.repo_root);
  const leases = getLeaseStore(deps, resolved);
  if (leases.corrupted) {
    throw new GantryDenied(
      'LEASE_STORE_CORRUPTED',
      'lease store corrupted; repair .gitagent/leases.json before verify bind',
    );
  }
  const bound = await leases.bindMissionRel(data.msn_id, data.mission_rel_path);
  if (!bound) {
    throw new GantryDenied('LEASE_BIND_FAILED', 'failed to bind mission on lease store');
  }
  try {
    getGovernanceBundle(deps, resolved, data.mission_rel_path);
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

export function createVerifyHandler(deps) {
  return async function gantryVerify(data) {
    const repoRoot = data?.repo_root;
    const key = JSON.stringify({
      repo_root: repoRoot ?? '',
      msn_id: data?.msn_id ?? '',
      mission_rel_path: data?.mission_rel_path ?? '',
      options: data?.options ?? null,
    });
    let result;
    try {
      result = await deps.coalescer.run(key, () => runVerify(data));
    } catch (e) {
      if (e instanceof VerifyCoalescerSaturationError) {
        return verifySaturatedPayload();
      }
      throw e;
    }
    if (result?.status === 'passed' && data?.msn_id && data?.mission_rel_path && repoRoot) {
      try {
        await onVerifyPassed(deps, data);
      } catch (e) {
        const hint =
          e instanceof GantryDenied ? e.hint : e instanceof Error ? e.message : String(e);
        return verifyBindFailedPayload(hint);
      }
    }
    return result;
  };
}
