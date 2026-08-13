export type PriorFilesystemKind = 'file' | 'dir' | null | undefined

export interface LiveReviewEventInput {
  path: string
  rawKind: string
  /** `null` means known missing; `undefined` means no tree snapshot. */
  priorKind: PriorFilesystemKind
  /** Undefined means uncaptured. An empty string is a real baseline. */
  priorBaseline?: string
  /** Whether the changed path is a readable file after the event burst. */
  existsNow: boolean
}

export type LiveReviewEventDecision =
  | { action: 'ignore-directory'; path: string }
  | { action: 'ignore-delete'; path: string }
  | {
      action: 'created' | 'modified' | 'deleted'
      path: string
      baseline: string | undefined
    }

/** Normalize noisy watcher events against the pre-burst filesystem view. */
export function normalizeLiveReviewEvent(input: LiveReviewEventInput): LiveReviewEventDecision {
  if (input.priorKind === 'dir' && !input.existsNow) {
    return { action: 'ignore-directory', path: input.path }
  }

  const existedBefore =
    input.priorKind === 'file' ||
    (input.priorKind === undefined && input.priorBaseline !== undefined)
  if (existedBefore) {
    return {
      action: input.existsNow ? 'modified' : 'deleted',
      path: input.path,
      baseline: input.priorBaseline,
    }
  }

  if (input.priorKind === null || input.priorKind === 'dir') {
    return input.existsNow
      ? { action: 'created', path: input.path, baseline: '' }
      : { action: 'ignore-delete', path: input.path }
  }

  if (!input.existsNow) {
    return { action: 'ignore-delete', path: input.path }
  }
  return input.rawKind === 'created'
    ? { action: 'created', path: input.path, baseline: '' }
    : { action: 'modified', path: input.path, baseline: undefined }
}
