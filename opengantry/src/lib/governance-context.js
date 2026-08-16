import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);

function loadBundle(repoRoot, missionRelPath) {
  try {
    const kernel = require('@jeger-ai/opengantry/kernel');
    if (typeof kernel.loadGovernanceBundle === 'function') {
      return kernel.loadGovernanceBundle(repoRoot, missionRelPath);
    }
  } catch {
    /* published kernel may not export loadGovernanceBundle yet */
  }
  const { loadManifest } = require('@jeger-ai/opengantry/dist/cli/lib/manifest.js');
  const { parseMissionFile } = require('@jeger-ai/opengantry/dist/cli/lib/missions/parser.js');
  return {
    manifest: loadManifest(repoRoot),
    mission: parseMissionFile(repoRoot, missionRelPath),
  };
}

export function governanceCacheKey(repoRoot, missionRel) {
  return `${repoRoot}\0${missionRel}`;
}

export function getGovernanceBundle(state, repoRoot, missionRel) {
  state.governance ??= new Map();
  const key = governanceCacheKey(repoRoot, missionRel);
  if (!state.governance.has(key)) {
    state.governance.set(key, loadBundle(repoRoot, missionRel));
  }
  return state.governance.get(key);
}
