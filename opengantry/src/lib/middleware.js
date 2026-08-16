import {
  evaluateFunctionScope,
  isPromoteClassFunctionId,
  verifyVerdictToken,
} from '@jeger-ai/opengantry/kernel';

import { isBypassMode } from './bypass.js';
import { LEASE_STATES, LeaseStore } from './lease-store.js';
import { defaultLeaseStorePath, resolveRepoRootFromContext } from './repo-path.js';

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

function ensureLease(leases, msnId, worktreePath) {
  let lease = leases.get(msnId);
  if (!lease) {
    lease = {
      msn_id: msnId,
      branch: worktreePath ?? `gxt/${msnId.toLowerCase()}`,
      state: LEASE_STATES.active,
      session_refs: {},
    };
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
    const msnId = context?.msn_id;
    const holderId = context?.holder_id;

    if (msnId && holderId) {
      ensureLease(leases, msnId, context?.worktree_path ?? context?.repo_root);
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
      const expected = context?.verdict_expected ?? payload?.verdict_expected;
      const keyringPath = context?.verdict_keyring_path ?? payload?.verdict_keyring_path;
      if (
        !token ||
        !expected ||
        !verifyVerdictToken({
          token,
          expected,
          keyringPath,
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

    if (msnId && lease?.mission_rel && state.manifest && state.mission) {
      const scope = evaluateFunctionScope(state.manifest, state.mission, function_id);
      if (!scope.ok) {
        return {
          status: 'failed',
          findings: [
            { failed_gate: 'defensive', resolution_hint: scope.message ?? 'scope violation' },
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
