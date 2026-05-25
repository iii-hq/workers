/**
 * TanStack Query hooks for provider auth and config.
 * Each row in the providers dialog owns its own query; mutations
 * invalidate the affected query key so the row re-fetches automatically.
 */

import {
  useMutation,
  useQueries,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import {
  clearProviderConfig,
  deleteToken,
  fetchAuthStatus,
  fetchProviderConfig,
  type SetProviderConfigPayload,
  type SetTokenPayload,
  setProviderConfig,
  setToken,
} from '@/lib/providers'

/* ------------------------------------------------------------------ */
/*  Queries                                                            */
/* ------------------------------------------------------------------ */

export function useAuthStatus(provider: string) {
  return useQuery({
    queryKey: ['provider', 'status', provider],
    queryFn: () => fetchAuthStatus(provider),
  })
}

export function useProviderConfig(provider: string) {
  return useQuery({
    queryKey: ['provider', 'config', provider],
    queryFn: () => fetchProviderConfig(provider),
  })
}

/**
 * Fan-out of `useAuthStatus` across a set of providers. React Query
 * dedupes by query key, so calling this at the page level alongside the
 * per-row `useAuthStatus` calls does not trigger additional fetches —
 * both share the same cache. Used by the Configuration page to detect
 * the zero-config empty state.
 */
export function useProviderStatuses(providers: readonly string[]) {
  return useQueries({
    queries: providers.map((provider) => ({
      queryKey: ['provider', 'status', provider],
      queryFn: () => fetchAuthStatus(provider),
    })),
  })
}

/* ------------------------------------------------------------------ */
/*  Mutations                                                          */
/* ------------------------------------------------------------------ */

export function useSetToken() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (payload: SetTokenPayload) => setToken(payload),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({
        queryKey: ['provider', 'status', variables.provider],
      })
    },
  })
}

export function useDeleteToken() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (provider: string) => deleteToken(provider),
    onSuccess: (_data, provider) => {
      qc.invalidateQueries({
        queryKey: ['provider', 'status', provider],
      })
    },
  })
}

export function useSetProviderConfig() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (payload: SetProviderConfigPayload) =>
      setProviderConfig(payload),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({
        queryKey: ['provider', 'config', variables.provider],
      })
    },
  })
}

export function useClearProviderConfig() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (provider: string) => clearProviderConfig(provider),
    onSuccess: (_data, provider) => {
      qc.invalidateQueries({
        queryKey: ['provider', 'config', provider],
      })
    },
  })
}
