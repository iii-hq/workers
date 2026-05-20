import { useEffect, useState } from 'react'
import { fetchFunctionsCatalog } from '@/lib/functions-catalog'
import { STATIC_FUNCTIONS, type FunctionEntry } from '@/lib/functions'

/**
 * Populate `@` mention autocomplete from `directory::engine::functions::list`
 * when the real backend is active; mock / playground uses `STATIC_FUNCTIONS`.
 */
export function useFunctionsCatalog(backendId: string): {
  functionEntries: FunctionEntry[]
} {
  const [functionEntries, setFunctionEntries] = useState<FunctionEntry[]>(() =>
    backendId === 'real' ? [] : STATIC_FUNCTIONS,
  )

  useEffect(() => {
    if (backendId !== 'real') {
      setFunctionEntries(STATIC_FUNCTIONS)
      return
    }

    let cancelled = false
    setFunctionEntries([])

    fetchFunctionsCatalog()
      .then((rows) => {
        if (cancelled) return
        setFunctionEntries(rows)
      })
      .catch(() => {
        if (!cancelled) setFunctionEntries([])
      })

    return () => {
      cancelled = true
    }
  }, [backendId])

  return { functionEntries }
}
