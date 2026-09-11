/* The revisions a file can be compared against: branches, tags and recent
   commits of the browsed root, loaded once per root when a compare pane
   asks for them. Shaped as Selector groups. */

import type { Host, SelectorGroup } from '@iii-dev/console-ui'
import { useEffect, useMemo, useRef, useState } from 'react'
import { errorMessage } from '../lib/format'
import { gitRecentCommits, gitRefs } from './git'
import { gitTags } from './git-actions'

export interface CompareRefs {
  loading: boolean
  error: string | null
  groups: readonly SelectorGroup[]
  /** `HEAD` is always offered; it is the sensible default. */
  defaultRef: string
}

export function useCompareRefs(host: Host, root: string | null, enabled: boolean): CompareRefs {
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [groups, setGroups] = useState<readonly SelectorGroup[]>([])
  const seqRef = useRef(0)

  useEffect(() => {
    if (!enabled || root === null) return
    const seq = ++seqRef.current
    setLoading(true)
    setError(null)
    void Promise.all([gitRefs(host, root), gitTags(host, root).catch(() => []), gitRecentCommits(host, root, 50)])
      .then(([refs, tags, commits]) => {
        if (seqRef.current !== seq) return
        const next: SelectorGroup[] = [
          { label: 'Working tree', options: [{ value: 'HEAD', label: 'HEAD', description: 'last commit' }] },
        ]
        if (refs.kind === 'ready' && refs.refs.length > 0) {
          next.push({
            label: 'Branches',
            options: refs.refs.map((ref) => ({
              value: ref.fullName,
              label: ref.name,
              description: ref.current ? 'current' : ref.kind === 'remote' ? 'remote' : undefined,
              keywords: [ref.sha.slice(0, 8)],
            })),
          })
        }
        if (tags.length > 0) {
          next.push({
            label: 'Tags',
            options: tags.map((tag) => ({ value: `refs/tags/${tag.name}`, label: tag.name, keywords: [tag.sha.slice(0, 8)] })),
          })
        }
        if (commits.kind === 'ready' && commits.commits.length > 0) {
          next.push({
            label: 'Commits',
            options: commits.commits.map((commit) => ({
              value: commit.sha,
              label: commit.subject || commit.sha.slice(0, 8),
              description: commit.sha.slice(0, 8),
              keywords: [commit.sha],
            })),
          })
        }
        setGroups(next)
        const failure =
          refs.kind === 'error' ? refs.message : commits.kind === 'error' ? commits.message : refs.kind === 'not-a-repo' ? 'not a git repository' : null
        setError(failure)
      })
      .catch((err: unknown) => {
        if (seqRef.current !== seq) return
        setError(errorMessage(err))
      })
      .finally(() => {
        if (seqRef.current === seq) setLoading(false)
      })
  }, [host, root, enabled])

  // biome-ignore lint/correctness/useExhaustiveDependencies: a new root starts from an empty list
  useEffect(() => {
    setGroups([])
    setError(null)
  }, [root])

  return useMemo(() => ({ loading, error, groups, defaultRef: 'HEAD' }), [loading, error, groups])
}
