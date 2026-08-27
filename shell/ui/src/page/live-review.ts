export type PriorFilesystemKind = 'file' | 'dir' | null | undefined

export interface LiveReviewEventInput {
  path: string
  rawKind: string
  /** `null` means known missing; `undefined` means no tree snapshot. */
  priorKind: PriorFilesystemKind
  /**
   * Whether the inventory PROVED `priorKind` rather than inferring it. A
   * truncated inventory cannot tell an omitted file from a new one, so it
   * guesses `file`; that guess must not outrank a creation the watcher
   * actually saw during this turn.
   */
  priorKindExact?: boolean
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

  // A guessed "it existed" loses to a witnessed creation with no captured
  // body: the watcher saw this path appear, the inventory never listed it.
  const guessedExisting =
    input.priorKind === 'file' &&
    input.priorKindExact === false &&
    input.priorBaseline === undefined
  if (guessedExisting && input.rawKind === 'created' && input.existsNow) {
    return { action: 'created', path: input.path, baseline: '' }
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

export function trackIgnoredPath(
  ignored: Set<string>,
  abs: string,
  isIgnored: boolean,
): void {
  if (isIgnored) ignored.add(abs)
  else ignored.delete(abs)
}

export function onlyIgnoredChanges(
  paths: readonly string[],
  ignored: ReadonlySet<string>,
): boolean {
  return paths.length > 0 && paths.every((path) => ignored.has(path))
}
