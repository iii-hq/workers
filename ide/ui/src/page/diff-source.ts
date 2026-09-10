/* What a diff tab compares. Every kind of diff the page shows is one of
   these sources against one root-relative path; the tab id derives from
   both, so the same file can sit open twice (staged and unstaged) and a
   second click on either row lands on its own tab instead of a new one. */

export type DiffSource =
  /** HEAD → index: what `git commit` would record. */
  | { type: 'staged' }
  /** index → working copy: what is not yet added. */
  | { type: 'unstaged' }
  /** One Harness turn's change: its pre-image → the body it left behind
      (the next turn's pre-image when one was kept, else the working copy). */
  | { type: 'turn'; turnId: string }
  /** A revision → working copy, chosen by the user. */
  | { type: 'compare'; ref: string }
  /** An exact recorded change (`coder::change-diff`) from a chat card. */
  | { type: 'change'; changeId: string }

export function diffSourceKey(source: DiffSource): string {
  switch (source.type) {
    case 'staged':
    case 'unstaged':
      return source.type
    case 'turn':
      return `turn=${source.turnId}`
    case 'compare':
      return `compare=${source.ref}`
    case 'change':
      return `change=${source.changeId}`
  }
}

export function sameDiffSource(a: DiffSource, b: DiffSource): boolean {
  return diffSourceKey(a) === diffSourceKey(b)
}

/** The short chip a diff tab shows beside the file name. `turnLabel` names
    a turn when the caller knows it. */
export function diffSourceLabel(source: DiffSource, turnLabel?: string): string {
  switch (source.type) {
    case 'staged':
      return 'Staged'
    case 'unstaged':
      return 'Changes'
    case 'turn':
      return turnLabel ?? 'Turn'
    case 'compare':
      return source.ref.replace(/^refs\/(heads|tags|remotes)\//, '')
    case 'change':
      return 'Change'
  }
}

/** The two sides, in words: "HEAD → index". */
export function diffSourceSides(source: DiffSource, turnLabel?: string): { old: string; new: string } {
  switch (source.type) {
    case 'staged':
      return { old: 'HEAD', new: 'index' }
    case 'unstaged':
      return { old: 'index', new: 'working copy' }
    case 'turn':
      return { old: `before ${turnLabel ?? 'the turn'}`, new: `after ${turnLabel ?? 'the turn'}` }
    case 'compare':
      return { old: diffSourceLabel(source), new: 'working copy' }
    case 'change':
      return { old: 'before the call', new: 'after the call' }
  }
}

/** Diffs that follow the working copy re-read when the disk changes; a
    recorded change is a fixed pair. */
export function diffSourceFollowsDisk(source: DiffSource): boolean {
  return source.type !== 'change'
}

/** Tabs worth keeping across reloads: a change id dies with the worker
    that recorded it. */
export function diffSourcePersists(source: DiffSource): boolean {
  return source.type !== 'change'
}

/** Parse the persisted form back; anything unknown is dropped. */
export function parseDiffSource(value: unknown): DiffSource | null {
  if (!value || typeof value !== 'object') return null
  const raw = value as Record<string, unknown>
  switch (raw.type) {
    case 'staged':
    case 'unstaged':
      return { type: raw.type }
    case 'turn':
      return typeof raw.turnId === 'string' && raw.turnId !== '' ? { type: 'turn', turnId: raw.turnId } : null
    case 'compare':
      return typeof raw.ref === 'string' && raw.ref !== '' ? { type: 'compare', ref: raw.ref } : null
    case 'change':
      return typeof raw.changeId === 'string' && raw.changeId !== ''
        ? { type: 'change', changeId: raw.changeId }
        : null
    default:
      return null
  }
}
