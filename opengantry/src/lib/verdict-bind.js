import path from 'node:path';

import { verifyVerdictToken } from '@jeger-ai/opengantry/kernel';

export function defaultVerdictKeyringPath(repoRoot) {
  const override = process.env.GANTRY_VERDICT_KEYRING?.trim();
  if (override) return override;
  return path.join(repoRoot, '.config/gantry/pepper-keyring.json');
}

/** Derive expected claims from the token body; never trust caller-supplied expected. */
export function expectedClaimsFromToken(token) {
  try {
    const raw = Buffer.from(token, 'base64url').toString('utf8');
    const parsed = JSON.parse(raw);
    const p = parsed?.payload;
    if (!p || typeof p !== 'object') return null;
    return {
      msn_id: p.msn_id,
      mission_sha256: p.mission_sha256,
      findings_digest: p.findings_digest,
      gate_command: p.gate_command,
      org_id: p.org_id,
    };
  } catch {
    return null;
  }
}

export function verifyPromoteVerdictToken({ token, msnId, repoRoot }) {
  const expected = expectedClaimsFromToken(token);
  if (!expected) return false;
  if (msnId && expected.msn_id !== msnId) return false;
  return verifyVerdictToken({
    token,
    expected,
    keyringPath: defaultVerdictKeyringPath(repoRoot),
  });
}
