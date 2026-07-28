import { EmptyState, StatusPanel } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'
import { AlertCircle, type IconProps } from './icons'

interface PanelShellProps {
  loading: boolean
  error: string | null
  empty: boolean
  emptyIcon: (props: IconProps) => ReactNode
  emptyTitle: string
  emptyDescription: string
  children: ReactNode
}

/**
 * Shared error / first-load / empty scaffolding for the github panels. Worker
 * errors carry gh's own stderr (auth failures, 404s), so the alert detail is
 * already the actionable message.
 */
export function PanelShell({
  loading,
  error,
  empty,
  emptyIcon,
  emptyTitle,
  emptyDescription,
  children,
}: PanelShellProps) {
  if (error) {
    return (
      <StatusPanel
        variant="alert"
        icon={<AlertCircle size={18} />}
        headline="github call failed"
        detail={error}
      />
    )
  }
  if (loading && empty) {
    return <p className="gh-msg gh-pulse">loading…</p>
  }
  if (empty) {
    return (
      <EmptyState
        icon={emptyIcon}
        title={emptyTitle}
        description={emptyDescription}
      />
    )
  }
  return <>{children}</>
}
