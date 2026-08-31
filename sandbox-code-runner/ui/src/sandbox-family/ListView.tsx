/**
 * `sandbox::list` — the fleet table. `EmptyState` and `StatusDot` come
 * from the console; each row's id is the interactive chip so a listed
 * sandbox is one keypress from its fleet-page row.
 */

import { EmptyState, StatusDot } from '@iii-dev/console-ui'
import { formatAgeSecs } from './format'
import { listResponseSchema, safeParseResponse } from './parsers'
import { SandboxIdChip } from './shared'

interface ListViewProps {
  output: unknown
}

export function ListView({ output }: ListViewProps) {
  const parsed = safeParseResponse(listResponseSchema, output)
  if (!parsed) return null
  const sandboxes = parsed.sandboxes

  if (sandboxes.length === 0) {
    return (
      <div className="cr-fam-card cr-fam-empty">
        <EmptyState title="no sandboxes" description="no live sandboxes for this worker." />
      </div>
    )
  }

  return (
    <div className="cr-fam-card cr-fam-scroll-x">
      <table className="cr-fam-table">
        <thead>
          <tr>
            <th>sandbox</th>
            <th>name</th>
            <th>image</th>
            <th>age</th>
            <th>exec</th>
            <th>state</th>
          </tr>
        </thead>
        <tbody>
          {sandboxes.map((s) => (
            <tr key={s.sandbox_id}>
              <td>
                <SandboxIdChip sandboxId={s.sandbox_id} jump={!s.stopped} />
              </td>
              <td className="faint">{s.name ?? '—'}</td>
              <td className="faint">{s.image}</td>
              <td className="faint num">{formatAgeSecs(s.age_secs)}</td>
              <td>
                <StatusDot tone={s.exec_in_progress ? 'accent' : 'ink'} pulse={s.exec_in_progress} />
              </td>
              <td>{s.stopped ? <span className="cr-fam-warn">stopped</span> : <span className="faint">live</span>}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
