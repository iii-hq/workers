import path from 'node:path';

import { verifyVerdictToken } from '@jeger-ai/opengantry/kernel';

export function defaultVerdictKeyringPath(repoRoot) {
  const override = process.env.GANTRY_VERDICT_KEYRING?.trim();
  if (override) return override;
  return path.join(repoRoot, '.config/gantry/pepper-keyring.json');
}

export function verifyPromoteVerdictToken({ token, msnId, repoRoot, boundExpected }) {
  if (!token || !boundExpected) return false;
  if (msnId && boundExpected.msn_id !== msnId) return false;
  return verifyVerdictToken({
    token,
    expected: boundExpected,
    keyringPath: defaultVerdictKeyringPath(repoRoot),
  });
}
