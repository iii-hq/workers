import { useEffect, useRef, useState } from 'react'
import type { FileHit, FileSearchFn } from '@/lib/file-search'

/** Keystrokes settle for this long before a query goes to the worker. */
const DEBOUNCE_MS = 90

/**
 * Debounced, sequence-guarded file search for a typeahead: the latest
 * query wins, a slow earlier answer never overwrites a newer one, and the
 * previous hits stay on screen while the next query is in flight so the
 * menu doesn't flash empty between keystrokes. Without a search function
 * (no working directory) the hook is inert.
 */
export function useFileSearch(
  searchFiles: FileSearchFn | undefined,
  query: string | null,
): { files: FileHit[]; loading: boolean } {
  const [files, setFiles] = useState<FileHit[]>([])
  const [loading, setLoading] = useState(false)
  const seqRef = useRef(0)

  useEffect(() => {
    if (!searchFiles || query === null) {
      seqRef.current += 1
      setFiles([])
      setLoading(false)
      return
    }
    const seq = ++seqRef.current
    setLoading(true)
    const timer = window.setTimeout(() => {
      searchFiles(query)
        .then((hits) => {
          if (seqRef.current !== seq) return
          setFiles(hits)
          setLoading(false)
        })
        .catch(() => {
          if (seqRef.current !== seq) return
          setLoading(false)
        })
    }, DEBOUNCE_MS)
    return () => window.clearTimeout(timer)
  }, [searchFiles, query])

  return { files, loading }
}
