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
  /** A revision → working copy, chosen by the user. `from` is the file's
      path at `ref` when it had another name there, relative to the
      repository's top level. */
  | { type: 'compare'; ref: string; from?: string }
  /** An exact recorded change (`coder::change-diff`) from a chat card. */
  | { type: 'change'; changeId: string }
  /** One revision against another: a commit against its parent, a stash
      against its base. `label` names the newer side (`0d5b60e`, `stash@{1}`). */
  | { type: 'revision'; from: string; to: string; label: string }
  /** One commit's change to the file: its parent (null for a root commit)
      → the commit. `from` is a rename's source, relative to the
      repository's top level rather than the browsed root. */
  | { type: 'commit'; sha: string; parent: string | null; from?: string }

const HEX = /^[0-9a-f]{4,64}$/i

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
      return source.from ? `compare=${source.ref}:${source.from}` : `compare=${source.ref}`
    case 'change':
      return `change=${source.changeId}`
    case 'commit':
      return `commit=${source.parent ?? ''}..${source.sha}`
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
    case 'commit':
      return source.sha.slice(0, 7)
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
    case 'commit':
      return { old: source.parent === null ? 'empty' : source.parent.slice(0, 7), new: source.sha.slice(0, 7) }
  }
}

/** Diffs that follow the working copy re-read when the disk changes; a
    recorded change, a pair of revisions and a commit are fixed. */
export function diffSourceFollowsDisk(source: DiffSource): boolean {
  return source.type !== 'change' && source.type !== 'revision' && source.type !== 'commit'
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
      if (typeof raw.ref !== 'string' || raw.ref === '') return null
      return typeof raw.from === 'string' && raw.from !== ''
        ? { type: 'compare', ref: raw.ref, from: raw.from }
        : { type: 'compare', ref: raw.ref }
    case 'change':
      return typeof raw.changeId === 'string' && raw.changeId !== ''
        ? { type: 'change', changeId: raw.changeId }
        : null
    case 'commit': {
      if (typeof raw.sha !== 'string' || !HEX.test(raw.sha)) return null
      if (raw.parent !== null && (typeof raw.parent !== 'string' || !HEX.test(raw.parent))) return null
      if (raw.from !== undefined && (typeof raw.from !== 'string' || raw.from === '')) return null
      const source: DiffSource = { type: 'commit', sha: raw.sha, parent: raw.parent }
      return typeof raw.from === 'string' ? { ...source, from: raw.from } : source
    }
    default:
      return null
  }
}
