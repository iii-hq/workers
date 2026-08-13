import { useEffect, useRef, useSyncExternalStore } from 'react'
import {
  clearShellReviewSummary,
  getShellReviewSummary,
  publishShellReviewSummary,
  subscribeShellReviewFileSelection,
  subscribeShellReviewSummary,
  type ShellReviewFileSummary,
} from './review-summary-state'

export {
  clearShellReviewSummary,
  emitShellReviewFileSelection,
  getShellReviewSummary,
  publishShellReviewSummary,
  subscribeShellReviewFileSelection,
  subscribeShellReviewSummary,
} from './review-summary-state'
export type {
  ShellReviewFileSelection,
  ShellReviewFileSummary,
  ShellReviewSummary,
} from './review-summary-state'

export function useShellReviewSummary(sessionId: string) {
  return useSyncExternalStore(
    (listener) => subscribeShellReviewSummary(sessionId, listener),
    () => getShellReviewSummary(sessionId),
    () => null,
  )
}

/**
 * Bridge a Shell explorer's ReviewPane state into the chat footer and route
 * file clicks back only to the explorer that published the visible snapshot.
 */
export function useShellReviewSummaryBridge({
  sessionId,
  sourceId,
  turnId,
  files,
  onSelectFile,
}: {
  sessionId: string | null | undefined
  sourceId: string
  turnId: string | null
  files: readonly ShellReviewFileSummary[]
  onSelectFile: (path: string) => void
}) {
  const selectRef = useRef(onSelectFile)
  selectRef.current = onSelectFile

  useEffect(() => {
    if (!sessionId) return
    publishShellReviewSummary(sessionId, { sourceId, turnId, files })
  }, [sessionId, sourceId, turnId, files])

  useEffect(() => {
    if (!sessionId) return
    return () => clearShellReviewSummary(sessionId, sourceId)
  }, [sessionId, sourceId])

  useEffect(() => {
    if (!sessionId) return
    return subscribeShellReviewFileSelection(sessionId, (selection) => {
      if (selection.sourceId === sourceId) selectRef.current(selection.path)
    })
  }, [sessionId, sourceId])
}
