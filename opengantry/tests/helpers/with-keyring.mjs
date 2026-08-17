import { writeKeyring } from './mini-repo.mjs';

/**
 * Run fn with GANTRY_VERDICT_KEYRING set to a temp keyring; restores prior env on exit.
 */
export async function withKeyring(dir, fn, { orgId = 'demo-org', pepper = 'demo-pepper' } = {}) {
  const keyring = writeKeyring(dir, { orgId, pepper });
  const prevKeyring = process.env.GANTRY_VERDICT_KEYRING;
  process.env.GANTRY_VERDICT_KEYRING = keyring;
  try {
    return await fn(keyring);
  } finally {
    if (prevKeyring === undefined) delete process.env.GANTRY_VERDICT_KEYRING;
    else process.env.GANTRY_VERDICT_KEYRING = prevKeyring;
  }
}
