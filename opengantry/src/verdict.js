/**
 * Promote-time token check. Rebuild expected claims from the mission on
 * disk, then HMAC-verify the token against that. The token is not a
 * bearer of its own claims: a rewritten mission fails even if the agent
 * still holds the old token.
 */
import path from 'node:path';

import { verifyVerdictToken, verdictClaimsFor } from '@jeger-ai/opengantry/kernel';

import { GantryDenied } from './denied.js';

export function defaultVerdictKeyringPath(repoRoot) {
  const override = process.env.GANTRY_VERDICT_KEYRING?.trim();
  if (override) return override;
  return path.join(repoRoot, '.config/gantry/pepper-keyring.json');
}

/** Maps kernel throw sites to GantryDenied codes (kernel exports no error codes). */
const KERNEL_CODE_TO_DENIED = {
  ORG_ID_MISSING: { code: 'ORG_ID_MISSING', hint: 'org_id not configured' },
  ORG_EXPORT_CONFIG_MISSING: { code: 'ORG_ID_MISSING', hint: 'org_id not configured' },
  MISSION_NO_GATE: { code: 'MISSION_NO_GATE', hint: 'mission has no gate' },
  MISSION_MSN_MISSING: { code: 'MISSION_MSN_MISSING', hint: 'mission msn_id missing' },
  ENOENT: { code: 'MISSION_NOT_FOUND', hint: 'mission file not found' },
};

const KERNEL_MESSAGE_PATTERNS = [
  {
    match: (msg) =>
      msg.includes('GXT_MISSION_SCHEMA_INVALID') || msg.includes('schema validation failed'),
    code: 'MISSION_SCHEMA_INVALID',
  },
  { match: (msg) => msg.includes('missing MISSION schema'), code: 'MISSION_SCHEMA_MISSING' },
];

function claimsDenial(e) {
  if (e && typeof e === 'object' && 'code' in e) {
    const kernelCode = String(e.code);
    const mapped = KERNEL_CODE_TO_DENIED[kernelCode];
    if (mapped) {
      return new GantryDenied(mapped.code, e.message ?? mapped.hint);
    }
  }
  const msg = e instanceof Error ? e.message : String(e);
  for (const { match, code } of KERNEL_MESSAGE_PATTERNS) {
    if (match(msg)) {
      return new GantryDenied(code, msg);
    }
  }
  return new GantryDenied('VERDICT_CLAIMS_FAILED', msg);
}

/** Recompute claims at promote time and verify token. Throws GantryDenied on failure. */
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
    throw claimsDenial(e);
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
