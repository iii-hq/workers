/**
 * What the main pane offers when nothing is open.
 *
 * Each card drives a surface this page already has: the review feed the navbar
 * scopes, the terminal workspace, the file tree in the sidebar, and — when the
 * browser worker is on the bus — its page beside this one. An empty editor is a
 * dead end; this is the same set of doors, one click each.
 */

import type { Host } from '@iii-dev/console-ui'
import { FileText, GitCompare, Globe, SquareTerminal } from 'lucide-react'
import type { ComponentType } from 'react'

interface LauncherAction {
  id: string
  label: string
  hint: string
  Icon: ComponentType<{ 'aria-hidden'?: boolean; className?: string }>
  onSelect: () => void
}

export function ShellLauncher({
  host,
  browserAvailable,
  onOpenChanges,
  onOpenTerminal,
  onOpenFiles,
}: {
  host: Host
  browserAvailable: boolean
  onOpenChanges: () => void
  onOpenTerminal: () => void
  onOpenFiles: () => void
}) {
  const actions: LauncherAction[] = [
    {
      id: 'changes',
      label: 'Changes',
      hint: 'Review what the last turn touched',
      Icon: GitCompare,
      onSelect: onOpenChanges,
    },
    {
      id: 'terminal',
      label: 'Terminal',
      hint: 'Open a shell in this workspace',
      Icon: SquareTerminal,
      onSelect: onOpenTerminal,
    },
    {
      id: 'file',
      label: 'File',
      hint: 'Browse the workspace tree',
      Icon: FileText,
      onSelect: onOpenFiles,
    },
  ]

  // The browser is another worker's page: offer it only when that worker is
  // connected and this console can place pages beside the current one.
  if (browserAvailable && host.panels) {
    actions.splice(1, 0, {
      id: 'browser',
      label: 'Browser',
      hint: 'Open the browser worker page',
      Icon: Globe,
      onSelect: () => host.panels?.open({ pageId: 'browser', context: {} }),
    })
  }

  return (
    <div className="shui-main-empty">
      <div className="shui-launcher">
        {actions.map(({ id, label, hint, Icon, onSelect }) => (
          <button key={id} type="button" className="shui-launcher-card" title={hint} onClick={onSelect}>
            <Icon aria-hidden className="shui-launcher-icon" />
            <span>{label}</span>
          </button>
        ))}
      </div>
    </div>
  )
}
