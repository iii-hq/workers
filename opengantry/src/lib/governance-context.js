import { loadGovernanceBundle } from '@jeger-ai/opengantry/kernel';

import { BoundedMap } from './bounded-map.js';

export function governanceCacheKey(repoRoot, missionRel) {
  return `${repoRoot}\0${missionRel}`;
}

export function getGovernanceBundle(state, repoRoot, missionRel) {
  state.governance ??= new BoundedMap(32);
  const key = governanceCacheKey(repoRoot, missionRel);
  if (!state.governance.has(key)) {
    state.governance.set(key, loadGovernanceBundle(repoRoot, missionRel));
  }
  return state.governance.get(key);
}
