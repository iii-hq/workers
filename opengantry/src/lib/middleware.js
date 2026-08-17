import { evaluateFunctionScope, isPromoteClassFunctionId } from '@jeger-ai/opengantry/kernel';

import { isBypassMode } from './bypass.js';
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

function getLeaseStore(state, repoRoot) {
  state.leaseStores ??= new Map();
  if (!state.leaseStores.has(repoRoot)) {
    state.leaseStores.set(repoRoot, new LeaseStore(defaultLeaseStorePath(repoRoot)));
  }
  return state.leaseStores.get(repoRoot);
}

function ensureLease(leases, msnId, worktreePath, missionRel) {
  let lease = leases.get(msnId);
  if (!lease) {
    lease = {
      msn_id: msnId,
      branch: worktreePath ?? `gxt/${msnId.toLowerCase()}`,
      state: LEASE_STATES.active,
      session_refs: Object.create(null),
      mission_rel: missionRel,
    };
    leases.upsert(lease);
  } else if (missionRel && !lease.mission_rel) {
    lease.mission_rel = missionRel;
    leases.upsert(lease);
  }
  return lease;
}

export function createMiddlewareHandler(state) {
  return async function gantryMiddleware(input) {
    const { function_id, payload, context } = input;

    if (isBypassMode()) {
      return state.forwardTrigger(function_id, payload);
    }

    const repoRoot = resolveRepoRootFromContext(context);
    const leases = getLeaseStore(state, repoRoot);
    if (leases.corrupted) {
      return {
        status: 'failed',
        findings: [
          {
            failed_gate: 'gate',
            resolution_hint: 'lease store corrupted; repair .gitagent/leases.json before promote',
          },
        ],
      };
    }

    const msnId = context?.msn_id;
    const holderId = context?.holder_id;
    const missionRel = context?.mission_rel_path ?? context?.mission_rel;

    if (msnId && holderId) {
      ensureLease(leases, msnId, context?.worktree_path ?? context?.repo_root, missionRel);
      leases.acquireSession(msnId, holderId);
    }

    const lease = msnId ? leases.get(msnId) : null;

    if (lease?.state === 'dirty_rewritten' && isPromoteClassFunctionId(function_id)) {
      return {
        status: 'failed',
        findings: [{ failed_gate: 'gate', resolution_hint: 'lineage dirty; re-verify required' }],
      };
    }

    if (isPromoteClassFunctionId(function_id)) {
      const token = context?.verdict_token ?? payload?.verdict_token;
      if (
        !token ||
        !verifyPromoteVerdictToken({
          token,
          msnId,
          repoRoot,
          boundExpected: lease?.verdict_expected,
        })
      ) {
        return {
          status: 'failed',
          findings: [
            { failed_gate: 'gate', resolution_hint: 'promote refused: no valid verdict token' },
          ],
        };
      }
      if (lease) {
        lease.state = LEASE_STATES.promoting;
        leases.upsert(lease);
      }
    }

    const boundMissionRel = lease?.mission_rel ?? missionRel;
    if (msnId && boundMissionRel) {
      try {
        const { manifest, mission } = getGovernanceBundle(state, repoRoot, boundMissionRel);
        const scope = evaluateFunctionScope(manifest, mission, function_id);
        if (!scope.ok) {
          return {
            status: 'failed',
            findings: [
              { failed_gate: 'defensive', resolution_hint: scope.message ?? 'scope violation' },
            ],
          };
        }
      } catch (e) {
        return {
          status: 'failed',
          findings: [
            {
              failed_gate: 'defensive',
              resolution_hint: `mission scope load failed: ${e.message}`,
            },
          ],
        };
      }
    }

    const result = await state.forwardTrigger(function_id, payload);
    if (lease?.state === LEASE_STATES.promoting) {
      lease.state = LEASE_STATES.active;
      leases.upsert(lease);
    }
    return result;
  };
}

export { defaultLeaseStorePath, resolveRepoRootFromContext };
