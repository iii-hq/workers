import path from 'node:path';

import { verifyVerdictToken, verdictClaimsFor } from '@jeger-ai/opengantry/kernel';

import { GantryDenied } from './denied.js';

export function defaultVerdictKeyringPath(repoRoot) {
  const override = process.env.GANTRY_VERDICT_KEYRING?.trim();
  if (override) return override;
  return path.join(repoRoot, '.config/gantry/pepper-keyring.json');
}

function mapClaimsError(e) {
  if (e && typeof e === 'object' && 'code' in e) {
    const code = String(e.code);
    if (code === 'ORG_ID_MISSING' || code === 'ORG_EXPORT_CONFIG_MISSING') {
      throw new GantryDenied('ORG_ID_MISSING', e.message ?? 'org_id not configured');
    }
    if (code === 'MISSION_NO_GATE') {
      throw new GantryDenied('MISSION_NO_GATE', e.message ?? 'mission has no gate');
    }
    if (code === 'MISSION_MSN_MISSING') {
      throw new GantryDenied('MISSION_MSN_MISSING', e.message ?? 'mission msn_id missing');
    }
  }
  if (e && typeof e === 'object' && 'code' in e && e.code === 'ENOENT') {
    throw new GantryDenied('MISSION_NOT_FOUND', 'mission file not found');
  }
  const msg = e instanceof Error ? e.message : String(e);
  if (msg.includes('GXT_MISSION_SCHEMA_INVALID') || msg.includes('schema validation failed')) {
    throw new GantryDenied('MISSION_SCHEMA_INVALID', msg);
  }
  if (msg.includes('missing MISSION schema')) {
    throw new GantryDenied('MISSION_SCHEMA_MISSING', msg);
  }
  throw new GantryDenied('VERDICT_CLAIMS_FAILED', msg);
}

/** Recompute claims at promote time and verify token — throws GantryDenied on failure. */
export function verifyPromoteVerdictToken({ token, msnId, repoRoot, missionRel }) {
  if (!token) {
    throw new GantryDenied('VERDICT_TOKEN_MISSING', 'promote refused: verdict token required');
  }
  if (!missionRel) {
    throw new GantryDenied(
      'MISSION_REL_MISSING',
      'promote refused: no mission bound on lease; run gantry::verify first',
    );
  }
  let expected;
  try {
    expected = verdictClaimsFor(repoRoot, missionRel);
  } catch (e) {
    mapClaimsError(e);
  }
  if (msnId && expected.msn_id !== msnId) {
    throw new GantryDenied('MSN_MISMATCH', `token msn_id does not match context ${msnId}`);
  }
  const ok = verifyVerdictToken({
    token,
    expected,
    keyringPath: defaultVerdictKeyringPath(repoRoot),
  });
  if (!ok) {
    throw new GantryDenied(
      'VERDICT_TOKEN_INVALID',
      'promote refused: verdict token does not match current mission revision',
    );
  }
  return true;
}
