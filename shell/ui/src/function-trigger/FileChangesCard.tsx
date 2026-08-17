import { ChevronDown, Eye, Files } from 'lucide-react'
import { useState } from 'react'
import type { FileChangeRow, FileChangesSummary } from './file-changes'

function titleFor(summary: FileChangesSummary, running: boolean): string {
  const count = summary.rows.length
  if (running) return `Changing ${count} ${count === 1 ? 'file' : 'files'}`
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
  const visible = expanded ? summary.rows : summary.rows.slice(0, 5)
  const hidden = summary.rows.length - visible.length
  const total = totals(summary.rows)

  return (
    <section className="shui-file-changes" aria-busy={running || undefined}>
      <header className="shui-file-changes-head">
        <Files aria-hidden className="shui-file-changes-icon" />
        <div className="shui-file-changes-title-group">
          <div className="shui-file-changes-title">{titleFor(summary, running)}</div>
          <div className="shui-file-changes-totals">
            {total.additions > 0 ? <span className="is-added">+{total.additions}</span> : null}
            {total.deletions > 0 ? <span className="is-deleted">−{total.deletions}</span> : null}
            {total.failures > 0 ? <span className="is-failed">{total.failures} failed</span> : null}
            {running ? <span className="is-running">working…</span> : null}
          </div>
        </div>
      </header>

      <ul className="shui-file-changes-list">
        {visible.map((row, index) => (
          <li className="shui-file-change-row" key={`${row.path}:${index}`}>
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
        ))}
      </ul>

      {hidden > 0 || expanded ? (
        <button
          type="button"
          className="shui-file-changes-more"
          aria-expanded={expanded}
          onClick={() => setExpanded((value) => !value)}
        >
          <span>{expanded ? 'Show fewer files' : `Show ${hidden} more files`}</span>
          <ChevronDown aria-hidden className={expanded ? 'is-expanded' : undefined} />
        </button>
      ) : null}
    </section>
  )
}
