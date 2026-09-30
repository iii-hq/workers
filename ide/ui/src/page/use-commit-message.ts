/* Generate: the worker writes a commit message for the ticked changes.
   The page describes them (a stat and a unified diff, via git.ts plumbing)
   and hands the text to `shell::scm::commit-message`, which reads the
   worker's `commit_messages` settings (model, reasoning, instructions) and
   asks the router. With no model configured the worker falls back to the
   model the chat composer has picked, which only the console knows, so it
   rides along as `fallback_model`. */

import type { Host } from '@iii-dev/console-ui'
import { errorCode, errorMessage } from '@iii-dev/console-ui/format'
import { useCallback, useEffect, useRef, useState } from 'react'
import type { GitComparisonEntry } from './git'
import { gitDescribeChanges, gitRecentSubjects } from './git-log'

export interface CommitMessageConfig {
  model: string | null
  thinking: string
  instructions: string
}

export type GenerateState =
  | { phase: 'idle' }
  | { phase: 'writing'; files: number }
  | { phase: 'error'; message: string; noModel: boolean }

export interface GeneratedMessage {
  message: string
  model: string
}

/** A router model id reads better without its provider prefix. */
export function modelLabel(model: string): string {
  const separator = model.indexOf('::')
  return separator === -1 ? model : model.slice(separator + 2)
}

export function isNoModelError(code: string | undefined, message: string): boolean {
  return code === 'NO_MODEL' || /no model/i.test(message)
}

export function useCommitMessage(host: Host, root: string | null, conversationId: string | null | undefined) {
  const [state, setState] = useState<GenerateState>({ phase: 'idle' })
  const [config, setConfig] = useState<CommitMessageConfig | null>(null)
  const seqRef = useRef(0)

  const fallbackModel = useCallback(
    () => host.chat?.composerModel?.(conversationId ?? null) ?? null,
    [host, conversationId],
  )

  const loadConfig = useCallback(() => {
    host.iii
      .trigger<CommitMessageConfig>('shell::scm::commit-message-config', {})
      .then(setConfig)
      // An older worker without the function: Generate still works on the fallback.
      .catch(() => setConfig(null))
  }, [host])

  useEffect(() => loadConfig(), [loadConfig])

  const generate = useCallback(
    async (entries: readonly GitComparisonEntry[]): Promise<GeneratedMessage | null> => {
      if (root === null || entries.length === 0) return null
      const seq = ++seqRef.current
      setState({ phase: 'writing', files: entries.length })
      try {
        const [changes, subjects] = await Promise.all([
          gitDescribeChanges(host, root, entries),
          gitRecentSubjects(host, root),
        ])
        const out = await host.iii.trigger<GeneratedMessage>(
          'shell::scm::commit-message',
          { changes, recent_subjects: subjects, fallback_model: fallbackModel() },
          // The router may take a while on a large diff; the worker caps it at 120 s.
          { timeoutMs: 150_000 },
        )
        if (seqRef.current !== seq) return null
        setState({ phase: 'idle' })
        return out
      } catch (err: unknown) {
        if (seqRef.current !== seq) return null
        const message = errorMessage(err)
        setState({ phase: 'error', message, noModel: isNoModelError(errorCode(err), message) })
        return null
      }
    },
    [host, root, fallbackModel],
  )

  /** Stop waiting: a late answer is dropped. */
  const stop = useCallback(() => {
    seqRef.current++
    setState({ phase: 'idle' })
  }, [])

  const effectiveModel = config?.model ?? fallbackModel()

  return {
    state,
    config,
    loadConfig,
    generate,
    stop,
    /** The model Generate will use, and whether it is the chat's. */
    model: effectiveModel,
    usesChatDefault: !config?.model,
  }
}
