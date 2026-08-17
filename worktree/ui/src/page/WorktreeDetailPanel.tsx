import { StatusDot } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'
import { useCallback, useState } from 'react'
import { Check, ChevronLeft, Copy, GitMerge, X } from './icons'
import {
  cn,
  integrationLabel,
  lifecycleTone,
  lifecycleToneClass,
  shortWorktreeId,
  type WorktreeInfo,
  worktreeIndicators,
} from './worktree-data'

/**
 * Detail column for one selected worktree. Renders only fields the worker
 * exposes: identity, paths, base, timestamps, and the git status block
 * when the list call computed one. Wide layouts show it as a fixed-width
 * sidebar column beside the graph with the standard ✕; the narrow
 * drill-in flow fills the pane with it and offers ← back instead (both
 * deselect — the graph owns the selection). Ported from the console page;
 * Tailwind utilities became scoped `wt-ui-*` classes (see styles.css).
 */

interface WorktreeDetailPanelProps {
  worktree: WorktreeInfo
  /** Drill-in mode: the panel fills the pane; ← back replaces the ✕. */
  narrow?: boolean
  onClose: () => void
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="wt-ui-row">
      <span className="wt-ui-row-label">{label}</span>
      <span className="wt-ui-row-value">{children}</span>
    </div>
  )
}

function formatMs(ms: number | undefined): string | null {
  if (ms == null || ms <= 0) return null
  return new Date(ms).toLocaleString()
}

export function WorktreeDetailPanel({
  worktree,
  narrow = false,
  onClose,
}: WorktreeDetailPanelProps) {
  const [copied, setCopied] = useState(false)
  const copyPath = useCallback(() => {
    if (typeof navigator === 'undefined' || !navigator.clipboard) return
    navigator.clipboard.writeText(worktree.path).then(
      () => {
        setCopied(true)
        window.setTimeout(() => setCopied(false), 1200)
      },
      () => {
        // Clipboard write can reject (permissions, insecure context); the
        // copy affordance simply no-ops rather than surfacing an error.
      },
    )
  }, [worktree.path])

  const tone = lifecycleTone(worktree.lifecycle)
  const { dirty, ahead } = worktreeIndicators(worktree.status)
  const status = worktree.status ?? null
  const created = formatMs(worktree.created_at)
  const updated = formatMs(worktree.updated_at)

  return (
    <aside
      aria-label={`worktree ${worktree.worktree_id} details`}
      className="wt-ui-detail"
    >
      <header className="wt-ui-detail-head">
        {narrow ? (
          <button
            type="button"
            onClick={onClose}
            aria-label="back to the graph"
            title="back to the graph"
            className="wt-ui-back"
          >
            <ChevronLeft size={16} aria-hidden />
          </button>
        ) : null}
        <span className="wt-ui-detail-title" title={worktree.branch}>
          {worktree.branch}
        </span>
        {!narrow ? (
          <button
            type="button"
            onClick={onClose}
            aria-label="close details"
            title="close details"
            className="wt-ui-detail-close"
          >
            <X size={14} aria-hidden />
          </button>
        ) : null}
      </header>

      <div className="wt-ui-detail-body">
        <Row label="worktree">
          {shortWorktreeId(worktree.worktree_id)}
          <span className={cn('wt-ui-inline-tone', lifecycleToneClass[tone])}>
            <StatusDot tone={tone} pulse={worktree.lifecycle === 'landing'} />
            {worktree.lifecycle}
          </span>
        </Row>
        <Row label="path">
          <span className="wt-ui-path">
            <span className="wt-ui-path-val">{worktree.path}</span>
            <button
              type="button"
              onClick={copyPath}
              aria-label="copy worktree path"
              title={copied ? 'copied' : 'copy path'}
              className="wt-ui-copy"
            >
              {copied ? (
                <Check size={12} className="wt-ui-accent" aria-hidden />
              ) : (
                <Copy size={12} aria-hidden />
              )}
            </button>
          </span>
        </Row>
        <Row label="repository">{worktree.repo_path}</Row>
        {worktree.base_ref ? (
          <Row label="base">
            {worktree.base_ref}
            {worktree.base_sha ? (
              <span className="wt-ui-sha" title={worktree.base_sha}>
                {worktree.base_sha.slice(0, 12)}
              </span>
            ) : null}
          </Row>
        ) : null}
        {worktree.session_id ? (
          <Row label="claimed by">{worktree.session_id}</Row>
        ) : null}
        {worktree.dev_port != null ? (
          <Row label="dev port">
            <span className="wt-ui-tabular">{worktree.dev_port}</span>
            <span className="wt-ui-note">advisory, derived from the id</span>
          </Row>
        ) : null}
        {status?.integrated ? (
          <Row label="integrated">
            <span className="wt-ui-integrated">
              <GitMerge size={12} className="wt-ui-icon-faint" aria-hidden />
              {integrationLabel(status)}
            </span>
          </Row>
        ) : null}
        {created ? <Row label="created">{created}</Row> : null}
        {updated ? <Row label="updated">{updated}</Row> : null}

        {status ? (
          <div className="wt-ui-status">
            <span className="wt-ui-row-label">status</span>
            <dl className="wt-ui-status-grid">
              <StatusEntry
                label="clean"
                value={status.clean ? 'yes' : 'no'}
                tint={dirty ? 'wt-ui-tone-warn' : undefined}
              />
              <StatusEntry label="ahead" value={String(ahead)} />
              <StatusEntry label="behind" value={String(status.behind)} />
              <StatusEntry label="staged" value={String(status.staged)} />
              <StatusEntry label="unstaged" value={String(status.unstaged)} />
              <StatusEntry label="untracked" value={String(status.untracked)} />
              <StatusEntry
                label="conflicted"
                value={String(status.conflicted)}
                tint={status.conflicted > 0 ? 'wt-ui-tone-alert' : undefined}
              />
              <StatusEntry label="unpushed" value={String(status.unpushed)} />
              <StatusEntry
                label="rebase"
                value={status.in_rebase ? 'in progress' : 'none'}
                tint={status.in_rebase ? 'wt-ui-tone-alert' : undefined}
              />
            </dl>
            {status.diffstat ? (
              <span className="wt-ui-diffstat">{status.diffstat}</span>
            ) : null}
            {status.head_sha ? (
              <span className="wt-ui-head-sha" title={status.head_sha}>
                head {status.head_sha.slice(0, 12)}
              </span>
            ) : null}
          </div>
        ) : null}
      </div>
    </aside>
  )
}

function StatusEntry({
  label,
  value,
  tint,
}: {
  label: string
  value: string
  tint?: string
}) {
  return (
    <>
      <dt className="wt-ui-status-dt">{label}</dt>
      <dd className={cn('wt-ui-status-dd', tint)}>{value}</dd>
    </>
  )
}
