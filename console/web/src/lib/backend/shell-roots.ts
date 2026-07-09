/**
 * Read-only peek at shell's permanent allowed folders
 * (`configuration::get { id: 'shell' }` → `value.fs.host_roots`), for the
 * filesystem-access management dialog's "always allowed (all sessions)" group.
 * Editing happens on the existing configuration editor
 * (`#/workers/configuration/shell`) — this is read-only here on purpose.
 */

import { getIiiClient } from '@/lib/iii-client'

const SHELL_CONFIG_ID = 'shell'

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
      id: SHELL_CONFIG_ID,
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
