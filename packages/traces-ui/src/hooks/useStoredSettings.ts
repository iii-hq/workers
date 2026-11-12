import { useQuery } from '@tanstack/react-query'
import { type TracesSettingsValue, useTracesSettings } from '../host'

const NO_SETTINGS_KEY = ['traces-ui', 'no-settings'] as const

/**
 * The host's stored settings value (see `TracesSettingsStore`). `data` stays
 * `undefined` when the host keeps no settings, and is `null` when its store
 * has nowhere to persist; both read as "settings live in memory".
 */
export function useStoredSettings() {
  const settings = useTracesSettings()
  const { data, isLoading } = useQuery<TracesSettingsValue | null>({
    queryKey: settings?.queryKey ?? NO_SETTINGS_KEY,
    queryFn: () => (settings ? settings.read() : null),
    enabled: settings !== undefined,
    staleTime: 30_000,
    retry: 1,
  })
  return { settings, data, isLoading }
}
