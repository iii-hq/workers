import { Folder, X } from 'lucide-react'
import type { ReactNode } from 'react'
import { useEffect, useState } from 'react'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import { getShellHostRoots } from '@/lib/backend/shell-roots'

const SHELL_CONFIG_HASH = '#/configuration/workers/shell'

interface FilesystemAccessDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** The conversation's session workspace — always allowed, no revoke. */
  workingDir?: string | null
  /** Session-scoped granted dirs (plain path strings). */
  grants: string[]
  /** False when the harness's filesystem-grants functions aren't registered. */
  grantsSupported: boolean
  onRevoke: (root: string) => void | Promise<void>
  /** Called on open so the grants list is fresh. */
  onRefreshGrants: () => void | Promise<void>
  /** Shows a hint that revoking mid-turn may make the agent ask again. */
  sessionBusy?: boolean
}

/**
 * Folder-access management surface (§5): three stacked groups — the
 * session's workspace (always allowed, no revoke), this session's one-shot
 * grants (revocable), and shell's permanent `fs.host_roots` (read-only,
 * deep-links to the existing editor). Opened from the prompt-card footer
 * link and the ChatView working-dir footer row.
 */
export function FilesystemAccessDialog({
  open,
  onOpenChange,
  workingDir,
  grants,
  grantsSupported,
  onRevoke,
  onRefreshGrants,
  sessionBusy,
}: FilesystemAccessDialogProps) {
  const [shellRoots, setShellRoots] = useState<string[] | null>(null)

  useEffect(() => {
    if (!open) return
    void onRefreshGrants()
    void getShellHostRoots().then(setShellRoots)
  }, [open, onRefreshGrants])

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogTitle className="text-[14px]">filesystem access</DialogTitle>
        <DialogDescription className="mt-1">
          folders the agent can read and write in this conversation.
        </DialogDescription>

        <div className="mt-5 flex flex-col gap-5">
          <FolderGroup title="workspace">
            {workingDir ? (
              <div className="flex items-center gap-2 px-2 py-1.5">
                <Folder
                  size={13}
                  className="shrink-0 text-ink-faint"
                  aria-hidden
                />
                <span
                  dir="rtl"
                  className="truncate text-left font-mono text-[12px] text-ink"
                  title={workingDir}
                >
                  {workingDir}
                </span>
                <span className="ml-auto shrink-0 font-mono text-[10px] lowercase text-ink-ghost">
                  always allowed — change it with the directory picker
                </span>
              </div>
            ) : (
              <div className="px-2 py-1.5 font-mono text-[11px] lowercase text-ink-ghost">
                no working directory set.
              </div>
            )}
          </FolderGroup>

          {grantsSupported ? (
            <FolderGroup title="allowed this session">
              {sessionBusy ? (
                <p className="px-2 pb-1.5 font-mono text-[11px] text-ink-faint">
                  the agent is running — revoking a folder it's using will make
                  it ask again.
                </p>
              ) : null}
              {grants.length === 0 ? (
                <div className="px-2 py-1.5 font-mono text-[11px] lowercase text-ink-ghost">
                  nothing granted beyond the workspace.
                </div>
              ) : (
                <div className="flex flex-col">
                  {grants.map((dir) => (
                    <div
                      key={dir}
                      className="group flex items-center gap-2 px-2 py-1.5 hover:bg-panel"
                    >
                      <Folder
                        size={13}
                        className="shrink-0 text-ink-faint"
                        aria-hidden
                      />
                      <span
                        dir="rtl"
                        className="min-w-0 flex-1 truncate text-left font-mono text-[12px] text-ink"
                        title={dir}
                      >
                        {dir}
                      </span>
                      <button
                        type="button"
                        aria-label={`revoke access to ${dir}`}
                        title="revoke this session's access"
                        onClick={() => void onRevoke(dir)}
                        className="shrink-0 p-1 text-ink-ghost opacity-0 transition-opacity hover:text-ink group-hover:opacity-100"
                      >
                        <X size={12} aria-hidden />
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </FolderGroup>
          ) : null}

          <FolderGroup title="always allowed (all sessions)">
            {shellRoots === null ? (
              <div className="px-2 py-1.5 font-mono text-[11px] lowercase text-ink-ghost">
                loading…
              </div>
            ) : shellRoots.length === 0 ? (
              <div className="px-2 py-1.5 font-mono text-[11px] lowercase text-ink-ghost">
                no permanent roots configured.
              </div>
            ) : (
              <div className="flex flex-col">
                {shellRoots.map((dir) => (
                  <div
                    key={dir}
                    className="flex items-center gap-2 px-2 py-1.5"
                  >
                    <Folder
                      size={13}
                      className="shrink-0 text-ink-faint"
                      aria-hidden
                    />
                    <span
                      dir="rtl"
                      className="min-w-0 flex-1 truncate text-left font-mono text-[12px] text-ink"
                      title={dir}
                    >
                      {dir}
                    </span>
                  </div>
                ))}
              </div>
            )}
            <a
              href={SHELL_CONFIG_HASH}
              className="mt-1 inline-block px-2 font-mono text-[11px] lowercase text-accent hover:underline"
            >
              edit in shell configuration →
            </a>
          </FolderGroup>
        </div>
      </DialogContent>
    </Dialog>
  )
}

function FolderGroup({
  title,
  children,
}: {
  title: string
  children: ReactNode
}) {
  return (
    <section>
      <h3 className="mb-1 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
        {title}
      </h3>
      <div className="border border-rule-2 bg-bg">{children}</div>
    </section>
  )
}
