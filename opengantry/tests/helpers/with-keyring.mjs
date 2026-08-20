import { writeKeyring } from './mini-repo.mjs';

/** Run fn with a temp keyring path; pass verdictKeyringPath into createGantryRuntime. */
export async function withKeyring(dir, fn, { orgId = 'demo-org', pepper = 'demo-pepper' } = {}) {
  const keyring = writeKeyring(dir, { orgId, pepper });
  return fn(keyring);
}
