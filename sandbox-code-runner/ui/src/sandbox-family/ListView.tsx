/**
 * `sandbox::list` — the fleet table. `EmptyState` and `StatusDot` come
 * from the console; each row's id is the interactive chip so a listed
 * sandbox is one keypress from its fleet-page row.
 */

import {
  EmptyState,
  StatusDot,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
} from '@iii-dev/console-ui'
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
        <EmptyState compact title="no sandboxes" description="no live sandboxes for this worker." />
      </div>
    )
  }

  return (
    <TableViewport className="cr-fam-card">
      <Table density="compact" inset>
        <TableHeader>
          <TableRow>
            <TableHead>sandbox</TableHead>
            <TableHead>name</TableHead>
            <TableHead>image</TableHead>
            <TableHead>age</TableHead>
            <TableHead>exec</TableHead>
            <TableHead>state</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {sandboxes.map((s) => (
            <TableRow key={s.sandbox_id}>
              <TableCell>
                <SandboxIdChip sandboxId={s.sandbox_id} jump={!s.stopped} />
              </TableCell>
              <TableCell className="faint">{s.name ?? '—'}</TableCell>
              <TableCell className="faint">{s.image}</TableCell>
              <TableCell className="faint num">{formatAgeSecs(s.age_secs)}</TableCell>
              <TableCell>
                <StatusDot tone={s.exec_in_progress ? 'accent' : 'ink'} pulse={s.exec_in_progress} />
              </TableCell>
              <TableCell>
                {s.stopped ? <span className="cr-fam-warn">stopped</span> : <span className="faint">live</span>}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </TableViewport>
  )
}
