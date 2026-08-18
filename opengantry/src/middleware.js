/**
 * Hot-path policy on the governed listener.
 *
 * Order: lease (if msn_id + holder_id) → dirty-lineage check → promote
 * verdict (promote-class only) → mission scope → forwardTrigger.
 * Throws GantryDenied on any failure. Missing token, corrupt store, or
 * rewritten mission never fall through to the target function.
 */
import { evaluateFunctionScope, isPromoteClassFunctionId } from '@jeger-ai/opengantry/kernel';

import { GantryDenied } from './denied.js';
import { LEASE_STATES } from './lease-store.js';
import { resolveRepoRootFromContext } from './repo-path.js';
import { getGovernanceBundle, getLeaseStore } from './stores.js';
import { verifyPromoteVerdictToken } from './verdict.js';

async function ensureLease(leases, msnId, worktreePath, missionRel) {
  const existing = leases.get(msnId);
  if (!existing) {
    await leases.upsert({
      msn_id: msnId,
      branch: worktreePath ?? `gxt/${msnId.toLowerCase()}`,
      state: LEASE_STATES.active,
      session_refs: Object.create(null),
      mission_rel: missionRel,
    });
  } else if (missionRel && !existing.mission_rel) {
    await leases.bindMissionRel(msnId, missionRel);
  }
}

export function createMiddlewareHandler(deps) {
  return async function gantryMiddleware(input) {
    const { function_id, payload, context } = input;

    const repoRoot = resolveRepoRootFromContext(context);
    const leases = getLeaseStore(deps, repoRoot);
    if (leases.corrupted) {
      throw new GantryDenied(
        'LEASE_STORE_CORRUPTED',
        'lease store corrupted; repair .gitagent/leases.json before promote',
      );
    }

    const msnId = context?.msn_id;
    const holderId = context?.holder_id;
    const missionRel = context?.mission_rel_path ?? context?.mission_rel;
    let sessionHeld = false;

    try {
      if (msnId && holderId) {
        await ensureLease(leases, msnId, context?.worktree_path ?? context?.repo_root, missionRel);
        await leases.acquireSession(msnId, holderId);
        sessionHeld = true;
      }

      const lease = msnId ? leases.get(msnId) : null;

      if (lease?.state === LEASE_STATES.dirty_rewritten && isPromoteClassFunctionId(function_id)) {
        throw new GantryDenied('LINEAGE_DIRTY', 'lineage dirty; re-verify required');
      }

      // Kernel classify: ::deploy / ::merge / ::publish / ::apply / ::push / ::promote.
      if (isPromoteClassFunctionId(function_id)) {
        const token = context?.verdict_token ?? payload?.verdict_token;
        verifyPromoteVerdictToken({
          token,
          msnId,
          repoRoot,
          missionRel: lease?.mission_rel ?? missionRel,
        });
        if (msnId) {
          const claimed = await leases.transition(
            msnId,
            LEASE_STATES.active,
            LEASE_STATES.promoting,
          );
          if (!claimed) {
            throw new GantryDenied(
              'LEASE_NOT_PROMOTABLE',
              'lease is not in active state; re-verify required',
            );
          }
        }
      }

      const boundMissionRel = lease?.mission_rel ?? missionRel;
      if (msnId && boundMissionRel) {
        try {
          const { manifest, mission } = getGovernanceBundle(deps, repoRoot, boundMissionRel);
          const scope = evaluateFunctionScope(manifest, mission, function_id);
          if (!scope.ok) {
            throw new GantryDenied('SCOPE_VIOLATION', scope.message ?? 'scope violation');
          }
        } catch (e) {
          if (e instanceof GantryDenied) throw e;
          throw new GantryDenied('SCOPE_LOAD_FAILED', `mission scope load failed: ${e.message}`);
        }
      }

      const result = await deps.forwardTrigger(function_id, payload);
      if (msnId) {
        await leases.transition(msnId, LEASE_STATES.promoting, LEASE_STATES.active);
      }
      return result;
    } finally {
      if (msnId && holderId && sessionHeld) {
        await leases.releaseSession(msnId, holderId);
      }
    }
  };
}
