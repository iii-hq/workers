/* What a diff tab compares. Every kind of diff the page shows is one of
   these sources against one root-relative path; the tab id derives from
   both, so the same file can sit open twice (staged and unstaged) and a
   second click on either row lands on its own tab instead of a new one. */

export type DiffSource =
  /** HEAD → working copy: what committing the file from the Commit panel records. */
  | { type: 'uncommitted' }
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
  /** One revision against another: a commit against its parent, a stash
      against its base. `label` names the newer side (`0d5b60e`, `stash@{1}`). */
  | { type: 'revision'; from: string; to: string; label: string }

export function diffSourceKey(source: DiffSource): string {
  switch (source.type) {
    case 'uncommitted':
    case 'staged':
    case 'unstaged':
      return source.type
    case 'revision':
      return `revision=${source.from}..${source.to}`
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
    case 'uncommitted':
      return 'Changes'
    case 'staged':
      return 'Staged'
    case 'unstaged':
      return 'Unstaged'
    case 'revision':
      return source.label
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
    case 'uncommitted':
      return { old: 'HEAD', new: 'working copy' }
    case 'revision':
      return { old: 'parent', new: source.label }
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
    recorded change or a pair of revisions is fixed. */
export function diffSourceFollowsDisk(source: DiffSource): boolean {
  return source.type !== 'change' && source.type !== 'revision'
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
    case 'uncommitted':
    case 'staged':
    case 'unstaged':
      return { type: raw.type }
    case 'revision':
      return typeof raw.from === 'string' && raw.from !== '' &&
        typeof raw.to === 'string' && raw.to !== '' &&
        typeof raw.label === 'string' && raw.label !== ''
        ? { type: 'revision', from: raw.from, to: raw.to, label: raw.label }
        : null
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
