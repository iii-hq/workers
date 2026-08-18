/**
 * Repo resolution. No process.cwd() fallback: a sandboxed worker that
 * cannot see the host tree must fail rather than verify /workspace.
 * GANTRY_III_LEASE_STORE may move leases.json, but only under .gitagent/.
 */
import fs from 'node:fs';
import path from 'node:path';

export function hasGxtSubstrate(root) {
  return fs.existsSync(path.join(root, '.gitagent/foreman/MANIFEST.json'));
}

/** Resolve adopters' repo root from iii middleware context (no cwd fallback). */
export function resolveRepoRootFromContext(context) {
  const raw = context?.worktree_path ?? context?.repo_root;
  if (!raw || typeof raw !== 'string') {
    throw new Error('opengantry: context.worktree_path or context.repo_root required');
  }
  return path.resolve(raw);
}

/** Resolve repo root for gantry::verify (absolute path only). */
export function resolveVerifyRepoRoot(repoRoot) {
  if (!repoRoot || typeof repoRoot !== 'string') {
    throw new Error('gantry::verify: repo_root required (absolute path)');
  }
  if (!path.isAbsolute(repoRoot)) {
    throw new Error('gantry::verify: repo_root must be an absolute path');
  }
  if (!fs.existsSync(repoRoot)) {
    throw new Error(`gantry::verify: repo_root ${repoRoot} is not visible from this worker`);
  }
  if (!hasGxtSubstrate(repoRoot)) {
    throw new Error(
      `gantry::verify: missing .gitagent under ${repoRoot}. Run gantry init in that repo`,
    );
  }
  return repoRoot;
}

function ensureResolvedUnderGitagent(repoRoot, targetPath) {
  const root = path.resolve(repoRoot);
  const gitagentDir = path.join(root, '.gitagent');
  fs.mkdirSync(gitagentDir, { recursive: true });
  const realGitagent = fs.realpathSync(gitagentDir);
  const resolved = path.resolve(targetPath);
  const parent = path.dirname(resolved);
  fs.mkdirSync(parent, { recursive: true });
  const realParent = fs.realpathSync(parent);
  const rel = path.relative(realGitagent, realParent);
  if (rel.startsWith('..') || path.isAbsolute(rel)) {
    throw new Error('opengantry: GANTRY_III_LEASE_STORE must resolve under <repo>/.gitagent/');
  }
}

export function defaultLeaseStorePath(repoRoot) {
  const root = path.resolve(repoRoot);
  const defaultPath = path.join(root, '.gitagent', 'leases.json');
  const override = process.env.GANTRY_III_LEASE_STORE?.trim();
  if (!override) return defaultPath;
  const resolved = path.resolve(override);
  ensureResolvedUnderGitagent(root, resolved);
  return resolved;
}
