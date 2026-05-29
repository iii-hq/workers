/**
 * TanStack Query hooks for the worker configuration registry. Shares a
 * single key namespace (`['configuration', …]`) so mutations can target
 * everything that depends on a given id with one `invalidateQueries` call.
 */

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
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
