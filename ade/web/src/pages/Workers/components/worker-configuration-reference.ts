import { getIiiClient } from '@/lib/iii-client'
import {
  type ConfigurationSchemaView,
  getConfigurationSchema,
  listConfigurations,
} from '@/pages/Configuration/tabs/WorkersTab/api'
import {
  fetchEngineWorkerInfo,
  fetchEngineWorkersList,
  isEngineLive,
} from '../api/workers'

export type ReferenceCatalogReason =
  | 'missing'
  | 'ambiguous'
  | 'ambiguous-worker'
  | 'unavailable'
  | 'unverified'

export type WorkerConfigurationReferenceResult =
  | { kind: 'schema'; entry: ConfigurationSchemaView }
  | {
      kind: 'catalog'
      reason: ReferenceCatalogReason
      entries: ConfigurationSchemaView[]
    }

export async function loadWorkerConfigurationReference(
  name: string,
): Promise<WorkerConfigurationReferenceResult> {
  const catalog = async (
    reason: ReferenceCatalogReason,
  ): Promise<WorkerConfigurationReferenceResult> => ({
    kind: 'catalog',
    reason,
    entries: await listConfigurations(),
  })
  const workers = await fetchEngineWorkersList()
  const matches = workers.workers.filter(
    (worker) => worker.name === name && isEngineLive(worker.status),
  )
  if (matches.length !== 1)
    return catalog(matches.length ? 'ambiguous-worker' : 'unavailable')
  const info = await fetchEngineWorkerInfo(name)
  if (!info || info.worker.id !== matches[0].id) return catalog('unverified')
  const identities = [
    ...new Map(
      info.functions
        .filter((fn) => fn.function_id.endsWith('::configuration-id'))
        .map((fn) => [`${fn.namespace ?? ''}\0${fn.function_id}`, fn]),
    ).values(),
  ]
  if (identities.length !== 1)
    return catalog(identities.length ? 'ambiguous' : 'missing')
  const advertised = identities[0]
  const namespace = advertised.namespace
  if (
    typeof namespace !== 'string' ||
    !namespace.trim() ||
    advertised.worker_name !== name ||
    info.worker.namespace !== namespace ||
    (matches[0].namespace !== undefined && matches[0].namespace !== namespace)
  )
    return catalog('unverified')
  const client = await getIiiClient()
  const identity = await client.trigger<{ id?: unknown } | null>(
    advertised.function_id,
    {},
    { namespace },
  )
  if (typeof identity?.id !== 'string' || !identity.id.trim())
    throw new Error('The worker did not return a configuration entry ID.')
  const entry = await getConfigurationSchema(identity.id)
  if (entry.id !== identity.id)
    throw new Error(
      'The returned schema does not match the advertised entry ID.',
    )
  return { kind: 'schema', entry }
}
