/**
 * TanStack Query hooks for the worker configuration registry. Shares a
 * single key namespace (`['configuration', …]`) so mutations can target
 * everything that depends on a given id with one `invalidateQueries` call.
 */

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useWorkerLifecycle } from '@/hooks/use-worker-lifecycle'
import { getDefaultBackend } from '@/lib/backend'
import { notifyHarnessConfigSaved } from '@/lib/harness-config-events'
import {
  type ConfigurationSchemaView,
  getConfiguration,
  getConfigurationSchema,
  type JsonValue,
  listConfigurations,
  type SetConfigurationPayload,
  type SetResponse,
  setConfiguration,
} from './api'

/* ------------------------------------------------------------------ */
/*  Query keys                                                         */
/* ------------------------------------------------------------------ */

const KEY_ROOT = ['configuration'] as const

export const configurationKeys = {
  all: KEY_ROOT,
  list: () => [...KEY_ROOT, 'list'] as const,
  schema: (id: string) => [...KEY_ROOT, 'schema', id] as const,
  /**
   * Raw value (env templates preserved). Used by the editor so saves are
   * lossless. Cached separately from the env-expanded preview below.
   */
  rawValue: (id: string) => [...KEY_ROOT, 'value', id, 'raw'] as const,
  /** Env-expanded value. Used by preview tooltips. */
  expandedValue: (id: string) =>
    [...KEY_ROOT, 'value', id, 'expanded'] as const,
}

/* ------------------------------------------------------------------ */
/*  Queries                                                            */
/* ------------------------------------------------------------------ */

export function useConfigurationsList() {
  return useQuery<ConfigurationSchemaView[]>({
    queryKey: configurationKeys.list(),
    queryFn: () => listConfigurations(),
  })
}

/** Browser-local handler id bound to the `worker` lifecycle trigger. */
const WORKER_REGISTRY_WATCH_FN = 'console::workers-config-watch'

/**
 * Keep the worker configuration list fresh when workers are added or removed
 * out of band — e.g. `iii worker add harness` run in a terminal — purely off
 * the engine's `worker` lifecycle trigger (its push channel). No polling.
 * Each event re-pulls `configuration::list` so a freshly-installed worker's
 * entry appears once it registers.
 */
export function useWorkerRegistryReactivity(): void {
  const qc = useQueryClient()
  const enabled = getDefaultBackend().id === 'real'

  useWorkerLifecycle({
    enabled,
    fnId: WORKER_REGISTRY_WATCH_FN,
    operations: ['add', 'remove'],
    onEvent: () => {
      qc.invalidateQueries({ queryKey: configurationKeys.list() })
    },
  })
}

export function useConfigurationSchema(id: string | null | undefined) {
  return useQuery<ConfigurationSchemaView>({
    queryKey: configurationKeys.schema(id ?? ''),
    queryFn: () => getConfigurationSchema(id as string),
    enabled: !!id,
  })
}

export function useConfigurationValue(id: string | null | undefined) {
  return useQuery<JsonValue>({
    queryKey: configurationKeys.rawValue(id ?? ''),
    queryFn: () => getConfiguration(id as string, { raw: true }),
    enabled: !!id,
  })
}

/* ------------------------------------------------------------------ */
/*  Mutations                                                          */
/* ------------------------------------------------------------------ */

export function useSetConfiguration(id: string | null | undefined) {
  const qc = useQueryClient()
  return useMutation<SetResponse, Error, SetConfigurationPayload>({
    mutationFn: (payload) => setConfiguration(payload),
    onSuccess: (_data, variables) => {
      const targetId = variables.id ?? id ?? ''
      if (targetId) {
        notifyHarnessConfigSaved(targetId)
        qc.invalidateQueries({
          queryKey: configurationKeys.rawValue(targetId),
        })
        qc.invalidateQueries({
          queryKey: configurationKeys.expandedValue(targetId),
        })
      }
      qc.invalidateQueries({ queryKey: configurationKeys.list() })
    },
  })
}
