import { Check, Copy, GitMerge, X } from 'lucide-react'
import { useCallback, useState } from 'react'
import { StatusDot } from '@/components/ui/StatusDot'
import { cn } from '@/lib/utils'
import {
  integrationLabel,
  lifecycleTone,
  lifecycleToneClass,
  shortWorktreeId,
  type WorktreeInfo,
  worktreeIndicators,
} from '@/lib/worktrees'

/**
 * Detail side panel for one selected worktree. Renders only fields the
 * worker exposes: identity, paths, base, timestamps, and the git status
 * block when the list call computed one.
 */

interface WorktreeDetailPanelProps {
  worktree: WorktreeInfo
  onClose: () => void
}

function Row({
  label,
  children,
}: {
  label: string
  children: React.ReactNode
}) {
  return (
    <div className="flex flex-col gap-0.5 px-3.5 py-2">
      <span className="font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint">
        {label}
      </span>
      <span className="min-w-0 break-all font-mono text-[12px] text-ink">
        {children}
      </span>
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
      className="flex w-[300px] shrink-0 flex-col overflow-y-auto border-l border-rule bg-bg"
    >
      <header className="flex items-center justify-between gap-2 border-b border-rule bg-panel px-3.5 py-2.5">
        <span className="min-w-0 truncate font-mono text-[12px] font-semibold lowercase text-ink">
          {worktree.branch}
        </span>
        <button
          type="button"
          onClick={onClose}
          aria-label="close details"
          className="shrink-0 text-ink-ghost transition-colors hover:text-ink"
        >
          <X size={14} aria-hidden />
        </button>
      </header>

      <div className="divide-y divide-rule-2">
        <Row label="worktree">
          {shortWorktreeId(worktree.worktree_id)}
          <span
            className={cn(
              'ml-2 inline-flex items-center gap-1 text-[11px] lowercase',
              lifecycleToneClass[tone],
            )}
          >
            <StatusDot tone={tone} pulse={worktree.lifecycle === 'landing'} />
            {worktree.lifecycle}
          </span>
        </Row>
        <Row label="path">
          <span className="flex items-start gap-1.5">
            <span className="min-w-0 flex-1 break-all">{worktree.path}</span>
            <button
              type="button"
              onClick={copyPath}
              aria-label="copy worktree path"
              title={copied ? 'copied' : 'copy path'}
              className="mt-0.5 shrink-0 text-ink-ghost transition-colors hover:text-ink"
            >
              {copied ? (
                <Check size={12} className="text-accent" aria-hidden />
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
              <span
                className="ml-2 text-ink-ghost tabular-nums"
                title={worktree.base_sha}
              >
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
            <span className="tabular-nums">{worktree.dev_port}</span>
            <span className="ml-2 text-[11px] lowercase text-ink-ghost">
              advisory, derived from the id
            </span>
          </Row>
        ) : null}
        {status?.integrated ? (
          <Row label="integrated">
            <span className="inline-flex items-center gap-1.5 text-ink">
              <GitMerge size={12} className="text-ink-faint" aria-hidden />
              {integrationLabel(status)}
            </span>
          </Row>
        ) : null}
        {created ? <Row label="created">{created}</Row> : null}
        {updated ? <Row label="updated">{updated}</Row> : null}

        {status ? (
          <div className="flex flex-col gap-1 px-3.5 py-2">
            <span className="font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint">
              status
            </span>
            <dl className="grid grid-cols-2 gap-x-3 gap-y-0.5 font-mono text-[11px] text-ink">
              <StatusEntry
                label="clean"
                value={status.clean ? 'yes' : 'no'}
                tint={dirty ? 'text-warn' : undefined}
              />
              <StatusEntry label="ahead" value={String(ahead)} />
              <StatusEntry label="behind" value={String(status.behind)} />
              <StatusEntry label="staged" value={String(status.staged)} />
              <StatusEntry label="unstaged" value={String(status.unstaged)} />
              <StatusEntry label="untracked" value={String(status.untracked)} />
              <StatusEntry
                label="conflicted"
                value={String(status.conflicted)}
                tint={status.conflicted > 0 ? 'text-alert' : undefined}
              />
              <StatusEntry label="unpushed" value={String(status.unpushed)} />
              <StatusEntry
                label="rebase"
                value={status.in_rebase ? 'in progress' : 'none'}
                tint={status.in_rebase ? 'text-alert' : undefined}
              />
            </dl>
            {status.diffstat ? (
              <span className="mt-1 font-mono text-[11px] text-ink-faint">
                {status.diffstat}
              </span>
            ) : null}
            {status.head_sha ? (
              <span
                className="font-mono text-[11px] text-ink-ghost tabular-nums"
                title={status.head_sha}
              >
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
      <dt className="text-ink-faint lowercase">{label}</dt>
      <dd className={cn('tabular-nums', tint)}>{value}</dd>
    </>
  )
}
