import { StatusDot } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'
import { useCallback, useState } from 'react'
import { Check, Copy, GitMerge, X } from './icons'
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
 * Detail side panel for one selected worktree. Renders only fields the
 * worker exposes: identity, paths, base, timestamps, and the git status
 * block when the list call computed one. Ported from the console page;
 * Tailwind utilities became scoped `wt-*` classes (see styles.css).
 */

interface WorktreeDetailPanelProps {
  worktree: WorktreeInfo
  onClose: () => void
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="wt-row">
      <span className="wt-row-label">{label}</span>
      <span className="wt-row-value">{children}</span>
    </div>
  )
}

function formatMs(ms: number | undefined): string | null {
  if (ms == null || ms <= 0) return null
  return new Date(ms).toLocaleString()
}

export function WorktreeDetailPanel({
  worktree,
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
      className="wt-detail"
    >
      <header className="wt-detail-head">
        <span className="wt-detail-title">{worktree.branch}</span>
        <button
          type="button"
          onClick={onClose}
          aria-label="close details"
          className="wt-detail-close"
        >
          <X size={14} aria-hidden />
        </button>
      </header>

      <div className="wt-detail-body">
        <Row label="worktree">
          {shortWorktreeId(worktree.worktree_id)}
          <span className={cn('wt-inline-tone', lifecycleToneClass[tone])}>
            <StatusDot tone={tone} pulse={worktree.lifecycle === 'landing'} />
            {worktree.lifecycle}
          </span>
        </Row>
        <Row label="path">
          <span className="wt-path">
            <span className="wt-path-val">{worktree.path}</span>
            <button
              type="button"
              onClick={copyPath}
              aria-label="copy worktree path"
              title={copied ? 'copied' : 'copy path'}
              className="wt-copy"
            >
              {copied ? (
                <Check size={12} className="wt-accent" aria-hidden />
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
              <span className="wt-sha" title={worktree.base_sha}>
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
            <span className="wt-tabular">{worktree.dev_port}</span>
            <span className="wt-note">advisory, derived from the id</span>
          </Row>
        ) : null}
        {status?.integrated ? (
          <Row label="integrated">
            <span className="wt-integrated">
              <GitMerge size={12} className="wt-icon-faint" aria-hidden />
              {integrationLabel(status)}
            </span>
          </Row>
        ) : null}
        {created ? <Row label="created">{created}</Row> : null}
        {updated ? <Row label="updated">{updated}</Row> : null}

        {status ? (
          <div className="wt-status">
            <span className="wt-row-label">status</span>
            <dl className="wt-status-grid">
              <StatusEntry
                label="clean"
                value={status.clean ? 'yes' : 'no'}
                tint={dirty ? 'wt-tone-warn' : undefined}
              />
              <StatusEntry label="ahead" value={String(ahead)} />
              <StatusEntry label="behind" value={String(status.behind)} />
              <StatusEntry label="staged" value={String(status.staged)} />
              <StatusEntry label="unstaged" value={String(status.unstaged)} />
              <StatusEntry label="untracked" value={String(status.untracked)} />
              <StatusEntry
                label="conflicted"
                value={String(status.conflicted)}
                tint={status.conflicted > 0 ? 'wt-tone-alert' : undefined}
              />
              <StatusEntry label="unpushed" value={String(status.unpushed)} />
              <StatusEntry
                label="rebase"
                value={status.in_rebase ? 'in progress' : 'none'}
                tint={status.in_rebase ? 'wt-tone-alert' : undefined}
              />
            </dl>
            {status.diffstat ? (
              <span className="wt-diffstat">{status.diffstat}</span>
            ) : null}
            {status.head_sha ? (
              <span className="wt-head-sha" title={status.head_sha}>
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
      <dt className="wt-status-dt">{label}</dt>
      <dd className={cn('wt-status-dd', tint)}>{value}</dd>
    </>
  )
}
