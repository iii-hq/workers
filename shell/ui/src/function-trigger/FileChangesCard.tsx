import { ChevronDown, Eye, Files } from 'lucide-react'
import { useId, useState } from 'react'
import type { FileChangeRow, FileChangesSummary } from './file-changes'

const VISIBLE_FILE_LIMIT = 5

function runningTitle(count: number): string {
  return `Changing ${count} ${count === 1 ? 'file' : 'files'}`
}

function settledTitle(summary: FileChangesSummary): string {
  const count = summary.rows.length
  const verb = summary.action === 'created' ? 'Created' : summary.action === 'updated' ? 'Updated' : 'Deleted'
  return `${verb} ${count} ${count === 1 ? 'file' : 'files'}`
}

function totals(rows: readonly FileChangeRow[]) {
  return rows.reduce(
    (total, row) => ({
      additions: total.additions + (row.additions ?? 0),
      deletions: total.deletions + (row.deletions ?? 0),
      failures: total.failures + (row.status === 'failed' ? 1 : 0),
    }),
    { additions: 0, deletions: 0, failures: 0 },
  )
}

function AnimatedMetric({
  className,
  value,
  children,
}: {
  className: string
  value: number
  children: React.ReactNode
}) {
  const [initialValue] = useState(value)
  const visible = value > 0

  return (
    <span className={`shui-file-changes-metric ${className}`} data-visible={visible} aria-hidden={!visible}>
      <span key={value} className={`shui-file-changes-metric-value${initialValue !== value ? ' is-updating' : ''}`}>
        {visible ? children : null}
      </span>
    </span>
  )
}

function FileChangeItem({
  row,
  onOpenDiff,
  onOpenFile,
}: {
  row: FileChangeRow
  onOpenDiff?: (row: FileChangeRow) => void
  onOpenFile?: (row: FileChangeRow) => void
}) {
  return (
    <li className="shui-file-change-row">
      {onOpenDiff && row.changeId ? (
        <button
          type="button"
          className="shui-file-change-path is-action"
          onClick={() => onOpenDiff(row)}
          title={`Open exact diff for ${row.path}`}
        >
          {row.path}
        </button>
      ) : (
        <span className="shui-file-change-path">{row.path}</span>
      )}
      <div className="shui-file-change-actions">
        <div className="shui-file-change-stats">
          {row.additions != null ? <span className="is-added">+{row.additions}</span> : null}
          {row.deletions != null ? <span className="is-deleted">−{row.deletions}</span> : null}
          {row.additions == null && row.deletions == null ? (
            <span className={`is-${row.status}`}>{row.status}</span>
          ) : null}
        </div>
        {onOpenFile && row.absolutePath && (row.status === 'created' || row.status === 'updated') ? (
          <button
            type="button"
            className="shui-file-change-view"
            onClick={() => onOpenFile(row)}
            aria-label={`View ${row.path} in shell`}
            title={`View ${row.path} in shell`}
          >
            <Eye aria-hidden />
            <span>View file</span>
          </button>
        ) : null}
      </div>
    </li>
  )
}

function FileChangeList({
  rows,
  onOpenDiff,
  onOpenFile,
}: {
  rows: readonly FileChangeRow[]
  onOpenDiff?: (row: FileChangeRow) => void
  onOpenFile?: (row: FileChangeRow) => void
}) {
  const pathOccurrences = new Map<string, number>()
  return (
    <ul className="shui-file-changes-list">
      {rows.map((row) => {
        // `path` comes from the immutable request, whereas absolutePath and
        // changeId arrive only with the result. Keeping those result fields
        // out of the key preserves each row across running -> settled.
        const occurrence = pathOccurrences.get(row.path) ?? 0
        pathOccurrences.set(row.path, occurrence + 1)
        return (
          <FileChangeItem key={`${row.path}:${occurrence}`} row={row} onOpenDiff={onOpenDiff} onOpenFile={onOpenFile} />
        )
      })}
    </ul>
  )
}

export function FileChangesCard({
  summary,
  running,
  onOpenDiff,
  onOpenFile,
}: {
  summary: FileChangesSummary
  running: boolean
  onOpenDiff?: (row: FileChangeRow) => void
  onOpenFile?: (row: FileChangeRow) => void
}) {
  const [expanded, setExpanded] = useState(false)
  const overflowId = useId()
  const primaryRows = summary.rows.slice(0, VISIBLE_FILE_LIMIT)
  const overflowRows = summary.rows.slice(VISIBLE_FILE_LIMIT)
  const total = totals(summary.rows)

  return (
    <section
      className="shui-file-changes"
      aria-busy={running || undefined}
      data-state={running ? 'running' : 'settled'}
    >
      <header className="shui-file-changes-head">
        <Files aria-hidden className="shui-file-changes-icon" />
        <div className="shui-file-changes-title-group">
          <div
            className="shui-file-changes-title"
            role="status"
            aria-live="polite"
            aria-atomic="true"
            aria-label={running ? runningTitle(summary.rows.length) : settledTitle(summary)}
          >
            <span className="shui-file-changes-title-state is-running" data-active={running} aria-hidden={!running}>
              {runningTitle(summary.rows.length)}
            </span>
            <span className="shui-file-changes-title-state is-settled" data-active={!running} aria-hidden={running}>
              {settledTitle(summary)}
            </span>
          </div>
          <div className="shui-file-changes-totals">
            <AnimatedMetric className="is-added" value={total.additions}>
              +{total.additions}
            </AnimatedMetric>
            <AnimatedMetric className="is-deleted" value={total.deletions}>
              −{total.deletions}
            </AnimatedMetric>
            <AnimatedMetric className="is-failed" value={total.failures}>
              {total.failures} failed
            </AnimatedMetric>
            <span className="shui-file-changes-working" data-visible={running} aria-hidden={!running}>
              working…
            </span>
          </div>
        </div>
      </header>

      <FileChangeList rows={primaryRows} onOpenDiff={onOpenDiff} onOpenFile={onOpenFile} />

      {overflowRows.length > 0 ? (
        <>
          <div id={overflowId} className="shui-file-changes-overflow" data-open={expanded} aria-hidden={!expanded}>
            <div className="shui-file-changes-overflow-inner" inert={expanded ? undefined : true}>
              <FileChangeList rows={overflowRows} onOpenDiff={onOpenDiff} onOpenFile={onOpenFile} />
            </div>
          </div>
          <button
            type="button"
            className="shui-file-changes-more"
            aria-expanded={expanded}
            aria-controls={overflowId}
            data-open={expanded}
            onClick={() => setExpanded((value) => !value)}
          >
            <span className="shui-file-changes-more-label">
              <span data-active={!expanded} aria-hidden={expanded}>
                Show {overflowRows.length} more {overflowRows.length === 1 ? 'file' : 'files'}
              </span>
              <span data-active={expanded} aria-hidden={!expanded}>
                Show fewer files
              </span>
            </span>
            <ChevronDown aria-hidden />
          </button>
        </>
      ) : null}
    </section>
  )
}
