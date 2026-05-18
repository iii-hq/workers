import { useEffect, useMemo, useState } from 'react'
import {
  catalogRowsToModelOptions,
  fetchModelsCatalog,
} from '@/lib/models-catalog'
import type { ModelOption } from '@/types/chat'
import { STATIC_MODEL_OPTIONS } from '@/types/chat'

/**
 * Populate model picker options from `models::list` when the real backend is
 * active; mock / playground uses `STATIC_MODEL_OPTIONS` only.
 */
export function useModelPickerSource(backendId: string): {
  modelOptions: ModelOption[]
  catalogKeys: string[]
  catalogLoading: boolean
} {
  const [modelOptions, setModelOptions] =
    useState<ModelOption[]>(STATIC_MODEL_OPTIONS)
  const [catalogLoading, setCatalogLoading] = useState(backendId === 'real')

  useEffect(() => {
    if (backendId !== 'real') {
      setModelOptions(STATIC_MODEL_OPTIONS)
      setCatalogLoading(false)
      return
    }

    let cancelled = false
    setCatalogLoading(true)

    fetchModelsCatalog()
      .then((rows) => {
        if (cancelled) return
        if (rows.length > 0) {
          setModelOptions(catalogRowsToModelOptions(rows))
        } else {
          setModelOptions(STATIC_MODEL_OPTIONS)
        }
      })
      .catch(() => {
        if (!cancelled) setModelOptions(STATIC_MODEL_OPTIONS)
      })
      .finally(() => {
        if (!cancelled) setCatalogLoading(false)
      })

    return () => {
      cancelled = true
    }
  }, [backendId])

  const catalogKeys = useMemo(
    () => modelOptions.map((o) => o.id),
    [modelOptions],
  )

  return { modelOptions, catalogKeys, catalogLoading }
}
