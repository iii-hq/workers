/**
 * TanStack Query hooks for the worker configuration registry. Shares a
 * single key namespace (`['configuration', …]`) so mutations can target
 * everything that depends on a given id with one `invalidateQueries` call.
 */

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import { useWorkersChanged } from '@/hooks/use-workers-changed'
import { getDefaultBackend } from '@/lib/backend'
import { configurationFormFamily } from '@/lib/configuration-family'
import { subscribeEngineTrigger } from '@/lib/engine-trigger'
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

/** Browser-local handler id bound to `engine::workers-available`. */
const WORKER_REGISTRY_WATCH_FN = 'console::workers-config-watch'
/** Browser-local handler id bound to the `configuration` trigger. */
const CONFIGURATION_WATCH_FN = 'iii::console::configuration-list-watch'

/**
 * Keep the worker configuration list fresh when workers are added or removed
 * out of band — e.g. `iii trigger compose::add worker=harness` run in a
 * terminal — purely off engine push triggers. No polling. A worker connects
 * before it registers its entry, so the list is re-pulled on both
 * `engine::workers-available` and `configuration` registered/updated events.
 */
export function useWorkerRegistryReactivity(): void {
  const qc = useQueryClient()
  const enabled = getDefaultBackend().id === 'real'

  useWorkersChanged({
    enabled,
    fnId: WORKER_REGISTRY_WATCH_FN,
    onChange: () => {
      qc.invalidateQueries({ queryKey: configurationKeys.list() })
    },
  })

  useEffect(() => {
    if (!enabled) return
    let cancelled = false
    let dispose: (() => void) | undefined
    // No configuration id in the binding: a per-id binding would hold that
    // entry's TTL slot (see waitForConfiguration).
    subscribeEngineTrigger(
      'configuration',
      { event_types: ['configuration:registered', 'configuration:updated'] },
      () => {
        qc.invalidateQueries({ queryKey: configurationKeys.list() })
      },
      { handler: CONFIGURATION_WATCH_FN },
    )
      .then((off) => {
        if (cancelled) off()
        else dispose = off
      })
      .catch(() => undefined)
    return () => {
      cancelled = true
      dispose?.()
    }
  }, [enabled, qc])
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
        const entry =
          qc.getQueryData<ConfigurationSchemaView>(
            configurationKeys.schema(targetId),
          ) ??
          qc
            .getQueryData<ConfigurationSchemaView[]>(configurationKeys.list())
            ?.find((item) => item.id === targetId)
        notifyHarnessConfigSaved(
          entry ? configurationFormFamily(entry) : targetId,
        )
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
