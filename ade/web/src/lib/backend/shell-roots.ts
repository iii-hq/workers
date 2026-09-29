/**
 * Read-only peek at shell's permanent allowed folders
 * (`ide::configuration-id` → `configuration::get` → `value.fs.host_roots`), for the
 * filesystem-access management dialog's "always allowed (all sessions)" group.
 * Editing happens on the existing configuration editor
 * for the resolved entry — this is read-only here on purpose.
 */

import { resolveConfigurationId } from '@iii-dev/console-ui/configuration'
import { getIiiClient } from '@/lib/iii-client'

interface ShellConfigValue {
  fs?: { host_roots?: unknown }
}

interface GetConfigResponse {
  value?: ShellConfigValue
}

/** Tolerant of a missing/unconfigured shell worker — resolves to []. */
export async function getShellHostRoots(): Promise<string[]> {
  try {
    const client = await getIiiClient()
    const res = await client.trigger<GetConfigResponse>('configuration::get', {
      id: await resolveConfigurationId(client, 'ide'),
      raw: false,
    })
    const roots = res?.value?.fs?.host_roots
    return Array.isArray(roots)
      ? roots.filter((r): r is string => typeof r === 'string')
      : []
  } catch {
    return []
  }
}
