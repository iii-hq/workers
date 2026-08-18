/**
 * gantry::verify. Runs kernel verifyMission (the repo's gate_command),
 * not a second scanner. Identical in-flight calls share one run so a
 * stampede of agents does not spawn 32 gate processes.
 */
import { verifyMission } from '@jeger-ai/opengantry/kernel';

import { GantryDenied } from './denied.js';
import { LEASE_STATES } from './lease-store.js';
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

function verifyFailedPayload(code, hint) {
  return {
    status: 'failed',
    error_code: code,
    findings: [
      {
        failed_gate: 'gate',
        resolution_hint: hint,
      },
    ],
  };
}

async function recoverLeaseAfterVerify(leases, msnId) {
  const lease = leases.get(msnId);
  if (lease?.state === LEASE_STATES.tombstoned) {
    const recovered = await leases.transition(msnId, LEASE_STATES.tombstoned, LEASE_STATES.active);
    if (!recovered) {
      throw new GantryDenied('LEASE_RECOVERY_FAILED', 'failed to recover tombstoned lease');
    }
    return;
  }
  if (lease?.state === LEASE_STATES.dirty_rewritten) {
    const recovered = await leases.transition(
      msnId,
      LEASE_STATES.dirty_rewritten,
      LEASE_STATES.active,
    );
    if (!recovered) {
      throw new GantryDenied('LEASE_RECOVERY_FAILED', 'failed to recover dirty lease');
    }
  }
}

/** After a pass, pin the mission onto the lease so promote can recompute claims. */
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
  await recoverLeaseAfterVerify(leases, data.msn_id);
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
    options: { skipStaleEvidence: true, ...data.options },
  });
}

async function emitVerdictForResult(deps, data, result) {
  try {
    await deps.emitVerdict({
      status: result?.status ?? 'failed',
      error_code: result?.error_code ?? null,
      repo_root: data?.repo_root ?? null,
      msn_id: data?.msn_id ?? result?.msn_id ?? null,
      mission_rel_path: data?.mission_rel_path ?? result?.mission_file_path ?? null,
    });
  } catch {
    /* verify result is authoritative; fan-out is best-effort */
  }
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
      // Return a failed payload so the agent can retry. Throwing would look
      // like a worker crash rather than a full verify queue.
      if (e instanceof VerifyCoalescerSaturationError) {
        const saturated = verifyFailedPayload(
          'GXT_VERIFY_SATURATED',
          'verify queue saturated; retry later',
        );
        await emitVerdictForResult(deps, data, saturated);
        return saturated;
      }
      throw e;
    }
    if (result?.status === 'passed' && data?.msn_id && data?.mission_rel_path && repoRoot) {
      try {
        await onVerifyPassed(deps, data);
      } catch (e) {
        const hint =
          e instanceof GantryDenied ? e.hint : e instanceof Error ? e.message : String(e);
        const bindFailed = verifyFailedPayload(
          'GXT_VERIFY_BIND_FAILED',
          `verdict scope bind failed: ${hint}`,
        );
        await emitVerdictForResult(deps, data, bindFailed);
        return bindFailed;
      }
    }
    await emitVerdictForResult(deps, data, result);
    return result;
  };
}
