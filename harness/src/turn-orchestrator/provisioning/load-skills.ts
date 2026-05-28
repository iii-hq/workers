/**
 * Load default skill bodies via provisioning ports.
 */

import { defaultSkillBody, skillIdFromUri, type DefaultSkillBody } from '../system-prompt.js';
import type { ProvisioningPorts } from './ports.js';

export async function loadDefaultSkillBodies(
  ports: Pick<ProvisioningPorts, 'fetchSkillBody'>,
  uris: readonly string[],
): Promise<DefaultSkillBody[]> {
  const bodies: DefaultSkillBody[] = [];
  for (const uri of uris) {
    const body = await ports.fetchSkillBody(skillIdFromUri(uri));
    bodies.push(defaultSkillBody(uri, body));
  }
  return bodies;
}
