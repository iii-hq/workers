import { evaluateFunctionScope, isPromoteClassFunctionId } from '@jeger-ai/opengantry/kernel';

import { BoundedMap } from './bounded-map.js';
import { GantryDenied } from './denied.js';
import { getGovernanceBundle } from './governance-context.js';
import { LEASE_STATES, LeaseStore } from './lease-store.js';
import { defaultLeaseStorePath, resolveRepoRootFromContext } from './repo-path.js';
import { verifyPromoteVerdictToken } from './verdict-bind.js';

const RESERVED_PREFIXES = ['gantry::', 'opengantry::'];
const RESERVED_SUFFIXES = ['::verify', '::attest', '::promote'];

export function isReservedGovernanceFunctionId(functionId, sessionIsControlPlane = false) {
  if (sessionIsControlPlane) return false;
  const id = functionId.toLowerCase();
  if (RESERVED_PREFIXES.some((p) => id.startsWith(p))) return true;
  if (RESERVED_SUFFIXES.some((s) => id.endsWith(s))) return true;
  return false;
}

export function getLeaseStore(state, repoRoot) {
  state.leaseStores ??= new BoundedMap(32);
  if (!state.leaseStores.has(repoRoot)) {
    state.leaseStores.set(repoRoot, new LeaseStore(defaultLeaseStorePath(repoRoot)));
  }
  return state.leaseStores.get(repoRoot);
}

function ensureLease(leases, msnId, worktreePath, missionRel) {
  const existing = leases.get(msnId);
  if (!existing) {
    leases.upsert({
      msn_id: msnId,
      branch: worktreePath ?? `gxt/${msnId.toLowerCase()}`,
      state: LEASE_STATES.active,
      session_refs: Object.create(null),
      mission_rel: missionRel,
    });
  } else if (missionRel && !existing.mission_rel) {
    leases.bindMissionRel(msnId, missionRel);
  }
}

export function createMiddlewareHandler(state) {
  return async function gantryMiddleware(input) {
    const { function_id, payload, context } = input;

    const repoRoot = resolveRepoRootFromContext(context);
    const leases = getLeaseStore(state, repoRoot);
    if (leases.corrupted) {
      throw new GantryDenied(
        'LEASE_STORE_CORRUPTED',
        'lease store corrupted; repair .gitagent/leases.json before promote',
      );
    }

    const msnId = context?.msn_id;
    const holderId = context?.holder_id;
    const missionRel = context?.mission_rel_path ?? context?.mission_rel;

    if (msnId && holderId) {
      ensureLease(leases, msnId, context?.worktree_path ?? context?.repo_root, missionRel);
      leases.acquireSession(msnId, holderId);
    }

    const lease = msnId ? leases.get(msnId) : null;

    if (lease?.state === LEASE_STATES.dirty_rewritten && isPromoteClassFunctionId(function_id)) {
      throw new GantryDenied('LINEAGE_DIRTY', 'lineage dirty; re-verify required');
    }

    if (isPromoteClassFunctionId(function_id)) {
      const token = context?.verdict_token ?? payload?.verdict_token;
      verifyPromoteVerdictToken({
        token,
        msnId,
        repoRoot,
        missionRel: lease?.mission_rel ?? missionRel,
      });
      if (msnId) {
        leases.transition(msnId, LEASE_STATES.active, LEASE_STATES.promoting);
      }
    }

    const boundMissionRel = lease?.mission_rel ?? missionRel;
    if (msnId && boundMissionRel) {
      try {
        const { manifest, mission } = getGovernanceBundle(state, repoRoot, boundMissionRel);
        const scope = evaluateFunctionScope(manifest, mission, function_id);
        if (!scope.ok) {
          throw new GantryDenied('SCOPE_VIOLATION', scope.message ?? 'scope violation');
        }
      } catch (e) {
        if (e instanceof GantryDenied) throw e;
        throw new GantryDenied('SCOPE_LOAD_FAILED', `mission scope load failed: ${e.message}`);
      }
    }

    const result = await state.forwardTrigger(function_id, payload);
    if (msnId) {
      leases.transition(msnId, LEASE_STATES.promoting, LEASE_STATES.active);
    }
    return result;
  };
}
