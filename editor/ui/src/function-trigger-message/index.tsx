/**
 * How `editor::*` calls render in chat and traces.
 *
 * A patch shown as JSON is unreadable — the escaped newlines are longer than
 * the diff itself. These renderers exist so the agent's edits read as edits:
 * a diff renders as a diff, a save renders as a file card with its line
 * counts, a find renders as the list you would have scanned anyway.
 *
 * Every renderer matches narrowly on its own function id and returns `null`
 * on anything unexpected, so an error or a shape we did not anticipate falls
 * through to the console's default card rather than rendering wrong.
 */

import {
  Badge,
  CodeHighlight,
  type FunctionTriggerMessage,
  type FunctionTriggerRenderer,
  type Host,
} from '@iii-dev/console-ui'

const HANDLED = new Set([
  'editor::workspace::open',
  'editor::workspace::get',
  'editor::buffers::list',
  'editor::buffers::close',
  'editor::move',
  'editor::diff',
  'editor::save',
  'editor::open',
  'editor::find',
  'editor::git::status',
  'editor::git::hunks',
])

/** Patches are bounded before they reach the DOM; a feed row is not a viewer. */
const MAX_PATCH_LINES = 400

function record(value: unknown): Record<string, unknown> | null {
  return typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : null
}

function str(value: unknown): string | null {
  return typeof value === 'string' ? value : null
}

function num(value: unknown): number | null {
  return typeof value === 'number' ? value : null
}

function clampPatch(patch: string): { text: string; clipped: boolean } {
  const lines = patch.split('\n')
  if (lines.length <= MAX_PATCH_LINES) return { text: patch, clipped: false }
  return { text: lines.slice(0, MAX_PATCH_LINES).join('\n'), clipped: true }
}

function Stat({ added, removed }: { added: number | null; removed: number | null }) {
  if (added === null && removed === null) return null
  return (
    <span className="ed-stat">
      {added !== null && added > 0 && <span className="ed-add">+{added}</span>}
      {removed !== null && removed > 0 && <span className="ed-del">−{removed}</span>}
      {added === 0 && removed === 0 && <span className="ed-muted">no change</span>}
    </span>
  )
}

function Patch({ patch }: { patch: string }) {
  const { text, clipped } = clampPatch(patch)
  return (
    <div className="ed-card-patch">
      <CodeHighlight code={text} language="diff" />
      {clipped && (
        <div className="ed-muted">shown to the first {MAX_PATCH_LINES} lines — open the file to see the rest</div>
      )}
    </div>
  )
}

/** Workspace-shaped responses: the root and the tabs open against it. */
function renderWorkspace(output: Record<string, unknown>) {
  const buffers = Array.isArray(output.buffers) ? output.buffers : null
  if (buffers === null) return null
  const root = str(output.root)
  return (
    <div className="ed-card">
      <div className="ed-card-head">
        {root !== null && <span className="ed-card-path">{root}</span>}
        <span className="ed-muted">{buffers.length === 0 ? 'no files open' : `${buffers.length} open`}</span>
      </div>
      {buffers.length > 0 && (
        <ul className="ed-hits">
          {buffers.slice(0, 12).map((raw, index) => {
            const buffer = record(raw)
            const path = buffer ? str(buffer.path) : null
            return (
              <li key={path ?? index} className="ed-card-path">
                {path ?? '—'}
              </li>
            )
          })}
        </ul>
      )}
    </div>
  )
}

function renderMove(output: Record<string, unknown>) {
  const from = str(output.from)
  const to = str(output.to)
  if (from === null || to === null) return null
  const remapped = num(output.remapped) ?? 0
  return (
    <div className="ed-card">
      <div className="ed-card-head">
        <span className="ed-card-path">{from}</span>
        <span className="ed-muted">→</span>
        <span className="ed-card-path">{to}</span>
        {remapped > 0 && <Badge variant="accent">{`${remapped} remapped`}</Badge>}
      </div>
    </div>
  )
}

function renderDiff(output: Record<string, unknown>) {
  if (output.identical === true) {
    return (
      <div className="ed-card">
        <span className="ed-muted">identical — no diff</span>
      </div>
    )
  }
  if (output.truncated === true) {
    return (
      <div className="ed-card">
        <Badge variant="warn">too large to diff</Badge>
      </div>
    )
  }
  const patch = str(output.patch)
  if (patch === null) return null
  return (
    <div className="ed-card">
      <Stat added={num(output.added)} removed={num(output.removed)} />
      <Patch patch={patch} />
    </div>
  )
}

