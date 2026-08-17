import fs from 'node:fs';
import path from 'node:path';

import { loadGovernanceBundle } from '@jeger-ai/opengantry/kernel';

import { LeaseStore } from './lease-store.js';
import { defaultLeaseStorePath } from './repo-path.js';

/** Size-capped Map: evicts oldest entry when at capacity. */
export class BoundedMap {
  constructor(maxSize = 32) {
    this.maxSize = maxSize;
    this.map = new Map();
  }

  get(key) {
    const v = this.map.get(key);
    if (v !== undefined) {
      this.map.delete(key);
      this.map.set(key, v);
    }
    return v;
  }

  set(key, value) {
    if (this.map.has(key)) this.map.delete(key);
    this.map.set(key, value);
    while (this.map.size > this.maxSize) {
      const oldest = this.map.keys().next().value;
      this.map.delete(oldest);
    }
  }

  has(key) {
    return this.map.has(key);
  }
}

function governanceRevision(repoRoot, missionRel) {
  const paths = [
    path.join(repoRoot, missionRel),
    path.join(repoRoot, '.gitagent/foreman/MANIFEST.json'),
  ];
  return paths
    .map((p) => {
      try {
        const stat = fs.statSync(p);
        return `${p}:${stat.mtimeMs}:${stat.size}`;
      } catch {
        return `${p}:missing`;
      }
    })
    .join('|');
}

export function getGovernanceBundle(deps, repoRoot, missionRel) {
  const key = `${repoRoot}\0${missionRel}`;
  const rev = governanceRevision(repoRoot, missionRel);
  const cached = deps.governance.get(key);
  if (cached?.rev === rev) {
    return cached.bundle;
  }
  const bundle = loadGovernanceBundle(repoRoot, missionRel);
  deps.governance.set(key, { rev, bundle });
  return bundle;
}

export function getLeaseStore(deps, repoRoot) {
  if (!deps.leaseStores.has(repoRoot)) {
    deps.leaseStores.set(repoRoot, new LeaseStore(defaultLeaseStorePath(repoRoot)));
  }
  return deps.leaseStores.get(repoRoot);
}
