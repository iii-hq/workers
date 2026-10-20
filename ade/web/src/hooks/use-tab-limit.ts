// The configured workspace tab limit, read from the engine's `console`
// configuration entry (`tabs.limit`) the same way the traces preferences
// are. The entry is shared by every browser pointing at this engine and is
// edited through the ADE configuration form, so a change there reaches the
// strip on the next read without any code knowing the number.

import { useQuery, useQueryClient } from '@tanstack/react-query'
import {
  CONSOLE_CONFIG_QUERY_KEY,
  consoleConfigWriter,
} from '@/hooks/lib/console-config-writer'
import type { ConsoleConfigValue } from '@/lib/console-config'
import { parseTabLimit } from '@/lib/tab-limit'

export function useTabLimit(): number {
  const qc = useQueryClient()
  const writer = consoleConfigWriter(qc)
  const { data } = useQuery<ConsoleConfigValue | null>({
    queryKey: CONSOLE_CONFIG_QUERY_KEY,
    queryFn: () => writer.readForQuery(),
    staleTime: 30_000,
    retry: 1,
  })
  return parseTabLimit(data)
}