function renderSave(output: Record<string, unknown>) {
  const path = str(output.path)
  if (path === null) return null
  if (output.conflict === true) {
    const patch = str(output.conflict_patch)
    return (
      <div className="ed-card">
        <div className="ed-card-head">
          <Badge variant="alert">conflict</Badge>
          <span className="ed-card-path">{path}</span>
        </div>
        <div className="ed-muted">Not written — the file changed since it was opened.</div>
        {patch !== null && <Patch patch={patch} />}
      </div>
    )
  }
  return (
    <div className="ed-card">
      <div className="ed-card-head">
        <Badge variant={output.created === true ? 'accent' : 'default'}>
          {output.created === true ? 'created' : 'saved'}
        </Badge>
        <span className="ed-card-path">{path}</span>
        <Stat added={num(output.added)} removed={num(output.removed)} />
      </div>
    </div>
  )
}

function renderOpen(output: Record<string, unknown>) {
  const path = str(output.path)
  if (path === null) return null
  const size = num(output.size)
  return (
    <div className="ed-card">
      <div className="ed-card-head">
        <span className="ed-card-path">{path}</span>
        {str(output.language) !== null && <Badge>{str(output.language)}</Badge>}
        {size !== null && <span className="ed-muted">{size} B</span>}
        {output.truncated === true && <Badge variant="warn">truncated</Badge>}
      </div>
    </div>
  )
}

function renderFind(output: Record<string, unknown>) {
  const matches = Array.isArray(output.matches) ? output.matches : null
  if (matches === null) return null
  if (matches.length === 0) {
    return (
      <div className="ed-card">
        <span className="ed-muted">no match</span>
      </div>
    )
  }
  return (
    <div className="ed-card">
      <ul className="ed-hits">
        {matches.slice(0, 12).map((raw, index) => {
          const match = record(raw)
          const path = match ? str(match.path) : null
          return (
            <li key={path ?? index} className="ed-card-path">
              {path ?? '—'}
            </li>
          )
        })}
      </ul>
      {matches.length > 12 && <div className="ed-muted">+{matches.length - 12} more</div>}
    </div>
  )
}

function renderStatus(output: Record<string, unknown>) {
  const entries = Array.isArray(output.entries) ? output.entries : null
  if (entries === null) return null
  const branch = str(output.branch)
  return (
    <div className="ed-card">
      <div className="ed-card-head">
        {branch !== null && <Badge variant="accent">{branch}</Badge>}
        <span className="ed-muted">{entries.length === 0 ? 'clean' : `${entries.length} changed`}</span>
      </div>
      <ul className="ed-hits">
        {entries.slice(0, 12).map((raw, index) => {
          const entry = record(raw)
          const path = entry ? str(entry.path) : null
          const state = entry ? str(entry.worktree) : null
          return (
            <li key={path ?? index} className="ed-card-path">
              <span className="ed-muted">{state ?? '?'}</span> {path ?? '—'}
            </li>
          )
        })}
      </ul>
    </div>
  )
}

function renderHunks(output: Record<string, unknown>) {
  const path = str(output.path)
  if (path === null) return null
  if (output.untracked === true) {
    return (
      <div className="ed-card">
        <div className="ed-card-head">
          <Badge>untracked</Badge>
          <span className="ed-card-path">{path}</span>
        </div>
      </div>
    )
  }
  const hunks = Array.isArray(output.hunks) ? output.hunks : []
  return (
    <div className="ed-card">
      <div className="ed-card-head">
        <span className="ed-card-path">{path}</span>
        <span className="ed-muted">
          {hunks.length === 0 ? 'unchanged' : `${hunks.length} hunk${hunks.length === 1 ? '' : 's'}`}
        </span>
        <Stat added={num(output.added)} removed={num(output.removed)} />
      </div>
    </div>
  )
}

export function createEditorTriggerRenderer(_host: Host): FunctionTriggerRenderer {
  return {
    id: 'editor',
    isMatch: (functionId: string) => HANDLED.has(functionId),
    tryRender(message: FunctionTriggerMessage) {
      const output = record(message.output)
      if (output === null) return null
      // An error payload keeps the console's default error card.
      if (output.error !== undefined) return null

      switch (message.functionId) {
        case 'editor::workspace::open':
        case 'editor::workspace::get':
        case 'editor::buffers::list':
        case 'editor::buffers::close':
          return renderWorkspace(output)
        case 'editor::move':
          return renderMove(output)
        case 'editor::diff':
          return renderDiff(output)
        case 'editor::save':
          return renderSave(output)
        case 'editor::open':
          return renderOpen(output)
        case 'editor::find':
          return renderFind(output)
        case 'editor::git::status':
          return renderStatus(output)
        case 'editor::git::hunks':
          return renderHunks(output)
        default:
          return null
      }
    },
  }
}
